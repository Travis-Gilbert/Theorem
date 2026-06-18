//! The cold tier's content-addressed object store (storage spine, cut 6).
//!
//! The operating store (`InMemoryGraphStore` / `RedCoreGraphStore`) stays
//! RAM-first; this is where the cold tail lives so the graph can exceed RAM
//! without inverting the store. It is the disk-backed home for the same
//! content-addressed payloads `versioned_graph.rs` already produces
//! ([`GraphContentObject`] keyed by hash, plus whole [`CompiledGraphPack`]s for
//! parked warm scopes). It is a SOURCE OF TRUTH, not a cache: unlike Valkey it
//! never evicts. This is the larger-than-memory pattern Memgraph uses (an
//! on-disk key-value store keyed by content hash, with the operating store as
//! the working-set cache); the disk impl here is a content-addressed file store
//! (one file per object, named by hash), which needs no external service so the
//! durability acceptance test runs in-process. A Postgres backing is provided
//! separately by the `rustyred-thg-catalog` crate.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::graph_store::{GraphStoreError, GraphStoreResult};
use crate::versioned_graph::{CompiledGraphPack, GraphContentObject};

/// Durable, content-addressed home for the cold tail.
///
/// Keys are object hashes (cold per-node tier) and commit hashes (warm per-scope
/// tier). Implementations must survive an operating-store restart -- that is the
/// whole point of the tier -- so a purely in-RAM impl is for tests only.
pub trait ColdObjectStore: Send {
    /// Persist a content object under its hash. Idempotent: writing the same
    /// hash twice is a no-op (content addressing -> identical bytes).
    fn put_object(&self, object: &GraphContentObject) -> GraphStoreResult<()>;

    /// Fetch a content object by hash. `Ok(None)` when absent.
    fn get_object(&self, hash: &str) -> GraphStoreResult<Option<GraphContentObject>>;

    /// Persist a compiled pack (a parked warm scope) under its commit hash.
    fn put_pack(&self, pack: &CompiledGraphPack) -> GraphStoreResult<()>;

    /// Fetch a parked pack by commit hash. `Ok(None)` when absent.
    fn get_pack(&self, commit_hash: &str) -> GraphStoreResult<Option<CompiledGraphPack>>;

    /// Number of distinct content objects held (diagnostics / RAM accounting).
    fn object_count(&self) -> usize;
}

/// In-RAM cold store. For tests and the in-process default only -- it does NOT
/// survive a restart, so it cannot satisfy the durability acceptance criterion;
/// use [`DiskObjectStore`] (or the Postgres catalog) for a real cold tier.
#[derive(Clone, Debug, Default)]
pub struct InMemoryObjectStore {
    objects: Arc<Mutex<BTreeMap<String, GraphContentObject>>>,
    packs: Arc<Mutex<BTreeMap<String, CompiledGraphPack>>>,
}

impl InMemoryObjectStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ColdObjectStore for InMemoryObjectStore {
    fn put_object(&self, object: &GraphContentObject) -> GraphStoreResult<()> {
        self.objects
            .lock()
            .map_err(poisoned)?
            .insert(object.hash.clone(), object.clone());
        Ok(())
    }

    fn get_object(&self, hash: &str) -> GraphStoreResult<Option<GraphContentObject>> {
        Ok(self.objects.lock().map_err(poisoned)?.get(hash).cloned())
    }

    fn put_pack(&self, pack: &CompiledGraphPack) -> GraphStoreResult<()> {
        self.packs
            .lock()
            .map_err(poisoned)?
            .insert(pack.commit.commit_hash.clone(), pack.clone());
        Ok(())
    }

    fn get_pack(&self, commit_hash: &str) -> GraphStoreResult<Option<CompiledGraphPack>> {
        Ok(self
            .packs
            .lock()
            .map_err(poisoned)?
            .get(commit_hash)
            .cloned())
    }

    fn object_count(&self) -> usize {
        self.objects.lock().map(|map| map.len()).unwrap_or(0)
    }
}

/// Disk-backed content-addressed object store. The cold tail lives on disk, not
/// in RAM. Layout (git-object-store shaped):
///
/// ```text
/// <root>/objects/<safe-hash>.json   # one GraphContentObject per file
/// <root>/packs/<safe-commit>.json   # one CompiledGraphPack per parked scope
/// ```
///
/// The hash is content-addressed, so a write is a durable, idempotent
/// "store this payload at this address". Survives process restart: a fresh
/// `DiskObjectStore::open` on the same `root` reads back every object.
#[derive(Clone, Debug)]
pub struct DiskObjectStore {
    root: PathBuf,
}

impl DiskObjectStore {
    /// Open (creating if absent) a disk object store rooted at `root`.
    pub fn open(root: impl AsRef<Path>) -> GraphStoreResult<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("objects")).map_err(io_err("create objects dir"))?;
        fs::create_dir_all(root.join("packs")).map_err(io_err("create packs dir"))?;
        Ok(Self { root })
    }

    fn object_path(&self, hash: &str) -> PathBuf {
        self.root
            .join("objects")
            .join(format!("{}.json", safe_filename(hash)))
    }

    fn pack_path(&self, commit_hash: &str) -> PathBuf {
        self.root
            .join("packs")
            .join(format!("{}.json", safe_filename(commit_hash)))
    }

    /// Atomic write: serialize to a sibling `.tmp` then rename, so a crash mid-
    /// write never leaves a half-written object at a content address.
    fn write_atomic(path: &Path, bytes: &[u8]) -> GraphStoreResult<()> {
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, bytes).map_err(io_err("write cold object tmp"))?;
        fs::rename(&tmp, path).map_err(io_err("rename cold object"))?;
        Ok(())
    }
}

impl ColdObjectStore for DiskObjectStore {
    fn put_object(&self, object: &GraphContentObject) -> GraphStoreResult<()> {
        let path = self.object_path(&object.hash);
        if path.exists() {
            return Ok(()); // content-addressed: already durable
        }
        let bytes = serde_json::to_vec(object)
            .map_err(|error| GraphStoreError::new("cold_object_encode", error.to_string()))?;
        Self::write_atomic(&path, &bytes)
    }

    fn get_object(&self, hash: &str) -> GraphStoreResult<Option<GraphContentObject>> {
        let path = self.object_path(hash);
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| GraphStoreError::new("cold_object_decode", error.to_string())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_err("read cold object")(error)),
        }
    }

    fn put_pack(&self, pack: &CompiledGraphPack) -> GraphStoreResult<()> {
        let path = self.pack_path(&pack.commit.commit_hash);
        let bytes = serde_json::to_vec(pack)
            .map_err(|error| GraphStoreError::new("cold_pack_encode", error.to_string()))?;
        Self::write_atomic(&path, &bytes)
    }

    fn get_pack(&self, commit_hash: &str) -> GraphStoreResult<Option<CompiledGraphPack>> {
        let path = self.pack_path(commit_hash);
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| GraphStoreError::new("cold_pack_decode", error.to_string())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_err("read cold pack")(error)),
        }
    }

    fn object_count(&self) -> usize {
        fs::read_dir(self.root.join("objects"))
            .map(|entries| entries.filter_map(Result::ok).count())
            .unwrap_or(0)
    }
}

/// `sha256:<hex>` carries a `:` that is not portable in filenames; map every
/// non-`[A-Za-z0-9._-]` byte to `_` so the address is a safe filename on every
/// platform (the hash stays unambiguous: hex + the single mapped separator).
pub(crate) fn safe_filename(hash: &str) -> String {
    hash.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn io_err(context: &'static str) -> impl Fn(std::io::Error) -> GraphStoreError {
    move |error| GraphStoreError::new("cold_object_store_io", format!("{context}: {error}"))
}

fn poisoned<T>(_: T) -> GraphStoreError {
    GraphStoreError::new(
        "cold_object_store_poisoned",
        "cold object store mutex was poisoned".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_store::NodeRecord;
    use crate::versioned_graph::{node_from_content_object, node_to_content_object};
    use serde_json::json;

    fn object_for(id: &str, name: &str) -> GraphContentObject {
        let node = NodeRecord::new(id, ["Memory"], json!({ "name": name }));
        node_to_content_object(&node)
    }

    #[test]
    fn in_memory_round_trips_objects() {
        let store = InMemoryObjectStore::new();
        let object = object_for("mem:a", "alpha");
        store.put_object(&object).unwrap();
        let fetched = store.get_object(&object.hash).unwrap().unwrap();
        assert_eq!(fetched.key, "node/mem:a");
        let node = node_from_content_object(&fetched).unwrap();
        assert_eq!(node.id, "mem:a");
        assert_eq!(store.get_object("sha256:absent").unwrap(), None);
    }

    #[test]
    fn disk_store_survives_reopen() {
        let dir = std::env::temp_dir().join(format!("cold-store-{}", std::process::id()));
        let object = object_for("mem:durable", "persists");
        {
            let store = DiskObjectStore::open(&dir).unwrap();
            store.put_object(&object).unwrap();
            assert_eq!(store.object_count(), 1);
        }
        // A fresh handle on the same root -- simulating a process restart with
        // the operating store gone -- still serves the cold object.
        let reopened = DiskObjectStore::open(&dir).unwrap();
        let fetched = reopened.get_object(&object.hash).unwrap().unwrap();
        let node = node_from_content_object(&fetched).unwrap();
        assert_eq!(node.id, "mem:durable");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn put_object_is_idempotent_by_content_address() {
        let dir = std::env::temp_dir().join(format!("cold-idem-{}", std::process::id()));
        let store = DiskObjectStore::open(&dir).unwrap();
        let object = object_for("mem:x", "same");
        store.put_object(&object).unwrap();
        store.put_object(&object).unwrap();
        assert_eq!(store.object_count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
