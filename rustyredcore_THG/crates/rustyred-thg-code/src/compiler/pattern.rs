use rustyred_thg_core::{
    now_ms, stable_hash, EdgeRecord, GraphStore, GraphStoreResult, NodeQuery, NodeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ir::{CODE_COMPILER_FEATURE_VERSION, CODE_COMPILER_VERSION};
use crate::{property_string, property_u64, query_terms, SOURCE};

pub const CODE_PATTERN_LABEL: &str = "CodePatternMemory";
pub const PATTERN_APPLIES_TO_CODE: &str = "PATTERN_APPLIES_TO_CODE";

#[derive(Clone, Debug, PartialEq)]
pub struct CodePatternMemoryInput {
    pub tenant_id: String,
    pub repo_id: String,
    pub title: String,
    pub feedback: String,
    pub root_cause: String,
    pub fix_summary: String,
    pub symbol_ids: Vec<String>,
    pub file_paths: Vec<String>,
    pub confidence: f64,
    pub source_event_id: Option<String>,
}

impl CodePatternMemoryInput {
    pub fn new(
        tenant_id: impl Into<String>,
        repo_id: impl Into<String>,
        title: impl Into<String>,
        fix_summary: impl Into<String>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            title: title.into(),
            feedback: String::new(),
            root_cause: String::new(),
            fix_summary: fix_summary.into(),
            symbol_ids: Vec::new(),
            file_paths: Vec::new(),
            confidence: 0.75,
            source_event_id: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CodePatternMemoryRecord {
    pub pattern_id: String,
    pub tenant_id: String,
    pub repo_id: String,
    pub title: String,
    pub feedback: String,
    pub root_cause: String,
    pub fix_summary: String,
    pub symbol_ids: Vec<String>,
    pub file_paths: Vec<String>,
    pub confidence: f64,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event_id: Option<String>,
}

pub fn record_code_pattern_memory_in_store<S: GraphStore>(
    store: &mut S,
    input: CodePatternMemoryInput,
) -> GraphStoreResult<CodePatternMemoryRecord> {
    let record = pattern_record(input);
    write_code_pattern_record(store, &record)?;
    Ok(record)
}

pub(super) fn write_code_pattern_record<S: GraphStore>(
    store: &mut S,
    record: &CodePatternMemoryRecord,
) -> GraphStoreResult<()> {
    store.upsert_node(pattern_node(record))?;
    for symbol_id in &record.symbol_ids {
        if store.get_node(symbol_id).is_none() {
            continue;
        }
        store.upsert_edge(EdgeRecord::new(
            pattern_edge_id(&record.pattern_id, symbol_id),
            &record.pattern_id,
            PATTERN_APPLIES_TO_CODE,
            symbol_id,
            json!({
                "tenant_id": &record.tenant_id,
                "repo_id": &record.repo_id,
                "compiler_version": CODE_COMPILER_VERSION,
                "feature_version": CODE_COMPILER_FEATURE_VERSION,
                "source": SOURCE,
            }),
        ))?;
    }
    Ok(())
}

pub fn relevant_code_patterns<S: GraphStore>(
    store: &S,
    tenant_id: &str,
    repo_id: &str,
    query: &str,
    path_prefix: &str,
    limit: usize,
) -> Vec<CodePatternMemoryRecord> {
    let terms = query_terms(query);
    let prefix = path_prefix.trim();
    let mut patterns = store
        .query_nodes(
            NodeQuery::label(CODE_PATTERN_LABEL)
                .with_property("tenant_id", json!(tenant_id))
                .with_property("repo_id", json!(repo_id))
                .with_limit(100_000),
        )
        .into_iter()
        .filter(|node| !node.tombstone)
        .filter_map(|node| pattern_from_node(&node))
        .filter(|pattern| {
            prefix.is_empty()
                || pattern
                    .file_paths
                    .iter()
                    .any(|file_path| file_path.starts_with(prefix))
        })
        .filter(|pattern| terms.is_empty() || pattern_matches_terms(pattern, &terms))
        .collect::<Vec<_>>();
    patterns.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.created_at_ms.cmp(&left.created_at_ms))
            .then_with(|| left.title.cmp(&right.title))
    });
    patterns.truncate(limit);
    patterns
}

pub(super) fn count_patterns<S: GraphStore>(store: &S, tenant_id: &str, repo_id: &str) -> usize {
    store
        .query_nodes(
            NodeQuery::label(CODE_PATTERN_LABEL)
                .with_property("tenant_id", json!(tenant_id))
                .with_property("repo_id", json!(repo_id))
                .with_limit(100_000),
        )
        .into_iter()
        .filter(|node| !node.tombstone)
        .count()
}

fn pattern_record(input: CodePatternMemoryInput) -> CodePatternMemoryRecord {
    let created_at_ms = now_ms() as u64;
    let pattern_id = format!(
        "code:pattern:{}",
        stable_hash(json!([
            &input.tenant_id,
            &input.repo_id,
            &input.title,
            &input.root_cause,
            &input.fix_summary,
            &input.symbol_ids,
            &input.file_paths,
            &input.source_event_id
        ]))
    );
    CodePatternMemoryRecord {
        pattern_id,
        tenant_id: input.tenant_id,
        repo_id: input.repo_id,
        title: input.title,
        feedback: input.feedback,
        root_cause: input.root_cause,
        fix_summary: input.fix_summary,
        symbol_ids: dedupe(input.symbol_ids),
        file_paths: dedupe(input.file_paths),
        confidence: input.confidence.clamp(0.0, 1.0),
        created_at_ms,
        source_event_id: input.source_event_id,
    }
}

fn pattern_node(record: &CodePatternMemoryRecord) -> NodeRecord {
    NodeRecord::new(
        &record.pattern_id,
        [CODE_PATTERN_LABEL],
        json!({
            "tenant_id": &record.tenant_id,
            "repo_id": &record.repo_id,
            "title": &record.title,
            "feedback": &record.feedback,
            "root_cause": &record.root_cause,
            "fix_summary": &record.fix_summary,
            "symbol_ids": &record.symbol_ids,
            "file_paths": &record.file_paths,
            "confidence": record.confidence,
            "created_at_ms": record.created_at_ms,
            "source_event_id": &record.source_event_id,
            "compiler_version": CODE_COMPILER_VERSION,
            "feature_version": CODE_COMPILER_FEATURE_VERSION,
            "source": SOURCE,
        }),
    )
}

fn pattern_from_node(node: &NodeRecord) -> Option<CodePatternMemoryRecord> {
    Some(CodePatternMemoryRecord {
        pattern_id: node.id.clone(),
        tenant_id: property_string(&node.properties, "tenant_id")?,
        repo_id: property_string(&node.properties, "repo_id")?,
        title: property_string(&node.properties, "title").unwrap_or_default(),
        feedback: property_string(&node.properties, "feedback").unwrap_or_default(),
        root_cause: property_string(&node.properties, "root_cause").unwrap_or_default(),
        fix_summary: property_string(&node.properties, "fix_summary").unwrap_or_default(),
        symbol_ids: string_vec(&node.properties, "symbol_ids"),
        file_paths: string_vec(&node.properties, "file_paths"),
        confidence: node
            .properties
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        created_at_ms: property_u64(&node.properties, "created_at_ms").unwrap_or(0),
        source_event_id: property_string(&node.properties, "source_event_id"),
    })
}

fn pattern_matches_terms(pattern: &CodePatternMemoryRecord, terms: &[String]) -> bool {
    let haystack = format!(
        "{} {} {} {} {}",
        pattern.title,
        pattern.feedback,
        pattern.root_cause,
        pattern.fix_summary,
        pattern.file_paths.join(" ")
    )
    .to_ascii_lowercase();
    terms.iter().any(|term| haystack.contains(term))
}

fn string_vec(properties: &Value, key: &str) -> Vec<String> {
    properties
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn dedupe(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for item in items {
        if !item.trim().is_empty() && seen.insert(item.clone()) {
            out.push(item);
        }
    }
    out
}

fn pattern_edge_id(pattern_id: &str, symbol_id: &str) -> String {
    format!(
        "code:edge:pattern:{}",
        stable_hash(json!([pattern_id, symbol_id]))
    )
}
