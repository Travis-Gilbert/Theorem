//! Zero-copy archive boundary for versioned graph objects.
//!
//! Live graph mutation still uses normal Rust records. This module turns the
//! immutable persistence/read boundary into rkyv archives whose bytes are also
//! their content address.

use std::fs::File;
use std::path::Path;

use memmap2::Mmap;
use rkyv::{
    api::high::{access, deserialize, HighSerializer, HighValidator},
    bytecheck::CheckBytes,
    rancor::Error as RkyvError,
    ser::allocator::ArenaHandle,
    util::AlignedVec,
    Portable,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::graph_store::{EdgeRecord, GraphMutation, GraphMutationBatch, NodeRecord};
use crate::symbolic::canonical_json;
use crate::versioned_graph::{
    GraphContentObject, GraphObjectKind, GraphTreeChild, GraphTreeEntry, GraphTreeNode,
};

pub const GRAPH_ARCHIVE_FORMAT_VERSION: u32 = 1;
pub const GRAPH_ARCHIVE_MIME_TYPE: &str = "application/vnd.theorem.graph-archive+rkyv";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZeroCopyArchiveError {
    pub code: String,
    pub message: String,
}

impl ZeroCopyArchiveError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphArchiveObjectBytes {
    pub key: String,
    pub kind: GraphObjectKind,
    pub hash: String,
    pub format_version: u32,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize, Clone, Debug, Eq, PartialEq)]
#[rkyv(derive(Debug))]
pub struct GraphArchiveEnvelope {
    pub format_version: u32,
    pub body: GraphArchiveBody,
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize, Clone, Debug, Eq, PartialEq)]
#[rkyv(derive(Debug))]
pub enum GraphArchiveBody {
    ContentObject(GraphArchiveContentObject),
    TreeNode(GraphArchiveTreeNode),
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize, Clone, Debug, Eq, PartialEq)]
#[rkyv(derive(Debug))]
pub struct GraphArchiveContentObject {
    pub key: String,
    pub kind: String,
    pub logical_hash: String,
    pub parent_hashes: Vec<String>,
    pub payload_json: Option<String>,
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize, Clone, Debug, Eq, PartialEq)]
#[rkyv(derive(Debug))]
pub struct GraphArchiveTreeEntry {
    pub key: String,
    pub kind: String,
    pub object_hash: String,
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize, Clone, Debug, Eq, PartialEq)]
#[rkyv(derive(Debug))]
pub struct GraphArchiveTreeChild {
    pub hash: String,
    pub first_key: String,
    pub last_key: String,
    pub entries_total: usize,
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize, Clone, Debug, Eq, PartialEq)]
#[rkyv(derive(Debug))]
pub struct GraphArchiveTreeNode {
    pub hash: String,
    pub level: u32,
    pub first_key: String,
    pub last_key: String,
    pub entries: Vec<GraphArchiveTreeEntry>,
    pub children: Vec<GraphArchiveTreeChild>,
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize, Clone, Debug, Eq, PartialEq)]
#[rkyv(derive(Debug))]
pub struct ArchiveNode {
    pub id: String,
    pub labels: Vec<String>,
    pub properties_json: String,
    pub version: u64,
    pub tombstone: bool,
    pub content_hash: Option<String>,
    pub parent_hashes: Vec<String>,
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize, Clone, Debug, PartialEq)]
#[rkyv(derive(Debug))]
pub struct ArchiveEdge {
    pub id: String,
    pub from_id: String,
    pub to_id: String,
    pub edge_type: String,
    pub properties_json: String,
    pub version: u64,
    pub tombstone: bool,
    pub confidence: Option<f64>,
    pub epistemic_type: Option<String>,
    pub provenance_json: Option<String>,
    pub content_hash: Option<String>,
    pub parent_hashes: Vec<String>,
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize, Clone, Debug, PartialEq)]
#[rkyv(derive(Debug))]
pub enum ArchiveGraphMutation {
    NodeUpsert(ArchiveNode),
    EdgeUpsert(ArchiveEdge),
}

#[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize, Clone, Debug, PartialEq)]
#[rkyv(derive(Debug))]
pub struct ArchiveGraphMutationLog {
    pub format_version: u32,
    pub mutations: Vec<ArchiveGraphMutation>,
}

pub fn to_archive<T>(value: &T) -> Result<Vec<u8>, ZeroCopyArchiveError>
where
    T: for<'a> rkyv::Serialize<HighSerializer<AlignedVec, ArenaHandle<'a>, RkyvError>>,
{
    rkyv::to_bytes::<RkyvError>(value)
        .map(|bytes| bytes.to_vec())
        .map_err(|error| ZeroCopyArchiveError::new("archive_serialize", error.to_string()))
}

pub fn access_archive<T>(bytes: &[u8]) -> Result<&T, ZeroCopyArchiveError>
where
    T: Portable + for<'a> CheckBytes<HighValidator<'a, RkyvError>>,
{
    access::<T, RkyvError>(bytes)
        .map_err(|error| ZeroCopyArchiveError::new("archive_access", error.to_string()))
}

pub fn access_graph_archive(
    bytes: &[u8],
) -> Result<&ArchivedGraphArchiveEnvelope, ZeroCopyArchiveError> {
    access_archive::<ArchivedGraphArchiveEnvelope>(bytes)
}

pub fn deserialize_graph_archive(
    bytes: &[u8],
) -> Result<GraphArchiveEnvelope, ZeroCopyArchiveError> {
    let archived = access_graph_archive(bytes)?;
    deserialize::<GraphArchiveEnvelope, RkyvError>(archived)
        .map_err(|error| ZeroCopyArchiveError::new("archive_deserialize", error.to_string()))
}

pub fn archive_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

pub fn content_object_archive_bytes(
    object: &GraphContentObject,
) -> Result<Vec<u8>, ZeroCopyArchiveError> {
    let envelope = GraphArchiveEnvelope {
        format_version: GRAPH_ARCHIVE_FORMAT_VERSION,
        body: GraphArchiveBody::ContentObject(GraphArchiveContentObject {
            key: object.key.clone(),
            kind: object.kind.as_str().to_string(),
            logical_hash: object
                .logical_hash
                .clone()
                .unwrap_or_else(|| object.hash.clone()),
            parent_hashes: object.parent_hashes.clone(),
            payload_json: object
                .payload
                .as_ref()
                .map(canonical_json)
                .transpose()
                .map_err(|message| ZeroCopyArchiveError::new("archive_payload_json", message))?,
        }),
    };
    to_archive(&envelope)
}

pub fn archive_content_object(
    object: &GraphContentObject,
) -> Result<GraphArchiveObjectBytes, ZeroCopyArchiveError> {
    let bytes = content_object_archive_bytes(object)?;
    Ok(GraphArchiveObjectBytes {
        key: object.key.clone(),
        kind: object.kind.clone(),
        hash: archive_hash(&bytes),
        format_version: GRAPH_ARCHIVE_FORMAT_VERSION,
        mime_type: GRAPH_ARCHIVE_MIME_TYPE.to_string(),
        bytes,
    })
}

pub fn archive_content_objects(
    objects: &[GraphContentObject],
) -> Result<Vec<GraphArchiveObjectBytes>, ZeroCopyArchiveError> {
    objects.iter().map(archive_content_object).collect()
}

pub fn tree_node_archive_bytes(node: &GraphTreeNode) -> Result<Vec<u8>, ZeroCopyArchiveError> {
    let envelope = GraphArchiveEnvelope {
        format_version: GRAPH_ARCHIVE_FORMAT_VERSION,
        body: GraphArchiveBody::TreeNode(GraphArchiveTreeNode {
            hash: node.hash.clone(),
            level: node.level,
            first_key: node.first_key.clone(),
            last_key: node.last_key.clone(),
            entries: node
                .entries
                .iter()
                .map(archive_tree_entry)
                .collect::<Vec<_>>(),
            children: node
                .children
                .iter()
                .map(archive_tree_child)
                .collect::<Vec<_>>(),
        }),
    };
    to_archive(&envelope)
}

pub fn node_to_archive(node: &NodeRecord) -> Result<ArchiveNode, ZeroCopyArchiveError> {
    Ok(ArchiveNode {
        id: node.id.clone(),
        labels: node.labels.clone(),
        properties_json: canonical_json(&node.properties)
            .map_err(|message| ZeroCopyArchiveError::new("archive_node_json", message))?,
        version: node.version,
        tombstone: node.tombstone,
        content_hash: node.content_hash.clone(),
        parent_hashes: node.parent_hashes.clone(),
    })
}

pub fn edge_to_archive(edge: &EdgeRecord) -> Result<ArchiveEdge, ZeroCopyArchiveError> {
    Ok(ArchiveEdge {
        id: edge.id.clone(),
        from_id: edge.from_id.clone(),
        to_id: edge.to_id.clone(),
        edge_type: edge.edge_type.clone(),
        properties_json: canonical_json(&edge.properties)
            .map_err(|message| ZeroCopyArchiveError::new("archive_edge_json", message))?,
        version: edge.version,
        tombstone: edge.tombstone,
        confidence: edge.confidence,
        epistemic_type: edge.epistemic_type.as_ref().map(ToString::to_string),
        provenance_json: edge
            .provenance
            .as_ref()
            .map(|value| {
                serde_json::to_value(value)
                    .map_err(|error| error.to_string())
                    .and_then(|value| canonical_json(&value))
            })
            .transpose()
            .map_err(|message| ZeroCopyArchiveError::new("archive_edge_provenance", message))?,
        content_hash: edge.content_hash.clone(),
        parent_hashes: edge.parent_hashes.clone(),
    })
}

pub fn mutation_to_archive(
    mutation: &GraphMutation,
) -> Result<ArchiveGraphMutation, ZeroCopyArchiveError> {
    match mutation {
        GraphMutation::NodeUpsert(node) => {
            node_to_archive(node).map(ArchiveGraphMutation::NodeUpsert)
        }
        GraphMutation::EdgeUpsert(edge) => {
            edge_to_archive(edge).map(ArchiveGraphMutation::EdgeUpsert)
        }
    }
}

pub fn archive_event_log(batch: &GraphMutationBatch) -> Result<Vec<u8>, ZeroCopyArchiveError> {
    let log = ArchiveGraphMutationLog {
        format_version: GRAPH_ARCHIVE_FORMAT_VERSION,
        mutations: batch
            .mutations
            .iter()
            .map(mutation_to_archive)
            .collect::<Result<Vec<_>, _>>()?,
    };
    to_archive(&log)
}

pub fn replay_event_log(
    bytes: &[u8],
) -> Result<&ArchivedArchiveGraphMutationLog, ZeroCopyArchiveError> {
    access_archive::<ArchivedArchiveGraphMutationLog>(bytes)
}

pub struct MappedArchive {
    mmap: Mmap,
}

impl MappedArchive {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ZeroCopyArchiveError> {
        let file = File::open(path.as_ref())
            .map_err(|error| ZeroCopyArchiveError::new("archive_mmap_open", error.to_string()))?;
        let mmap = unsafe {
            Mmap::map(&file)
                .map_err(|error| ZeroCopyArchiveError::new("archive_mmap_map", error.to_string()))?
        };
        Ok(Self { mmap })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.mmap
    }
}

fn archive_tree_entry(entry: &GraphTreeEntry) -> GraphArchiveTreeEntry {
    GraphArchiveTreeEntry {
        key: entry.key.clone(),
        kind: entry.kind.as_str().to_string(),
        object_hash: entry.object_hash.clone(),
    }
}

fn archive_tree_child(child: &GraphTreeChild) -> GraphArchiveTreeChild {
    GraphArchiveTreeChild {
        hash: child.hash.clone(),
        first_key: child.first_key.clone(),
        last_key: child.last_key.clone(),
        entries_total: child.entries_total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_store::{EdgeRecord, NodeRecord};
    use crate::versioned_graph::{edge_to_content_object, node_to_content_object};
    use serde_json::json;

    #[test]
    fn archive_hash_is_over_canonical_bytes() {
        let node = NodeRecord::new("n1", ["Claim"], json!({"claim_text": "A"}));
        let object = node_to_content_object(&node);
        let first = archive_content_object(&object).unwrap();
        let second = archive_content_object(&object).unwrap();
        assert_eq!(first.hash, second.hash);
        assert_eq!(first.hash, archive_hash(&first.bytes));
        access_graph_archive(&first.bytes).unwrap();
    }

    #[test]
    fn event_log_replays_from_archive_bytes() {
        let batch = GraphMutationBatch::new([
            GraphMutation::NodeUpsert(NodeRecord::new("n1", ["Claim"], json!({}))),
            GraphMutation::EdgeUpsert(EdgeRecord::new("e1", "n1", "SUPPORTS", "n1", json!({}))),
        ]);
        let bytes = archive_event_log(&batch).unwrap();
        let archived = replay_event_log(&bytes).unwrap();
        assert_eq!(archived.format_version, GRAPH_ARCHIVE_FORMAT_VERSION);
        assert_eq!(archived.mutations.len(), 2);
    }

    #[test]
    fn edge_archive_bytes_are_accessible_without_json_decode() {
        let edge = EdgeRecord::new("e1", "a", "SUPPORTS", "b", json!({"confidence": 1.0}));
        let object = edge_to_content_object(&edge);
        let archived = archive_content_object(&object).unwrap();
        access_graph_archive(&archived.bytes).unwrap();
    }
}
