//! Read-only graph archive filesystem core.
//!
//! The default build is pure Rust and has no macFUSE/libfuse dependency. It
//! indexes zero-copy archive bytes from `CompiledGraphPack` into the path scheme
//! that a FUSE host can serve.

use std::collections::{BTreeMap, BTreeSet};

use rustyred_thg_core::{
    object_bytes, CompiledGraphPack, EdgeRecord, GraphArchiveObjectBytes, GraphObjectKind,
};

pub const ROOT_INODE: u64 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphArchiveFileKind {
    Directory,
    ArchiveObject,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphArchiveAttr {
    pub inode: u64,
    pub kind: GraphArchiveFileKind,
    pub len: u64,
    pub hash: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphArchiveDirEntry {
    pub name: String,
    pub inode: u64,
    pub kind: GraphArchiveFileKind,
}

#[derive(Clone, Debug)]
struct FileEntry {
    inode: u64,
    hash: String,
    bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct GraphArchiveFuseIndex {
    dirs: BTreeMap<String, u64>,
    files: BTreeMap<String, FileEntry>,
    children: BTreeMap<String, BTreeSet<String>>,
    next_inode: u64,
}

impl GraphArchiveFuseIndex {
    pub fn from_pack(pack: &CompiledGraphPack) -> Self {
        let mut index = Self {
            next_inode: ROOT_INODE + 1,
            ..Self::default()
        };
        index.ensure_dir("/");
        let version = pack.manifest.graph_version.to_string();
        let root = format!("/snapshots/{version}");
        index.ensure_dir("/snapshots");
        index.ensure_dir(&root);
        index.ensure_dir(&format!("{root}/statements"));
        index.ensure_dir(&format!("{root}/nodes"));
        index.ensure_dir(&format!("{root}/edges"));
        index.ensure_dir(&format!("{root}/by-subject"));

        for archived in &pack.archive_objects {
            let Some(bytes) = object_bytes(pack, &archived.hash) else {
                continue;
            };
            let statement_id = safe_path_segment(&archived.key);
            index.add_file(
                &format!("{root}/statements/{statement_id}.archive"),
                archived,
                bytes,
            );
            match archived.kind {
                GraphObjectKind::Node => {
                    if let Some(node_id) = archived.key.strip_prefix("node/") {
                        index.add_file(
                            &format!("{root}/nodes/{}.archive", safe_path_segment(node_id)),
                            archived,
                            bytes,
                        );
                    }
                }
                GraphObjectKind::Edge => {
                    if let Some(edge_id) = archived.key.strip_prefix("edge/") {
                        index.add_file(
                            &format!("{root}/edges/{}.archive", safe_path_segment(edge_id)),
                            archived,
                            bytes,
                        );
                        if let Some(subject) = edge_subject(pack, &archived.hash) {
                            let subject_dir =
                                format!("{root}/by-subject/{}", safe_path_segment(&subject));
                            index.ensure_dir(&subject_dir);
                            index.add_file(
                                &format!("{subject_dir}/{}.archive", safe_path_segment(edge_id)),
                                archived,
                                bytes,
                            );
                        }
                    }
                }
            }
        }
        index
    }

    pub fn getattr(&self, path: &str) -> Option<GraphArchiveAttr> {
        let path = normalize_path(path);
        if let Some(inode) = self.dirs.get(&path) {
            return Some(GraphArchiveAttr {
                inode: *inode,
                kind: GraphArchiveFileKind::Directory,
                len: 0,
                hash: None,
            });
        }
        self.files.get(&path).map(|file| GraphArchiveAttr {
            inode: file.inode,
            kind: GraphArchiveFileKind::ArchiveObject,
            len: file.bytes.len() as u64,
            hash: Some(file.hash.clone()),
        })
    }

    pub fn list_dir(&self, path: &str) -> Vec<GraphArchiveDirEntry> {
        let path = normalize_path(path);
        self.children
            .get(&path)
            .into_iter()
            .flat_map(|children| children.iter())
            .filter_map(|child| {
                let child_path = join_path(&path, child);
                self.getattr(&child_path).map(|attr| GraphArchiveDirEntry {
                    name: child.clone(),
                    inode: attr.inode,
                    kind: attr.kind,
                })
            })
            .collect()
    }

    pub fn read<'a>(&'a self, path: &str, offset: u64, size: u32) -> Option<&'a [u8]> {
        let path = normalize_path(path);
        let file = self.files.get(&path)?;
        let start = (offset as usize).min(file.bytes.len());
        let end = start.saturating_add(size as usize).min(file.bytes.len());
        Some(&file.bytes[start..end])
    }

    pub fn path_for_inode(&self, inode: u64) -> Option<String> {
        self.dirs
            .iter()
            .find_map(|(path, value)| (*value == inode).then(|| path.clone()))
            .or_else(|| {
                self.files
                    .iter()
                    .find_map(|(path, file)| (file.inode == inode).then(|| path.clone()))
            })
    }

    fn ensure_dir(&mut self, path: &str) -> u64 {
        let path = normalize_path(path);
        if let Some(inode) = self.dirs.get(&path) {
            return *inode;
        }
        let inode = if path == "/" {
            ROOT_INODE
        } else {
            self.allocate_inode()
        };
        self.dirs.insert(path.clone(), inode);
        self.children.entry(path.clone()).or_default();
        if path != "/" {
            let (parent, name) = split_parent(&path);
            self.ensure_dir(&parent);
            self.children.entry(parent).or_default().insert(name);
        }
        inode
    }

    fn add_file(&mut self, path: &str, archived: &GraphArchiveObjectBytes, bytes: &[u8]) {
        let path = normalize_path(path);
        let (parent, name) = split_parent(&path);
        self.ensure_dir(&parent);
        if self.files.contains_key(&path) {
            return;
        }
        let inode = self.allocate_inode();
        self.files.insert(
            path,
            FileEntry {
                inode,
                hash: archived.hash.clone(),
                bytes: bytes.to_vec(),
            },
        );
        self.children.entry(parent).or_default().insert(name);
    }

    fn allocate_inode(&mut self) -> u64 {
        let inode = self.next_inode;
        self.next_inode = self.next_inode.saturating_add(1);
        inode
    }
}

fn edge_subject(pack: &CompiledGraphPack, hash: &str) -> Option<String> {
    pack.objects
        .iter()
        .find(|object| object.hash == hash)
        .and_then(|object| object.payload.clone())
        .and_then(|payload| serde_json::from_value::<EdgeRecord>(payload).ok())
        .map(|edge| edge.from_id)
}

fn normalize_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return "/".to_string();
    }
    format!("/{}", trimmed.trim_matches('/'))
}

fn join_path(parent: &str, child: &str) -> String {
    if parent == "/" {
        format!("/{child}")
    } else {
        format!("{parent}/{child}")
    }
}

fn split_parent(path: &str) -> (String, String) {
    let path = normalize_path(path);
    let Some((parent, name)) = path.rsplit_once('/') else {
        return ("/".to_string(), path);
    };
    let parent = if parent.is_empty() { "/" } else { parent };
    (parent.to_string(), name.to_string())
}

fn safe_path_segment(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::{
        compile_graph_pack, object_bytes, EdgeRecord, GraphCompileOptions, GraphSnapshot,
        NodeRecord,
    };
    use serde_json::json;

    fn pack() -> CompiledGraphPack {
        compile_graph_pack(
            &GraphSnapshot {
                version: 7,
                nodes: vec![NodeRecord::new(
                    "claim:a",
                    ["Claim"],
                    json!({"claim_text": "A"}),
                )],
                edges: vec![EdgeRecord::new(
                    "edge:ab",
                    "claim:a",
                    "SUPPORTS",
                    "claim:a",
                    json!({}),
                )],
            },
            GraphCompileOptions::default(),
        )
    }

    #[test]
    fn fuse_index_reads_object_bytes_by_spec_paths() {
        let pack = pack();
        let index = GraphArchiveFuseIndex::from_pack(&pack);
        let node_archive = pack
            .archive_objects
            .iter()
            .find(|object| object.key == "node/claim:a")
            .expect("node archive");
        let path = "/snapshots/7/nodes/claim_a.archive";
        assert_eq!(
            index.read(path, 0, u32::MAX).unwrap(),
            object_bytes(&pack, &node_archive.hash).unwrap()
        );
        assert!(index.getattr(path).unwrap().len > 0);
    }

    #[test]
    fn fuse_index_lists_statement_and_by_subject_views() {
        let pack = pack();
        let index = GraphArchiveFuseIndex::from_pack(&pack);
        let statements = index.list_dir("/snapshots/7/statements");
        assert!(statements
            .iter()
            .any(|entry| entry.name == "node_claim_a.archive"));
        let by_subject = index.list_dir("/snapshots/7/by-subject/claim_a");
        assert!(by_subject
            .iter()
            .any(|entry| entry.name == "edge_ab.archive"));
    }

    #[test]
    fn fuse_index_read_slices_are_offset_bounded() {
        let pack = pack();
        let index = GraphArchiveFuseIndex::from_pack(&pack);
        let full = index
            .read("/snapshots/7/statements/node_claim_a.archive", 0, u32::MAX)
            .unwrap();
        let slice = index
            .read("/snapshots/7/statements/node_claim_a.archive", 2, 4)
            .unwrap();
        assert_eq!(slice, &full[2..6]);
    }
}
