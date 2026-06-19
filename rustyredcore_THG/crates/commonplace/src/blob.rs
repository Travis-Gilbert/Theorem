//! The content-addressed blob store seam (plan unit F1).
//!
//! A `File` item's body is a blob addressed by its content hash. The store is a
//! seam so the object model stays portable: tests use [`InMemoryBlobStore`];
//! durable deployments use the substrate's [`DiskObjectStore`], which persists
//! one zstd-compressed file per content hash. Both produce the identical
//! `sha256:<hex>` address for the same bytes, so a hash written by one resolves
//! against the other.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rustyred_thg_core::{DiskObjectStore, GraphStoreError, GraphStoreResult};
use sha2::{Digest, Sha256};

/// A content-addressed store of opaque byte blobs.
pub trait BlobStore {
    /// Store `body` and return its content address. Idempotent: the same bytes
    /// always yield the same address and a second write is a no-op.
    fn put(&self, body: &[u8]) -> GraphStoreResult<String>;

    /// Fetch the bytes at `content_hash`, or `Ok(None)` when absent.
    fn get(&self, content_hash: &str) -> GraphStoreResult<Option<Vec<u8>>>;
}

/// The content address for `body`: `sha256:<lowercase-hex>`. Matches the address
/// scheme [`DiskObjectStore`] uses for documents, so the two stores agree.
pub fn content_hash(body: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(body))
}

/// In-process blob store for tests and scratch. Does not survive a restart.
#[derive(Clone, Default)]
pub struct InMemoryBlobStore {
    inner: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl InMemoryBlobStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of distinct blobs held (diagnostics).
    pub fn len(&self) -> usize {
        self.inner.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl BlobStore for InMemoryBlobStore {
    fn put(&self, body: &[u8]) -> GraphStoreResult<String> {
        let hash = content_hash(body);
        self.inner
            .lock()
            .map_err(lock_poisoned)?
            .insert(hash.clone(), body.to_vec());
        Ok(hash)
    }

    fn get(&self, content_hash: &str) -> GraphStoreResult<Option<Vec<u8>>> {
        Ok(self
            .inner
            .lock()
            .map_err(lock_poisoned)?
            .get(content_hash)
            .cloned())
    }
}

/// The substrate's disk object store is a durable blob store: documents are
/// addressed by `sha256(raw bytes)` exactly as [`content_hash`] computes.
impl BlobStore for DiskObjectStore {
    fn put(&self, body: &[u8]) -> GraphStoreResult<String> {
        self.put_document_bytes(body)
    }

    fn get(&self, content_hash: &str) -> GraphStoreResult<Option<Vec<u8>>> {
        self.get_document_bytes(content_hash)
    }
}

fn lock_poisoned<T>(_: T) -> GraphStoreError {
    GraphStoreError::new(
        "commonplace_blob_lock_poisoned",
        "commonplace in-memory blob store mutex was poisoned",
    )
}
