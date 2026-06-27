//! Harness capability pack `theorem.ingest.datawave`: the agent-facing surface
//! over the DATAWAVE-style intake, mirroring `rustyred-thg-reconstruct-harness`.
//!
//! Agents call data-driven operations rather than linking the data layer:
//! - `ingest.describe`  : list the pack's operations (no graph write).
//! - `ingest.record`    : ingest one record into normalized field-facts + edges.
//! - `ingest.batch`     : ingest many records; optionally write the dictionary.
//! - `ingest.lookup`    : event ids matching one value+field predicate.
//! - `ingest.intersect` : event ids matching an AND of value+field predicates.
//!
//! Every operation is fully data-driven (a serializable `HelperSpec` selects and
//! configures the CSV/JSON/Mapped data-type), so "point at a source and ingest by
//! configuration" is reachable over the plugin bus with no bespoke Rust. The
//! lookup/intersect operations read the persisted `FieldFact` nodes through the
//! GraphStore's value+field property index.

use rustyred_thg_core::plugin::{
    PluginCapability, PluginCapabilityKind, PluginOperationContext, PluginOperationRegistration,
    RustyRedPlugin,
};
use rustyred_thg_core::{GraphStoreError, GraphStoreResult, NodeQuery};
use rustyred_thg_datawave::{
    CsvHelper, DatawaveError, DatawaveIngest, EdgeDef, FieldConfig, FieldMapRule, IngestHelper,
    IngestStats, JsonHelper, MappedHelper, MaterializeConfig, RawRecord, FIELD_FACT_LABEL,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const INGEST_CAPABILITY_PACK: &str = "theorem.ingest.datawave";

/// A self-describing summary of the pack's operations.
#[derive(Clone, Debug, Serialize)]
pub struct IngestCapabilityPack {
    pub capability: String,
    pub operations: Vec<IngestToolSpec>,
}

#[derive(Clone, Debug, Serialize)]
pub struct IngestToolSpec {
    pub operation: String,
    pub summary: String,
    pub writes_graph: bool,
}

pub fn capability_pack() -> IngestCapabilityPack {
    IngestCapabilityPack {
        capability: INGEST_CAPABILITY_PACK.to_string(),
        operations: operations()
            .iter()
            .map(|op| IngestToolSpec {
                operation: op.operation.to_string(),
                summary: op.summary.to_string(),
                writes_graph: op.writes_graph,
            })
            .collect(),
    }
}

/// The RustyRed plugin that registers the ingest operations.
#[derive(Clone, Debug, Default)]
pub struct DatawaveIngestPlugin;

impl RustyRedPlugin for DatawaveIngestPlugin {
    fn name(&self) -> &'static str {
        INGEST_CAPABILITY_PACK
    }

    fn capabilities(&self) -> Vec<PluginCapability> {
        operations()
            .iter()
            .map(|op| PluginCapability {
                kind: PluginCapabilityKind::Operation,
                name: op.operation.to_string(),
            })
            .collect()
    }

    fn operations(&self) -> Vec<PluginOperationRegistration> {
        operations()
    }
}

fn operations() -> Vec<PluginOperationRegistration> {
    vec![
        PluginOperationRegistration {
            operation: "ingest.describe",
            command: "ingest.describe",
            aliases: &["theorem.ingest.datawave.describe"],
            summary: "Describe the datawave ingest capability pack operations.",
            writes_graph: false,
            handler: describe_handler,
        },
        PluginOperationRegistration {
            operation: "ingest.record",
            command: "ingest.record",
            aliases: &["theorem.ingest.datawave.record"],
            summary: "Ingest one record into normalized field-facts plus declared edges.",
            writes_graph: true,
            handler: record_handler,
        },
        PluginOperationRegistration {
            operation: "ingest.batch",
            command: "ingest.batch",
            aliases: &["theorem.ingest.datawave.batch"],
            summary: "Ingest a batch of records; optionally write the data + edge dictionary.",
            writes_graph: true,
            handler: batch_handler,
        },
        PluginOperationRegistration {
            operation: "ingest.lookup",
            command: "ingest.lookup",
            aliases: &["theorem.ingest.datawave.lookup"],
            summary: "Look up event ids by a value+field predicate over field-facts.",
            writes_graph: false,
            handler: lookup_handler,
        },
        PluginOperationRegistration {
            operation: "ingest.intersect",
            command: "ingest.intersect",
            aliases: &["theorem.ingest.datawave.intersect"],
            summary: "AND-intersect event ids across multiple value+field predicates.",
            writes_graph: false,
            handler: intersect_handler,
        },
    ]
}

// ---- request shapes (all serde-driven) ----

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum HelperSpec {
    Csv {
        columns: Vec<String>,
        #[serde(default)]
        delimiter: Option<char>,
        #[serde(default)]
        extra_fields: bool,
        #[serde(default)]
        config: FieldConfig,
    },
    Json {
        #[serde(default)]
        uppercase_keys: bool,
        #[serde(default)]
        delimiter: Option<String>,
        #[serde(default)]
        config: FieldConfig,
    },
    Mapped {
        rules: Vec<FieldMapRule>,
        #[serde(default)]
        config: FieldConfig,
    },
}

impl HelperSpec {
    fn build(self, data_type: &str) -> Box<dyn IngestHelper> {
        match self {
            HelperSpec::Csv { columns, delimiter, extra_fields, config } => {
                let mut helper = CsvHelper::new(data_type, columns, config);
                if let Some(d) = delimiter {
                    helper = helper.with_delimiter(d);
                }
                if extra_fields {
                    helper = helper.with_extra_fields();
                }
                Box::new(helper)
            }
            HelperSpec::Json { uppercase_keys, delimiter, config } => {
                let mut helper = JsonHelper::new(data_type, config);
                if let Some(d) = delimiter {
                    helper = helper.with_delimiter(d);
                }
                if uppercase_keys {
                    helper = helper.with_uppercase_keys();
                }
                Box::new(helper)
            }
            HelperSpec::Mapped { rules, config } => Box::new(MappedHelper::new(data_type, rules, config)),
        }
    }
}

#[derive(Deserialize)]
struct IngestRequest {
    data_type: String,
    helper: HelperSpec,
    #[serde(default)]
    edges: Vec<EdgeDef>,
    record: RawRecord,
}

#[derive(Deserialize)]
struct BatchRequest {
    data_type: String,
    helper: HelperSpec,
    #[serde(default)]
    edges: Vec<EdgeDef>,
    records: Vec<RawRecord>,
}

#[derive(Deserialize)]
struct Predicate {
    field: String,
    value: String,
}

#[derive(Deserialize)]
struct LookupRequest {
    field: String,
    value: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct IntersectRequest {
    predicates: Vec<Predicate>,
}

fn parse<T: for<'de> Deserialize<'de>>(args: Value) -> GraphStoreResult<T> {
    serde_json::from_value(args)
        .map_err(|err| GraphStoreError::new("invalid_arguments", err.to_string()))
}

fn ingest_error(err: DatawaveError) -> GraphStoreError {
    GraphStoreError::new("datawave_ingest_error", err.to_string())
}

fn build_ingest(data_type: &str, helper: HelperSpec, edges: Vec<EdgeDef>, tenant: &str) -> DatawaveIngest {
    let mut ingest = DatawaveIngest::new(MaterializeConfig::new(Some(tenant.to_string())));
    ingest.register(helper.build(data_type));
    for edge in edges {
        ingest.with_edge(edge);
    }
    ingest
}

/// Scan ceiling for one value+field lookup over persisted FieldFact nodes.
/// ponytail: a store-level scan bound; unbounded pagination rides the core global
/// index (docs/plans/reconstruction-retrieval-substrate).
const MAX_LOOKUP_SCAN: usize = 100_000;

/// Read the distinct event ids whose `FieldFact` matches one value+field predicate
/// for the active tenant. `limit` is applied to the DEDUPLICATED event set, not to
/// the node scan, so repeated facts from a single event never consume the quota.
fn lookup_events(
    context: &mut PluginOperationContext<'_>,
    field: &str,
    value: &str,
    limit: Option<usize>,
) -> GraphStoreResult<BTreeSet<String>> {
    let vf = format!("{field}={value}");
    let query = NodeQuery::label(FIELD_FACT_LABEL)
        .with_property("vf", json!(vf))
        .with_property("tenant_id", json!(context.tenant_id))
        .with_limit(MAX_LOOKUP_SCAN);
    let nodes = context.store.query_nodes(query)?;
    let mut events: BTreeSet<String> = nodes
        .iter()
        .filter_map(|node| node.properties.get("event_id").and_then(Value::as_str).map(str::to_string))
        .collect();
    if let Some(max) = limit {
        events = events.into_iter().take(max).collect();
    }
    Ok(events)
}

// ---- handlers ----

fn describe_handler(_context: PluginOperationContext<'_>, _arguments: Value) -> GraphStoreResult<Value> {
    serde_json::to_value(capability_pack())
        .map_err(|err| GraphStoreError::new("describe_serialize_error", err.to_string()))
}

fn record_handler(context: PluginOperationContext<'_>, arguments: Value) -> GraphStoreResult<Value> {
    let request: IngestRequest = parse(arguments)?;
    let ingest = build_ingest(&request.data_type, request.helper, request.edges, context.tenant_id);
    let mut stats = IngestStats::new();
    let outcome = ingest
        .ingest_record(context.store, &request.record, &mut stats)
        .map_err(ingest_error)?;
    Ok(json!({
        "event_id": outcome.event_id,
        "fields_written": outcome.fields_written,
        "edges_written": outcome.edges_written,
        "deduped": outcome.deduped,
    }))
}

fn batch_handler(context: PluginOperationContext<'_>, arguments: Value) -> GraphStoreResult<Value> {
    let request: BatchRequest = parse(arguments)?;
    let ingest = build_ingest(&request.data_type, request.helper, request.edges, context.tenant_id);
    let mut stats = IngestStats::new();
    let report = ingest.ingest_batch(context.store, &request.records, &mut stats);
    // No dictionary write here: a request-local IngestStats only sees this batch,
    // so writing dictionary nodes (keyed stably per field/edge) would overwrite
    // corpus-wide cardinality/counts with batch-local numbers. Corpus dictionary
    // generation needs cumulative stats (DatawaveIngest::write_dictionary fed a
    // long-lived IngestStats) or a from-store rebuild, a named follow-up.
    Ok(json!({
        "ingested": report.ingested,
        "deduped": report.deduped,
        "skipped": report.skipped,
        "errors": report.errors,
    }))
}

fn lookup_handler(mut context: PluginOperationContext<'_>, arguments: Value) -> GraphStoreResult<Value> {
    let request: LookupRequest = parse(arguments)?;
    let limit = Some(request.limit.unwrap_or(1000));
    let events = lookup_events(&mut context, &request.field, &request.value, limit)?;
    Ok(json!({
        "field": request.field,
        "value": request.value,
        "event_ids": events.iter().collect::<Vec<_>>(),
        "count": events.len(),
    }))
}

fn intersect_handler(mut context: PluginOperationContext<'_>, arguments: Value) -> GraphStoreResult<Value> {
    let request: IntersectRequest = parse(arguments)?;
    let mut accumulator: Option<BTreeSet<String>> = None;
    for predicate in &request.predicates {
        // No per-predicate cap: intersection needs the full event set per term.
        let events = lookup_events(&mut context, &predicate.field, &predicate.value, None)?;
        accumulator = Some(match accumulator {
            Some(acc) => acc.intersection(&events).cloned().collect(),
            None => events,
        });
    }
    let events = accumulator.unwrap_or_default();
    Ok(json!({
        "event_ids": events.iter().collect::<Vec<_>>(),
        "count": events.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::plugin::PluginRegistry;
    use rustyred_thg_core::{RedCoreGraphStore, RedCoreOptions};

    fn registry() -> PluginRegistry {
        let mut registry = PluginRegistry::new();
        registry.register(DatawaveIngestPlugin);
        registry
    }

    #[test]
    fn record_then_lookup_then_intersect_over_the_plugin_bus() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = RedCoreGraphStore::open(dir.path(), RedCoreOptions::default()).unwrap();
        let registry = registry();

        // describe: no graph write, lists the operations.
        let described = registry
            .execute(&mut store, "acme", "ingest.describe", json!({}))
            .unwrap();
        assert_eq!(described.result["capability"], json!(INGEST_CAPABILITY_PACK));

        // record: ingest one CSV row of a netflow event, deriving a CONNECTS edge.
        let args = json!({
            "data_type": "netflow",
            "helper": {
                "kind": "csv",
                "columns": ["src_ip", "dst_ip", "proto"],
                "config": {
                    "types": { "src_ip": "ip", "dst_ip": "ip", "proto": "lc_text" },
                    "policies": {
                        "src_ip": { "indexed": true, "reverse_indexed": false, "tokenized": false, "index_only": false },
                        "dst_ip": { "indexed": true, "reverse_indexed": false, "tokenized": false, "index_only": false },
                        "proto":  { "indexed": true, "reverse_indexed": false, "tokenized": false, "index_only": false }
                    }
                }
            },
            "edges": [ { "edge_type": "CONNECTS", "from_field": "src_ip", "to_field": "dst_ip" } ],
            "record": { "data_type": "netflow", "body": { "kind": "text", "data": "1.2.3.4,5.6.7.8,TCP" }, "event_time_ms": 1000 }
        });
        let recorded = registry.execute(&mut store, "acme", "ingest.record", args).unwrap();
        assert_eq!(recorded.result["fields_written"], json!(3));
        assert_eq!(recorded.result["edges_written"], json!(1));
        let event_id = recorded.result["event_id"].as_str().unwrap().to_string();

        // lookup: the proto field-fact (lc_text normalizes TCP -> tcp).
        let looked = registry
            .execute(&mut store, "acme", "ingest.lookup", json!({ "field": "proto", "value": "tcp" }))
            .unwrap();
        assert_eq!(looked.result["count"], json!(1));
        assert_eq!(looked.result["event_ids"][0], json!(event_id));

        // intersect: src_ip zero-padded AND proto -> the same event.
        let intersected = registry
            .execute(
                &mut store,
                "acme",
                "ingest.intersect",
                json!({ "predicates": [ { "field": "src_ip", "value": "001.002.003.004" }, { "field": "proto", "value": "tcp" } ] }),
            )
            .unwrap();
        assert_eq!(intersected.result["count"], json!(1));

        // a non-matching predicate intersects to empty.
        let empty = registry
            .execute(
                &mut store,
                "acme",
                "ingest.intersect",
                json!({ "predicates": [ { "field": "proto", "value": "tcp" }, { "field": "proto", "value": "udp" } ] }),
            )
            .unwrap();
        assert_eq!(empty.result["count"], json!(0));
    }

    #[test]
    fn lookup_is_tenant_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = RedCoreGraphStore::open(dir.path(), RedCoreOptions::default()).unwrap();
        let registry = registry();
        let record = json!({
            "data_type": "doc",
            "helper": { "kind": "json", "config": { "types": { "k": "lc_text" }, "policies": { "k": { "indexed": true } } } },
            "record": { "data_type": "doc", "body": { "kind": "json", "data": { "k": "shared" } }, "event_time_ms": 1 }
        });
        // tenant-a ingests k=shared.
        registry.execute(&mut store, "tenant-a", "ingest.record", record).unwrap();
        // tenant-b must NOT see tenant-a's event ids.
        let b = registry
            .execute(&mut store, "tenant-b", "ingest.lookup", json!({ "field": "k", "value": "shared" }))
            .unwrap();
        assert_eq!(b.result["count"], json!(0));
        // tenant-a sees its own.
        let a = registry
            .execute(&mut store, "tenant-a", "ingest.lookup", json!({ "field": "k", "value": "shared" }))
            .unwrap();
        assert_eq!(a.result["count"], json!(1));
    }

    #[test]
    fn dry_run_does_not_commit() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = RedCoreGraphStore::open(dir.path(), RedCoreOptions::default()).unwrap();
        let registry = registry();
        // The registry honors dry_run on writes_graph operations.
        let out = registry
            .execute(&mut store, "acme", "ingest.record", json!({ "dry_run": true }))
            .unwrap();
        assert_eq!(out.result["status"], json!("dry_run"));
    }
}
