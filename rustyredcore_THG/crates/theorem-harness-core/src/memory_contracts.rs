use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PrepareMemoryHydrationHandle {
    pub handle_id: String,
    pub handle_type: String,
    pub source: String,
    pub reason: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PrepareMemoryBank {
    pub bank_id: String,
    pub kind: String,
    pub scope: String,
    pub selector: String,
    #[serde(default)]
    pub rationale: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrepareMemoryRecallPolicy {
    pub policy_id: String,
    pub kind: String,
    #[serde(default)]
    pub scope_filters: Vec<String>,
    #[serde(default)]
    pub selected_banks: Vec<String>,
    #[serde(default)]
    pub bank_weights: BTreeMap<String, f64>,
    #[serde(default)]
    pub rationale: String,
    #[serde(default = "active_status")]
    pub status: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct PrepareMemoryRecallPreview {
    #[serde(default)]
    pub read_first: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub do_not: Vec<String>,
    #[serde(default)]
    pub next_actions: Vec<String>,
    #[serde(default)]
    pub hydration_handles: Vec<PrepareMemoryHydrationHandle>,
    #[serde(default)]
    pub recalled_evidence: Vec<String>,
    #[serde(default)]
    pub selected_banks: Vec<String>,
    #[serde(default)]
    pub recall_policy: Vec<String>,
    #[serde(default)]
    pub active_policy: Vec<String>,
    #[serde(default)]
    pub proposed_policy: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrepareMemoryEvidence {
    pub evidence_id: String,
    pub kind: String,
    pub source: String,
    pub immutable: bool,
    #[serde(default)]
    pub payload: Map<String, Value>,
    #[serde(default)]
    pub rationale: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrepareMemoryPolicy {
    pub policy_id: String,
    pub kind: String,
    pub scope: String,
    pub editable: bool,
    #[serde(default)]
    pub payload: Map<String, Value>,
    #[serde(default)]
    pub rationale: String,
    #[serde(default = "active_status")]
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct PrepareMemoryContract {
    #[serde(default)]
    pub evidence: Vec<PrepareMemoryEvidence>,
    #[serde(default)]
    pub operational_policy: Vec<PrepareMemoryPolicy>,
    #[serde(default)]
    pub memory_banks: Vec<PrepareMemoryBank>,
    #[serde(default)]
    pub evidence_hash: String,
    #[serde(default)]
    pub policy_hash: String,
    #[serde(default)]
    pub recall_policy: Option<PrepareMemoryRecallPolicy>,
    #[serde(default)]
    pub recall_preview: Option<PrepareMemoryRecallPreview>,
}

impl PrepareMemoryHydrationHandle {
    pub fn from_value(value: &Value) -> Self {
        let data = value.as_object().cloned().unwrap_or_default();
        Self {
            handle_id: string_or(&data, "handle_id", ""),
            handle_type: string_or(&data, "handle_type", ""),
            source: string_or(&data, "source", ""),
            reason: string_or(&data, "reason", ""),
            scope: string_or(&data, "scope", ""),
            status: string_or(&data, "status", ""),
        }
    }
}

impl PrepareMemoryBank {
    pub fn from_value(value: &Value) -> Self {
        let data = value.as_object().cloned().unwrap_or_default();
        Self {
            bank_id: string_or(&data, "bank_id", ""),
            kind: string_or(&data, "kind", ""),
            scope: string_or(&data, "scope", ""),
            selector: string_or(&data, "selector", ""),
            rationale: string_or(&data, "rationale", ""),
        }
    }
}

impl PrepareMemoryRecallPolicy {
    pub fn from_value(value: &Value) -> Self {
        let data = value.as_object().cloned().unwrap_or_default();
        Self {
            policy_id: string_or(&data, "policy_id", ""),
            kind: string_or(&data, "kind", ""),
            scope_filters: string_vec(data.get("scope_filters")),
            selected_banks: string_vec(data.get("selected_banks")),
            bank_weights: float_map(data.get("bank_weights")),
            rationale: string_or(&data, "rationale", ""),
            status: string_or(&data, "status", "active"),
        }
    }
}

impl PrepareMemoryRecallPreview {
    pub fn from_value(value: &Value) -> Self {
        let data = value.as_object().cloned().unwrap_or_default();
        Self {
            read_first: string_vec(data.get("read_first")),
            risks: string_vec(data.get("risks")),
            do_not: string_vec(data.get("do_not")),
            next_actions: string_vec(data.get("next_actions")),
            hydration_handles: data
                .get("hydration_handles")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|item| item.is_object())
                .map(PrepareMemoryHydrationHandle::from_value)
                .collect(),
            recalled_evidence: string_vec(data.get("recalled_evidence")),
            selected_banks: string_vec(data.get("selected_banks")),
            recall_policy: string_vec(data.get("recall_policy")),
            active_policy: string_vec(data.get("active_policy")),
            proposed_policy: string_vec(data.get("proposed_policy")),
        }
    }
}

impl PrepareMemoryEvidence {
    pub fn from_value(value: &Value) -> Self {
        let data = value.as_object().cloned().unwrap_or_default();
        Self {
            evidence_id: string_or(&data, "evidence_id", ""),
            kind: string_or(&data, "kind", ""),
            source: string_or(&data, "source", ""),
            immutable: bool_or(&data, "immutable", true),
            payload: object_or_empty(&data, "payload"),
            rationale: string_or(&data, "rationale", ""),
        }
    }
}

impl PrepareMemoryPolicy {
    pub fn from_value(value: &Value) -> Self {
        let data = value.as_object().cloned().unwrap_or_default();
        Self {
            policy_id: string_or(&data, "policy_id", ""),
            kind: string_or(&data, "kind", ""),
            scope: string_or(&data, "scope", ""),
            editable: bool_or(&data, "editable", true),
            payload: object_or_empty(&data, "payload"),
            rationale: string_or(&data, "rationale", ""),
            status: string_or(&data, "status", "active"),
        }
    }
}

impl PrepareMemoryContract {
    pub fn from_value(value: &Value) -> Self {
        let data = value.as_object().cloned().unwrap_or_default();
        Self {
            evidence: data
                .get("evidence")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|item| item.is_object())
                .map(PrepareMemoryEvidence::from_value)
                .collect(),
            operational_policy: data
                .get("operational_policy")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|item| item.is_object())
                .map(PrepareMemoryPolicy::from_value)
                .collect(),
            memory_banks: data
                .get("memory_banks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|item| item.is_object())
                .map(PrepareMemoryBank::from_value)
                .collect(),
            evidence_hash: string_or(&data, "evidence_hash", ""),
            policy_hash: string_or(&data, "policy_hash", ""),
            recall_policy: data
                .get("recall_policy")
                .filter(|item| item.is_object())
                .map(PrepareMemoryRecallPolicy::from_value),
            recall_preview: data
                .get("recall_preview")
                .filter(|item| item.is_object())
                .map(PrepareMemoryRecallPreview::from_value),
        }
    }
}

fn string_or(data: &Map<String, Value>, key: &str, fallback: &str) -> String {
    match data.get(key) {
        None | Some(Value::Null) => fallback.to_string(),
        Some(Value::String(text)) => {
            if text.is_empty() {
                fallback.to_string()
            } else {
                text.to_string()
            }
        }
        Some(Value::Bool(value)) => {
            if *value {
                "True".to_string()
            } else {
                fallback.to_string()
            }
        }
        Some(Value::Number(number)) => {
            if number.as_f64().is_some_and(|value| value == 0.0) {
                fallback.to_string()
            } else {
                number.to_string()
            }
        }
        Some(Value::Array(items)) => {
            if items.is_empty() {
                fallback.to_string()
            } else {
                Value::Array(items.clone()).to_string()
            }
        }
        Some(Value::Object(object)) => {
            if object.is_empty() {
                fallback.to_string()
            } else {
                Value::Object(object.clone()).to_string()
            }
        }
    }
}

fn bool_or(data: &Map<String, Value>, key: &str, missing_default: bool) -> bool {
    match data.get(key) {
        None => missing_default,
        Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|value| value != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(object)) => !object.is_empty(),
    }
}

fn string_vec(value: Option<&Value>) -> Vec<String> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(text)) => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![text.to_string()]
            }
        }
        Some(Value::Array(items)) => items
            .iter()
            .map(value_to_string)
            .filter(|item| !item.is_empty())
            .collect(),
        Some(other) => {
            let text = value_to_string(other);
            if text.is_empty() {
                Vec::new()
            } else {
                vec![text]
            }
        }
    }
}

fn float_map(value: Option<&Value>) -> BTreeMap<String, f64> {
    let mut weights = BTreeMap::new();
    let Some(map) = value.and_then(Value::as_object) else {
        return weights;
    };
    for (bank, raw_weight) in map {
        let weight = match raw_weight {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.parse::<f64>().ok(),
            _ => None,
        };
        if let Some(weight) = weight {
            if weight >= 0.0 {
                weights.insert(bank.trim().to_lowercase(), weight);
            }
        }
    }
    weights
}

fn object_or_empty(data: &Map<String, Value>, key: &str) -> Map<String, Value> {
    data.get(key)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn active_status() -> String {
    "active".to_string()
}
