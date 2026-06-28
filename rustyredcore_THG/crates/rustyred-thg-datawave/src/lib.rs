//! # rustyred-thg-datawave
//!
//! DATAWAVE-style intake for the RustyRed substrate: turn any source record into
//! typed normalized field-facts plus declared entity-edges in a `GraphStore`,
//! with per-field index policy, a self-describing dictionary, cell-level
//! visibility/masking, and content + fuzzy hashing.
//!
//! Reference: `NationalSecurityAgency/datawave`, the `warehouse` ingest modules.
//! This is the *write* side (record -> field-facts -> corpus); the read-side
//! tiered index and boolean-pushdown planner are a sibling concern that composes
//! over the same facts.
//!
//! The thesis, borrowed from DATAWAVE, is "no bespoke loader per source": a new
//! source is a registered data-type (an [`IngestHelper`]) plus a [`FieldConfig`],
//! and everything downstream -- normalization, index policy, derived edges, the
//! dictionary, dedup, similarity -- follows from the normalized fields it emits.
//! Because field-facts land in the same graph as the binary-reconstruction facts
//! ([`rustyred_thg_core`] + the `rustyred-thg-binformat`/`-reconstruct` crates),
//! a reconstructed binary, an ingested document, a crawled URL, and a parsed repo
//! intersect in one corpus.
//!
//! ## Pipeline
//!
//! ```text
//! RawRecord --(IngestHelper::event_fields)--> [NormalizedField]
//!           --(derive_edges)--------------->  [DerivedEdge]
//!           --(materialize_event)---------->  IngestEvent + FieldFact + FieldEntity nodes/edges
//!           --(write_dictionary)----------->  DataDictionaryField + EdgeDictionaryType nodes
//! ```
//!
//! ## Example
//!
//! ```
//! use rustyred_thg_datawave::{
//!     CsvHelper, DatawaveIngest, EdgeDef, FieldConfig, FieldType, IndexPolicy,
//!     IngestStats, MaterializeConfig, RawRecord,
//! };
//! use rustyred_thg_core::InMemoryGraphStore;
//!
//! let config = FieldConfig::new()
//!     .with_field("src_ip", FieldType::Ip, IndexPolicy::INDEXED)
//!     .with_field("dst_ip", FieldType::Ip, IndexPolicy::INDEXED);
//! let helper = CsvHelper::new("netflow", ["src_ip", "dst_ip"], config);
//!
//! let mut ingest = DatawaveIngest::new(MaterializeConfig::default());
//! ingest.register(Box::new(helper));
//! ingest.with_edge(EdgeDef::new("CONNECTS", "src_ip", "dst_ip"));
//!
//! let mut store = InMemoryGraphStore::default();
//! let mut stats = IngestStats::new();
//! let record = RawRecord::text("netflow", "10.0.0.1,10.0.0.2", 0);
//! let outcome = ingest.ingest_record(&mut store, &record, &mut stats).unwrap();
//! assert_eq!(outcome.fields_written, 2);
//! assert_eq!(outcome.edges_written, 1);
//! ```

pub mod dictionary;
pub mod edge;
pub mod field;
pub mod hash;
pub mod helper;
pub mod materialize;
pub mod record;
pub mod tiered;
pub mod training;

pub use dictionary::{
    data_dictionary, edge_dictionary, write_dictionary, DataDictionaryEntry, EdgeDictionaryEntry,
};
pub use edge::{derive_edges, DerivedEdge, EdgeCondition, EdgeDef};
pub use field::{
    CompositeDef, FieldConfig, FieldOrigin, FieldType, IndexPolicy, MaskRule, NormalizeError,
    NormalizedField, VirtualDef, VirtualTransform,
};
pub use hash::{content_hash, fuzzy_compare, fuzzy_hash};
pub use helper::{
    derive_fields, CsvHelper, FieldMapRule, IngestError, IngestHelper, JsonHelper, MappedHelper,
};
pub use materialize::{
    materialize_event, IngestOutcome, IngestStats, MaterializeConfig, DATA_TYPE_LABEL,
    EVENT_HAS_FIELD, EVENT_LABEL, EVENT_OF_TYPE, FIELD_ENTITY_LABEL, FIELD_FACT_LABEL, SOURCE,
    VERSION,
};
pub use record::{RawRecord, RecordBody, TypeRegistry};
pub use tiered::TieredIndex;
pub use training::{
    export_parity_receipts, parity_receipt_to_labeled_run, write_parity_receipt, ParityReceipt,
};

use rustyred_thg_core::{GraphStore, GraphStoreError};
use std::fmt;

/// Anything that can stop one record from being ingested.
#[derive(Debug)]
pub enum DatawaveError {
    /// No helper is registered for the record's declared data-type.
    UnknownDataType(String),
    /// The record could not be parsed into raw fields.
    Ingest(IngestError),
    /// A graph write failed.
    Graph(GraphStoreError),
}

impl fmt::Display for DatawaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DatawaveError::UnknownDataType(dt) => write!(f, "no helper registered for data-type {dt:?}"),
            DatawaveError::Ingest(err) => write!(f, "{err}"),
            DatawaveError::Graph(err) => write!(f, "graph write failed: {err:?}"),
        }
    }
}

impl std::error::Error for DatawaveError {}

impl From<IngestError> for DatawaveError {
    fn from(err: IngestError) -> Self {
        DatawaveError::Ingest(err)
    }
}

impl From<GraphStoreError> for DatawaveError {
    fn from(err: GraphStoreError) -> Self {
        DatawaveError::Graph(err)
    }
}

/// Summary of a batch ingest. Errored records are skipped, not fatal; their
/// index and reason are carried so the caller can see what was dropped.
#[derive(Clone, Debug, Default)]
pub struct BatchReport {
    pub ingested: usize,
    pub deduped: usize,
    pub skipped: usize,
    pub errors: Vec<(usize, String)>,
}

/// The intake driver: a data-type registry, the declared edge definitions, and
/// the materialization scope. One `DatawaveIngest` ingests any record whose
/// data-type it has a helper for.
pub struct DatawaveIngest {
    pub registry: TypeRegistry,
    pub edges: Vec<EdgeDef>,
    pub config: MaterializeConfig,
}

impl DatawaveIngest {
    pub fn new(config: MaterializeConfig) -> Self {
        Self { registry: TypeRegistry::new(), edges: Vec::new(), config }
    }

    /// Register a data-type's ingest helper.
    pub fn register(&mut self, helper: Box<dyn IngestHelper>) -> &mut Self {
        self.registry.register(helper);
        self
    }

    /// Declare an edge definition that fires for every ingested record.
    pub fn with_edge(&mut self, def: EdgeDef) -> &mut Self {
        self.edges.push(def);
        self
    }

    /// Ingest one record: resolve its data-type, derive field-facts and edges,
    /// and materialize them.
    pub fn ingest_record<S: GraphStore>(
        &self,
        store: &mut S,
        record: &RawRecord,
        stats: &mut IngestStats,
    ) -> Result<IngestOutcome, DatawaveError> {
        let helper = self
            .registry
            .resolve(&record.data_type)
            .ok_or_else(|| DatawaveError::UnknownDataType(record.data_type.clone()))?;
        let fields = helper.event_fields(record)?;
        let edges = derive_edges(&self.edges, &fields);
        let outcome = materialize_event(store, record, &fields, &edges, &self.config, stats)?;
        Ok(outcome)
    }

    /// Ingest a batch. A failing record is skipped with its reason recorded; the
    /// rest of the batch still ingests.
    pub fn ingest_batch<S: GraphStore>(
        &self,
        store: &mut S,
        records: &[RawRecord],
        stats: &mut IngestStats,
    ) -> BatchReport {
        let mut report = BatchReport::default();
        for (index, record) in records.iter().enumerate() {
            match self.ingest_record(store, record, stats) {
                Ok(outcome) => {
                    report.ingested += 1;
                    if outcome.deduped {
                        report.deduped += 1;
                    }
                }
                Err(err) => {
                    report.skipped += 1;
                    report.errors.push((index, err.to_string()));
                }
            }
        }
        report
    }

    /// Write the data + edge dictionaries describing everything ingested so far.
    pub fn write_dictionary<S: GraphStore>(
        &self,
        store: &mut S,
        stats: &IngestStats,
    ) -> Result<usize, DatawaveError> {
        let data = data_dictionary(stats);
        let edges = edge_dictionary(&self.edges, stats);
        Ok(write_dictionary(store, &data, &edges, &self.config)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::InMemoryGraphStore;
    use serde_json::json;

    #[test]
    fn unknown_data_type_is_reported_not_panicked() {
        let ingest = DatawaveIngest::new(MaterializeConfig::default());
        let mut store = InMemoryGraphStore::default();
        let mut stats = IngestStats::new();
        let record = RawRecord::text("never_registered", "x", 0);
        let err = ingest.ingest_record(&mut store, &record, &mut stats).unwrap_err();
        assert!(matches!(err, DatawaveError::UnknownDataType(_)));
    }

    #[test]
    fn batch_skips_bad_records_and_keeps_going() {
        let cfg = FieldConfig::new().with_field("name", FieldType::LcText, IndexPolicy::INDEXED);
        let mut ingest = DatawaveIngest::new(MaterializeConfig::default());
        ingest.register(Box::new(JsonHelper::new("doc", cfg)));

        let mut store = InMemoryGraphStore::default();
        let mut stats = IngestStats::new();
        let records = vec![
            RawRecord::json("doc", json!({ "name": "Ada" }), 0),
            RawRecord::text("doc", "this is not json for the json helper", 0), // skipped
            RawRecord::json("doc", json!({ "name": "Bjarne" }), 0),
        ];
        let report = ingest.ingest_batch(&mut store, &records, &mut stats);
        assert_eq!(report.ingested, 2);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].0, 1);

        let written = ingest.write_dictionary(&mut store, &stats).unwrap();
        assert!(written >= 1);
    }
}
