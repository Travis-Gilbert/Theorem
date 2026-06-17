use std::collections::{BTreeMap, BTreeSet};

use rustyred_thg_core::{EdgeRecord, GraphStore, NodeQuery, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::embedding::{write_vector_payload, HippoTextEmbedder, VectorPayload};
use crate::schema::{
    hash_vector, stable_digest, HippoEdge, HippoError, HippoResult, LABEL_PAGE, LABEL_PHRASE,
    NODE_SPECIFICITY_PROPERTY, SEMANTIC_VECTOR_PROPERTY,
};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct IndexStats {
    pub passage_id: String,
    pub phrases_upserted: usize,
    pub contains_edges: usize,
    pub relates_edges: usize,
    pub synonym_edges: usize,
    pub embedded_nodes: usize,
    pub embedding_model: Option<String>,
}

pub fn index_passage<S: GraphStore>(store: &mut S, passage_id: &str) -> HippoResult<IndexStats> {
    let prepared = prepare_index(store, passage_id)?;
    let dim = vector_dim(&prepared.passage).unwrap_or(2560);
    let phrase_vectors = prepared
        .phrases
        .iter()
        .map(|phrase| {
            (
                phrase.clone(),
                VectorPayload::hash(hash_vector(phrase, dim)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    write_index(
        store,
        prepared,
        None,
        phrase_vectors,
        SEMANTIC_VECTOR_PROPERTY,
    )
}

pub async fn index_passage_with_embedder<S: GraphStore, E: HippoTextEmbedder>(
    store: &mut S,
    passage_id: &str,
    embedder: &E,
) -> HippoResult<IndexStats> {
    let prepared = prepare_index(store, passage_id)?;
    if prepared.text.trim().is_empty() {
        return Ok(IndexStats {
            passage_id: passage_id.to_string(),
            ..IndexStats::default()
        });
    }

    let inputs = std::iter::once(prepared.text.clone())
        .chain(prepared.phrases.iter().cloned())
        .collect::<Vec<_>>();
    let vectors = embedder.embed(&inputs).await?;
    if vectors.len() != inputs.len() {
        return Err(HippoError::new(
            "embedding_response",
            format!(
                "embedder {} returned {} vectors for {} HippoRAG inputs",
                embedder.model_id(),
                vectors.len(),
                inputs.len()
            ),
        ));
    }

    let mut vectors = vectors.into_iter();
    let page_vector = vectors
        .next()
        .map(|vector| VectorPayload::embedded(embedder, vector))
        .transpose()?;
    let phrase_vectors = prepared
        .phrases
        .iter()
        .cloned()
        .zip(vectors)
        .map(|(phrase, vector)| Ok((phrase, VectorPayload::embedded(embedder, vector)?)))
        .collect::<HippoResult<BTreeMap<_, _>>>()?;
    write_index(
        store,
        prepared,
        page_vector,
        phrase_vectors,
        embedder.property(),
    )
}

struct PreparedIndex {
    passage: NodeRecord,
    text: String,
    phrases: Vec<String>,
    total_pages: usize,
    existing_df: BTreeMap<String, usize>,
}

fn prepare_index<S: GraphStore>(store: &S, passage_id: &str) -> HippoResult<PreparedIndex> {
    let passage = store.get_node(passage_id).cloned().ok_or_else(|| {
        HippoError::new(
            "missing_passage",
            format!("passage node {passage_id:?} does not exist"),
        )
    })?;
    if !passage.labels.iter().any(|label| label == LABEL_PAGE) {
        return Err(HippoError::new(
            "not_a_passage",
            format!("node {passage_id:?} is not labeled {LABEL_PAGE}"),
        ));
    }

    let text = passage_text(&passage);
    let phrases = extract_phrases(&text);
    let total_pages = store
        .query_nodes(NodeQuery::label(LABEL_PAGE).with_limit(100_000))
        .len()
        .max(1);
    let existing_df = phrase_document_frequencies(store);
    Ok(PreparedIndex {
        passage,
        text,
        phrases,
        total_pages,
        existing_df,
    })
}

fn write_index<S: GraphStore>(
    store: &mut S,
    prepared: PreparedIndex,
    page_vector: Option<VectorPayload>,
    phrase_vectors: BTreeMap<String, VectorPayload>,
    vector_property: &str,
) -> HippoResult<IndexStats> {
    let mut stats = IndexStats {
        passage_id: prepared.passage.id.clone(),
        ..IndexStats::default()
    };
    if prepared.text.trim().is_empty() {
        return Ok(stats);
    }

    if let Some(payload) = page_vector {
        if payload.model_id.is_some() {
            stats.embedded_nodes += 1;
            stats.embedding_model = payload.model_id.clone();
        }
        let mut passage = prepared.passage.clone();
        write_vector_payload(&mut passage.properties, vector_property, payload);
        store.upsert_node(passage)?;
    }

    for phrase in &prepared.phrases {
        let phrase_id = phrase_id(phrase);
        let df = prepared.existing_df.get(phrase).copied().unwrap_or(0) + 1;
        let specificity = ((prepared.total_pages as f32 + 1.0) / (df as f32 + 1.0)).ln() + 1.0;
        let mut node = NodeRecord::new(
            &phrase_id,
            [LABEL_PHRASE],
            json!({
                "text": phrase,
                NODE_SPECIFICITY_PROPERTY: specificity,
            }),
        );
        if let Some(payload) = phrase_vectors.get(phrase).cloned() {
            if payload.model_id.is_some() {
                stats.embedded_nodes += 1;
                stats.embedding_model = payload.model_id.clone();
            }
            write_vector_payload(&mut node.properties, vector_property, payload);
        }
        store.upsert_node(node)?;
        stats.phrases_upserted += 1;

        store.upsert_edge(EdgeRecord::new(
            edge_id(HippoEdge::Contains, &prepared.passage.id, &phrase_id),
            &prepared.passage.id,
            HippoEdge::Contains.as_str(),
            &phrase_id,
            json!({ "passage_id": prepared.passage.id.clone() }),
        ))?;
        stats.contains_edges += 1;
    }

    for window in prepared.phrases.windows(2) {
        let from = phrase_id(&window[0]);
        let to = phrase_id(&window[1]);
        if from == to {
            continue;
        }
        store.upsert_edge(EdgeRecord::new(
            edge_id(HippoEdge::Relates, &from, &to),
            &from,
            HippoEdge::Relates.as_str(),
            &to,
            json!({ "source": "deterministic_openie_window" }),
        ))?;
        stats.relates_edges += 1;
    }

    for (left, right) in synonym_pairs(&prepared.phrases) {
        let from = phrase_id(&left);
        let to = phrase_id(&right);
        store.upsert_edge(EdgeRecord::new(
            edge_id(HippoEdge::Synonym, &from, &to),
            &from,
            HippoEdge::Synonym.as_str(),
            &to,
            json!({ "source": "deterministic_suffix_normalizer" }),
        ))?;
        stats.synonym_edges += 1;
    }

    Ok(stats)
}

pub(crate) fn phrase_id(text: &str) -> String {
    format!("hippo:phrase:{}", stable_digest(&normalize_phrase(text)))
}

pub(crate) fn edge_id(edge: HippoEdge, from: &str, to: &str) -> String {
    format!(
        "hippo:edge:{}:{}:{}",
        edge.as_str().to_ascii_lowercase(),
        stable_digest(from),
        stable_digest(to)
    )
}

pub(crate) fn passage_text(node: &NodeRecord) -> String {
    ["text", "body", "content", "summary", "title", "url"]
        .iter()
        .filter_map(|key| node.properties.get(*key).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn vector_dim(node: &NodeRecord) -> Option<usize> {
    node.properties
        .get(SEMANTIC_VECTOR_PROPERTY)
        .and_then(Value::as_array)
        .map(|values| values.len())
        .filter(|dim| *dim > 0)
}

pub(crate) fn extract_phrases(text: &str) -> Vec<String> {
    let stop = stopwords();
    let mut seen = BTreeSet::new();
    text.to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() > 2)
        .filter(|token| !stop.contains(*token))
        .map(normalize_phrase)
        .filter(|phrase| seen.insert(phrase.clone()))
        .collect()
}

fn normalize_phrase(text: &str) -> String {
    text.trim().to_ascii_lowercase()
}

fn phrase_document_frequencies<S: GraphStore>(store: &S) -> BTreeMap<String, usize> {
    let mut df = BTreeMap::new();
    for phrase in store.query_nodes(NodeQuery::label(LABEL_PHRASE).with_limit(100_000)) {
        let Some(text) = phrase.properties.get("text").and_then(Value::as_str) else {
            continue;
        };
        let contains_count = store
            .neighbors(
                rustyred_thg_core::NeighborQuery::in_(&phrase.id)
                    .with_edge_type(HippoEdge::Contains.as_str()),
            )
            .len();
        df.insert(normalize_phrase(text), contains_count);
    }
    df
}

fn synonym_pairs(phrases: &[String]) -> Vec<(String, String)> {
    let mut by_stem: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for phrase in phrases {
        by_stem
            .entry(simple_stem(phrase))
            .or_default()
            .push(phrase.clone());
    }
    let mut pairs = Vec::new();
    for variants in by_stem.values() {
        for left in variants {
            for right in variants {
                if left != right {
                    pairs.push((left.clone(), right.clone()));
                }
            }
        }
    }
    pairs.sort();
    pairs.dedup();
    pairs
}

fn simple_stem(text: &str) -> String {
    text.strip_suffix("ies")
        .map(|base| format!("{base}y"))
        .or_else(|| text.strip_suffix('s').map(str::to_string))
        .unwrap_or_else(|| text.to_string())
}

fn stopwords() -> BTreeSet<&'static str> {
    [
        "and", "are", "for", "from", "into", "the", "this", "that", "with", "over", "under",
        "about", "what", "when", "where", "which", "while", "their", "there", "then", "than",
    ]
    .into_iter()
    .collect()
}
