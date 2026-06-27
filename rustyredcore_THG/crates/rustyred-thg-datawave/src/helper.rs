//! Phase 2 (keystone) + phase 4: the ingest-helper contract and the shared
//! derive pipeline that turns one record into typed normalized field-facts,
//! plus the three worked data-types.
//!
//! DATAWAVE references:
//! - `IngestHelperInterface` / `BaseIngestHelper`: the central contract that
//!   turns one record into a multimap of normalized field-and-value units, with
//!   index policy per field.
//! - `warehouse/ingest-csv`, `warehouse/ingest-json`: the worked data-types.
//!
//! The thesis is "no bespoke loader per source": a helper only implements
//! `extract` (the source-specific parse). Aliasing, normalization, composite,
//! virtual, and masking all run in the shared `derive_fields` pipeline, so a new
//! source is a small `extract` plus a `FieldConfig`, not a new ingest stack.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

use crate::field::{FieldConfig, FieldOrigin, FieldType, NormalizedField};
use crate::record::RawRecord;

/// A record that could not be parsed into raw (name, value) pairs. The driver
/// records it on the record's error trail and skips the record; one bad record
/// never aborts a batch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum IngestError {
    Extract(String),
}

impl fmt::Display for IngestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IngestError::Extract(msg) => write!(f, "extract: {msg}"),
        }
    }
}

impl std::error::Error for IngestError {}

/// The record-to-field-facts contract. Implement `extract` and supply a
/// `field_config`; `event_fields` runs the shared derive pipeline for free.
pub trait IngestHelper {
    fn data_type(&self) -> &str;

    fn field_config(&self) -> &FieldConfig;

    /// Pull raw (external-name, value) pairs from one record, before aliasing or
    /// normalization. This is the only source-specific code a new data-type adds.
    fn extract(&self, record: &RawRecord) -> Result<Vec<(String, String)>, IngestError>;

    /// One record in, normalized field-facts out (DATAWAVE getEventFields).
    fn event_fields(&self, record: &RawRecord) -> Result<Vec<NormalizedField>, IngestError> {
        let pairs = self.extract(record)?;
        Ok(derive_fields(self.field_config(), &pairs, record.visibility.as_deref()))
    }
}

/// The shared derive pipeline: alias -> normalize -> mask for extracted fields,
/// then composite and virtual derivation. A field whose value fails its declared
/// normalizer is dropped (DATAWAVE tolerance); the record still ingests.
pub fn derive_fields(
    config: &FieldConfig,
    raw_pairs: &[(String, String)],
    visibility: Option<&str>,
) -> Vec<NormalizedField> {
    let mut fields: Vec<NormalizedField> = Vec::with_capacity(raw_pairs.len());

    for (external, raw_value) in raw_pairs {
        let internal = config.resolve_alias(external).to_string();
        let field_type = config.field_type(&internal);
        let normalized = match field_type.normalize(raw_value) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let masked = config.mask_rule(&internal).map(|rule| rule.apply(raw_value));
        fields.push(NormalizedField {
            field: internal,
            raw_value: raw_value.clone(),
            normalized,
            group: None,
            visibility: visibility.map(str::to_string),
            masked,
            policy: config.policy(config.resolve_alias(external)),
            field_type,
            origin: FieldOrigin::Extracted,
        });
    }

    // All normalized values per internal field name. DATAWAVE composites/virtuals
    // combine across every value of a multi-valued field, not just the first.
    let mut values_by_field: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for field in &fields {
        values_by_field
            .entry(field.field.as_str())
            .or_default()
            .push(field.normalized.as_str());
    }

    let mut derived: Vec<NormalizedField> = Vec::new();

    // Composite: Cartesian product across all sources' values (DATAWAVE
    // GroupingPolicy.IGNORE_GROUPS), one compound key per combination.
    for composite in config.composites() {
        let source_values: Option<Vec<&Vec<&str>>> = composite
            .sources
            .iter()
            .map(|src| values_by_field.get(src.as_str()))
            .collect();
        let Some(source_values) = source_values else {
            continue;
        };
        for combo in cartesian(&source_values) {
            let value = combo.join(&composite.separator);
            derived.push(NormalizedField {
                field: composite.name.clone(),
                raw_value: value.clone(),
                normalized: value,
                group: None,
                visibility: visibility.map(str::to_string),
                masked: None,
                policy: composite.policy,
                field_type: FieldType::Text,
                origin: FieldOrigin::Composite,
            });
        }
    }

    // Virtual: transform each of the source field's values.
    for virtual_def in config.virtuals() {
        let Some(source_values) = values_by_field.get(virtual_def.source.as_str()) else {
            continue;
        };
        for source_value in source_values {
            if let Some(transformed) = virtual_def.transform.apply(source_value) {
                derived.push(NormalizedField {
                    field: virtual_def.name.clone(),
                    raw_value: source_value.to_string(),
                    normalized: transformed,
                    group: None,
                    visibility: visibility.map(str::to_string),
                    masked: None,
                    policy: virtual_def.policy,
                    field_type: FieldType::Text,
                    origin: FieldOrigin::Virtual,
                });
            }
        }
    }

    fields.extend(derived);
    fields
}

/// Cartesian product of per-source value lists, preserving source order. An empty
/// list yields no combinations.
fn cartesian<'a>(lists: &[&Vec<&'a str>]) -> Vec<Vec<&'a str>> {
    let mut result: Vec<Vec<&str>> = vec![Vec::new()];
    for list in lists {
        let mut next = Vec::with_capacity(result.len() * list.len());
        for prefix in &result {
            for value in list.iter() {
                let mut combo = prefix.clone();
                combo.push(value);
                next.push(combo);
            }
        }
        result = next;
    }
    result
}

// ---- CSV data-type (warehouse/ingest-csv) ----

/// Maps one delimited line to fields by a configured column header list, the way
/// DATAWAVE's CSVIngestHelper gets field names from configuration rather than a
/// header row.
pub struct CsvHelper {
    data_type: String,
    columns: Vec<String>,
    delimiter: char,
    process_extra_fields: bool,
    config: FieldConfig,
}

impl CsvHelper {
    pub fn new(
        data_type: impl Into<String>,
        columns: impl IntoIterator<Item = impl Into<String>>,
        config: FieldConfig,
    ) -> Self {
        Self {
            data_type: data_type.into(),
            columns: columns.into_iter().map(Into::into).collect(),
            delimiter: ',',
            process_extra_fields: false,
            config,
        }
    }

    pub fn with_delimiter(mut self, delimiter: char) -> Self {
        self.delimiter = delimiter;
        self
    }

    /// Parse trailing `name=value` tokens beyond the configured columns as their
    /// own fields (DATAWAVE `processExtraFields`).
    pub fn with_extra_fields(mut self) -> Self {
        self.process_extra_fields = true;
        self
    }
}

impl IngestHelper for CsvHelper {
    fn data_type(&self) -> &str {
        &self.data_type
    }

    fn field_config(&self) -> &FieldConfig {
        &self.config
    }

    fn extract(&self, record: &RawRecord) -> Result<Vec<(String, String)>, IngestError> {
        let line = record
            .body
            .as_text()
            .ok_or_else(|| IngestError::Extract("csv helper expects a text record body".into()))?;
        let values = split_delimited(line, self.delimiter);
        // Positional columns; a row shorter than the header drops the missing
        // tail, a row longer keeps the extras for name=value parsing below.
        let mut pairs: Vec<(String, String)> = self
            .columns
            .iter()
            .zip(values.iter())
            .map(|(col, val)| (col.clone(), val.clone()))
            .collect();
        if self.process_extra_fields && values.len() > self.columns.len() {
            for extra in &values[self.columns.len()..] {
                if let Some((name, val)) = extra.split_once('=') {
                    pairs.push((name.to_string(), val.to_string()));
                }
            }
        }
        Ok(pairs)
    }
}

/// Quote-aware delimited split: a field wrapped in double quotes may contain the
/// delimiter, and `""` inside a quoted field is a literal quote.
fn split_delimited(line: &str, delimiter: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == delimiter {
            out.push(std::mem::take(&mut field));
        } else {
            field.push(c);
        }
    }
    out.push(field);
    out
}

// ---- JSON data-type (warehouse/ingest-json) ----

/// Flattens a nested JSON record to dotted field names. Nested objects join with
/// the delimiter (`a.b`); array elements append their index (`a.b.0`). Null
/// scalars produce no field.
pub struct JsonHelper {
    data_type: String,
    delimiter: String,
    uppercase_keys: bool,
    config: FieldConfig,
}

impl JsonHelper {
    pub fn new(data_type: impl Into<String>, config: FieldConfig) -> Self {
        Self {
            data_type: data_type.into(),
            delimiter: ".".to_string(),
            uppercase_keys: false,
            config,
        }
    }

    pub fn with_delimiter(mut self, delimiter: impl Into<String>) -> Self {
        self.delimiter = delimiter.into();
        self
    }

    /// Upper-case object keys in the flattened field names (DATAWAVE
    /// canonicalizes all field names to upper-case at ingest).
    pub fn with_uppercase_keys(mut self) -> Self {
        self.uppercase_keys = true;
        self
    }
}

impl IngestHelper for JsonHelper {
    fn data_type(&self) -> &str {
        &self.data_type
    }

    fn field_config(&self) -> &FieldConfig {
        &self.config
    }

    fn extract(&self, record: &RawRecord) -> Result<Vec<(String, String)>, IngestError> {
        let value = record
            .body
            .as_json()
            .ok_or_else(|| IngestError::Extract("json helper expects a json record body".into()))?;
        let mut pairs = Vec::new();
        flatten_json(value, "", &self.delimiter, self.uppercase_keys, &mut pairs);
        Ok(pairs)
    }
}

/// DATAWAVE NORMAL-mode flatten: objects descend by key; array *primitives*
/// share the parent key (producing a multi-valued field), while array members
/// that are objects or arrays descend with their index appended.
fn flatten_json(value: &Value, prefix: &str, delimiter: &str, uppercase: bool, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let key = if uppercase { key.to_uppercase() } else { key.clone() };
                let next = join_path(prefix, &key, delimiter);
                flatten_json(child, &next, delimiter, uppercase, out);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                match child {
                    Value::Null => {}
                    Value::Object(_) | Value::Array(_) => {
                        let next = join_path(prefix, &index.to_string(), delimiter);
                        flatten_json(child, &next, delimiter, uppercase, out);
                    }
                    scalar => {
                        if !prefix.is_empty() {
                            out.push((prefix.to_string(), scalar_to_string(scalar)));
                        }
                    }
                }
            }
        }
        Value::Null => {}
        scalar => {
            if !prefix.is_empty() {
                out.push((prefix.to_string(), scalar_to_string(scalar)));
            }
        }
    }
}

fn join_path(prefix: &str, segment: &str, delimiter: &str) -> String {
    if prefix.is_empty() {
        segment.to_string()
    } else {
        format!("{prefix}{delimiter}{segment}")
    }
}

fn scalar_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => value.to_string(),
    }
}

// ---- Mapped data-type: the universal config-driven path ----

/// One mapping rule: read a dotted JSON path out of the record and emit it under
/// a target field name.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FieldMapRule {
    pub source_path: String,
    pub target_field: String,
}

impl FieldMapRule {
    pub fn new(source_path: impl Into<String>, target_field: impl Into<String>) -> Self {
        Self {
            source_path: source_path.into(),
            target_field: target_field.into(),
        }
    }
}

/// The universal path: a list of path->field rules drives extraction from data,
/// so any source describable as "these JSON paths are these fields" becomes a
/// data-type with no bespoke Rust. This is how a URL, a repo manifest, or an API
/// response is pointed at and ingested by configuration alone.
pub struct MappedHelper {
    data_type: String,
    rules: Vec<FieldMapRule>,
    delimiter: String,
    config: FieldConfig,
}

impl MappedHelper {
    pub fn new(
        data_type: impl Into<String>,
        rules: impl IntoIterator<Item = FieldMapRule>,
        config: FieldConfig,
    ) -> Self {
        Self {
            data_type: data_type.into(),
            rules: rules.into_iter().collect(),
            delimiter: ".".to_string(),
            config,
        }
    }

    pub fn with_delimiter(mut self, delimiter: impl Into<String>) -> Self {
        self.delimiter = delimiter.into();
        self
    }
}

impl IngestHelper for MappedHelper {
    fn data_type(&self) -> &str {
        &self.data_type
    }

    fn field_config(&self) -> &FieldConfig {
        &self.config
    }

    fn extract(&self, record: &RawRecord) -> Result<Vec<(String, String)>, IngestError> {
        let value = record
            .body
            .as_json()
            .ok_or_else(|| IngestError::Extract("mapped helper expects a json record body".into()))?;
        let mut pairs = Vec::new();
        for rule in &self.rules {
            if let Some(found) = lookup_path(value, &rule.source_path, &self.delimiter) {
                pairs.push((rule.target_field.clone(), found));
            }
        }
        Ok(pairs)
    }
}

/// Walk a delimited path into a JSON value, descending objects by key and arrays
/// by numeric index. Returns the scalar at the path, if any.
fn lookup_path(value: &Value, path: &str, delimiter: &str) -> Option<String> {
    let mut current = value;
    for segment in path.split(delimiter) {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => {
                let index: usize = segment.parse().ok()?;
                items.get(index)?
            }
            _ => return None,
        };
    }
    match current {
        Value::Null | Value::Object(_) | Value::Array(_) => None,
        scalar => Some(scalar_to_string(scalar)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::IndexPolicy;
    use serde_json::json;

    fn ip_config() -> FieldConfig {
        FieldConfig::new().with_field("ip", FieldType::Ip, IndexPolicy::INDEXED)
    }

    #[test]
    fn csv_splits_with_quotes_and_normalizes() {
        let helper = CsvHelper::new("net", ["ip", "note"], ip_config());
        let record = RawRecord::text("net", "1.2.3.4,\"hello, world\"", 0);
        let fields = helper.event_fields(&record).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].field, "ip");
        assert_eq!(fields[0].normalized, "001.002.003.004");
        assert_eq!(fields[1].field, "note");
        assert_eq!(fields[1].raw_value, "hello, world");
    }

    #[test]
    fn json_flattens_nested_and_shares_array_primitive_key() {
        let helper = JsonHelper::new("doc", FieldConfig::new());
        let record = RawRecord::json("doc", json!({"a": {"b": 1}, "tags": ["x", "y"]}), 0);
        let pairs = helper.extract(&record).unwrap();
        assert!(pairs.contains(&("a.b".to_string(), "1".to_string())));
        // Array primitives share the parent key (a multi-valued field), no index.
        assert!(pairs.contains(&("tags".to_string(), "x".to_string())));
        assert!(pairs.contains(&("tags".to_string(), "y".to_string())));
        assert!(!pairs.iter().any(|(k, _)| k == "tags.0"));
    }

    #[test]
    fn json_uppercases_keys_when_asked() {
        let helper = JsonHelper::new("doc", FieldConfig::new()).with_uppercase_keys();
        let record = RawRecord::json("doc", json!({"User": {"Email": "a@b.com"}}), 0);
        let pairs = helper.extract(&record).unwrap();
        assert!(pairs.contains(&("USER.EMAIL".to_string(), "a@b.com".to_string())));
    }

    #[test]
    fn mapped_selects_paths_into_named_fields() {
        let rules = [
            FieldMapRule::new("user.email", "email"),
            FieldMapRule::new("items.0.sku", "first_sku"),
        ];
        let helper = MappedHelper::new("api", rules, FieldConfig::new());
        let record = RawRecord::json(
            "api",
            json!({"user": {"email": "A@B.com"}, "items": [{"sku": "Z1"}]}),
            0,
        );
        let pairs = helper.extract(&record).unwrap();
        assert_eq!(pairs, vec![
            ("email".to_string(), "A@B.com".to_string()),
            ("first_sku".to_string(), "Z1".to_string()),
        ]);
    }

    #[test]
    fn composites_and_virtuals_expand_every_value() {
        use crate::field::{CompositeDef, FieldType, VirtualDef, VirtualTransform};
        let cfg = FieldConfig::new()
            .with_field("tag", FieldType::Text, IndexPolicy::INDEXED)
            .with_field("env", FieldType::Text, IndexPolicy::INDEXED)
            .with_virtual(VirtualDef {
                name: "tag_v".into(),
                source: "tag".into(),
                transform: VirtualTransform::Copy,
                policy: IndexPolicy::INDEXED,
            })
            .with_composite(CompositeDef::new("tag_env", ["tag", "env"]).with_separator("|"));
        let helper = JsonHelper::new("doc", cfg);
        // tag is multi-valued (array primitives share the key).
        let record = RawRecord::json("doc", json!({ "tag": ["x", "y"], "env": "prod" }), 0);
        let fields = helper.event_fields(&record).unwrap();

        let virt: Vec<&str> = fields.iter().filter(|f| f.field == "tag_v").map(|f| f.normalized.as_str()).collect();
        assert!(virt.contains(&"x") && virt.contains(&"y"), "virtual covers every source value: {virt:?}");

        let comp: Vec<&str> = fields.iter().filter(|f| f.field == "tag_env").map(|f| f.normalized.as_str()).collect();
        assert!(comp.contains(&"x|prod") && comp.contains(&"y|prod"), "composite is the cross-product: {comp:?}");
    }
}
