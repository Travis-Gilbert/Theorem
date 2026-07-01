use std::fs;
use std::path::{Path, PathBuf};

use iroh::{endpoint::presets, Endpoint, SecretKey};
use serde::{Deserialize, Serialize};

use crate::{FederationError, Result};

pub const FEDERATION_ALPN: &[u8] = b"theorem/federation/1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityConfig {
    pub data_dir: PathBuf,
}

impl IdentityConfig {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    pub fn key_path(&self) -> PathBuf {
        self.data_dir.join("federation").join("iroh-secret-key.hex")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IdentityStatus {
    pub endpoint_id: String,
    pub key_path: PathBuf,
    pub persistent_key: bool,
    pub alpn: String,
}

pub fn load_or_create_secret_key(config: &IdentityConfig) -> Result<SecretKey> {
    let key_path = config.key_path();
    if key_path.exists() {
        return read_secret_key(&key_path);
    }

    if let Some(parent) = key_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)?;
    let key = SecretKey::from_bytes(&bytes);
    let encoded = format!("{}\n", hex::encode(key.to_bytes()));
    match fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&key_path)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(encoded.as_bytes())?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return read_secret_key(&key_path);
        }
        Err(error) => return Err(error.into()),
    }
    Ok(key)
}

pub fn identity_status(config: &IdentityConfig) -> Result<IdentityStatus> {
    let key = load_or_create_secret_key(config)?;
    Ok(IdentityStatus {
        endpoint_id: key.public().to_string(),
        key_path: config.key_path(),
        persistent_key: true,
        alpn: String::from_utf8_lossy(FEDERATION_ALPN).to_string(),
    })
}

pub async fn bind_endpoint(config: &IdentityConfig) -> Result<Endpoint> {
    let key = load_or_create_secret_key(config)?;
    Endpoint::builder(presets::N0)
        .secret_key(key)
        .alpns(vec![FEDERATION_ALPN.to_vec()])
        .bind()
        .await
        .map_err(FederationError::iroh)
}

fn read_secret_key(path: &Path) -> Result<SecretKey> {
    let encoded = fs::read_to_string(path)?;
    let bytes = hex::decode(encoded.trim())?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|bytes: Vec<u8>| {
        FederationError::Config(format!(
            "secret key at {} must be 32 bytes, got {}",
            path.display(),
            bytes.len()
        ))
    })?;
    Ok(SecretKey::from_bytes(&bytes))
}
