use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

const FORBIDDEN_KEYS: &[&str] = &[
    "prompt",
    "raw_prompt",
    "system_prompt",
    "user_prompt",
    "code",
    "source_code",
    "raw_code",
    "body",
    "raw_body",
    "tenant_id",
    "source_tenant",
    "owner_user_id",
    "user_email",
    "user_name",
    "full_name",
    "harness_run_id",
    "patch_id",
    "blueprint_id",
    "source_blueprint_id",
    "target_blueprint_id",
    "created_at",
    "decided_at",
    "started_at",
    "finished_at",
    "computed_at",
    "updated_at",
    "why",
    "reason",
    "drafted_prose",
    "explanation",
    "note",
    "evidence_run_ids",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FederatedSignal {
    pub kind: String,
    pub edge_types: Vec<String>,
    pub success_rate_bucket: String,
    pub observed_count: i64,
    pub visual_class: String,
    pub suggestion_type: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StructuralSignalInput {
    #[serde(default = "agent_kind")]
    pub kind: String,
    #[serde(default)]
    pub structural_signal: Map<String, Value>,
    #[serde(default)]
    pub evidence_count: i64,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub visual_class: String,
    #[serde(default)]
    pub suggestion_type: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivacyViolation {
    pub message: String,
}

impl PrivacyViolation {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for PrivacyViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for PrivacyViolation {}

pub fn assert_no_raw_content(signal: &Value) -> Result<(), PrivacyViolation> {
    assert_no_raw_content_at(signal, "")
}

pub fn success_rate_bucket(rate: f64) -> String {
    if rate >= 0.66 {
        "high".to_string()
    } else if rate >= 0.33 {
        "medium".to_string()
    } else {
        "low".to_string()
    }
}

pub fn observed_count_bucket(count: i64) -> i64 {
    if count <= 0 {
        return 0;
    }
    let rounded = python_round_half_even(count as f64 / 10.0) * 10;
    if rounded == 0 {
        10
    } else {
        rounded
    }
}

pub fn extract_structural_signal(
    input: StructuralSignalInput,
) -> Result<FederatedSignal, PrivacyViolation> {
    let mut edge_types = BTreeSet::new();
    for (key, value) in &input.structural_signal {
        if key.ends_with("_ids") && value.as_array().is_some_and(|items| !items.is_empty()) {
            edge_types.insert(key.trim_end_matches("_ids").to_string());
        }
    }
    let signal = FederatedSignal {
        kind: if input.kind.is_empty() {
            agent_kind()
        } else {
            input.kind
        },
        edge_types: edge_types.into_iter().collect(),
        success_rate_bucket: success_rate_bucket(input.confidence),
        observed_count: observed_count_bucket(input.evidence_count),
        visual_class: input.visual_class,
        suggestion_type: input.suggestion_type,
    };
    assert_no_raw_content(&serde_json::to_value(&signal).expect("signal should serialize"))?;
    Ok(signal)
}

pub fn receive_federated_signal(signal: &Value) -> Result<FederatedSignal, PrivacyViolation> {
    let object = signal.as_object().ok_or_else(|| {
        PrivacyViolation::new(format!(
            "Federated signal must be a dict; got {}.",
            value_type(signal)
        ))
    })?;
    assert_no_raw_content(signal)?;
    Ok(FederatedSignal {
        kind: string_field(object, "kind", "agent"),
        edge_types: string_array_field(object, "edge_types"),
        success_rate_bucket: string_field(object, "success_rate_bucket", "low"),
        observed_count: int_field(object, "observed_count", 0),
        visual_class: string_field(object, "visual_class", ""),
        suggestion_type: string_field(object, "suggestion_type", ""),
    })
}

fn assert_no_raw_content_at(signal: &Value, path: &str) -> Result<(), PrivacyViolation> {
    match signal {
        Value::Object(map) => {
            for (key, value) in map {
                let key_str = key.to_lowercase();
                if FORBIDDEN_KEYS.contains(&key_str.as_str()) {
                    return Err(PrivacyViolation::new(format!(
                        "Forbidden key {key:?} at {}; federated structural signals must not carry raw content or tenant identifiers.",
                        if path.is_empty() { "<root>" } else { path }
                    )));
                }
                let next_path = if path.is_empty() {
                    format!(".{key}")
                } else {
                    format!("{path}.{key}")
                };
                assert_no_raw_content_at(value, &next_path)?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                assert_no_raw_content_at(item, &format!("{path}[{idx}]"))?;
            }
            Ok(())
        }
        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => Ok(()),
    }
}

fn python_round_half_even(value: f64) -> i64 {
    let floor = value.floor();
    let fraction = value - floor;
    if (fraction - 0.5).abs() < f64::EPSILON {
        let floor_i = floor as i64;
        if floor_i % 2 == 0 {
            floor_i
        } else {
            floor_i + 1
        }
    } else {
        value.round() as i64
    }
}

fn string_field(data: &Map<String, Value>, key: &str, fallback: &str) -> String {
    data.get(key)
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

fn string_array_field(data: &Map<String, Value>, key: &str) -> Vec<String> {
    data.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| item.as_str().unwrap_or("").to_string())
        .collect()
}

fn int_field(data: &Map<String, Value>, key: &str, fallback: i64) -> i64 {
    match data.get(key) {
        Some(Value::Number(number)) => number.as_i64().unwrap_or(fallback),
        Some(Value::String(text)) => text.parse::<i64>().unwrap_or(fallback),
        _ => fallback,
    }
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

fn agent_kind() -> String {
    "agent".to_string()
}
