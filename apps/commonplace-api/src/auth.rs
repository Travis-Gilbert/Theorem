//! Per-instance API-key auth (plan unit F3).
//!
//! A client connects to an instance with a URL and a key. The key resolves to a
//! [`Principal`] (the identity behind the key); an unknown key is rejected. The
//! presented key arrives as an [`ApiKeyToken`] in the GraphQL request data,
//! injected by the HTTP layer from the `x-api-key` header (or by tests).

use std::collections::HashMap;

/// The identity a valid API key resolves to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    pub id: String,
    pub label: String,
}

/// The set of valid API keys for an instance.
#[derive(Clone, Debug, Default)]
pub struct ApiKeyRegistry {
    keys: HashMap<String, Principal>,
}

impl ApiKeyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a key mapped to a principal id (label defaults to the id).
    pub fn with_key(mut self, key: impl Into<String>, principal_id: impl Into<String>) -> Self {
        let id = principal_id.into();
        self.keys.insert(
            key.into(),
            Principal {
                id: id.clone(),
                label: id,
            },
        );
        self
    }

    pub fn insert(&mut self, key: impl Into<String>, principal: Principal) {
        self.keys.insert(key.into(), principal);
    }

    /// Resolve a presented key to its principal, or `None` if unknown.
    pub fn resolve(&self, key: &str) -> Option<&Principal> {
        self.keys.get(key)
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }
}

/// The API key presented on a single request.
#[derive(Clone, Debug)]
pub struct ApiKeyToken(pub String);
