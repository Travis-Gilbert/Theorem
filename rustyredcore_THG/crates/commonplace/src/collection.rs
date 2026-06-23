//! Collections: named groupings of items (plan unit F1).
//!
//! A [`Collection`] is a first-class graph node (so it can enumerate its members
//! by reverse `IN_COLLECTION` traversal). It may be `Manual` (user-made) or
//! `Auto` (coined by the F2 ingest pipeline when a cluster forms).

use serde::{Deserialize, Serialize};

/// How a collection came to exist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", from = "String")]
pub enum CollectionKind {
    #[default]
    Manual,
    Auto,
}

impl CollectionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CollectionKind::Manual => "manual",
            CollectionKind::Auto => "auto",
        }
    }
}

impl From<CollectionKind> for String {
    fn from(kind: CollectionKind) -> Self {
        kind.as_str().to_string()
    }
}

impl From<String> for CollectionKind {
    fn from(value: String) -> Self {
        match value.as_str() {
            "auto" => CollectionKind::Auto,
            _ => CollectionKind::Manual,
        }
    }
}

/// A named grouping of items.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: CollectionKind,
    #[serde(default)]
    pub created_at_ms: i64,
}

impl Collection {
    pub fn new(name: impl Into<String>, kind: CollectionKind) -> Self {
        Self {
            id: String::new(),
            name: name.into(),
            kind,
            created_at_ms: 0,
        }
    }
}
