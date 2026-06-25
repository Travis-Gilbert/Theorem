use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigState {
    #[serde(default)]
    pub values: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_version_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigValueDelta {
    pub key: String,
    pub before: Value,
    pub after: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigDelta {
    pub delta_id: String,
    pub description: String,
    pub values: Vec<ConfigValueDelta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_version_before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_version_after: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigDeltaStatus {
    Active,
    RolledBack,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppliedConfigDelta {
    pub delta_id: String,
    pub forward_delta: ConfigDelta,
    pub inverse_delta: ConfigDelta,
    pub status: ConfigDeltaStatus,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigLedger {
    pub entries: Vec<AppliedConfigDelta>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RollbackReceipt {
    pub delta_id: String,
    pub restored_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_version_to_checkout: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigLedgerError {
    DeltaAlreadyApplied(String),
    DeltaNotFound(String),
    DeltaAlreadyRolledBack(String),
    PreconditionFailed { key: String },
}

impl fmt::Display for ConfigLedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigLedgerError::DeltaAlreadyApplied(id) => {
                write!(f, "config delta already applied: {id}")
            }
            ConfigLedgerError::DeltaNotFound(id) => write!(f, "config delta not found: {id}"),
            ConfigLedgerError::DeltaAlreadyRolledBack(id) => {
                write!(f, "config delta already rolled back: {id}")
            }
            ConfigLedgerError::PreconditionFailed { key } => {
                write!(f, "config delta precondition failed for key: {key}")
            }
        }
    }
}

impl Error for ConfigLedgerError {}

impl ConfigDelta {
    pub fn inverse(&self) -> Self {
        Self {
            delta_id: format!("{}:inverse", self.delta_id),
            description: format!("inverse of {}", self.delta_id),
            values: self
                .values
                .iter()
                .map(|value| ConfigValueDelta {
                    key: value.key.clone(),
                    before: value.after.clone(),
                    after: value.before.clone(),
                })
                .collect(),
            graph_version_before: self.graph_version_after.clone(),
            graph_version_after: self.graph_version_before.clone(),
        }
    }
}

impl ConfigLedger {
    pub fn apply_delta(
        &mut self,
        state: &mut ConfigState,
        delta: ConfigDelta,
    ) -> Result<(), ConfigLedgerError> {
        if self
            .entries
            .iter()
            .any(|entry| entry.delta_id == delta.delta_id)
        {
            return Err(ConfigLedgerError::DeltaAlreadyApplied(delta.delta_id));
        }

        apply_values(state, &delta.values)?;
        state.graph_version_id = delta.graph_version_after.clone();

        self.entries.push(AppliedConfigDelta {
            delta_id: delta.delta_id.clone(),
            inverse_delta: delta.inverse(),
            forward_delta: delta,
            status: ConfigDeltaStatus::Active,
        });
        Ok(())
    }

    pub fn rollback(
        &mut self,
        state: &mut ConfigState,
        delta_id: &str,
    ) -> Result<RollbackReceipt, ConfigLedgerError> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.delta_id == delta_id)
            .ok_or_else(|| ConfigLedgerError::DeltaNotFound(delta_id.to_string()))?;
        if entry.status == ConfigDeltaStatus::RolledBack {
            return Err(ConfigLedgerError::DeltaAlreadyRolledBack(
                delta_id.to_string(),
            ));
        }

        apply_values(state, &entry.inverse_delta.values)?;
        state.graph_version_id = entry.inverse_delta.graph_version_after.clone();
        entry.status = ConfigDeltaStatus::RolledBack;

        Ok(RollbackReceipt {
            delta_id: delta_id.to_string(),
            restored_keys: entry
                .inverse_delta
                .values
                .iter()
                .map(|value| value.key.clone())
                .collect(),
            graph_version_to_checkout: entry.inverse_delta.graph_version_after.clone(),
        })
    }
}

fn apply_values(
    state: &mut ConfigState,
    values: &[ConfigValueDelta],
) -> Result<(), ConfigLedgerError> {
    for value in values {
        let current = state.values.get(&value.key).unwrap_or(&Value::Null);
        if current != &value.before {
            return Err(ConfigLedgerError::PreconditionFailed {
                key: value.key.clone(),
            });
        }
    }

    for value in values {
        if value.after.is_null() {
            state.values.remove(&value.key);
        } else {
            state.values.insert(value.key.clone(), value.after.clone());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn apply_then_rollback_restores_config_and_graph_version() {
        let mut state = ConfigState {
            values: BTreeMap::from([("routing.weight.codex".to_string(), json!(0.4))]),
            graph_version_id: Some("graph:v1".to_string()),
        };
        let original = serde_json::to_vec(&state).unwrap();
        let mut ledger = ConfigLedger::default();
        let delta = ConfigDelta {
            delta_id: "delta:1".to_string(),
            description: "raise codex routing weight".to_string(),
            values: vec![ConfigValueDelta {
                key: "routing.weight.codex".to_string(),
                before: json!(0.4),
                after: json!(0.6),
            }],
            graph_version_before: Some("graph:v1".to_string()),
            graph_version_after: Some("graph:v2".to_string()),
        };

        ledger.apply_delta(&mut state, delta).unwrap();
        assert_eq!(state.values.get("routing.weight.codex"), Some(&json!(0.6)));
        assert_eq!(state.graph_version_id.as_deref(), Some("graph:v2"));

        let receipt = ledger.rollback(&mut state, "delta:1").unwrap();

        assert_eq!(
            receipt.graph_version_to_checkout.as_deref(),
            Some("graph:v1")
        );
        assert_eq!(serde_json::to_vec(&state).unwrap(), original);
    }
}
