//! Phase 1: the raw-record container and the data-type registry.
//!
//! DATAWAVE references:
//! - `warehouse/ingest-core/.../data/RawRecordContainer.java`: one raw-record
//!   abstraction carrying raw bytes, declared data-type, timestamp, visibility,
//!   and errors, used as the intake unit for every source.
//! - `warehouse/ingest-core/.../data/Type.java` + `TypeRegistry.java`: each
//!   source registers as a data-type with its helper, and the registry resolves
//!   a record to its type.
//!
//! A new source is therefore a registered data-type plus an ingest helper; the
//! index, edges, and dictionary all follow from the normalized fields it emits.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::helper::IngestHelper;

/// The body of a raw record, in whichever shape its source produced.
///
/// `Text` carries a delimited line or a raw blob (the CSV/plain-text path);
/// `Json` carries an already-structured record (the JSON / mapped path). The
/// helper for the record's data-type knows which body shape it expects.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum RecordBody {
    Text(String),
    Json(Value),
}

impl RecordBody {
    /// The bytes used for content-addressed dedup and fuzzy similarity.
    pub fn content_bytes(&self) -> Vec<u8> {
        match self {
            RecordBody::Text(s) => s.as_bytes().to_vec(),
            // Canonical serialization so two structurally-equal JSON records
            // hash identically regardless of input key order.
            RecordBody::Json(v) => canonical_json(v).into_bytes(),
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            RecordBody::Text(s) => Some(s.as_str()),
            RecordBody::Json(_) => None,
        }
    }

    pub fn as_json(&self) -> Option<&Value> {
        match self {
            RecordBody::Json(v) => Some(v),
            RecordBody::Text(_) => None,
        }
    }
}

/// The single intake entry point: source bytes/payload, declared data-type,
/// event time, and cell-level visibility, plus any parse errors carried with
/// the record rather than thrown away.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RawRecord {
    pub data_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    pub body: RecordBody,
    pub event_time_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

impl RawRecord {
    /// A text-bodied record (one CSV line, a log line, a raw blob).
    pub fn text(data_type: impl Into<String>, line: impl Into<String>, event_time_ms: i64) -> Self {
        Self {
            data_type: data_type.into(),
            external_id: None,
            body: RecordBody::Text(line.into()),
            event_time_ms,
            visibility: None,
            errors: Vec::new(),
        }
    }

    /// A structured record (a JSON object from an API, a flattened document).
    pub fn json(data_type: impl Into<String>, value: Value, event_time_ms: i64) -> Self {
        Self {
            data_type: data_type.into(),
            external_id: None,
            body: RecordBody::Json(value),
            event_time_ms,
            visibility: None,
            errors: Vec::new(),
        }
    }

    pub fn with_external_id(mut self, id: impl Into<String>) -> Self {
        self.external_id = Some(id.into());
        self
    }

    pub fn with_visibility(mut self, marking: impl Into<String>) -> Self {
        self.visibility = Some(marking.into());
        self
    }
}

/// The data-type registry: a source declares its ingest helper once, keyed by
/// the helper's `data_type()`, and the registry resolves a record to its helper.
#[derive(Default)]
pub struct TypeRegistry {
    helpers: BTreeMap<String, Box<dyn IngestHelper>>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a helper under its declared data-type name. A second helper for
    /// the same name replaces the first (last registration wins).
    pub fn register(&mut self, helper: Box<dyn IngestHelper>) -> &mut Self {
        self.helpers.insert(helper.data_type().to_string(), helper);
        self
    }

    pub fn resolve(&self, data_type: &str) -> Option<&dyn IngestHelper> {
        self.helpers.get(data_type).map(|boxed| boxed.as_ref())
    }

    pub fn data_types(&self) -> Vec<&str> {
        self.helpers.keys().map(String::as_str).collect()
    }

    pub fn len(&self) -> usize {
        self.helpers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.helpers.is_empty()
    }
}

/// Stable, key-sorted JSON serialization so structurally-equal records produce
/// identical content hashes. serde_json with BTreeMap-backed objects sorts keys.
pub(crate) fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<&String, &Value> = map.iter().collect();
            let parts: Vec<String> = sorted
                .iter()
                .map(|(k, v)| format!("{}:{}", serde_json::to_string(k).unwrap_or_default(), canonical_json(v)))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", parts.join(","))
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_json_is_key_order_independent() {
        let a = json!({ "b": 1, "a": 2 });
        let b = json!({ "a": 2, "b": 1 });
        assert_eq!(canonical_json(&a), canonical_json(&b));
    }

    #[test]
    fn record_bodies_expose_their_shape() {
        let t = RawRecord::text("csv", "a,b,c", 0);
        assert_eq!(t.body.as_text(), Some("a,b,c"));
        assert!(t.body.as_json().is_none());

        let j = RawRecord::json("json", json!({"x": 1}), 0);
        assert!(j.body.as_text().is_none());
        assert!(j.body.as_json().is_some());
    }
}
