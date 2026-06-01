use crate::types::RunState;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

pub fn empty_state_hash() -> String {
    stable_value_hash(&json!({ "run": null }))
}

pub fn hash_run_state(run: &RunState) -> String {
    let data = serde_json::to_value(run).expect("RunState serialization should be infallible");
    let mut state = Map::new();
    for field in STATE_HASH_FIELDS {
        state.insert(
            (*field).to_string(),
            data.get(*field).cloned().unwrap_or(Value::Null),
        );
    }
    stable_value_hash(&Value::Object(state))
}

pub fn stable_value_hash(value: &Value) -> String {
    let encoded = canonical_json(value);
    let mut hasher = Sha256::new();
    hasher.update(encoded.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => {
            serde_json::to_string(v).expect("string serialization should be infallible")
        }
        Value::Array(items) => {
            let rendered = items.iter().map(canonical_json).collect::<Vec<_>>();
            format!("[{}]", rendered.join(","))
        }
        Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            let rendered = keys
                .into_iter()
                .map(|key| {
                    let value = map.get(key).unwrap_or(&Value::Null);
                    format!(
                        "{}:{}",
                        canonical_json(&Value::String(key.clone())),
                        canonical_json(value)
                    )
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", rendered.join(","))
        }
    }
}

const STATE_HASH_FIELDS: &[&str] = &[
    "run_id",
    "task_signature",
    "status",
    "host",
    "profile",
    "toolkit",
    "context",
    "cache_events",
    "validators",
    "outcome",
    "learning_patches",
    "federation_signals",
    "last_event_seq",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Payload;
    use serde_json::json;

    #[test]
    fn canonical_hash_ignores_object_insertion_order() {
        let mut left = Payload::new();
        left.insert("b".to_string(), json!(2));
        left.insert("a".to_string(), json!(1));

        let mut right = Payload::new();
        right.insert("a".to_string(), json!(1));
        right.insert("b".to_string(), json!(2));

        let mut left_run = RunState::new("task", "codex", Payload::new());
        left_run.context = Some(left);
        let mut right_run = RunState::new("task", "codex", Payload::new());
        right_run.run_id = left_run.run_id.clone();
        right_run.context = Some(right);

        assert_eq!(hash_run_state(&left_run), hash_run_state(&right_run));
    }

    #[test]
    fn empty_hash_is_stable() {
        assert_eq!(
            empty_state_hash(),
            "82fa09fc2e7aed5e143d95b3059d2dde20fb5780a5d43edf8ffa7adf575636f6"
        );
    }
}
