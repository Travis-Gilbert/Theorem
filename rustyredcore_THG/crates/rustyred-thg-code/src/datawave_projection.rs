use std::collections::{BTreeSet, HashSet};

use rustyred_thg_core::{stable_hash, GraphStore, GraphStoreError, GraphStoreResult, NodeQuery};
use rustyred_thg_datawave::{
    export_parity_receipts, write_parity_receipt, DatawaveError, DatawaveIngest, EdgeDef,
    FieldConfig, FieldType, IndexPolicy, IngestStats, JsonHelper, MaterializeConfig, ParityReceipt,
    RawRecord, FIELD_FACT_LABEL,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    property_string, CODE_FILE_LABEL, CODE_SYMBOL_LABEL, CODE_TO_DATAWAVE_SOURCE, DECLARES_SYMBOL,
    SOURCE,
};

pub const CODE_TO_DATAWAVE_VERSION: &str = "rustyred-thg-code-to-datawave-v0";
pub const CODE_FILE_DATA_TYPE: &str = "code_file_projection";
pub const CODE_SYMBOL_DATA_TYPE: &str = "code_symbol_projection";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodeToDatawaveProjectionInput {
    pub tenant_id: String,
    pub repo_id: String,
}

impl CodeToDatawaveProjectionInput {
    pub fn new(tenant_id: impl Into<String>, repo_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectionReceipt {
    pub tenant_id: String,
    pub repo_id: String,
    pub files_projected: usize,
    pub symbols_projected: usize,
    pub facts_written: usize,
    pub edges_written: usize,
    pub parity_receipt_id: String,
    pub training_export_jsonl: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FieldFactSummary {
    pub fact_id: String,
    pub event_id: String,
    pub field: String,
    pub value: String,
    pub vf: String,
    pub source: String,
}

#[derive(Clone, Debug)]
struct CodeFileProjection {
    node_id: String,
    path: String,
    language: String,
    content_hash: Option<String>,
}

#[derive(Clone, Debug)]
struct CodeSymbolProjection {
    node_id: String,
    file_path: String,
    language: String,
    kind: String,
    name: String,
}

pub fn project_code_to_datawave<S: GraphStore>(
    store: &mut S,
    input: &CodeToDatawaveProjectionInput,
) -> GraphStoreResult<ProjectionReceipt> {
    let files = collect_files(store, input);
    let symbols = collect_symbols(store, input);
    let mut ingest = DatawaveIngest::new(materialize_config(input));
    ingest.register(Box::new(JsonHelper::new(
        CODE_FILE_DATA_TYPE,
        code_projection_config(),
    )));
    ingest.register(Box::new(JsonHelper::new(
        CODE_SYMBOL_DATA_TYPE,
        code_projection_config(),
    )));
    ingest.with_edge(EdgeDef::new(DECLARES_SYMBOL, "file_path", "symbol_name"));

    let mut stats = IngestStats::new();
    let mut facts_written = 0;
    let mut edges_written = 0;
    let mut files_projected = 0;
    let mut symbols_projected = 0;

    for file in files {
        let record = RawRecord::json(
            CODE_FILE_DATA_TYPE,
            json!({
                "repo_id": &input.repo_id,
                "code_node_id": file.node_id,
                "file_path": file.path,
                "language": file.language,
                "content_hash": file.content_hash.unwrap_or_default(),
                "record_kind": "code_file",
            }),
            0,
        )
        .with_external_id(format!("{}:{}", input.repo_id, file.node_id));
        let outcome = ingest
            .ingest_record(store, &record, &mut stats)
            .map_err(datawave_error)?;
        facts_written += outcome.fields_written;
        edges_written += outcome.edges_written;
        files_projected += 1;
    }

    for symbol in symbols {
        let record = RawRecord::json(
            CODE_SYMBOL_DATA_TYPE,
            json!({
                "repo_id": &input.repo_id,
                "code_node_id": symbol.node_id,
                "file_path": symbol.file_path,
                "language": symbol.language,
                "symbol_kind": symbol.kind,
                "symbol_name": symbol.name,
                "record_kind": "code_symbol",
            }),
            0,
        )
        .with_external_id(format!("{}:{}", input.repo_id, symbol.node_id));
        let outcome = ingest
            .ingest_record(store, &record, &mut stats)
            .map_err(datawave_error)?;
        facts_written += outcome.fields_written;
        edges_written += outcome.edges_written;
        symbols_projected += 1;
    }

    let mut parity = ParityReceipt::new(
        format!("code-to-datawave/{}", input.repo_id),
        "code_to_datawave",
        "files_projected",
        files_projected.to_string(),
        files_projected.to_string(),
    );
    parity.notes.push(format!(
        "projected {files_projected} files and {symbols_projected} symbols from {SOURCE}"
    ));
    write_parity_receipt(store, &parity, Some(&input.tenant_id))?;
    let training_export_jsonl = export_parity_receipts(
        &[parity.clone()],
        &format!(
            "code-to-datawave:{}",
            stable_hash((&input.tenant_id, &input.repo_id))
        ),
        "rustyred-thg-code-to-datawave",
        store.stats().version,
    )?;

    Ok(ProjectionReceipt {
        tenant_id: input.tenant_id.clone(),
        repo_id: input.repo_id.clone(),
        files_projected,
        symbols_projected,
        facts_written,
        edges_written,
        parity_receipt_id: format!("{}:{}", parity.receipt_id, input.tenant_id),
        training_export_jsonl,
    })
}

pub fn datawave_fact_summaries_for_repo<S: GraphStore>(
    store: &S,
    tenant_id: &str,
    repo_id: &str,
    limit: usize,
) -> Vec<FieldFactSummary> {
    let bounded_limit = limit.max(1);
    let event_ids = store
        .query_nodes(
            NodeQuery::label(FIELD_FACT_LABEL)
                .with_property("tenant_id", json!(tenant_id))
                .with_property("source", json!(CODE_TO_DATAWAVE_SOURCE))
                .with_property("vf", json!(format!("repo_id={repo_id}")))
                .with_limit(100_000),
        )
        .into_iter()
        .filter_map(|node| property_string(&node.properties, "event_id"))
        .collect::<HashSet<_>>();

    if event_ids.is_empty() {
        return Vec::new();
    }

    let mut summaries = store
        .query_nodes(
            NodeQuery::label(FIELD_FACT_LABEL)
                .with_property("tenant_id", json!(tenant_id))
                .with_property("source", json!(CODE_TO_DATAWAVE_SOURCE))
                .with_limit(500_000),
        )
        .into_iter()
        .filter(|node| !node.tombstone)
        .filter(|node| {
            property_string(&node.properties, "event_id")
                .as_deref()
                .is_some_and(|event_id| event_ids.contains(event_id))
        })
        .filter_map(|node| fact_summary(&node))
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| {
        left.event_id
            .cmp(&right.event_id)
            .then_with(|| left.field.cmp(&right.field))
            .then_with(|| left.value.cmp(&right.value))
            .then_with(|| left.fact_id.cmp(&right.fact_id))
    });
    summaries.truncate(bounded_limit);
    summaries
}

fn collect_files<S: GraphStore>(
    store: &S,
    input: &CodeToDatawaveProjectionInput,
) -> Vec<CodeFileProjection> {
    let mut files = store
        .query_nodes(
            NodeQuery::label(CODE_FILE_LABEL)
                .with_property("tenant_id", json!(input.tenant_id))
                .with_property("repo_id", json!(input.repo_id))
                .with_limit(100_000),
        )
        .into_iter()
        .filter(|node| !node.tombstone)
        .filter_map(|node| {
            Some(CodeFileProjection {
                node_id: property_string(&node.properties, "file_id")
                    .unwrap_or_else(|| node.id.clone()),
                path: property_string(&node.properties, "path")?,
                language: property_string(&node.properties, "language").unwrap_or_default(),
                content_hash: property_string(&node.properties, "content_hash")
                    .filter(|value| !value.trim().is_empty()),
            })
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    files
}

fn collect_symbols<S: GraphStore>(
    store: &S,
    input: &CodeToDatawaveProjectionInput,
) -> Vec<CodeSymbolProjection> {
    let mut symbols = store
        .query_nodes(
            NodeQuery::label(CODE_SYMBOL_LABEL)
                .with_property("tenant_id", json!(input.tenant_id))
                .with_property("repo_id", json!(input.repo_id))
                .with_limit(200_000),
        )
        .into_iter()
        .filter(|node| !node.tombstone)
        .filter_map(|node| {
            let name = property_string(&node.properties, "name").unwrap_or_else(|| node.id.clone());
            let file_path = property_string(&node.properties, "file_path")?;
            Some(CodeSymbolProjection {
                node_id: property_string(&node.properties, "symbol_id")
                    .unwrap_or_else(|| node.id.clone()),
                file_path,
                language: property_string(&node.properties, "language").unwrap_or_default(),
                kind: property_string(&node.properties, "kind")
                    .unwrap_or_else(|| "symbol".to_string()),
                name,
            })
        })
        .collect::<Vec<_>>();
    symbols.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    symbols
}

fn materialize_config(input: &CodeToDatawaveProjectionInput) -> MaterializeConfig {
    let mut config = MaterializeConfig::new(Some(input.tenant_id.clone()));
    config.source = CODE_TO_DATAWAVE_SOURCE.to_string();
    config.version = CODE_TO_DATAWAVE_VERSION.to_string();
    config
}

fn code_projection_config() -> FieldConfig {
    FieldConfig::new()
        .with_field("repo_id", FieldType::Text, IndexPolicy::INDEXED)
        .with_field("code_node_id", FieldType::Text, IndexPolicy::INDEXED)
        .with_field("file_path", FieldType::Text, IndexPolicy::INDEXED)
        .with_field("language", FieldType::Text, IndexPolicy::INDEXED)
        .with_field("content_hash", FieldType::Text, IndexPolicy::INDEXED)
        .with_field("symbol_kind", FieldType::Text, IndexPolicy::INDEXED)
        .with_field("symbol_name", FieldType::Text, IndexPolicy::INDEXED)
        .with_field("record_kind", FieldType::Text, IndexPolicy::INDEXED)
}

fn fact_summary(node: &rustyred_thg_core::NodeRecord) -> Option<FieldFactSummary> {
    Some(FieldFactSummary {
        fact_id: node.id.clone(),
        event_id: property_string(&node.properties, "event_id")?,
        field: property_string(&node.properties, "fld")?,
        value: property_string(&node.properties, "nv")?,
        vf: property_string(&node.properties, "vf")?,
        source: property_string(&node.properties, "source")?,
    })
}

fn datawave_error(error: DatawaveError) -> GraphStoreError {
    GraphStoreError::new("code_to_datawave_projection_failed", error.to_string())
}

pub fn intersect_field_values<'a>(
    left: impl IntoIterator<Item = &'a FieldFactSummary>,
    right: impl IntoIterator<Item = &'a FieldFactSummary>,
    field: &str,
) -> BTreeSet<String> {
    let left_values = left
        .into_iter()
        .filter(|fact| fact.field == field)
        .map(|fact| fact.value.clone())
        .collect::<BTreeSet<_>>();
    right
        .into_iter()
        .filter(|fact| fact.field == field)
        .filter(|fact| left_values.contains(&fact.value))
        .map(|fact| fact.value.clone())
        .collect()
}
