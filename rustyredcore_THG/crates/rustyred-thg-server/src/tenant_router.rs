use std::fmt;
use std::path::{Path, PathBuf};

use rustyred_thg_core::stable_hash;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TenantId(String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TenantIdError {
    message: String,
}

impl TenantId {
    pub fn new(value: impl AsRef<str>) -> Result<Self, TenantIdError> {
        let value = value.as_ref().trim();
        if value.is_empty() {
            return Err(TenantIdError::new("tenant id must be non-empty"));
        }
        if value.chars().any(char::is_control) {
            return Err(TenantIdError::new(
                "tenant id must not contain control characters",
            ));
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl fmt::Display for TenantIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for TenantIdError {}

impl TenantIdError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub fn tenant_key_segment(tenant_id: &TenantId) -> String {
    let hash = stable_hash(tenant_id.as_str());
    format!("tenant-{}", hash.trim_start_matches("sha256:"))
}

pub fn tenant_data_dir(root: impl AsRef<Path>, tenant_id: &TenantId) -> PathBuf {
    root.as_ref()
        .join("tenants")
        .join(tenant_key_segment(tenant_id))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::{tenant_data_dir, tenant_key_segment, TenantId};

    #[test]
    fn tenant_data_dirs_are_distinct_for_collision_corpus() {
        let long = "x".repeat(512);
        let corpus = vec![
            "acme/prod".to_string(),
            "acme.prod".to_string(),
            "acme_prod".to_string(),
            "München/研究".to_string(),
            long,
        ];
        let mut dirs = BTreeSet::new();
        let mut keys = BTreeSet::new();

        for raw in &corpus {
            let tenant = TenantId::new(raw).unwrap();
            assert!(
                dirs.insert(tenant_data_dir("/tmp/rustyred", &tenant)),
                "duplicate directory for {raw:?}"
            );
            assert!(
                keys.insert(tenant_key_segment(&tenant)),
                "duplicate key segment for {raw:?}"
            );
        }

        assert_eq!(dirs.len(), corpus.len());
        assert!(dirs
            .iter()
            .all(|dir| dir.starts_with(Path::new("/tmp/rustyred/tenants"))));
    }

    #[test]
    fn historically_colliding_tenants_resolve_to_distinct_directories() {
        let slash = TenantId::new("acme/prod").unwrap();
        let dot = TenantId::new("acme.prod").unwrap();

        assert_ne!(
            tenant_data_dir("/tmp/rustyred", &slash),
            tenant_data_dir("/tmp/rustyred", &dot)
        );
    }

    #[test]
    fn tenant_id_rejects_empty_or_control_identifiers() {
        assert!(TenantId::new("").is_err());
        assert!(TenantId::new(" \t ").is_err());
        assert!(TenantId::new("tenant\nid").is_err());
    }
}
