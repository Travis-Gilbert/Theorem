use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use rustyred_membrane::{Candidate, SourceArm};
use rustyred_thg_core::{personalized_pagerank, GraphStore, NeighborQuery, NodeQuery, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::embedding::{HippoTextEmbedder, SEMANTIC_VECTOR_METRIC};
use crate::indexing::{extract_phrases, passage_text, phrase_id};
use crate::schema::{
    cosine, hash_vector, HippoEdge, HippoResult, CENTRALITY_PROPERTY, HUB_SCORE_PROPERTY,
    LABEL_HUB, LABEL_PAGE, LABEL_PHRASE, NODE_SPECIFICITY_PROPERTY, SEMANTIC_VECTOR_PROPERTY,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HippoQuery<'a> {
    pub text: &'a str,
    pub top_k: usize,
    pub include_hubs: bool,
}

impl<'a> HippoQuery<'a> {
    pub fn new(text: &'a str, top_k: usize) -> Self {
        Self {
            text,
            top_k,
            include_hubs: true,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RetrievalTrace {
    pub seeds: Vec<String>,
    pub warm_centrality_reads: usize,
    pub ran_query_ppr: bool,
    pub ran_global_ppr: bool,
}

pub fn retrieve<S: GraphStore>(store: &S, q: HippoQuery<'_>) -> Vec<Candidate> {
    retrieve_with_trace(store, q).0
}

pub fn retrieve_with_trace<S: GraphStore>(
    store: &S,
    q: HippoQuery<'_>,
) -> (Vec<Candidate>, RetrievalTrace) {
    let query = q.text.trim();
    if query.is_empty() {
        return (Vec::new(), RetrievalTrace::default());
    }
    let mut trace = RetrievalTrace::default();
    let top_k = q.top_k.max(1);
    let nodes = hippo_nodes(store);
    let dim = nodes
        .iter()
        .find_map(|node| vector(node).map(|vec| vec.len()))
        .unwrap_or(2560);
    let query_vec = hash_vector(query, dim);
    retrieve_with_query_vector_and_nodes(store, q, query, &query_vec, nodes, &mut trace, top_k)
}

pub async fn retrieve_with_embedder<S: GraphStore, E: HippoTextEmbedder + ?Sized>(
    store: &S,
    q: HippoQuery<'_>,
    embedder: &E,
) -> HippoResult<(Vec<Candidate>, RetrievalTrace)> {
    let query = q.text.trim();
    if query.is_empty() {
        return Ok((Vec::new(), RetrievalTrace::default()));
    }
    let inputs = [query.to_string()];
    let vectors = embedder.embed(&inputs).await?;
    if vectors.len() != 1 {
        return Err(crate::schema::HippoError::new(
            "embedding_response",
            format!(
                "embedder {} returned {} query vectors for HippoRAG retrieval",
                embedder.model_id(),
                vectors.len()
            ),
        ));
    }
    Ok(retrieve_with_query_vector(store, q, &vectors[0]))
}

pub fn retrieve_with_query_vector<S: GraphStore>(
    store: &S,
    q: HippoQuery<'_>,
    query_vec: &[f32],
) -> (Vec<Candidate>, RetrievalTrace) {
    let query = q.text.trim();
    if query.is_empty() || query_vec.is_empty() {
        return (Vec::new(), RetrievalTrace::default());
    }
    let mut trace = RetrievalTrace::default();
    let top_k = q.top_k.max(1);
    let nodes = hippo_nodes(store);
    retrieve_with_query_vector_and_nodes(store, q, query, query_vec, nodes, &mut trace, top_k)
}

fn retrieve_with_query_vector_and_nodes<S: GraphStore>(
    store: &S,
    q: HippoQuery<'_>,
    query: &str,
    query_vec: &[f32],
    nodes: Vec<NodeRecord>,
    trace: &mut RetrievalTrace,
    top_k: usize,
) -> (Vec<Candidate>, RetrievalTrace) {
    let query_phrases = extract_phrases(query);
    let mut seeds = BTreeMap::new();

    for phrase in &query_phrases {
        let id = phrase_id(phrase);
        if let Some(node) = store.get_node(&id) {
            let specificity = numeric(node, NODE_SPECIFICITY_PROPERTY).unwrap_or(1.0) as f64;
            seeds.insert(id, specificity.max(0.01));
        }
    }

    for node in nodes
        .iter()
        .filter(|node| has_label(node, LABEL_PHRASE) || has_label(node, LABEL_HUB))
    {
        let dense = vector(node)
            .map(|candidate_vec| cosine(query_vec, &candidate_vec))
            .unwrap_or_else(|| lexical_overlap(query, &node_text(node)));
        if dense <= 0.0 {
            continue;
        }
        let centrality = numeric(node, CENTRALITY_PROPERTY).unwrap_or(0.0).max(0.0) as f64;
        if has_label(node, LABEL_HUB) {
            trace.warm_centrality_reads += 1;
        }
        let seed_weight = if has_label(node, LABEL_PHRASE) {
            let specificity = numeric(node, NODE_SPECIFICITY_PROPERTY)
                .unwrap_or(1.0)
                .max(0.01);
            dense as f64 * specificity as f64
        } else {
            dense as f64 + centrality
        };
        if seed_weight > 0.0 {
            seeds
                .entry(node.id.clone())
                .and_modify(|weight| *weight = (*weight + seed_weight).max(*weight))
                .or_insert(seed_weight);
        }
    }

    if seeds.is_empty() {
        return (Vec::new(), trace.clone());
    }
    trace.seeds = seeds.keys().cloned().collect();
    let seed_total = seeds.values().sum::<f64>().max(1e-9);
    let seed_scores = seeds
        .into_iter()
        .map(|(id, score)| (id, score / seed_total))
        .collect::<HashMap<_, _>>();
    let adjacency = hippo_adjacency(store, &nodes);
    let ppr = personalized_pagerank(&adjacency, &seed_scores, 0.15, 1e-5, 100_000);
    trace.ran_query_ppr = true;
    trace.ran_global_ppr = false;

    let mut scored = nodes
        .into_iter()
        .filter(|node| {
            has_label(node, LABEL_PAGE) || (q.include_hubs && has_label(node, LABEL_HUB))
        })
        .filter_map(|node| {
            let ppr_mass = ppr.get(&node.id).copied().unwrap_or(0.0) as f32;
            let dense = vector(&node)
                .map(|candidate_vec| cosine(query_vec, &candidate_vec))
                .unwrap_or_else(|| lexical_overlap(query, &node_text(&node)))
                .max(0.0);
            let score = 0.75 * ppr_mass + 0.25 * dense;
            (score > 0.0).then_some((score, candidate_from_node(&node, score)))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.1.node_id.cmp(&right.1.node_id))
    });
    let candidates = scored
        .into_iter()
        .take(top_k)
        .map(|(_, candidate)| candidate)
        .collect();
    (candidates, trace.clone())
}

fn hippo_nodes<S: GraphStore>(store: &S) -> Vec<NodeRecord> {
    let mut seen = BTreeSet::new();
    let mut nodes = Vec::new();
    for label in [LABEL_PAGE, LABEL_PHRASE, LABEL_HUB] {
        for node in store.query_nodes(NodeQuery::label(label).with_limit(100_000)) {
            if seen.insert(node.id.clone()) {
                nodes.push(node);
            }
        }
    }
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    nodes
}

fn hippo_adjacency<S: GraphStore>(
    store: &S,
    nodes: &[NodeRecord],
) -> HashMap<String, Vec<(String, f64)>> {
    let known = nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    let mut adjacency: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    for node in nodes {
        adjacency.entry(node.id.clone()).or_default();
        for edge_type in [
            HippoEdge::Contains,
            HippoEdge::Relates,
            HippoEdge::Synonym,
            HippoEdge::Summarizes,
            HippoEdge::HubParent,
        ] {
            for hit in
                store.neighbors(NeighborQuery::out(&node.id).with_edge_type(edge_type.as_str()))
            {
                if known.contains(&hit.node_id) {
                    adjacency
                        .entry(node.id.clone())
                        .or_default()
                        .entry(hit.node_id.clone())
                        .or_insert(1.0);
                    adjacency
                        .entry(hit.node_id)
                        .or_default()
                        .entry(node.id.clone())
                        .or_insert(1.0);
                }
            }
        }
    }
    adjacency
        .into_iter()
        .map(|(source, targets)| (source, targets.into_iter().collect()))
        .collect()
}

fn candidate_from_node(node: &NodeRecord, score: f32) -> Candidate {
    let text = node_text(node);
    let mut candidate = Candidate::new(node.id.clone(), text.clone(), approximate_tokens(&text))
        .with_source_arm(SourceArm::Web);
    candidate.ppr_proximity = score;
    candidate
        .metadata
        .insert("pool".to_string(), "hipporag".to_string());
    if has_label(node, LABEL_HUB) {
        candidate
            .metadata
            .insert("hippo_label".to_string(), "hub".to_string());
        candidate
            .metadata
            .insert("node_kind".to_string(), "Hub".to_string());
    } else {
        candidate
            .metadata
            .insert("hippo_label".to_string(), "passage".to_string());
        candidate
            .metadata
            .insert("node_kind".to_string(), "Page".to_string());
    }
    if let Some(score) = numeric(node, HUB_SCORE_PROPERTY) {
        candidate
            .metadata
            .insert(HUB_SCORE_PROPERTY.to_string(), format!("{score:.6}"));
    }
    copy_vector_metadata(node, &mut candidate);
    candidate
}

fn copy_vector_metadata(node: &NodeRecord, candidate: &mut Candidate) {
    for key in [
        format!("{SEMANTIC_VECTOR_PROPERTY}_model"),
        format!("{SEMANTIC_VECTOR_PROPERTY}_dimension"),
        format!("{SEMANTIC_VECTOR_PROPERTY}_metric"),
        format!("{SEMANTIC_VECTOR_PROPERTY}_normalized"),
    ] {
        if let Some(value) = node.properties.get(&key) {
            candidate
                .metadata
                .insert(key.clone(), metadata_value(value));
        }
    }
    if node.properties.get(SEMANTIC_VECTOR_PROPERTY).is_some() {
        candidate
            .metadata
            .entry("semantic_vec_metric".to_string())
            .or_insert_with(|| {
                node.properties
                    .get(&format!("{SEMANTIC_VECTOR_PROPERTY}_metric"))
                    .map(metadata_value)
                    .unwrap_or_else(|| SEMANTIC_VECTOR_METRIC.to_string())
            });
    }
}

fn metadata_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        _ => value.to_string(),
    }
}

fn node_text(node: &NodeRecord) -> String {
    if has_label(node, LABEL_PAGE) {
        return passage_text(node);
    }
    for key in ["summary", "text", "title", "url"] {
        if let Some(value) = node.properties.get(key).and_then(Value::as_str) {
            if !value.trim().is_empty() {
                return value.trim().to_string();
            }
        }
    }
    node.id.clone()
}

fn has_label(node: &NodeRecord, label: &str) -> bool {
    node.labels.iter().any(|candidate| candidate == label)
}

fn vector(node: &NodeRecord) -> Option<Vec<f32>> {
    node.properties
        .get(SEMANTIC_VECTOR_PROPERTY)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_f64().map(|value| value as f32))
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
}

fn numeric(node: &NodeRecord, key: &str) -> Option<f32> {
    node.properties
        .get(key)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
}

fn lexical_overlap(query: &str, text: &str) -> f32 {
    let query_terms = terms(query);
    if query_terms.is_empty() {
        return 0.0;
    }
    let text_terms = terms(text);
    let matches = query_terms
        .iter()
        .filter(|term| text_terms.contains(*term))
        .count();
    matches as f32 / query_terms.len() as f32
}

fn terms(text: &str) -> BTreeSet<String> {
    text.to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| token.len() > 2)
        .map(str::to_string)
        .collect()
}

fn approximate_tokens(text: &str) -> usize {
    (text.chars().count() / 4).max(1)
}
