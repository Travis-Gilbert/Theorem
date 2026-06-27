//! Phase 5 + 8: materialize normalized field-facts and derived edges into a
//! GraphStore, carrying per-fact visibility and masking.
//!
//! DATAWAVE references:
//! - `mapreduce/handler/DataTypeHandler.java` + `handler/shard/`: handlers
//!   materialize normalized fields into the shard, global-index, and field-index
//!   tables.
//! - `MarkingsHelper` / `MaskedFieldHelper`: per-field visibility and a masked
//!   alternate value.
//!
//! Materialization writes three node kinds: an `IngestEvent` (the source record,
//! content-addressed for dedup), a `FieldFact` per normalized field, and a
//! `FieldEntity` per distinct (field, value) that an edge touches. Each field-fact
//! carries a `vf = "field=normalized"` property; the GraphStore's existing
//! property index turns that into DATAWAVE's value+field -> facts global index for
//! free, so no parallel tiered index is built here. A dedicated cardinality-tiered
//! global/field index (the read-side DATAWAVE spec) composes over these same facts.
//!
//! ponytail: visibility is carried as a per-fact property layered over `tenant_id`.
//! Enforcement (filtering reads by clearance) is a query-side concern; this layer
//! records the markings, it does not police reads.

use rustyred_thg_core::{
    stable_hash, EdgeRecord, GraphStore, GraphStoreResult, NodeRecord, Provenance,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

use crate::edge::DerivedEdge;
use crate::field::{FieldType, IndexPolicy, NormalizedField};
use crate::hash::{content_hash, fuzzy_hash};
use crate::record::RawRecord;

pub const SOURCE: &str = "rustyred-thg-datawave";
pub const VERSION: &str = "rustyred-thg-datawave-v0";

pub const EVENT_LABEL: &str = "IngestEvent";
pub const FIELD_FACT_LABEL: &str = "FieldFact";
pub const FIELD_ENTITY_LABEL: &str = "FieldEntity";
pub const DATA_TYPE_LABEL: &str = "DataType";
pub const EVENT_HAS_FIELD: &str = "EVENT_HAS_FIELD";
pub const EVENT_OF_TYPE: &str = "EVENT_OF_TYPE";

/// Scope and provenance for a materialization run.
#[derive(Clone, Debug)]
pub struct MaterializeConfig {
    pub tenant_id: Option<String>,
    pub source: String,
    pub version: String,
    /// A logical clock the caller advances; kept off the wall clock so the data
    /// layer stays deterministic and test-replayable.
    pub generation: u64,
}

impl Default for MaterializeConfig {
    fn default() -> Self {
        Self {
            tenant_id: None,
            source: SOURCE.to_string(),
            version: VERSION.to_string(),
            generation: 0,
        }
    }
}

impl MaterializeConfig {
    pub fn new(tenant_id: Option<String>) -> Self {
        Self { tenant_id, ..Self::default() }
    }

    pub fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }
}

/// What one materialized record produced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestOutcome {
    pub event_id: String,
    pub fields_written: usize,
    pub edges_written: usize,
    /// True when an event with this content hash already existed (idempotent
    /// re-ingest of identical content).
    pub deduped: bool,
}

/// Running observations across a batch, feeding the data + edge dictionaries.
#[derive(Clone, Debug, Default)]
pub struct IngestStats {
    pub events: u64,
    pub deduped: u64,
    field_values: BTreeMap<String, BTreeSet<String>>,
    field_policy: BTreeMap<String, IndexPolicy>,
    field_type: BTreeMap<String, FieldType>,
    edge_counts: BTreeMap<(String, u32), u64>,
}

impl IngestStats {
    pub fn new() -> Self {
        Self::default()
    }

    fn observe_field(&mut self, field: &NormalizedField) {
        self.field_values
            .entry(field.field.clone())
            .or_default()
            .insert(field.normalized.clone());
        self.field_policy.insert(field.field.clone(), field.policy);
        self.field_type.insert(field.field.clone(), field.field_type);
    }

    fn observe_edge(&mut self, edge: &DerivedEdge) {
        *self
            .edge_counts
            .entry((edge.edge_type.clone(), edge.version))
            .or_default() += 1;
    }

    /// Distinct normalized values seen for a field. ponytail: an exact set per
    /// field; swap for a HyperLogLog estimate if a corpus's field cardinality
    /// outgrows memory.
    pub fn cardinality(&self, field: &str) -> usize {
        self.field_values.get(field).map(BTreeSet::len).unwrap_or(0)
    }

    pub fn field_names(&self) -> Vec<&str> {
        self.field_values.keys().map(String::as_str).collect()
    }

    pub fn field_policy(&self, field: &str) -> IndexPolicy {
        self.field_policy.get(field).copied().unwrap_or_default()
    }

    pub fn field_type(&self, field: &str) -> FieldType {
        self.field_type.get(field).copied().unwrap_or_default()
    }

    /// (edge_type, version, count) for every edge kind observed.
    pub fn edge_kinds(&self) -> Vec<(&str, u32, u64)> {
        self.edge_counts
            .iter()
            .map(|((edge_type, version), count)| (edge_type.as_str(), *version, *count))
            .collect()
    }
}

/// Build a tenant-scoped graph id.
fn graph_id(prefix: &str, key: &str, tenant: Option<&str>) -> String {
    match tenant {
        Some(t) => format!("{prefix}:{t}:{key}"),
        None => format!("{prefix}:{key}"),
    }
}

fn entity_id(field: &str, value: &str, tenant: Option<&str>) -> String {
    graph_id("dw:entity", &stable_hash((field, value)), tenant)
}

/// Materialize one record's facts and edges into the store.
pub fn materialize_event<S: GraphStore>(
    store: &mut S,
    record: &RawRecord,
    fields: &[NormalizedField],
    edges: &[DerivedEdge],
    config: &MaterializeConfig,
    stats: &mut IngestStats,
) -> GraphStoreResult<IngestOutcome> {
    let tenant = config.tenant_id.as_deref();
    let content = record.body.content_bytes();
    let hash = content_hash(&content);
    let fuzzy = fuzzy_hash(&content);
    let event_id = graph_id("dw:event", &hash, tenant);
    let deduped = store.get_node(&event_id).is_some();

    store.upsert_node(NodeRecord::new(
        &event_id,
        [EVENT_LABEL],
        json!({
            "data_type": record.data_type,
            "external_id": record.external_id,
            "event_time_ms": record.event_time_ms,
            "visibility": record.visibility,
            "content_hash": hash,
            "fuzzy_hash": fuzzy,
            "errors": record.errors,
            "authority": "observed_record",
            "source": config.source,
            "version": config.version,
            "tenant_id": config.tenant_id,
            "generation": config.generation,
        }),
    ))?;

    // Link the event to its data-type (DATAWAVE Type) as a first-class node, so
    // "all events of type X" is a graph traversal as well as a property filter.
    let data_type_id = graph_id("dw:datatype", &record.data_type, tenant);
    store.upsert_node(NodeRecord::new(
        &data_type_id,
        [DATA_TYPE_LABEL],
        json!({ "data_type": record.data_type, "source": config.source, "tenant_id": config.tenant_id }),
    ))?;
    store.upsert_edge(
        EdgeRecord::new(
            format!("dw:edge:oftype:{}", stable_hash((event_id.as_str(), data_type_id.as_str()))),
            &event_id,
            EVENT_OF_TYPE,
            &data_type_id,
            json!({ "authority": "observed_record", "source": config.source, "version": config.version }),
        )
        .with_provenance(provenance(&config.source, "datawave.datatype")),
    )?;

    let mut fields_written = 0;
    for field in fields {
        let fact_id = format!(
            "dw:field:{}",
            stable_hash((event_id.as_str(), field.field.as_str(), field.normalized.as_str(), &field.origin))
        );
        store.upsert_node(NodeRecord::new(
            &fact_id,
            [FIELD_FACT_LABEL],
            field_fact_props(&event_id, field, config),
        ))?;
        store.upsert_edge(
            EdgeRecord::new(
                format!("dw:edge:hasfield:{}", stable_hash((event_id.as_str(), fact_id.as_str()))),
                &event_id,
                EVENT_HAS_FIELD,
                &fact_id,
                json!({ "authority": "observed_fact", "source": config.source, "version": config.version }),
            )
            .with_provenance(provenance(&config.source, "datawave.field")),
        )?;
        stats.observe_field(field);
        fields_written += 1;
    }

    let mut edges_written = 0;
    for edge in edges {
        let from_id = entity_id(&edge.from_field, &edge.from_value, tenant);
        let to_id = entity_id(&edge.to_field, &edge.to_value, tenant);
        store.upsert_node(entity_node(&from_id, &edge.from_field, &edge.from_value, config))?;
        store.upsert_node(entity_node(&to_id, &edge.to_field, &edge.to_value, config))?;
        store.upsert_edge(
            EdgeRecord::new(
                format!(
                    "dw:edge:{}:{}",
                    edge.edge_type,
                    stable_hash((from_id.as_str(), edge.edge_type.as_str(), to_id.as_str(), edge.version))
                ),
                &from_id,
                &edge.edge_type,
                &to_id,
                json!({
                    "edge_version": edge.version,
                    "group": edge.group,
                    "event_id": event_id,
                    "visibility": record.visibility,
                    "authority": "derived_edge",
                    "source": config.source,
                    "version": config.version,
                    "tenant_id": config.tenant_id,
                }),
            )
            .with_provenance(provenance(&config.source, "datawave.edge")),
        )?;
        stats.observe_edge(edge);
        edges_written += 1;
    }

    stats.events += 1;
    if deduped {
        stats.deduped += 1;
    }

    Ok(IngestOutcome { event_id, fields_written, edges_written, deduped })
}

fn field_fact_props(event_id: &str, field: &NormalizedField, config: &MaterializeConfig) -> Value {
    json!({
        "event_id": event_id,
        "fld": field.field,
        "raw_value": field.raw_value,
        "nv": field.normalized,
        // value+field key: the existing property index over this is DATAWAVE's
        // global value->field index.
        "vf": format!("{}={}", field.field, field.normalized),
        "group": field.group,
        "visibility": field.visibility,
        "masked": field.masked,
        "indexed": field.policy.indexed,
        "reverse_indexed": field.policy.reverse_indexed,
        "tokenized": field.policy.tokenized,
        "index_only": field.policy.index_only,
        "field_type": serde_json::to_value(field.field_type).unwrap_or(Value::Null),
        "origin": serde_json::to_value(field.origin).unwrap_or(Value::Null),
        "authority": field.origin.authority(),
        "source": config.source,
        "version": config.version,
        "tenant_id": config.tenant_id,
        "generation": config.generation,
    })
}

fn entity_node(id: &str, field: &str, value: &str, config: &MaterializeConfig) -> NodeRecord {
    NodeRecord::new(
        id,
        [FIELD_ENTITY_LABEL],
        json!({
            "field": field,
            "value": value,
            "vf": format!("{field}={value}"),
            "source": config.source,
            "tenant_id": config.tenant_id,
        }),
    )
}

fn provenance(source: &str, method: &str) -> Provenance {
    Provenance {
        source_id: Some(source.to_string()),
        timestamp: None,
        method: Some(method.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::EdgeDef;
    use crate::field::{FieldConfig, FieldType, IndexPolicy};
    use crate::helper::{CsvHelper, IngestHelper};
    use rustyred_thg_core::InMemoryGraphStore;

    fn net_helper() -> CsvHelper {
        let cfg = FieldConfig::new()
            .with_field("src_ip", FieldType::Ip, IndexPolicy::INDEXED)
            .with_field("dst_ip", FieldType::Ip, IndexPolicy::INDEXED);
        CsvHelper::new("net", ["src_ip", "dst_ip"], cfg)
    }

    #[test]
    fn materializes_event_fields_and_edges() {
        let mut store = InMemoryGraphStore::default();
        let mut stats = IngestStats::new();
        let helper = net_helper();
        let defs = [EdgeDef::new("CONNECTS", "src_ip", "dst_ip")];
        let record = RawRecord::text("net", "1.2.3.4,5.6.7.8", 1000);
        let fields = helper.event_fields(&record).unwrap();
        let edges = crate::edge::derive_edges(&defs, &fields);

        let outcome = materialize_event(&mut store, &record, &fields, &edges, &MaterializeConfig::default(), &mut stats).unwrap();
        assert_eq!(outcome.fields_written, 2);
        assert_eq!(outcome.edges_written, 1);
        assert!(!outcome.deduped);

        // The event, two field-facts, and two entities exist.
        assert!(store.get_node(&outcome.event_id).is_some());
        assert_eq!(stats.events, 1);
        assert_eq!(stats.cardinality("src_ip"), 1);
    }

    #[test]
    fn identical_content_dedups_by_hash() {
        let mut store = InMemoryGraphStore::default();
        let mut stats = IngestStats::new();
        let helper = net_helper();
        let record = RawRecord::text("net", "1.2.3.4,5.6.7.8", 1000);
        let fields = helper.event_fields(&record).unwrap();

        let first = materialize_event(&mut store, &record, &fields, &[], &MaterializeConfig::default(), &mut stats).unwrap();
        let second = materialize_event(&mut store, &record, &fields, &[], &MaterializeConfig::default(), &mut stats).unwrap();
        assert_eq!(first.event_id, second.event_id);
        assert!(!first.deduped);
        assert!(second.deduped);
        assert_eq!(stats.deduped, 1);
    }
}
