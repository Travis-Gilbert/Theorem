use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use rustyred_thg_core::{
    personalized_pagerank, stable_hash, GraphSnapshot, NodeRecord, RedCoreGraphStore,
};
use serde_json::{json, Map, Value};

use crate::{
    code_graph_snapshot_in_store, CODE_FILE_LABEL, CODE_REPO_LABEL, CODE_SYMBOL_LABEL,
    DECLARES_SYMBOL, DEPENDS_ON_SYMBOL,
};

pub type CodebaseMapEntry = Map<String, Value>;

pub const CODEBASE_MAP_DEFAULT_TOP_K: usize = 24;

#[derive(Clone, Debug, PartialEq)]
pub struct CodebaseMapProjection {
    pub tenant_id: String,
    pub repo_id: String,
    pub entries: Vec<CodebaseMapEntry>,
    pub markdown_body: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodebaseMapProjectionEvent {
    pub tenant_id: String,
    pub repo_id: String,
    pub operation: String,
    pub repo_path: Option<PathBuf>,
    pub projection: CodebaseMapProjection,
}

pub trait CodebaseMapProjectionSink: Send + Sync {
    fn publish_codebase_map(&self, event: &CodebaseMapProjectionEvent) -> Result<(), String>;
}

#[derive(Clone, Debug, Default)]
pub struct CodebaseMarkdownFileSink;

impl CodebaseMapProjectionSink for CodebaseMarkdownFileSink {
    fn publish_codebase_map(&self, event: &CodebaseMapProjectionEvent) -> Result<(), String> {
        let Some(repo_path) = &event.repo_path else {
            return Ok(());
        };
        if !repo_path.is_dir() {
            return Err(format!(
                "repo_path {} is not a directory",
                repo_path.display()
            ));
        }
        std::fs::write(
            repo_path.join("codebase.md"),
            &event.projection.markdown_body,
        )
        .map_err(|err| format!("could not write codebase.md: {err}"))
    }
}

pub fn project_codebase_map_in_store(
    store: &RedCoreGraphStore,
    tenant_id: &str,
    repo_id: &str,
    top_k: usize,
) -> CodebaseMapProjection {
    let snapshot = code_graph_snapshot_in_store(store, tenant_id, repo_id);
    let entries = project_codebase_map_entries(&snapshot, top_k);
    CodebaseMapProjection {
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        markdown_body: render_codebase_map_markdown(repo_id, &entries),
        entries,
    }
}

pub fn project_codebase_map_entries(
    snapshot: &GraphSnapshot,
    top_k: usize,
) -> Vec<CodebaseMapEntry> {
    let top_k = top_k.max(1);
    let nodes_by_id = snapshot
        .nodes
        .iter()
        .filter(|node| !node.tombstone)
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    if nodes_by_id.is_empty() {
        return Vec::new();
    }

    let active_edges = snapshot
        .edges
        .iter()
        .filter(|edge| {
            !edge.tombstone
                && nodes_by_id.contains_key(edge.from_id.as_str())
                && nodes_by_id.contains_key(edge.to_id.as_str())
        })
        .collect::<Vec<_>>();
    let mut adjacency: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut out_degree: HashMap<String, usize> = HashMap::new();
    let mut edge_types: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut file_symbol_counts: HashMap<String, usize> = HashMap::new();
    let mut dependency_targets: BTreeSet<String> = BTreeSet::new();

    for edge in active_edges {
        adjacency
            .entry(edge.from_id.clone())
            .or_default()
            .push((edge.to_id.clone(), edge.effective_confidence().max(1e-6)));
        *out_degree.entry(edge.from_id.clone()).or_default() += 1;
        *in_degree.entry(edge.to_id.clone()).or_default() += 1;
        edge_types
            .entry(edge.from_id.clone())
            .or_default()
            .insert(edge.edge_type.clone());
        edge_types
            .entry(edge.to_id.clone())
            .or_default()
            .insert(edge.edge_type.clone());
        if edge.edge_type == DECLARES_SYMBOL {
            *file_symbol_counts.entry(edge.from_id.clone()).or_default() += 1;
        }
        if edge.edge_type == DEPENDS_ON_SYMBOL {
            dependency_targets.insert(edge.to_id.clone());
        }
    }

    let seed_mass = 1.0 / nodes_by_id.len() as f64;
    let seeds = nodes_by_id
        .keys()
        .map(|node_id| ((*node_id).to_string(), seed_mass))
        .collect::<HashMap<_, _>>();
    let scores = personalized_pagerank(&adjacency, &seeds, 0.15, 1e-5, 200_000);
    let mut ranked = nodes_by_id.values().copied().collect::<Vec<_>>();
    ranked.sort_by(|a, b| {
        score_for(&scores, b)
            .partial_cmp(&score_for(&scores, a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                total_degree(&in_degree, &out_degree, b).cmp(&total_degree(
                    &in_degree,
                    &out_degree,
                    a,
                ))
            })
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut counts = BTreeMap::<&'static str, usize>::new();
    let per_kind_cap = (top_k / 4).max(1);
    let mut entries = Vec::new();
    let mut emitted_nodes = BTreeSet::new();
    for node in ranked {
        if node_has_label(node, CODE_REPO_LABEL) {
            continue;
        }
        let Some(kind) = classify_node(node, &dependency_targets) else {
            continue;
        };
        if *counts.get(kind).unwrap_or(&0) >= per_kind_cap {
            continue;
        }
        if !emitted_nodes.insert(node.id.clone()) {
            continue;
        }
        entries.push(entry_for_node(
            node,
            kind,
            &scores,
            &in_degree,
            &out_degree,
            &edge_types,
            &file_symbol_counts,
        ));
        *counts.entry(kind).or_default() += 1;
        if entries.len() >= top_k {
            break;
        }
    }
    entries
}

pub fn render_codebase_map_markdown(repo_id: &str, entries: &[CodebaseMapEntry]) -> String {
    let mut lines = vec![
        format!("# CodebaseMap for {repo_id}"),
        String::new(),
        format!(
            "CodebaseMap exposes {} compact KG-derived orientation entries for {repo_id}.",
            entries.len()
        ),
    ];
    for (kind, heading) in [
        ("module", "Modules"),
        ("entry_point", "Entry points"),
        ("dependency", "Dependencies"),
        ("key_symbol", "Key symbols"),
    ] {
        let group = entries
            .iter()
            .filter(|entry| string_field(entry, "kind") == Some(kind))
            .collect::<Vec<_>>();
        if group.is_empty() {
            continue;
        }
        lines.push(String::new());
        lines.push(format!("## {heading}"));
        for entry in group {
            lines.push(format!(
                "- {}: {}",
                string_field(entry, "title").unwrap_or_default(),
                string_field(entry, "summary").unwrap_or_default()
            ));
        }
    }
    lines.join("\n").trim().to_string()
}

pub(crate) fn publish_codebase_map_projection(
    store: &RedCoreGraphStore,
    sink: Option<&dyn CodebaseMapProjectionSink>,
    tenant_id: &str,
    repo_id: &str,
    operation: &str,
    repo_path: Option<&Path>,
) {
    let Some(sink) = sink else {
        return;
    };
    let projection =
        project_codebase_map_in_store(store, tenant_id, repo_id, CODEBASE_MAP_DEFAULT_TOP_K);
    let event = CodebaseMapProjectionEvent {
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        operation: operation.to_string(),
        repo_path: repo_path.map(Path::to_path_buf),
        projection,
    };
    if let Err(error) = sink.publish_codebase_map(&event) {
        eprintln!("codebase map projection sink failed: {error}");
    }
}

fn classify_node(node: &NodeRecord, dependency_targets: &BTreeSet<String>) -> Option<&'static str> {
    if node_has_label(node, CODE_FILE_LABEL) {
        return Some("module");
    }
    if !node_has_label(node, CODE_SYMBOL_LABEL) {
        return None;
    }
    if dependency_targets.contains(&node.id) {
        return Some("dependency");
    }
    if is_entry_point_symbol(node) {
        return Some("entry_point");
    }
    Some("key_symbol")
}

fn is_entry_point_symbol(node: &NodeRecord) -> bool {
    let name = string_prop(&node.properties, "name").unwrap_or_default();
    let kind = string_prop(&node.properties, "kind").unwrap_or_default();
    let visibility = string_prop(&node.properties, "visibility").unwrap_or_default();
    name == "main"
        || visibility == "public"
        || matches!(kind.as_str(), "function" | "method" | "constructor")
}

fn entry_for_node(
    node: &NodeRecord,
    kind: &str,
    scores: &HashMap<String, f64>,
    in_degree: &HashMap<String, usize>,
    out_degree: &HashMap<String, usize>,
    edge_types: &HashMap<String, BTreeSet<String>>,
    file_symbol_counts: &HashMap<String, usize>,
) -> CodebaseMapEntry {
    let title = node_title(node);
    let score = score_for(scores, node);
    let incoming = *in_degree.get(&node.id).unwrap_or(&0);
    let outgoing = *out_degree.get(&node.id).unwrap_or(&0);
    let mut metadata = Map::new();
    metadata.insert("node_id".to_string(), json!(node.id));
    metadata.insert("labels".to_string(), json!(node.labels));
    metadata.insert("pagerank_score".to_string(), json!(score));
    metadata.insert("in_degree".to_string(), json!(incoming));
    metadata.insert("out_degree".to_string(), json!(outgoing));
    metadata.insert(
        "edge_types".to_string(),
        json!(edge_types
            .get(&node.id)
            .map(|types| types.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default()),
    );
    for key in ["path", "file_path", "line", "signature", "language", "kind"] {
        if let Some(value) = node.properties.get(key) {
            metadata.insert(key.to_string(), value.clone());
        }
    }
    if node_has_label(node, CODE_FILE_LABEL) {
        metadata.insert(
            "declared_symbols".to_string(),
            json!(*file_symbol_counts.get(&node.id).unwrap_or(&0)),
        );
    }

    Map::from_iter([
        (
            "entry_id".to_string(),
            json!(format!("kg-map:{kind}:{}", stable_hash(&node.id))),
        ),
        ("kind".to_string(), json!(kind)),
        ("title".to_string(), json!(title)),
        (
            "summary".to_string(),
            json!(summary_for_node(
                node,
                kind,
                score,
                incoming,
                outgoing,
                file_symbol_counts
            )),
        ),
        ("metadata".to_string(), Value::Object(metadata)),
    ])
}

fn summary_for_node(
    node: &NodeRecord,
    kind: &str,
    score: f64,
    incoming: usize,
    outgoing: usize,
    file_symbol_counts: &HashMap<String, usize>,
) -> String {
    match kind {
        "module" => format!(
            "{} declares {} symbols and carries {:.5} PageRank mass.",
            node_title(node),
            file_symbol_counts.get(&node.id).copied().unwrap_or(0),
            score
        ),
        "entry_point" => format!(
            "{} is a graph-ranked entry point with {outgoing} outgoing and {incoming} incoming edges.",
            node_title(node)
        ),
        "dependency" => format!(
            "{} is depended on by the code graph and carries {:.5} PageRank mass.",
            node_title(node),
            score
        ),
        _ => format!(
            "{} is a high-centrality symbol with {outgoing} outgoing and {incoming} incoming edges.",
            node_title(node)
        ),
    }
}

fn node_title(node: &NodeRecord) -> String {
    string_prop(&node.properties, "path")
        .or_else(|| string_prop(&node.properties, "file_path"))
        .or_else(|| string_prop(&node.properties, "name"))
        .unwrap_or_else(|| node.id.clone())
}

fn node_has_label(node: &NodeRecord, label: &str) -> bool {
    node.labels.iter().any(|candidate| candidate == label)
}

fn score_for(scores: &HashMap<String, f64>, node: &NodeRecord) -> f64 {
    *scores.get(&node.id).unwrap_or(&0.0)
}

fn total_degree(
    in_degree: &HashMap<String, usize>,
    out_degree: &HashMap<String, usize>,
    node: &NodeRecord,
) -> usize {
    in_degree.get(&node.id).copied().unwrap_or(0) + out_degree.get(&node.id).copied().unwrap_or(0)
}

fn string_prop(properties: &Value, key: &str) -> Option<String> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn string_field<'a>(entry: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    entry.get(key).and_then(Value::as_str)
}
