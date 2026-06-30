//! Native cold document tree.
//!
//! The object store remains the immutable blob backend. This module adds the
//! mutable namespace over it: a copy-on-write B-tree keyed by hierarchical path,
//! with clustered inline leaves for small bodies and overflow blobs for large
//! bodies.

use imbl::OrdMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;

use crate::cold_index::ColdTierKind;
use crate::graph_store::{GraphStoreError, GraphStoreResult, NodeRecord};
use crate::object_store::{
    compress_cold_bytes, content_hash_bytes, decompress_cold_bytes, DiskObjectStore,
};

const PATH_SEPARATOR: u8 = 0;
pub const DEFAULT_INLINE_THRESHOLD: usize = 4096;
pub const DOC_TREE_PATH_PROPERTY: &str = "doc_tree_path";
pub const DOC_TREE_CONTENT_HASH_PROPERTY: &str = "doc_tree_content_hash";

/// Hierarchical path whose byte order is the tree order. Segments are separated
/// by NUL, which sorts below ordinary path bytes and prevents `a/b2` from
/// matching the `a/b` prefix.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct PathKey(Vec<u8>);

impl PathKey {
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> GraphStoreResult<Self> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(invalid_path("path key must not be empty"));
        }
        Ok(Self(bytes))
    }

    pub fn from_slash_path(path: &str) -> GraphStoreResult<Self> {
        Self::from_segments(path.split('/').filter(|segment| !segment.is_empty()))
    }

    pub fn from_segments<'a>(
        segments: impl IntoIterator<Item = &'a str>,
    ) -> GraphStoreResult<Self> {
        Ok(Self(encode_segments(segments, false)?))
    }

    pub fn prefix_from_segments<'a>(
        segments: impl IntoIterator<Item = &'a str>,
    ) -> GraphStoreResult<Self> {
        Ok(Self(encode_segments(segments, true)?))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn starts_with(&self, prefix: &[u8]) -> bool {
        self.0.starts_with(prefix)
    }

    pub fn to_slash_path(&self) -> String {
        self.0
            .split(|byte| *byte == PATH_SEPARATOR)
            .filter(|segment| !segment.is_empty())
            .map(|segment| String::from_utf8_lossy(segment))
            .collect::<Vec<_>>()
            .join("/")
    }
}

/// Leaf payload for one document path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocEntry {
    /// Raw-body sha256. Inline entries keep this too so updates retain an
    /// addressable identity even when no overflow fetch is needed.
    pub content_hash: Option<String>,
    /// Zstd-compressed small body bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<Vec<u8>>,
    /// Absolute path to a real filesystem body. Code mirrors use this so the
    /// DocTree is a view over canonical files, not the canonical byte store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub real_path: Option<String>,
    #[serde(default)]
    pub inline_compressed: bool,
    pub tier: ColdTierKind,
    pub size: u64,
    pub created_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gist: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous_hashes: Vec<String>,
}

impl DocEntry {
    pub fn is_inline(&self) -> bool {
        self.inline.is_some()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DocTree {
    entries: OrdMap<PathKey, DocEntry>,
    inline_threshold: usize,
}

impl Default for DocTree {
    fn default() -> Self {
        Self::new(DEFAULT_INLINE_THRESHOLD)
    }
}

impl DocTree {
    pub fn new(inline_threshold: usize) -> Self {
        Self {
            entries: OrdMap::new(),
            inline_threshold,
        }
    }

    pub fn inline_threshold(&self) -> usize {
        self.inline_threshold
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, path: &PathKey) -> Option<&DocEntry> {
        self.entries.get(path)
    }

    pub fn put(&mut self, path: PathKey, mut entry: DocEntry) {
        if let Some(previous) = self.entries.get(&path) {
            if let Some(hash) = previous.content_hash.as_ref() {
                entry.previous_hashes.push(hash.clone());
            }
            entry
                .previous_hashes
                .extend(previous.previous_hashes.clone());
            entry.previous_hashes.dedup();
        }
        self.entries.insert(path, entry);
    }

    pub fn remove(&mut self, path: &PathKey) -> bool {
        self.entries.remove(path).is_some()
    }

    pub fn range_prefix<'a>(
        &'a self,
        prefix: &'a [u8],
    ) -> impl Iterator<Item = (&'a PathKey, &'a DocEntry)> + 'a {
        self.entries
            .range(PathKey(prefix.to_vec())..)
            .take_while(move |(path, _)| path.starts_with(prefix))
    }

    pub fn snapshot(&self) -> DocTree {
        self.clone()
    }

    pub fn put_body(
        &mut self,
        path: PathKey,
        body: &[u8],
        tier: ColdTierKind,
        created_ms: i64,
        gist: Option<String>,
        object_store: &DiskObjectStore,
    ) -> GraphStoreResult<DocEntry> {
        let hash = content_hash_bytes(body);
        let entry = if body.len() <= self.inline_threshold {
            DocEntry {
                content_hash: Some(hash),
                inline: Some(compress_cold_bytes(body)?),
                real_path: None,
                inline_compressed: true,
                tier,
                size: body.len() as u64,
                created_ms,
                gist,
                previous_hashes: Vec::new(),
            }
        } else {
            let stored_hash = object_store.put_document_bytes(body)?;
            DocEntry {
                content_hash: Some(stored_hash),
                inline: None,
                real_path: None,
                inline_compressed: false,
                tier,
                size: body.len() as u64,
                created_ms,
                gist,
                previous_hashes: Vec::new(),
            }
        };
        self.put(path.clone(), entry.clone());
        Ok(self.entries.get(&path).cloned().unwrap_or(entry))
    }

    pub fn put_real_file(
        &mut self,
        path: PathKey,
        real_path: impl Into<String>,
        content_hash: String,
        size: u64,
        tier: ColdTierKind,
        created_ms: i64,
        gist: Option<String>,
    ) -> DocEntry {
        let entry = DocEntry {
            content_hash: Some(content_hash),
            inline: None,
            real_path: Some(real_path.into()),
            inline_compressed: false,
            tier,
            size,
            created_ms,
            gist,
            previous_hashes: Vec::new(),
        };
        self.put(path.clone(), entry.clone());
        self.entries.get(&path).cloned().unwrap_or(entry)
    }

    pub fn resolve_body(
        &self,
        path: &PathKey,
        object_store: &DiskObjectStore,
    ) -> GraphStoreResult<Option<Vec<u8>>> {
        let Some(entry) = self.entries.get(path) else {
            return Ok(None);
        };
        resolve_entry_body(entry, object_store).map(Some)
    }

    pub fn resolve_memory_node_body(
        &self,
        node: &NodeRecord,
        object_store: &DiskObjectStore,
    ) -> GraphStoreResult<Option<Vec<u8>>> {
        let Some(path) = node
            .properties
            .get(DOC_TREE_PATH_PROPERTY)
            .and_then(Value::as_str)
        else {
            return Ok(None);
        };
        self.resolve_body(&PathKey::from_slash_path(path)?, object_store)
    }

    pub fn materialize_node_body(
        &mut self,
        node: &mut NodeRecord,
        path: PathKey,
        body: &[u8],
        created_ms: i64,
        object_store: &DiskObjectStore,
    ) -> GraphStoreResult<DocEntry> {
        let gist = node
            .properties
            .get("gist")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let entry = self.put_body(
            path.clone(),
            body,
            ColdTierKind::Cold,
            created_ms,
            gist,
            object_store,
        )?;
        let Some(properties) = node.properties.as_object_mut() else {
            return Err(GraphStoreError::new(
                "invalid_doc_tree_node",
                "memory node properties must be an object",
            ));
        };
        properties.remove("body");
        properties.insert(
            DOC_TREE_PATH_PROPERTY.to_string(),
            Value::String(path.to_slash_path()),
        );
        if let Some(hash) = entry.content_hash.as_ref() {
            properties.insert(
                DOC_TREE_CONTENT_HASH_PROPERTY.to_string(),
                Value::String(hash.clone()),
            );
        }
        Ok(entry)
    }
}

fn resolve_entry_body(
    entry: &DocEntry,
    object_store: &DiskObjectStore,
) -> GraphStoreResult<Vec<u8>> {
    if let Some(inline) = entry.inline.as_ref() {
        return if entry.inline_compressed {
            decompress_cold_bytes(inline)
        } else {
            Ok(inline.clone())
        };
    }
    if let Some(real_path) = entry.real_path.as_ref() {
        let metadata = std::fs::metadata(real_path).map_err(|error| {
            GraphStoreError::new(
                "real_doc_entry_read",
                format!("stat mirrored body {real_path}: {error}"),
            )
        })?;
        if metadata.len() > entry.size {
            return Err(GraphStoreError::new(
                "real_doc_entry_read",
                format!(
                    "mirrored body {real_path} grew from {} to {} bytes",
                    entry.size,
                    metadata.len()
                ),
            ));
        }
        let file = std::fs::File::open(real_path).map_err(|error| {
            GraphStoreError::new(
                "real_doc_entry_read",
                format!("opening mirrored body {real_path}: {error}"),
            )
        })?;
        let mut body = Vec::with_capacity(metadata.len() as usize);
        file.take(entry.size.saturating_add(1))
            .read_to_end(&mut body)
            .map_err(|error| {
                GraphStoreError::new(
                    "real_doc_entry_read",
                    format!("reading mirrored body {real_path}: {error}"),
                )
            })?;
        if body.len() as u64 > entry.size {
            return Err(GraphStoreError::new(
                "real_doc_entry_read",
                format!("mirrored body {real_path} grew while reading"),
            ));
        }
        return Ok(body);
    }
    let Some(hash) = entry.content_hash.as_ref() else {
        return Err(GraphStoreError::new(
            "invalid_doc_entry",
            "document entry has neither inline body nor content hash",
        ));
    };
    object_store.get_document_bytes(hash)?.ok_or_else(|| {
        GraphStoreError::new(
            "missing_doc_body",
            format!("document body {hash} is absent from object store"),
        )
    })
}

fn encode_segments<'a>(
    segments: impl IntoIterator<Item = &'a str>,
    trailing_separator: bool,
) -> GraphStoreResult<Vec<u8>> {
    let mut out = Vec::new();
    let mut count = 0usize;
    for segment in segments {
        if segment.is_empty() {
            return Err(invalid_path("path segment must not be empty"));
        }
        if segment.as_bytes().contains(&PATH_SEPARATOR) {
            return Err(invalid_path("path segment must not contain NUL"));
        }
        if count > 0 {
            out.push(PATH_SEPARATOR);
        }
        out.extend_from_slice(segment.as_bytes());
        count += 1;
    }
    if count == 0 {
        return Err(invalid_path("path requires at least one segment"));
    }
    if trailing_separator {
        out.push(PATH_SEPARATOR);
    }
    Ok(out)
}

fn invalid_path(message: impl Into<String>) -> GraphStoreError {
    GraphStoreError::new("invalid_doc_tree_path", message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_store::NodeRecord;
    use serde_json::json;

    fn temp_store(name: &str) -> (std::path::PathBuf, DiskObjectStore) {
        let dir = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = DiskObjectStore::open(&dir).unwrap();
        (dir, store)
    }

    #[test]
    fn prefix_scan_returns_namespace_in_path_order() {
        let (_dir, store) = temp_store("doc-tree-prefix");
        let mut tree = DocTree::new(64);
        for path in [
            ["tenant", "project", "episode", "2"],
            ["tenant", "project", "episode", "1"],
            ["tenant", "project2", "episode", "3"],
        ] {
            let key = PathKey::from_segments(path).unwrap();
            tree.put_body(
                key,
                path.join("/").as_bytes(),
                ColdTierKind::Cold,
                1,
                None,
                &store,
            )
            .unwrap();
        }
        let prefix = PathKey::prefix_from_segments(["tenant", "project"]).unwrap();
        let paths = tree
            .range_prefix(prefix.as_bytes())
            .map(|(path, _)| path.to_slash_path())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                "tenant/project/episode/1".to_string(),
                "tenant/project/episode/2".to_string()
            ]
        );
    }

    #[test]
    fn inline_and_overflow_documents_round_trip_identical_bytes() {
        let (dir, store) = temp_store("doc-tree-roundtrip");
        let mut tree = DocTree::new(8);
        let small = b"tiny";
        let large = b"this body is larger than the inline threshold";
        let small_path = PathKey::from_slash_path("tenant/project/doc/small").unwrap();
        let large_path = PathKey::from_slash_path("tenant/project/doc/large").unwrap();

        let small_entry = tree
            .put_body(
                small_path.clone(),
                small,
                ColdTierKind::Cold,
                10,
                Some("small".to_string()),
                &store,
            )
            .unwrap();
        let large_entry = tree
            .put_body(
                large_path.clone(),
                large,
                ColdTierKind::Cold,
                10,
                None,
                &store,
            )
            .unwrap();

        assert!(small_entry.is_inline());
        assert!(small_entry.inline_compressed);
        assert!(!large_entry.is_inline());
        assert_eq!(
            tree.resolve_body(&small_path, &store).unwrap().unwrap(),
            small
        );
        assert_eq!(
            tree.resolve_body(&large_path, &store).unwrap().unwrap(),
            large
        );
        let expected_large_hash = content_hash_bytes(large);
        assert_eq!(
            large_entry.content_hash.as_deref(),
            Some(expected_large_hash.as_str())
        );
        assert!(store
            .document_path(large_entry.content_hash.as_ref().unwrap())
            .exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_file_entry_refuses_to_read_past_indexed_size() {
        let (dir, store) = temp_store("doc-tree-real-file-size");
        let real_path = dir.join("mirror.rs");
        std::fs::write(&real_path, b"small").unwrap();
        let mut tree = DocTree::new(8);
        let path = PathKey::from_slash_path("tenant/project/src/mirror.rs").unwrap();
        tree.put_real_file(
            path.clone(),
            real_path.to_string_lossy().into_owned(),
            content_hash_bytes(b"small"),
            5,
            ColdTierKind::Cold,
            10,
            None,
        );

        std::fs::write(&real_path, b"small but now much larger").unwrap();
        let error = tree.resolve_body(&path, &store).unwrap_err();
        assert_eq!(error.code, "real_doc_entry_read");
        assert!(
            error.message.contains("grew"),
            "expected growth guard, got {error:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_keeps_previous_body_after_path_update() {
        let (_dir, store) = temp_store("doc-tree-snapshot");
        let mut tree = DocTree::new(64);
        let path = PathKey::from_slash_path("tenant/project/doc/a").unwrap();
        tree.put_body(path.clone(), b"before", ColdTierKind::Cold, 1, None, &store)
            .unwrap();
        let snapshot = tree.snapshot();
        let updated = tree
            .put_body(path.clone(), b"after", ColdTierKind::Cold, 2, None, &store)
            .unwrap();

        assert_eq!(
            snapshot.resolve_body(&path, &store).unwrap().unwrap(),
            b"before"
        );
        assert_eq!(tree.resolve_body(&path, &store).unwrap().unwrap(), b"after");
        assert_eq!(updated.previous_hashes, vec![content_hash_bytes(b"before")]);
    }

    #[test]
    fn memory_node_docks_resolve_cold_body_and_remain_findable() {
        let (_dir, store) = temp_store("doc-tree-dock");
        let mut tree = DocTree::new(8);
        let mut node = NodeRecord::new(
            "mem:1",
            ["Memory", "Episode"],
            json!({
                "gist": "short summary",
                "body": "full body that should leave the hot node",
                "topic": "planning"
            }),
        );
        let path = PathKey::from_slash_path("tenant/memory/mem:1").unwrap();
        tree.materialize_node_body(
            &mut node,
            path,
            b"full body that should leave the hot node",
            42,
            &store,
        )
        .unwrap();

        assert!(node.labels.contains(&"Memory".to_string()));
        assert_eq!(node.properties.get("topic"), Some(&json!("planning")));
        assert!(node.properties.get("body").is_none());
        assert_eq!(
            tree.resolve_memory_node_body(&node, &store)
                .unwrap()
                .unwrap(),
            b"full body that should leave the hot node"
        );
    }
}
