//! UserModel: the explicit, content-addressed snapshot of "what the agent has
//! learned about the user". Mounted onto the binding's working memory at
//! `MEMORY_SCOPE.MOUNTED` (before HEADS.CONTRIBUTE fires), so every head reads
//! the same picture of the user when it composes its turn.
//!
//! The model is intentionally minimal and serde-friendly: preferences are a
//! deterministic key/value map; notes carry timestamps; recent_focus and
//! working_on point at graph nodes (or any opaque ref) for the heads to expand
//! into context if they need to.
//!
//! Back-compat: every field defaults, and `UserModel::default()` is the empty
//! model. A binding mount with no `user_model` payload behaves exactly as it
//! did before this module existed.
//!
//! Stability: `user_model_hash` is a SHA-256 over the canonical JSON form of
//! the model, so the same content always hashes to the same string regardless
//! of map insertion order (matches the existing `stable_value_hash` contract).

use crate::state_hash::stable_value_hash;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A single dated note about the user (style preferences, frustrations, etc.).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserModelNote {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub recorded_at: String,
}

impl UserModelNote {
    pub fn new(text: impl Into<String>, recorded_at: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            recorded_at: recorded_at.into(),
        }
    }
}

/// A pointer at something the user has been engaging with recently.
/// `ref_id` is opaque (graph node id, file path, conversation id — caller picks).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserModelReference {
    #[serde(default)]
    pub ref_id: String,
    #[serde(default)]
    pub label: String,
}

impl UserModelReference {
    pub fn new(ref_id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            ref_id: ref_id.into(),
            label: label.into(),
        }
    }
}

/// A project the user is actively working on.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserModelProjectRef {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
}

impl UserModelProjectRef {
    pub fn new(
        project_id: impl Into<String>,
        name: impl Into<String>,
        status: impl Into<String>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            name: name.into(),
            status: status.into(),
        }
    }
}

/// What the agent currently knows about the user. Mounted onto the binding's
/// scratchpad at MEMORY_SCOPE.MOUNTED so heads share a single picture.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserModel {
    #[serde(default)]
    pub preferences: BTreeMap<String, String>,
    #[serde(default)]
    pub style_notes: Vec<UserModelNote>,
    #[serde(default)]
    pub recent_focus: Vec<UserModelReference>,
    #[serde(default)]
    pub open_frustrations: Vec<UserModelNote>,
    #[serde(default)]
    pub working_on: Vec<UserModelProjectRef>,
}

impl UserModel {
    /// Whether this model has any signal at all. `MEMORY_SCOPE.MOUNTED` still
    /// fires for an empty model — but callers may use this to decide whether
    /// to emit a `user_model` payload field in the first place.
    pub fn is_empty(&self) -> bool {
        self.preferences.is_empty()
            && self.style_notes.is_empty()
            && self.recent_focus.is_empty()
            && self.open_frustrations.is_empty()
            && self.working_on.is_empty()
    }
}

/// Content-addressed hash of a UserModel. Same bytes -> same hash, regardless
/// of map insertion order. Uses the kernel's `stable_value_hash` so it ties
/// into the same content-addressing convention every other receipt uses.
pub fn user_model_hash(model: &UserModel) -> String {
    let value: Value =
        serde_json::to_value(model).expect("UserModel serialization should be infallible");
    stable_value_hash(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_are_empty() {
        let model = UserModel::default();
        assert!(model.is_empty());
        assert!(model.preferences.is_empty());
        assert!(model.style_notes.is_empty());
        assert!(model.recent_focus.is_empty());
        assert!(model.open_frustrations.is_empty());
        assert!(model.working_on.is_empty());
    }

    #[test]
    fn hash_is_stable_across_btreemap_insertion_order() {
        let mut left = UserModel::default();
        left.preferences.insert("voice".to_string(), "spare".to_string());
        left.preferences.insert("emojis".to_string(), "never".to_string());

        let mut right = UserModel::default();
        right.preferences.insert("emojis".to_string(), "never".to_string());
        right.preferences.insert("voice".to_string(), "spare".to_string());

        assert_eq!(user_model_hash(&left), user_model_hash(&right));
    }

    #[test]
    fn hash_changes_when_content_changes() {
        let mut base = UserModel::default();
        base.preferences.insert("voice".to_string(), "spare".to_string());
        let base_hash = user_model_hash(&base);

        let mut other = base.clone();
        other.style_notes.push(UserModelNote::new("no em-dashes", "2026-06-28T00:00:00Z"));
        let other_hash = user_model_hash(&other);

        assert_ne!(base_hash, other_hash);
    }

    #[test]
    fn serde_roundtrip_preserves_all_fields() {
        let mut model = UserModel::default();
        model.preferences.insert("voice".to_string(), "spare".to_string());
        model
            .style_notes
            .push(UserModelNote::new("no em-dashes", "2026-06-28T00:00:00Z"));
        model.recent_focus.push(UserModelReference::new(
            "node:theorem.harness.core",
            "harness kernel",
        ));
        model
            .open_frustrations
            .push(UserModelNote::new("clippy 1.95 is noisy", "2026-06-28T00:00:00Z"));
        model.working_on.push(UserModelProjectRef::new(
            "project:agent-theorem",
            "Agent Theorem",
            "in_progress",
        ));

        let json = serde_json::to_value(&model).unwrap();
        let parsed: UserModel = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(parsed, model);
        let reserialized = serde_json::to_value(&parsed).unwrap();
        assert_eq!(reserialized, json);
    }

    #[test]
    fn deserialization_fills_missing_fields_with_defaults() {
        let parsed: UserModel = serde_json::from_value(json!({})).unwrap();
        assert_eq!(parsed, UserModel::default());
    }
}
