use std::collections::{BTreeMap, BTreeSet};

use rustyred_thg_core::{
    stable_hash, EdgeRecord, GraphStore, GraphStoreResult, NeighborQuery, NodeQuery, NodeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::code_to_spec::collect_code_symbols;
use super::ir::{CodeSymbolSnapshot, CODE_COMPILER_FEATURE_VERSION, CODE_COMPILER_VERSION};
use crate::{property_string, CALLS_SYMBOL, CENTRALITY_PROPERTY, DEPENDS_ON_SYMBOL, SOURCE};

pub const CODE_FEATURE_LABEL: &str = "CodeConnectionFeature";
pub const FEATURE_SOURCE_CODE: &str = "FEATURE_SOURCE_CODE";
pub const FEATURE_TARGET_CODE: &str = "FEATURE_TARGET_CODE";

const DEFAULT_MAX_FEATURE_PAIRS: usize = 512;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeFeatureExtractInput {
    pub tenant_id: String,
    pub repo_id: String,
    pub max_pairs: usize,
    pub include_candidate_pairs: bool,
}

impl CodeFeatureExtractInput {
    pub fn new(tenant_id: impl Into<String>, repo_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            max_pairs: DEFAULT_MAX_FEATURE_PAIRS,
            include_candidate_pairs: true,
        }
    }

    fn pair_limit(&self) -> usize {
        if self.max_pairs == 0 {
            DEFAULT_MAX_FEATURE_PAIRS
        } else {
            self.max_pairs
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct CodeConnectionFeatureVector {
    pub jaccard_coefficient: f64,
    pub bm25_score: f64,
    pub sbert_cosine: f64,
    pub tfidf_similarity: f64,
    pub shared_entity_count: f64,
    pub nli_entailment_score: f64,
    pub nli_contradiction_score: f64,
    pub kge_prediction_score: f64,
    pub same_object_type: f64,
    pub same_notebook: f64,
    pub time_gap_days: f64,
    pub shared_cluster: f64,
    pub gnn_structural_score: f64,
    pub gnn_edge_prediction: f64,
    pub rule_support_count: f64,
    pub rule_net_score: f64,
    pub source_entrenchment: f64,
    pub target_entrenchment: f64,
    pub deep_analogy_score: f64,
    pub rnd_novelty_score: f64,
    pub spacetime_temporal_score: f64,
}

impl CodeConnectionFeatureVector {
    pub fn active_feature_count(&self) -> usize {
        self.values()
            .iter()
            .filter(|(_, value)| value.abs() > 0.01)
            .count()
    }

    pub fn values(&self) -> [(&'static str, f64); 21] {
        [
            ("jaccard_coefficient", self.jaccard_coefficient),
            ("bm25_score", self.bm25_score),
            ("sbert_cosine", self.sbert_cosine),
            ("tfidf_similarity", self.tfidf_similarity),
            ("shared_entity_count", self.shared_entity_count),
            ("nli_entailment_score", self.nli_entailment_score),
            ("nli_contradiction_score", self.nli_contradiction_score),
            ("kge_prediction_score", self.kge_prediction_score),
            ("same_object_type", self.same_object_type),
            ("same_notebook", self.same_notebook),
            ("time_gap_days", self.time_gap_days),
            ("shared_cluster", self.shared_cluster),
            ("gnn_structural_score", self.gnn_structural_score),
            ("gnn_edge_prediction", self.gnn_edge_prediction),
            ("rule_support_count", self.rule_support_count),
            ("rule_net_score", self.rule_net_score),
            ("source_entrenchment", self.source_entrenchment),
            ("target_entrenchment", self.target_entrenchment),
            ("deep_analogy_score", self.deep_analogy_score),
            ("rnd_novelty_score", self.rnd_novelty_score),
            ("spacetime_temporal_score", self.spacetime_temporal_score),
        ]
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CodeFeatureRecord {
    pub feature_id: String,
    pub source_symbol_id: String,
    pub target_symbol_id: String,
    pub feature_version: String,
    pub model_id: Option<String>,
    pub features: CodeConnectionFeatureVector,
    pub provenance: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CodeFeatureExtractOutput {
    pub tenant_id: String,
    pub repo_id: String,
    pub records: Vec<CodeFeatureRecord>,
}

pub fn extract_code_features_in_store<S: GraphStore>(
    store: &mut S,
    input: CodeFeatureExtractInput,
) -> GraphStoreResult<CodeFeatureExtractOutput> {
    let symbols = collect_code_symbols(store, &input.tenant_id, &input.repo_id, 100_000);
    let symbol_map = symbols
        .iter()
        .cloned()
        .map(|symbol| (symbol.symbol_id.clone(), symbol))
        .collect::<BTreeMap<_, _>>();
    let mut pairs = dependency_pairs(store, &symbols);
    if input.include_candidate_pairs {
        pairs.extend(candidate_pairs(&symbols));
    }
    pairs.sort();
    pairs.dedup();
    pairs.truncate(input.pair_limit());

    let mut records = Vec::new();
    for (source_id, target_id) in pairs {
        let (Some(source), Some(target)) = (symbol_map.get(&source_id), symbol_map.get(&target_id))
        else {
            continue;
        };
        let record = feature_record(store, &input, source, target);
        write_code_feature_record(store, &input.tenant_id, &input.repo_id, &record)?;
        records.push(record);
    }

    Ok(CodeFeatureExtractOutput {
        tenant_id: input.tenant_id,
        repo_id: input.repo_id,
        records,
    })
}

pub(super) fn write_code_feature_record<S: GraphStore>(
    store: &mut S,
    tenant_id: &str,
    repo_id: &str,
    record: &CodeFeatureRecord,
) -> GraphStoreResult<()> {
    store.upsert_node(feature_node(tenant_id, repo_id, record))?;
    if store.get_node(&record.source_symbol_id).is_some() {
        store.upsert_edge(EdgeRecord::new(
            feature_edge_id(
                &record.feature_id,
                FEATURE_SOURCE_CODE,
                &record.source_symbol_id,
            ),
            &record.feature_id,
            FEATURE_SOURCE_CODE,
            &record.source_symbol_id,
            feature_edge_props(tenant_id, repo_id),
        ))?;
    }
    if store.get_node(&record.target_symbol_id).is_some() {
        store.upsert_edge(EdgeRecord::new(
            feature_edge_id(
                &record.feature_id,
                FEATURE_TARGET_CODE,
                &record.target_symbol_id,
            ),
            &record.feature_id,
            FEATURE_TARGET_CODE,
            &record.target_symbol_id,
            feature_edge_props(tenant_id, repo_id),
        ))?;
    }
    Ok(())
}

pub(super) fn count_features<S: GraphStore>(store: &S, tenant_id: &str, repo_id: &str) -> usize {
    store
        .query_nodes(
            NodeQuery::label(CODE_FEATURE_LABEL)
                .with_property("tenant_id", json!(tenant_id))
                .with_property("repo_id", json!(repo_id))
                .with_limit(100_000),
        )
        .into_iter()
        .filter(|node| !node.tombstone)
        .count()
}

pub(super) fn feature_record_from_node(node: &NodeRecord) -> Option<CodeFeatureRecord> {
    Some(CodeFeatureRecord {
        feature_id: node.id.clone(),
        source_symbol_id: property_string(&node.properties, "source_symbol_id")?,
        target_symbol_id: property_string(&node.properties, "target_symbol_id")?,
        feature_version: property_string(&node.properties, "feature_version")
            .unwrap_or_else(|| CODE_COMPILER_FEATURE_VERSION.to_string()),
        model_id: property_string(&node.properties, "model_id"),
        features: serde_json::from_value(node.properties.get("features")?.clone()).ok()?,
        provenance: node
            .properties
            .get("provenance")
            .cloned()
            .unwrap_or_else(|| json!({})),
    })
}

fn dependency_pairs<S: GraphStore>(
    store: &S,
    symbols: &[CodeSymbolSnapshot],
) -> Vec<(String, String)> {
    let ids = symbols
        .iter()
        .map(|symbol| symbol.symbol_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut pairs = Vec::new();
    for symbol in symbols {
        for edge_type in [CALLS_SYMBOL, DEPENDS_ON_SYMBOL] {
            for hit in store
                .neighbors(NeighborQuery::out(&symbol.symbol_id).with_edge_type(edge_type))
                .into_iter()
                .filter(|hit| ids.contains(hit.node_id.as_str()))
            {
                pairs.push((symbol.symbol_id.clone(), hit.node_id));
            }
        }
    }
    pairs
}

fn candidate_pairs(symbols: &[CodeSymbolSnapshot]) -> Vec<(String, String)> {
    let mut by_file: BTreeMap<&str, Vec<&CodeSymbolSnapshot>> = BTreeMap::new();
    for symbol in symbols {
        by_file
            .entry(symbol.file_path.as_str())
            .or_default()
            .push(symbol);
    }
    let mut pairs = Vec::new();
    for symbols in by_file.values() {
        for window in symbols.windows(2) {
            if let [left, right] = window {
                pairs.push((left.symbol_id.clone(), right.symbol_id.clone()));
            }
        }
    }
    pairs
}

fn feature_record<S: GraphStore>(
    store: &S,
    input: &CodeFeatureExtractInput,
    source: &CodeSymbolSnapshot,
    target: &CodeSymbolSnapshot,
) -> CodeFeatureRecord {
    let features = feature_vector(store, source, target);
    let feature_id = format!(
        "code:feature:{}",
        stable_hash(json!([
            &input.tenant_id,
            &input.repo_id,
            &source.symbol_id,
            &target.symbol_id,
            CODE_COMPILER_FEATURE_VERSION
        ]))
    );
    CodeFeatureRecord {
        feature_id,
        source_symbol_id: source.symbol_id.clone(),
        target_symbol_id: target.symbol_id.clone(),
        feature_version: CODE_COMPILER_FEATURE_VERSION.to_string(),
        model_id: None,
        features,
        provenance: json!({
            "kind": "rust_local_feature_contract",
            "compiler_version": CODE_COMPILER_VERSION,
            "note": "Model-owned features may overwrite zero placeholders via RunPod import.",
        }),
    }
}

fn feature_vector<S: GraphStore>(
    store: &S,
    source: &CodeSymbolSnapshot,
    target: &CodeSymbolSnapshot,
) -> CodeConnectionFeatureVector {
    let source_tokens = symbol_tokens(source);
    let target_tokens = symbol_tokens(target);
    let intersection = source_tokens.intersection(&target_tokens).count() as f64;
    let union = source_tokens.union(&target_tokens).count().max(1) as f64;
    let jaccard = intersection / union;
    let same_file = (source.file_path == target.file_path) as u8 as f64;
    let same_kind = (source.kind == target.kind) as u8 as f64;
    let direct_edge = has_direct_edge(store, &source.symbol_id, &target.symbol_id) as u8 as f64;
    let line_gap = match (source.line, target.line) {
        (Some(left), Some(right)) => left.abs_diff(right) as f64,
        _ => 0.0,
    };
    let temporal = if line_gap == 0.0 {
        1.0
    } else {
        1.0 / (1.0 + line_gap)
    };
    let source_entrenchment = node_centrality(store, &source.symbol_id);
    let target_entrenchment = node_centrality(store, &target.symbol_id);
    CodeConnectionFeatureVector {
        jaccard_coefficient: round6(jaccard),
        bm25_score: round6(jaccard),
        sbert_cosine: 0.0,
        tfidf_similarity: round6(jaccard),
        shared_entity_count: intersection,
        nli_entailment_score: 0.0,
        nli_contradiction_score: 0.0,
        kge_prediction_score: 0.0,
        same_object_type: same_kind,
        same_notebook: same_file,
        time_gap_days: line_gap,
        shared_cluster: same_file,
        gnn_structural_score: direct_edge,
        gnn_edge_prediction: 0.0,
        rule_support_count: direct_edge,
        rule_net_score: direct_edge,
        source_entrenchment,
        target_entrenchment,
        deep_analogy_score: 0.0,
        rnd_novelty_score: 0.0,
        spacetime_temporal_score: round6(temporal),
    }
}

fn feature_node(tenant_id: &str, repo_id: &str, record: &CodeFeatureRecord) -> NodeRecord {
    NodeRecord::new(
        &record.feature_id,
        [CODE_FEATURE_LABEL],
        json!({
            "tenant_id": tenant_id,
            "repo_id": repo_id,
            "source_symbol_id": &record.source_symbol_id,
            "target_symbol_id": &record.target_symbol_id,
            "feature_version": &record.feature_version,
            "model_id": &record.model_id,
            "features": &record.features,
            "active_feature_count": record.features.active_feature_count(),
            "provenance": &record.provenance,
            "compiler_version": CODE_COMPILER_VERSION,
            "source": SOURCE,
        }),
    )
}

fn feature_edge_props(tenant_id: &str, repo_id: &str) -> Value {
    json!({
        "tenant_id": tenant_id,
        "repo_id": repo_id,
        "compiler_version": CODE_COMPILER_VERSION,
        "feature_version": CODE_COMPILER_FEATURE_VERSION,
        "source": SOURCE,
    })
}

fn feature_edge_id(feature_id: &str, edge_type: &str, symbol_id: &str) -> String {
    format!(
        "code:edge:feature:{}",
        stable_hash(json!([feature_id, edge_type, symbol_id]))
    )
}

fn symbol_tokens(symbol: &CodeSymbolSnapshot) -> BTreeSet<String> {
    [
        symbol.name.as_str(),
        symbol.kind.as_str(),
        symbol.signature.as_deref().unwrap_or(""),
        symbol.file_path.as_str(),
    ]
    .join(" ")
    .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
    .filter(|token| !token.is_empty())
    .map(str::to_ascii_lowercase)
    .collect()
}

fn has_direct_edge<S: GraphStore>(store: &S, source_id: &str, target_id: &str) -> bool {
    [CALLS_SYMBOL, DEPENDS_ON_SYMBOL].iter().any(|edge_type| {
        store
            .neighbors(NeighborQuery::out(source_id).with_edge_type(*edge_type))
            .into_iter()
            .any(|hit| hit.node_id == target_id)
    })
}

fn node_centrality<S: GraphStore>(store: &S, symbol_id: &str) -> f64 {
    store
        .get_node(symbol_id)
        .and_then(|node| node.properties.get(CENTRALITY_PROPERTY))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0)
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}
