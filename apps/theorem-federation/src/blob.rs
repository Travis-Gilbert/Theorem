use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::trust::TrustPolicy;
use crate::Result;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BlobMapping {
    pub sha256: String,
    pub blake3: String,
    pub len: u64,
    pub kind: PayloadKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadKind {
    ProllyPack,
    ColdObject,
    Document,
    Other,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResolverState {
    pub mappings: Vec<BlobMapping>,
}

#[derive(Clone, Debug)]
pub struct BlobResolver {
    path: PathBuf,
    state: ResolverState,
}

impl BlobResolver {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let state = if path.exists() {
            serde_json::from_slice(&fs::read(&path)?)?
        } else {
            ResolverState::default()
        };
        Ok(Self { path, state })
    }

    pub fn record_bytes(&mut self, bytes: &[u8], kind: PayloadKind) -> Result<BlobMapping> {
        let mapping = BlobMapping {
            sha256: sha256_identity(bytes),
            blake3: blake3_transfer_link(bytes),
            len: bytes.len() as u64,
            kind,
        };
        self.state
            .mappings
            .retain(|existing| existing.sha256 != mapping.sha256);
        self.state.mappings.push(mapping.clone());
        self.save()?;
        Ok(mapping)
    }

    pub fn record_bytes_from_peer(
        &mut self,
        endpoint_id: &str,
        trust: &TrustPolicy,
        bytes: &[u8],
        kind: PayloadKind,
    ) -> Result<BlobMapping> {
        trust.require_inbound(endpoint_id)?;
        self.record_bytes(bytes, kind)
    }

    pub fn resolve_blake3(&self, sha256: &str) -> Option<&str> {
        self.state
            .mappings
            .iter()
            .find(|mapping| mapping.sha256 == sha256)
            .map(|mapping| mapping.blake3.as_str())
    }

    pub fn resolve_sha256(&self, blake3: &str) -> Option<&str> {
        self.state
            .mappings
            .iter()
            .find(|mapping| mapping.blake3 == blake3)
            .map(|mapping| mapping.sha256.as_str())
    }

    pub fn mappings(&self) -> &[BlobMapping] {
        &self.state.mappings
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_atomic_json(&self.path, &self.state)
    }
}

pub fn sha256_identity(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

pub fn blake3_transfer_link(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

pub fn verify_transfer_boundary(bytes: &[u8], mapping: &BlobMapping) -> bool {
    sha256_identity(bytes) == mapping.sha256
        && blake3_transfer_link(bytes) == mapping.blake3
        && bytes.len() as u64 == mapping.len
}

fn write_atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}
