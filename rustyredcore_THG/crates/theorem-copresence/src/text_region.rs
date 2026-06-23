use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, GetString, ReadTxn, StateVector, Text, TextRef, Transact, Update};

use rustyred_thg_core::{now_ms, ColdTierKind, DiskObjectStore, DocEntry, DocTree, PathKey};

use crate::{CoError, CoResult};

const TEXT_ROOT: &str = "body";
const TEXT_BLOB_LEAF: &str = "yrs-update-v1";

#[derive(Debug, Deserialize, Serialize)]
struct PersistedDocTree {
    inline_threshold: usize,
    entries: Vec<(PathKey, DocEntry)>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextRegionUpdate {
    pub region_id: String,
    pub state_vector_v1: Vec<u8>,
    pub update_v1: Vec<u8>,
}

#[derive(Clone)]
pub struct TextRegionHandle {
    region_id: String,
    scope: String,
    doc: Doc,
    text: TextRef,
    doc_tree: Arc<Mutex<DocTree>>,
    doc_tree_path: PathBuf,
    object_store: DiskObjectStore,
}

impl std::fmt::Debug for TextRegionHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TextRegionHandle")
            .field("region_id", &self.region_id)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl TextRegionHandle {
    pub(crate) fn open(
        region_id: impl Into<String>,
        scope: impl Into<String>,
        client_id: u64,
        doc_tree: Arc<Mutex<DocTree>>,
        doc_tree_path: PathBuf,
        object_store: DiskObjectStore,
    ) -> CoResult<Self> {
        let region_id = region_id.into();
        let scope = scope.into();
        let doc = Doc::with_client_id(client_id);
        let text = doc.get_or_insert_text(TEXT_ROOT);
        let handle = Self {
            region_id,
            scope,
            doc,
            text,
            doc_tree,
            doc_tree_path,
            object_store,
        };
        if let Some(update) = handle.persisted_update()? {
            handle.apply_update_bytes(&update)?;
        } else {
            handle.persist()?;
        }
        Ok(handle)
    }

    pub fn region_id(&self) -> &str {
        &self.region_id
    }

    pub fn insert(&self, index: u32, chunk: &str) -> CoResult<TextRegionUpdate> {
        let mut txn = self.doc.transact_mut_with(self.doc.client_id());
        self.text.insert(&mut txn, index, chunk);
        drop(txn);
        self.persist()?;
        self.current_update()
    }

    pub fn push(&self, chunk: &str) -> CoResult<TextRegionUpdate> {
        let mut txn = self.doc.transact_mut_with(self.doc.client_id());
        self.text.push(&mut txn, chunk);
        drop(txn);
        self.persist()?;
        self.current_update()
    }

    pub fn apply_update(&self, update_v1: &[u8]) -> CoResult<()> {
        self.apply_update_bytes(update_v1)?;
        self.persist()
    }

    pub fn encode_state_vector(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }

    pub fn encode_update_since(&self, remote_state_vector_v1: &[u8]) -> CoResult<Vec<u8>> {
        let state_vector = StateVector::decode_v1(remote_state_vector_v1)
            .map_err(|error| CoError::Yrs(error.to_string()))?;
        Ok(self.doc.transact().encode_diff_v1(&state_vector))
    }

    pub fn contents(&self) -> String {
        self.text.get_string(&self.doc.transact())
    }

    pub(crate) fn persisted_update(&self) -> CoResult<Option<Vec<u8>>> {
        let path = text_path(&self.scope, &self.region_id)?;
        self.doc_tree
            .lock()
            .map_err(|_| CoError::Lock("text region doc tree"))?
            .resolve_body(&path, &self.object_store)
            .map_err(CoError::from)
    }

    fn current_update(&self) -> CoResult<TextRegionUpdate> {
        let state_vector_v1 = self.encode_state_vector();
        let update_v1 = self.encode_update_since(&StateVector::default().encode_v1())?;
        Ok(TextRegionUpdate {
            region_id: self.region_id.clone(),
            state_vector_v1,
            update_v1,
        })
    }

    fn apply_update_bytes(&self, update_v1: &[u8]) -> CoResult<()> {
        let update =
            Update::decode_v1(update_v1).map_err(|error| CoError::Yrs(error.to_string()))?;
        self.doc
            .transact_mut()
            .apply_update(update)
            .map_err(|error| CoError::Yrs(error.to_string()))
    }

    fn persist(&self) -> CoResult<()> {
        let full_update = self
            .doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let path = text_path(&self.scope, &self.region_id)?;
        let mut doc_tree = self
            .doc_tree
            .lock()
            .map_err(|_| CoError::Lock("text region doc tree"))?;
        doc_tree.put_body(
            path,
            &full_update,
            ColdTierKind::Cold,
            now_ms(),
            Some(format!("yrs text region {}", self.region_id)),
            &self.object_store,
        )?;
        persist_doc_tree(&self.doc_tree_path, &doc_tree)?;
        Ok(())
    }
}

pub(crate) fn open_object_store(root: impl AsRef<Path>) -> CoResult<DiskObjectStore> {
    DiskObjectStore::open(root).map_err(CoError::from)
}

pub(crate) fn doc_tree_path(root: impl AsRef<Path>) -> PathBuf {
    root.as_ref().join("copresence-doc-tree.json")
}

pub(crate) fn load_doc_tree(root: impl AsRef<Path>) -> CoResult<DocTree> {
    let path = doc_tree_path(root);
    match fs::read(&path) {
        Ok(bytes) => {
            let persisted: PersistedDocTree = serde_json::from_slice(&bytes)?;
            Ok(DocTree::from_entries(
                persisted.inline_threshold,
                persisted.entries,
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(DocTree::default()),
        Err(error) => Err(CoError::Io(error.to_string())),
    }
}

fn persist_doc_tree(path: &Path, doc_tree: &DocTree) -> CoResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| CoError::Io(error.to_string()))?;
    }
    let persisted = PersistedDocTree {
        inline_threshold: doc_tree.inline_threshold(),
        entries: doc_tree.entries(),
    };
    let bytes = serde_json::to_vec(&persisted)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes).map_err(|error| CoError::Io(error.to_string()))?;
    fs::rename(&tmp, path).map_err(|error| CoError::Io(error.to_string()))?;
    Ok(())
}

fn text_path(scope: &str, region_id: &str) -> CoResult<PathKey> {
    PathKey::from_segments(["copresence", scope, region_id, TEXT_BLOB_LEAF]).map_err(CoError::from)
}
