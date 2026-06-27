//! Phase 7: the data dictionary and edge dictionary that make the corpus
//! self-describing.
//!
//! DATAWAVE references:
//! - `warehouse/data-dictionary-core`, `warehouse/edge-dictionary-core`: a data
//!   dictionary describes fields, their types, and cardinality; an edge
//!   dictionary describes edge types, derived from ingest metadata.
//!
//! Entries derive from the observed `IngestStats` plus the configured edge
//! definitions, and can be written back as `DataDictionaryField` /
//! `EdgeDictionaryType` nodes so the corpus describes itself in-graph.

use rustyred_thg_core::{stable_hash, GraphStore, GraphStoreResult, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::edge::EdgeDef;
use crate::field::{FieldType, IndexPolicy};
use crate::materialize::{IngestStats, MaterializeConfig};

pub const DATA_DICT_LABEL: &str = "DataDictionaryField";
pub const EDGE_DICT_LABEL: &str = "EdgeDictionaryType";

/// One field's dictionary entry: name, type, index policy, and observed
/// cardinality (distinct normalized values seen).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DataDictionaryEntry {
    pub field: String,
    pub field_type: FieldType,
    pub policy: IndexPolicy,
    pub cardinality: usize,
}

/// One edge type's dictionary entry: type, endpoints, definition version, and
/// observed count.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EdgeDictionaryEntry {
    pub edge_type: String,
    pub from_field: String,
    pub to_field: String,
    pub version: u32,
    pub count: u64,
}

/// Build the data dictionary from observed field stats.
pub fn data_dictionary(stats: &IngestStats) -> Vec<DataDictionaryEntry> {
    stats
        .field_names()
        .into_iter()
        .map(|field| DataDictionaryEntry {
            field: field.to_string(),
            field_type: stats.field_type(field),
            policy: stats.field_policy(field),
            cardinality: stats.cardinality(field),
        })
        .collect()
}

/// Build the edge dictionary from the configured definitions and observed counts.
pub fn edge_dictionary(defs: &[EdgeDef], stats: &IngestStats) -> Vec<EdgeDictionaryEntry> {
    let counts = stats.edge_kinds();
    defs.iter()
        .map(|def| {
            let count = counts
                .iter()
                .find(|(edge_type, version, _)| *edge_type == def.edge_type && *version == def.version)
                .map(|(_, _, count)| *count)
                .unwrap_or(0);
            EdgeDictionaryEntry {
                edge_type: def.edge_type.clone(),
                from_field: def.from_field.clone(),
                to_field: def.to_field.clone(),
                version: def.version,
                count,
            }
        })
        .collect()
}

fn dict_id(prefix: &str, key: &str, tenant: Option<&str>) -> String {
    match tenant {
        Some(t) => format!("{prefix}:{t}:{key}"),
        None => format!("{prefix}:{key}"),
    }
}

/// Write the dictionaries into the store as self-describing nodes. Returns the
/// number of dictionary nodes written.
pub fn write_dictionary<S: GraphStore>(
    store: &mut S,
    data: &[DataDictionaryEntry],
    edges: &[EdgeDictionaryEntry],
    config: &MaterializeConfig,
) -> GraphStoreResult<usize> {
    let tenant = config.tenant_id.as_deref();
    let mut written = 0;

    for entry in data {
        store.upsert_node(NodeRecord::new(
            dict_id("dw:dict:field", &entry.field, tenant),
            [DATA_DICT_LABEL],
            json!({
                "field": entry.field,
                "field_type": serde_json::to_value(entry.field_type).unwrap_or(Value::Null),
                "indexed": entry.policy.indexed,
                "reverse_indexed": entry.policy.reverse_indexed,
                "tokenized": entry.policy.tokenized,
                "index_only": entry.policy.index_only,
                "cardinality": entry.cardinality,
                "source": config.source,
                "tenant_id": config.tenant_id,
            }),
        ))?;
        written += 1;
    }

    for entry in edges {
        let key = stable_hash((entry.edge_type.as_str(), entry.version));
        store.upsert_node(NodeRecord::new(
            dict_id("dw:dict:edge", &key, tenant),
            [EDGE_DICT_LABEL],
            json!({
                "edge_type": entry.edge_type,
                "from_field": entry.from_field,
                "to_field": entry.to_field,
                "edge_version": entry.version,
                "count": entry.count,
                "source": config.source,
                "tenant_id": config.tenant_id,
            }),
        ))?;
        written += 1;
    }

    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::{FieldType, IndexPolicy, NormalizedField, FieldOrigin};
    use crate::edge::{derive_edges, DerivedEdge};

    fn fact(field: &str, value: &str, ft: FieldType) -> NormalizedField {
        NormalizedField {
            field: field.to_string(),
            raw_value: value.to_string(),
            normalized: value.to_string(),
            group: None,
            visibility: None,
            masked: None,
            policy: IndexPolicy::INDEXED,
            field_type: ft,
            origin: FieldOrigin::Extracted,
        }
    }

    #[test]
    fn data_dictionary_reports_type_policy_cardinality() {
        let mut stats = IngestStats::new();
        // Observe two distinct values for `ip` via the materialize stat hooks.
        let mut store = rustyred_thg_core::InMemoryGraphStore::default();
        for ip in ["001.000.000.001", "001.000.000.002"] {
            let f = vec![fact("ip", ip, FieldType::Ip)];
            crate::materialize::materialize_event(
                &mut store,
                &crate::record::RawRecord::text("net", ip, 0),
                &f,
                &[],
                &MaterializeConfig::default(),
                &mut stats,
            )
            .unwrap();
        }
        let dict = data_dictionary(&stats);
        let ip_entry = dict.iter().find(|e| e.field == "ip").unwrap();
        assert_eq!(ip_entry.field_type, FieldType::Ip);
        assert!(ip_entry.policy.indexed);
        assert_eq!(ip_entry.cardinality, 2);
    }

    #[test]
    fn edge_dictionary_counts_derived_edges() {
        let mut stats = IngestStats::new();
        let mut store = rustyred_thg_core::InMemoryGraphStore::default();
        let defs = [EdgeDef::new("CONNECTS", "a", "b")];
        let fields = vec![fact("a", "1", FieldType::Text), fact("b", "2", FieldType::Text)];
        let edges: Vec<DerivedEdge> = derive_edges(&defs, &fields);
        crate::materialize::materialize_event(
            &mut store,
            &crate::record::RawRecord::text("t", "x", 0),
            &fields,
            &edges,
            &MaterializeConfig::default(),
            &mut stats,
        )
        .unwrap();

        let dict = edge_dictionary(&defs, &stats);
        assert_eq!(dict.len(), 1);
        assert_eq!(dict[0].edge_type, "CONNECTS");
        assert_eq!(dict[0].count, 1);

        let written = write_dictionary(&mut store, &data_dictionary(&stats), &dict, &MaterializeConfig::default()).unwrap();
        assert!(written >= 3); // 2 field entries + 1 edge entry
    }
}
