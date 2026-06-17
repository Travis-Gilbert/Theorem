use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::graph_store::{
    unix_ms, EdgeRecord, GraphMutation, GraphMutationBatch, GraphSnapshot, NodeRecord,
};
use crate::state::stable_hash;

pub const VERSIONED_GRAPH_PROTOCOL_VERSION: &str = "rustyred-versioned-graph-v1";
pub const GRAPH_PACK_COMPILER_VERSION: &str = "rustyred-graph-pack-compiler-v1";
pub const DEFAULT_GRAPH_BRANCH: &str = "main";

const PROLLY_MIN_ENTRIES: usize = 4;
const PROLLY_MAX_ENTRIES: usize = 16;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphObjectKind {
    Node,
    Edge,
}

impl GraphObjectKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Edge => "edge",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphContentObject {
    pub key: String,
    pub kind: GraphObjectKind,
    pub hash: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parent_hashes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphTreeEntry {
    pub key: String,
    pub kind: GraphObjectKind,
    pub object_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphTreeChild {
    pub hash: String,
    pub first_key: String,
    pub last_key: String,
    pub entries_total: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphTreeNode {
    pub hash: String,
    pub level: u32,
    pub first_key: String,
    pub last_key: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<GraphTreeEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<GraphTreeChild>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphProllyTree {
    pub root_hash: String,
    pub root_level: u32,
    pub entries_total: usize,
    pub nodes: Vec<GraphTreeNode>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphCommit {
    pub commit_hash: String,
    pub tree_hash: String,
    pub branch: String,
    pub parent_commits: Vec<String>,
    pub author: String,
    pub message: String,
    pub timestamp_unix_ms: u128,
    pub graph_version: u64,
    pub objects_total: usize,
    pub compiler_version: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphCompilerCapability {
    pub name: String,
    pub kind: String,
    pub mime_type: String,
    pub content_hash: String,
    pub body: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphPackManifest {
    pub protocol_version: String,
    pub compiler_version: String,
    pub name: String,
    pub branch: String,
    pub commit_hash: String,
    pub tree_hash: String,
    pub graph_version: u64,
    pub nodes_total: usize,
    pub edges_total: usize,
    pub objects_total: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompiledGraphPack {
    pub protocol_version: String,
    pub compiler_version: String,
    pub manifest: GraphPackManifest,
    pub commit: GraphCommit,
    pub tree: GraphProllyTree,
    pub capabilities: Vec<GraphCompilerCapability>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<GraphContentObject>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphCompileOptions {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub parent_commits: Vec<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub timestamp_unix_ms: Option<u128>,
    #[serde(default = "default_include_payloads")]
    pub include_payloads: bool,
}

impl Default for GraphCompileOptions {
    fn default() -> Self {
        Self {
            name: None,
            branch: None,
            parent_commits: Vec::new(),
            author: None,
            message: None,
            timestamp_unix_ms: None,
            include_payloads: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphDiffEntry {
    pub key: String,
    pub kind: GraphObjectKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_hash: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphVersionDiff {
    pub protocol_version: String,
    pub base_tree_hash: String,
    pub target_tree_hash: String,
    pub added: Vec<GraphDiffEntry>,
    pub removed: Vec<GraphDiffEntry>,
    pub modified: Vec<GraphDiffEntry>,
    pub unchanged: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphVersionRef {
    pub name: String,
    pub commit_hash: String,
    pub tree_hash: String,
    pub graph_version: u64,
    pub updated_at_unix_ms: u128,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphVersionRepository {
    #[serde(default = "versioned_graph_protocol_string")]
    pub protocol_version: String,
    #[serde(default)]
    pub refs: Vec<GraphVersionRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commits: Vec<GraphCommit>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub objects: BTreeMap<String, GraphContentObject>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tree_nodes: BTreeMap<String, GraphTreeNode>,
    #[serde(default)]
    pub packs: Vec<CompiledGraphPack>,
}

impl Default for GraphVersionRepository {
    fn default() -> Self {
        Self {
            protocol_version: VERSIONED_GRAPH_PROTOCOL_VERSION.to_string(),
            refs: Vec::new(),
            commits: Vec::new(),
            objects: BTreeMap::new(),
            tree_nodes: BTreeMap::new(),
            packs: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphRefUpdate {
    pub protocol_version: String,
    pub repository: GraphVersionRepository,
    pub reference: GraphVersionRef,
    pub pack: CompiledGraphPack,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IncrementalGraphPack {
    pub protocol_version: String,
    pub pack: CompiledGraphPack,
    pub changed_object_keys: Vec<String>,
    pub changed_tree_nodes: usize,
    pub reused_tree_nodes: usize,
    /// Per-commit structural-sharing accounting for this incremental commit
    /// (the genuine O(changed + log n) work + the persisted delta). Defaulted
    /// for backward compatibility with packs serialized before this field.
    #[serde(default)]
    pub commit_cost: CommitCost,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommitCost {
    pub changed_objects: usize,
    pub changed_tree_nodes: usize,
    pub reused_tree_nodes: usize,
    pub chunks_written: usize,
    pub bytes_written: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphRefConflict {
    pub protocol_version: String,
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_commit_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_commit_hash: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphVersionLog {
    pub protocol_version: String,
    pub target: String,
    pub commits: Vec<GraphCommit>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphCheckoutResult {
    pub protocol_version: String,
    pub target: String,
    pub commit: GraphCommit,
    pub tree: GraphProllyTree,
    pub snapshot: GraphSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphMergeStrategy {
    AutoConfidence,
    PreferOurs,
    PreferTheirs,
    Manual,
}

impl Default for GraphMergeStrategy {
    fn default() -> Self {
        Self::AutoConfidence
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphMergeOptions {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub parent_commits: Vec<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub timestamp_unix_ms: Option<u128>,
    #[serde(default = "default_include_payloads")]
    pub include_payloads: bool,
    #[serde(default)]
    pub strategy: GraphMergeStrategy,
    #[serde(default)]
    pub min_confidence_delta: f64,
}

impl Default for GraphMergeOptions {
    fn default() -> Self {
        Self {
            name: None,
            branch: None,
            parent_commits: Vec::new(),
            author: None,
            message: None,
            timestamp_unix_ms: None,
            include_payloads: true,
            strategy: GraphMergeStrategy::AutoConfidence,
            min_confidence_delta: 0.0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphMergeSide {
    Ours,
    Theirs,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphMergeResolution {
    pub key: String,
    pub kind: GraphObjectKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<GraphMergeSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ours_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theirs_hash: Option<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphMergeConflict {
    pub key: String,
    pub kind: GraphObjectKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ours_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theirs_hash: Option<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphMergeResult {
    pub protocol_version: String,
    pub status: String,
    pub base_tree_hash: String,
    pub ours_tree_hash: String,
    pub theirs_tree_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_tree_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_snapshot: Option<GraphSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_pack: Option<CompiledGraphPack>,
    pub resolved: Vec<GraphMergeResolution>,
    pub conflicts: Vec<GraphMergeConflict>,
}

fn default_include_payloads() -> bool {
    true
}

fn versioned_graph_protocol_string() -> String {
    VERSIONED_GRAPH_PROTOCOL_VERSION.to_string()
}

pub fn compile_graph_pack(
    snapshot: &GraphSnapshot,
    options: GraphCompileOptions,
) -> CompiledGraphPack {
    let name = clean_or_default(options.name, "graph-pack");
    let branch = clean_or_default(options.branch, DEFAULT_GRAPH_BRANCH);
    let author = clean_or_default(options.author, "rustyred");
    let message = clean_or_default(options.message, "compile graph snapshot");
    let timestamp_unix_ms = options.timestamp_unix_ms.unwrap_or_else(unix_ms);
    let objects = snapshot_content_objects(snapshot, options.include_payloads);
    let tree = build_prolly_tree(&objects);
    let commit = build_commit(
        &tree,
        &branch,
        options.parent_commits,
        &author,
        &message,
        timestamp_unix_ms,
        snapshot.version,
    );
    let manifest = GraphPackManifest {
        protocol_version: VERSIONED_GRAPH_PROTOCOL_VERSION.to_string(),
        compiler_version: GRAPH_PACK_COMPILER_VERSION.to_string(),
        name: name.clone(),
        branch: branch.clone(),
        commit_hash: commit.commit_hash.clone(),
        tree_hash: tree.root_hash.clone(),
        graph_version: snapshot.version,
        nodes_total: snapshot.nodes.len(),
        edges_total: snapshot.edges.len(),
        objects_total: objects.len(),
    };
    let manifest_body = serde_json::to_value(&manifest).unwrap_or_else(|_| json!({}));
    let validator_body = json!({
        "validator": "rustyred.verify_tree_root",
        "tree_hash": tree.root_hash,
        "objects_total": objects.len(),
        "protocol_version": VERSIONED_GRAPH_PROTOCOL_VERSION,
    });
    let capabilities = vec![
        GraphCompilerCapability {
            name: format!("{name}.manifest"),
            kind: "graph_manifest".to_string(),
            mime_type: "application/json".to_string(),
            content_hash: stable_hash(&manifest_body),
            body: manifest_body,
        },
        GraphCompilerCapability {
            name: format!("{name}.verify_tree_root"),
            kind: "validator".to_string(),
            mime_type: "application/json".to_string(),
            content_hash: stable_hash(&validator_body),
            body: validator_body,
        },
    ];

    CompiledGraphPack {
        protocol_version: VERSIONED_GRAPH_PROTOCOL_VERSION.to_string(),
        compiler_version: GRAPH_PACK_COMPILER_VERSION.to_string(),
        manifest,
        commit,
        tree,
        capabilities,
        objects: if options.include_payloads {
            objects
        } else {
            Vec::new()
        },
    }
}

pub fn apply_graph_mutation_batch(
    snapshot: &GraphSnapshot,
    batch: &GraphMutationBatch,
) -> GraphSnapshot {
    let mut nodes = snapshot
        .nodes
        .iter()
        .cloned()
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let mut edges = snapshot
        .edges
        .iter()
        .cloned()
        .map(|edge| (edge.id.clone(), edge))
        .collect::<BTreeMap<_, _>>();

    for mutation in &batch.mutations {
        match mutation {
            GraphMutation::NodeUpsert(node) => {
                nodes.insert(node.id.clone(), node.clone());
            }
            GraphMutation::EdgeUpsert(edge) => {
                edges.insert(edge.id.clone(), edge.clone());
            }
        }
    }

    GraphSnapshot {
        version: snapshot.version.saturating_add(1),
        nodes: nodes.into_values().collect(),
        edges: edges.into_values().collect(),
    }
}

pub fn compile_graph_pack_incremental(
    prior_pack: &CompiledGraphPack,
    batch: &GraphMutationBatch,
    mut options: GraphCompileOptions,
) -> IncrementalGraphPack {
    let name = clean_or_default(options.name.take(), "graph-pack");
    let branch = clean_or_default(options.branch.take(), DEFAULT_GRAPH_BRANCH);
    let author = clean_or_default(options.author.take(), "rustyred");
    let message = clean_or_default(options.message.take(), "incremental graph mutation batch");
    let timestamp_unix_ms = options.timestamp_unix_ms.unwrap_or_else(unix_ms);
    let parent_commits = if options.parent_commits.is_empty() {
        vec![prior_pack.commit.commit_hash.clone()]
    } else {
        options.parent_commits
    };

    // Prior object-hash by key, sourced from the prior tree leaves so a
    // COMPACTED prior pack (objects stripped for storage) still drives changed
    // detection -- and so we never clone the parent commit's whole object set
    // (the O(graph) materialization the audit flagged). Only the changed objects
    // ride along as this commit's delta payload.
    let prior_entry_hash: BTreeMap<String, String> = prior_pack
        .tree
        .nodes
        .iter()
        .filter(|node| node.level == 0)
        .flat_map(|node| node.entries.iter())
        .map(|entry| (entry.key.clone(), entry.object_hash.clone()))
        .collect();

    let mut upserts: BTreeMap<String, GraphTreeEntry> = BTreeMap::new();
    let mut changed_objects: Vec<GraphContentObject> = Vec::new();
    let mut changed_object_keys: BTreeSet<String> = BTreeSet::new();
    for mutation in &batch.mutations {
        let object = match mutation {
            GraphMutation::NodeUpsert(node) => node_content_object(node, options.include_payloads),
            GraphMutation::EdgeUpsert(edge) => edge_content_object(edge, options.include_payloads),
        };
        let entry = GraphTreeEntry {
            key: object.key.clone(),
            kind: object.kind.clone(),
            object_hash: object.hash.clone(),
        };
        let changed = prior_entry_hash
            .get(&object.key)
            .map(|hash| hash != &object.hash)
            .unwrap_or(true);
        if changed {
            changed_object_keys.insert(object.key.clone());
            changed_objects.push(object);
        }
        upserts.insert(entry.key.clone(), entry);
    }

    // O(changed + log n) build: re-chunk only the touched leaves + spine,
    // reuse every unchanged chunk by hash. The tree carries the full node set
    // (so the cost set-diff and all downstream consumers are unchanged) but only
    // the changed window + spine are re-hashed/re-serialized.
    let build = build_prolly_tree_incremental(&prior_pack.tree, &upserts);

    // Dual-commit validation (bring-up gate, handoff cautions): build the same
    // final tree the canonical full path would and assert byte-identity + that
    // every node the incremental build emitted belongs to it. On by default in
    // debug/test, off in release, overridable via RUSTYRED_PROLLY_VALIDATE.
    if prolly_validation_enabled() {
        let prior_entries = leaf_entries_in_order(&prior_pack.tree);
        let full = build_prolly_tree_from_entries(merge_entries(&prior_entries, &upserts));
        assert_eq!(
            build.tree.root_hash, full.root_hash,
            "incremental prolly tree root diverged from full rebuild"
        );
        assert_eq!(
            build.tree.entries_total, full.entries_total,
            "incremental entry total diverged from full rebuild"
        );
        let full_hashes: HashSet<&str> = full.nodes.iter().map(|node| node.hash.as_str()).collect();
        for node in &build.tree.nodes {
            assert!(
                full_hashes.contains(node.hash.as_str()),
                "incremental delta node {} absent from full rebuild",
                node.hash
            );
        }
    }

    let tree = build.tree;
    let nodes_total = build.nodes_total;
    let edges_total = build.edges_total;
    let objects = changed_objects;
    let graph_version = prior_pack.commit.graph_version.saturating_add(1);
    let commit = build_commit(
        &tree,
        &branch,
        parent_commits,
        &author,
        &message,
        timestamp_unix_ms,
        graph_version,
    );
    let manifest = GraphPackManifest {
        protocol_version: VERSIONED_GRAPH_PROTOCOL_VERSION.to_string(),
        compiler_version: GRAPH_PACK_COMPILER_VERSION.to_string(),
        name: name.clone(),
        branch: branch.clone(),
        commit_hash: commit.commit_hash.clone(),
        tree_hash: tree.root_hash.clone(),
        graph_version,
        nodes_total,
        edges_total,
        objects_total: tree.entries_total,
    };
    let manifest_body = serde_json::to_value(&manifest).unwrap_or_else(|_| json!({}));
    let validator_body = json!({
        "validator": "rustyred.verify_tree_root",
        "tree_hash": tree.root_hash,
        "objects_total": tree.entries_total,
        "protocol_version": VERSIONED_GRAPH_PROTOCOL_VERSION,
    });
    let changed_tree_nodes = build.cost.chunks_written;
    let reused_tree_nodes = build.cost.chunks_reused;
    let commit_cost = CommitCost {
        changed_objects: changed_object_keys.len(),
        changed_tree_nodes,
        reused_tree_nodes,
        chunks_written: build.cost.chunks_written,
        bytes_written: build.cost.bytes_written,
    };

    IncrementalGraphPack {
        protocol_version: VERSIONED_GRAPH_PROTOCOL_VERSION.to_string(),
        pack: CompiledGraphPack {
            protocol_version: VERSIONED_GRAPH_PROTOCOL_VERSION.to_string(),
            compiler_version: GRAPH_PACK_COMPILER_VERSION.to_string(),
            manifest,
            commit,
            tree,
            capabilities: vec![
                GraphCompilerCapability {
                    name: format!("{name}.manifest"),
                    kind: "graph_manifest".to_string(),
                    mime_type: "application/json".to_string(),
                    content_hash: stable_hash(&manifest_body),
                    body: manifest_body,
                },
                GraphCompilerCapability {
                    name: format!("{name}.verify_tree_root"),
                    kind: "validator".to_string(),
                    mime_type: "application/json".to_string(),
                    content_hash: stable_hash(&validator_body),
                    body: validator_body,
                },
            ],
            objects: if options.include_payloads {
                objects
            } else {
                Vec::new()
            },
        },
        changed_object_keys: changed_object_keys.into_iter().collect(),
        changed_tree_nodes,
        reused_tree_nodes,
        commit_cost,
    }
}

/// Persisted chunk-encoding format version. Bump when the on-disk tree-node or
/// content-object encoding changes, so an older store is migrated rather than
/// silently misread. The chunk HASH stays sha256 (`stable_hash`, this crate's
/// content-address primitive) -- a deliberate, documented divergence from the
/// handoff's "blake3" naming: switching the hash would change every existing
/// pack root and break the snapshot/branch/diff/merge byte-parity tests.
pub const GRAPH_CHUNK_FORMAT_VERSION: u8 = 1;

/// Internal per-commit structural-sharing counters for an incremental build.
/// `chunks_written` = tree nodes whose hash is new vs the parent (the genuine
/// O(changed + log n) re-hash/re-serialize work and the persisted delta);
/// `chunks_reused` = nodes carried over by hash; `bytes_written` = serialized
/// byte size of the written nodes. Mapped onto the public `CommitCost`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TreeBuildCost {
    pub(crate) chunks_written: usize,
    pub(crate) chunks_reused: usize,
    pub(crate) bytes_written: usize,
}

/// Result of an O(changed) incremental Prolly-tree build. `tree` carries the
/// FULL content-addressed node set -- unchanged chunks reused by struct, only
/// the changed window + spine re-hashed -- so `root_hash` is byte-identical to
/// a full `build_prolly_tree` rebuild and every downstream consumer (cost set-
/// diff, checkout, persistence) is unchanged. The expensive work and the
/// persisted delta are O(changed + log n), reported by `cost`. No whole-graph
/// snapshot or NodeRecord/payload clone is allocated; the working set is the
/// tiny key-sorted entry stream.
pub struct IncrementalTreeBuild {
    pub tree: GraphProllyTree,
    pub(crate) cost: TreeBuildCost,
    pub nodes_total: usize,
    pub edges_total: usize,
}

/// Whether to commit both ways (incremental + full rebuild) and assert the
/// trees match. Defaults ON in debug/test builds (so every test cross-checks
/// the incremental commit against the canonical full build) and OFF in release,
/// overridable by `RUSTYRED_PROLLY_VALIDATE=1|true|on|yes`.
pub fn prolly_validation_enabled() -> bool {
    match std::env::var("RUSTYRED_PROLLY_VALIDATE") {
        Ok(value) => matches!(value.trim(), "1" | "true" | "on" | "yes"),
        Err(_) => cfg!(debug_assertions),
    }
}

fn account_tree_node(
    node: &GraphTreeNode,
    prior_hashes: &BTreeSet<String>,
    cost: &mut TreeBuildCost,
) {
    if prior_hashes.contains(&node.hash) {
        cost.chunks_reused += 1;
    } else {
        cost.chunks_written += 1;
        cost.bytes_written += serde_json::to_vec(node)
            .map(|bytes| bytes.len())
            .unwrap_or(0);
    }
}

fn child_descriptor(node: &GraphTreeNode) -> GraphTreeChild {
    GraphTreeChild {
        hash: node.hash.clone(),
        first_key: node.first_key.clone(),
        last_key: node.last_key.clone(),
        entries_total: node.entry_count(),
    }
}

fn prior_level_nodes(tree: &GraphProllyTree, level: u32) -> Vec<GraphTreeNode> {
    let mut nodes: Vec<GraphTreeNode> = tree
        .nodes
        .iter()
        .filter(|node| node.level == level)
        .cloned()
        .collect();
    nodes.sort_by(|a, b| a.first_key.cmp(&b.first_key));
    nodes
}

fn leaf_entries_in_order(tree: &GraphProllyTree) -> Vec<GraphTreeEntry> {
    prior_level_nodes(tree, 0)
        .into_iter()
        .flat_map(|leaf| leaf.entries.into_iter())
        .collect()
}

/// Merge an upsert set into a key-sorted prior entry stream (a
/// `GraphMutationBatch` only upserts, so there are no deletions). Two-pointer
/// merge over two sorted sequences -> O(n + k) cheap iteration, no hashing.
fn merge_entries(
    prior_entries: &[GraphTreeEntry],
    upserts: &BTreeMap<String, GraphTreeEntry>,
) -> Vec<GraphTreeEntry> {
    let mut out = Vec::with_capacity(prior_entries.len() + upserts.len());
    let mut pending = upserts.iter().peekable();
    for entry in prior_entries {
        while let Some((key, value)) = pending.peek() {
            if key.as_str() < entry.key.as_str() {
                out.push((*value).clone());
                pending.next();
            } else {
                break;
            }
        }
        if let Some((key, value)) = pending.peek() {
            if key.as_str() == entry.key.as_str() {
                out.push((*value).clone());
                pending.next();
                continue;
            }
        }
        out.push(entry.clone());
    }
    for (_, value) in pending {
        out.push(value.clone());
    }
    out
}

/// Re-chunk one tree level incrementally, reusing the prior level's unchanged
/// prefix and suffix nodes by hash and re-chunking only the window touched by
/// the change. Returns the full ordered new level (so the next level up can be
/// derived). Content-defined boundaries (`should_split_chunk`) re-synchronize
/// quickly past the last change, so the re-hashed window is O(changed + chunk).
fn rechunk_level<I: Clone + Serialize>(
    prior_level: &[GraphTreeNode],
    new_items: &[I],
    item_key: impl Fn(&I) -> String,
    make_node: impl Fn(&[I]) -> GraphTreeNode,
    changed_keys: &BTreeSet<String>,
    prior_hashes: &BTreeSet<String>,
    cost: &mut TreeBuildCost,
) -> Vec<GraphTreeNode> {
    if new_items.is_empty() {
        let node = make_node(&[]);
        account_tree_node(&node, prior_hashes, cost);
        return vec![node];
    }
    // No prior structure at this level (e.g. growing from empty): full chunk.
    if prior_level.is_empty() {
        let nodes = chunk_by_boundary(new_items.to_vec(), |chunk| make_node(chunk));
        for node in &nodes {
            account_tree_node(node, prior_hashes, cost);
        }
        return nodes;
    }

    // Window bounds: first and last new-item indices whose key changed.
    let mut lo: Option<usize> = None;
    let mut hi = 0usize;
    for (idx, item) in new_items.iter().enumerate() {
        if changed_keys.contains(&item_key(item)) {
            if lo.is_none() {
                lo = Some(idx);
            }
            hi = idx;
        }
    }
    let Some(lo) = lo else {
        // Nothing changed at this level: reuse every prior node verbatim.
        cost.chunks_reused += prior_level.len();
        return prior_level.to_vec();
    };
    let max_changed_key = item_key(&new_items[hi]);
    let lo_key = item_key(&new_items[lo]);

    // The re-chunk window starts at the first item of the prior node containing
    // (or immediately preceding) the first change. That node's left boundary is
    // stable -- its predecessors are unchanged -- so the prefix is reusable.
    let mut start_key = prior_level[0].first_key.clone();
    for node in prior_level {
        if node.first_key.as_str() <= lo_key.as_str() {
            start_key = node.first_key.clone();
        } else {
            break;
        }
    }
    let start_idx = new_items.partition_point(|item| item_key(item).as_str() < start_key.as_str());

    let mut result: Vec<GraphTreeNode> = Vec::new();
    for node in prior_level {
        if node.first_key.as_str() < start_key.as_str() {
            cost.chunks_reused += 1;
            result.push(node.clone());
        } else {
            break;
        }
    }

    let mut idx = start_idx;
    let mut chunk: Vec<I> = Vec::new();
    while idx < new_items.len() {
        // Re-sync: at a fresh chunk boundary, past the last change, where the
        // next item starts a prior node -> the unchanged suffix is reusable.
        if chunk.is_empty() && idx > start_idx {
            let key = item_key(&new_items[idx]);
            if key.as_str() > max_changed_key.as_str() {
                if let Ok(pos) =
                    prior_level.binary_search_by(|node| node.first_key.as_str().cmp(key.as_str()))
                {
                    for node in &prior_level[pos..] {
                        cost.chunks_reused += 1;
                        result.push(node.clone());
                    }
                    return result;
                }
            }
        }
        chunk.push(new_items[idx].clone());
        let hash = stable_hash(chunk.last().expect("chunk has item"));
        if should_split_chunk(chunk.len(), &hash) {
            let node = make_node(&chunk);
            account_tree_node(&node, prior_hashes, cost);
            result.push(node);
            chunk.clear();
        }
        idx += 1;
    }
    if !chunk.is_empty() {
        let node = make_node(&chunk);
        account_tree_node(&node, prior_hashes, cost);
        result.push(node);
    }
    result
}

/// Build the new Prolly tree from the parent commit's tree plus an upsert set,
/// re-chunking only the affected leaves and the O(log n) spine above them and
/// reusing every unchanged chunk by hash. The returned tree is FULL (so it is a
/// drop-in for `build_prolly_tree`) and its `root_hash` is byte-identical to a
/// full rebuild over the same final entry set; the expensive re-hash work and
/// the persisted delta are O(changed + log n).
pub fn build_prolly_tree_incremental(
    prior_tree: &GraphProllyTree,
    upserts: &BTreeMap<String, GraphTreeEntry>,
) -> IncrementalTreeBuild {
    let prior_hashes: BTreeSet<String> = prior_tree
        .nodes
        .iter()
        .map(|node| node.hash.clone())
        .collect();
    let prior_entries = leaf_entries_in_order(prior_tree);
    let new_entries = merge_entries(&prior_entries, upserts);
    let entries_total = new_entries.len();
    let nodes_total = new_entries
        .iter()
        .filter(|entry| entry.kind == GraphObjectKind::Node)
        .count();
    let edges_total = entries_total - nodes_total;

    let mut cost = TreeBuildCost::default();
    let mut all_nodes: Vec<GraphTreeNode> = Vec::new();

    if new_entries.is_empty() {
        let root = empty_leaf_node();
        account_tree_node(&root, &prior_hashes, &mut cost);
        let root_hash = root.hash.clone();
        all_nodes.push(root);
        return IncrementalTreeBuild {
            tree: GraphProllyTree {
                root_hash,
                root_level: 0,
                entries_total: 0,
                nodes: all_nodes,
            },
            cost,
            nodes_total: 0,
            edges_total: 0,
        };
    }

    // Every upsert key bounds the change window. A no-op upsert (identical
    // entry) re-chunks one leaf to an identical hash -> counted as reused, root
    // unchanged: correctness holds, the window is just slightly wider.
    let changed_entry_keys: BTreeSet<String> = upserts.keys().cloned().collect();

    let prior_leaves = prior_level_nodes(prior_tree, 0);
    let mut level_nodes = rechunk_level(
        &prior_leaves,
        &new_entries,
        |entry: &GraphTreeEntry| entry.key.clone(),
        |chunk: &[GraphTreeEntry]| make_leaf_node(chunk.to_vec()),
        &changed_entry_keys,
        &prior_hashes,
        &mut cost,
    );
    all_nodes.extend(level_nodes.iter().cloned());

    let mut level = 1u32;
    while level_nodes.len() > 1 {
        let prior_parents = prior_level_nodes(prior_tree, level);
        let descriptors: Vec<GraphTreeChild> = level_nodes.iter().map(child_descriptor).collect();
        // A parent changes iff one of its child node hashes is new.
        let changed_child_keys: BTreeSet<String> = descriptors
            .iter()
            .filter(|child| !prior_hashes.contains(&child.hash))
            .map(|child| child.first_key.clone())
            .collect();
        level_nodes = rechunk_level(
            &prior_parents,
            &descriptors,
            |child: &GraphTreeChild| child.first_key.clone(),
            |chunk: &[GraphTreeChild]| make_parent_node(level, chunk.to_vec()),
            &changed_child_keys,
            &prior_hashes,
            &mut cost,
        );
        all_nodes.extend(level_nodes.iter().cloned());
        level += 1;
    }

    let root = level_nodes
        .into_iter()
        .next()
        .unwrap_or_else(empty_leaf_node);
    IncrementalTreeBuild {
        tree: GraphProllyTree {
            root_hash: root.hash.clone(),
            root_level: root.level,
            entries_total,
            nodes: all_nodes,
        },
        cost,
        nodes_total,
        edges_total,
    }
}

/// O(k + log n) adjacent-commit diff over two FULL Prolly trees. Prunes any
/// subtree whose root hash is shared with the other side (identical content),
/// so only the changed region plus its O(log n) spine is visited. Returns the
/// diff plus the tree-node visit count (handoff acceptance #3's chunk-visit
/// counter): O(differences), never O(graph), for trees that share structure.
pub fn diff_graph_trees(
    base: &GraphProllyTree,
    target: &GraphProllyTree,
) -> (GraphVersionDiff, usize) {
    let base_nodes: HashMap<&str, &GraphTreeNode> = base
        .nodes
        .iter()
        .map(|node| (node.hash.as_str(), node))
        .collect();
    let target_nodes: HashMap<&str, &GraphTreeNode> = target
        .nodes
        .iter()
        .map(|node| (node.hash.as_str(), node))
        .collect();
    let base_hashes: HashSet<&str> = base.nodes.iter().map(|node| node.hash.as_str()).collect();
    let target_hashes: HashSet<&str> = target.nodes.iter().map(|node| node.hash.as_str()).collect();

    let mut visits = 0usize;
    let mut base_changed: BTreeMap<String, GraphTreeEntry> = BTreeMap::new();
    collect_changed_entries(
        &base.root_hash,
        &base_nodes,
        &target_hashes,
        &mut base_changed,
        &mut visits,
    );
    let mut target_changed: BTreeMap<String, GraphTreeEntry> = BTreeMap::new();
    collect_changed_entries(
        &target.root_hash,
        &target_nodes,
        &base_hashes,
        &mut target_changed,
        &mut visits,
    );

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();
    for (key, target_entry) in &target_changed {
        match base_changed.get(key) {
            None => added.push(GraphDiffEntry {
                key: key.clone(),
                kind: target_entry.kind.clone(),
                old_hash: None,
                new_hash: Some(target_entry.object_hash.clone()),
            }),
            Some(base_entry) if base_entry.object_hash != target_entry.object_hash => {
                modified.push(GraphDiffEntry {
                    key: key.clone(),
                    kind: target_entry.kind.clone(),
                    old_hash: Some(base_entry.object_hash.clone()),
                    new_hash: Some(target_entry.object_hash.clone()),
                });
            }
            Some(_) => {}
        }
    }
    for (key, base_entry) in &base_changed {
        if !target_changed.contains_key(key) {
            removed.push(GraphDiffEntry {
                key: key.clone(),
                kind: base_entry.kind.clone(),
                old_hash: Some(base_entry.object_hash.clone()),
                new_hash: None,
            });
        }
    }
    let unchanged = target
        .entries_total
        .saturating_sub(added.len() + modified.len());

    (
        GraphVersionDiff {
            protocol_version: VERSIONED_GRAPH_PROTOCOL_VERSION.to_string(),
            base_tree_hash: base.root_hash.clone(),
            target_tree_hash: target.root_hash.clone(),
            added,
            removed,
            modified,
            unchanged,
        },
        visits,
    )
}

fn collect_changed_entries(
    hash: &str,
    nodes: &HashMap<&str, &GraphTreeNode>,
    other_hashes: &HashSet<&str>,
    out: &mut BTreeMap<String, GraphTreeEntry>,
    visits: &mut usize,
) {
    *visits += 1;
    // Identical subtree present on the other side -> nothing in it changed.
    if other_hashes.contains(hash) {
        return;
    }
    let Some(node) = nodes.get(hash) else {
        return;
    };
    if node.level == 0 {
        for entry in &node.entries {
            out.insert(entry.key.clone(), entry.clone());
        }
    } else {
        for child in &node.children {
            collect_changed_entries(&child.hash, nodes, other_hashes, out, visits);
        }
    }
}

pub fn diff_graph_snapshots(base: &GraphSnapshot, target: &GraphSnapshot) -> GraphVersionDiff {
    let base_objects = snapshot_content_objects(base, false);
    let target_objects = snapshot_content_objects(target, false);
    let base_tree = build_prolly_tree(&base_objects);
    let target_tree = build_prolly_tree(&target_objects);
    let base_map = base_objects
        .into_iter()
        .map(|object| (object.key.clone(), object))
        .collect::<std::collections::BTreeMap<_, _>>();
    let target_map = target_objects
        .into_iter()
        .map(|object| (object.key.clone(), object))
        .collect::<std::collections::BTreeMap<_, _>>();

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();
    let mut unchanged = 0usize;

    for (key, target_object) in &target_map {
        match base_map.get(key) {
            None => added.push(GraphDiffEntry {
                key: key.clone(),
                kind: target_object.kind.clone(),
                old_hash: None,
                new_hash: Some(target_object.hash.clone()),
            }),
            Some(base_object) if base_object.hash != target_object.hash => {
                modified.push(GraphDiffEntry {
                    key: key.clone(),
                    kind: target_object.kind.clone(),
                    old_hash: Some(base_object.hash.clone()),
                    new_hash: Some(target_object.hash.clone()),
                });
            }
            Some(_) => unchanged += 1,
        }
    }

    for (key, base_object) in &base_map {
        if !target_map.contains_key(key) {
            removed.push(GraphDiffEntry {
                key: key.clone(),
                kind: base_object.kind.clone(),
                old_hash: Some(base_object.hash.clone()),
                new_hash: None,
            });
        }
    }

    GraphVersionDiff {
        protocol_version: VERSIONED_GRAPH_PROTOCOL_VERSION.to_string(),
        base_tree_hash: base_tree.root_hash,
        target_tree_hash: target_tree.root_hash,
        added,
        removed,
        modified,
        unchanged,
    }
}

pub fn update_graph_ref(
    repository: GraphVersionRepository,
    pack: CompiledGraphPack,
    branch: Option<String>,
    updated_at_unix_ms: Option<u128>,
) -> GraphRefUpdate {
    update_graph_ref_cas(repository, pack, branch, None, updated_at_unix_ms)
        .expect("unconditional graph ref update cannot conflict")
}

pub fn update_graph_ref_cas(
    mut repository: GraphVersionRepository,
    pack: CompiledGraphPack,
    branch: Option<String>,
    expected_commit_hash: Option<String>,
    updated_at_unix_ms: Option<u128>,
) -> Result<GraphRefUpdate, GraphRefConflict> {
    repository.protocol_version = VERSIONED_GRAPH_PROTOCOL_VERSION.to_string();
    let ref_name = clean_or_default(
        branch.or_else(|| Some(pack.commit.branch.clone())),
        DEFAULT_GRAPH_BRANCH,
    );
    let actual_commit_hash = repository
        .refs
        .iter()
        .find(|reference| reference.name == ref_name)
        .map(|reference| reference.commit_hash.clone());
    if let Some(expected) = expected_commit_hash.as_ref() {
        if actual_commit_hash.as_deref() != Some(expected.as_str()) {
            return Err(GraphRefConflict {
                protocol_version: VERSIONED_GRAPH_PROTOCOL_VERSION.to_string(),
                branch: ref_name,
                expected_commit_hash,
                actual_commit_hash,
                message: "graph ref compare-and-swap failed".to_string(),
            });
        }
    }

    let updated_at_unix_ms = updated_at_unix_ms.unwrap_or_else(unix_ms);
    let reference = GraphVersionRef {
        name: ref_name.clone(),
        commit_hash: pack.commit.commit_hash.clone(),
        tree_hash: pack.tree.root_hash.clone(),
        graph_version: pack.commit.graph_version,
        updated_at_unix_ms,
    };

    repository
        .refs
        .retain(|reference| reference.name != ref_name);
    repository.refs.push(reference.clone());
    repository.refs.sort_by(|a, b| a.name.cmp(&b.name));

    index_pack_content(&mut repository, &pack);
    let stored_pack = compact_repository_pack(&pack);
    repository
        .packs
        .retain(|existing| existing.commit.commit_hash != stored_pack.commit.commit_hash);
    repository.packs.push(stored_pack);
    repository
        .packs
        .sort_by(|a, b| a.commit.commit_hash.cmp(&b.commit.commit_hash));

    Ok(GraphRefUpdate {
        protocol_version: VERSIONED_GRAPH_PROTOCOL_VERSION.to_string(),
        repository,
        reference,
        pack,
    })
}

pub fn graph_version_log(
    repository: &GraphVersionRepository,
    target: Option<&str>,
) -> GraphVersionLog {
    let target = target
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_GRAPH_BRANCH)
        .to_string();
    let commits_by_hash = repository
        .commits
        .iter()
        .cloned()
        .map(|commit| (commit.commit_hash.clone(), commit))
        .chain(
            repository
                .packs
                .iter()
                .map(|pack| (pack.commit.commit_hash.clone(), pack.commit.clone())),
        )
        .collect::<BTreeMap<_, _>>();
    let mut commits = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    if let Some(head) = resolve_repository_target(repository, &target) {
        queue.push_back(head);
    }

    while let Some(commit_hash) = queue.pop_front() {
        if !seen.insert(commit_hash.clone()) {
            continue;
        }
        let Some(commit) = commits_by_hash.get(&commit_hash) else {
            continue;
        };
        commits.push(commit.clone());
        for parent in &commit.parent_commits {
            queue.push_back(parent.clone());
        }
    }

    GraphVersionLog {
        protocol_version: VERSIONED_GRAPH_PROTOCOL_VERSION.to_string(),
        target,
        commits,
    }
}

fn index_pack_content(repository: &mut GraphVersionRepository, pack: &CompiledGraphPack) {
    if !repository
        .commits
        .iter()
        .any(|commit| commit.commit_hash == pack.commit.commit_hash)
    {
        repository.commits.push(pack.commit.clone());
        repository
            .commits
            .sort_by(|a, b| a.commit_hash.cmp(&b.commit_hash));
    }
    for object in &pack.objects {
        repository
            .objects
            .entry(object.hash.clone())
            .or_insert_with(|| object.clone());
    }
    for node in &pack.tree.nodes {
        repository
            .tree_nodes
            .entry(node.hash.clone())
            .or_insert_with(|| node.clone());
    }
}

fn compact_repository_pack(pack: &CompiledGraphPack) -> CompiledGraphPack {
    let mut compact = pack.clone();
    compact.objects.clear();
    compact.tree.nodes.clear();
    compact
}

pub fn checkout_graph_version(
    repository: &GraphVersionRepository,
    target: &str,
) -> Option<GraphCheckoutResult> {
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    let commit_hash = resolve_repository_target(repository, target)?;
    let pack = repository
        .packs
        .iter()
        .find(|pack| pack.commit.commit_hash == commit_hash)?;
    let tree = repository_tree_for_pack(repository, pack)?;
    let snapshot = graph_snapshot_from_repository(repository, pack)?;
    Some(GraphCheckoutResult {
        protocol_version: VERSIONED_GRAPH_PROTOCOL_VERSION.to_string(),
        target: target.to_string(),
        commit: pack.commit.clone(),
        tree,
        snapshot,
    })
}

pub fn merge_graph_snapshots(
    base: &GraphSnapshot,
    ours: &GraphSnapshot,
    theirs: &GraphSnapshot,
    options: GraphMergeOptions,
) -> GraphMergeResult {
    let base_tree = build_prolly_tree(&snapshot_content_objects(base, false));
    let ours_tree = build_prolly_tree(&snapshot_content_objects(ours, false));
    let theirs_tree = build_prolly_tree(&snapshot_content_objects(theirs, false));
    let base_records = merge_records_by_key(base);
    let ours_records = merge_records_by_key(ours);
    let theirs_records = merge_records_by_key(theirs);
    let mut keys = BTreeSet::new();
    keys.extend(base_records.keys().cloned());
    keys.extend(ours_records.keys().cloned());
    keys.extend(theirs_records.keys().cloned());

    let mut merged_records = Vec::new();
    let mut resolved = Vec::new();
    let mut conflicts = Vec::new();

    for key in keys {
        let base_record = base_records.get(&key);
        let ours_record = ours_records.get(&key);
        let theirs_record = theirs_records.get(&key);
        let kind = merge_record_kind(base_record, ours_record, theirs_record);
        let base_hash = base_record.map(MergeRecord::hash);
        let ours_hash = ours_record.map(MergeRecord::hash);
        let theirs_hash = theirs_record.map(MergeRecord::hash);

        match resolve_merge_record(
            base_record,
            ours_record,
            theirs_record,
            &options.strategy,
            options.min_confidence_delta,
        ) {
            MergeDecision::Resolved {
                selected,
                record,
                reason,
            } => {
                if let Some(record) = record {
                    merged_records.push(record);
                }
                resolved.push(GraphMergeResolution {
                    key,
                    kind,
                    selected,
                    base_hash,
                    ours_hash,
                    theirs_hash,
                    reason,
                });
            }
            MergeDecision::Conflict { reason } => conflicts.push(GraphMergeConflict {
                key,
                kind,
                base_hash,
                ours_hash,
                theirs_hash,
                reason,
            }),
        }
    }

    let (merged_snapshot, merged_pack, merged_tree_hash) = if conflicts.is_empty() {
        let snapshot = snapshot_from_merge_records(
            base.version
                .max(ours.version)
                .max(theirs.version)
                .saturating_add(1),
            merged_records,
        );
        let pack = compile_graph_pack(
            &snapshot,
            GraphCompileOptions {
                name: options
                    .name
                    .or_else(|| Some("merged-graph-pack".to_string())),
                branch: options.branch,
                parent_commits: options.parent_commits,
                author: options.author,
                message: options
                    .message
                    .or_else(|| Some("merge graph snapshots".to_string())),
                timestamp_unix_ms: options.timestamp_unix_ms,
                include_payloads: options.include_payloads,
            },
        );
        (
            Some(snapshot),
            Some(pack.clone()),
            Some(pack.tree.root_hash.clone()),
        )
    } else {
        (None, None, None)
    };

    GraphMergeResult {
        protocol_version: VERSIONED_GRAPH_PROTOCOL_VERSION.to_string(),
        status: if conflicts.is_empty() {
            "clean".to_string()
        } else {
            "conflicted".to_string()
        },
        base_tree_hash: base_tree.root_hash,
        ours_tree_hash: ours_tree.root_hash,
        theirs_tree_hash: theirs_tree.root_hash,
        merged_tree_hash,
        merged_snapshot,
        merged_pack,
        resolved,
        conflicts,
    }
}

pub fn snapshot_content_objects(
    snapshot: &GraphSnapshot,
    include_payloads: bool,
) -> Vec<GraphContentObject> {
    let mut objects = Vec::with_capacity(snapshot.nodes.len() + snapshot.edges.len());
    for node in &snapshot.nodes {
        objects.push(node_content_object(node, include_payloads));
    }
    for edge in &snapshot.edges {
        objects.push(edge_content_object(edge, include_payloads));
    }
    objects.sort_by(|a, b| a.key.cmp(&b.key));
    objects
}

pub fn build_prolly_tree(objects: &[GraphContentObject]) -> GraphProllyTree {
    let entries = objects
        .iter()
        .map(|object| GraphTreeEntry {
            key: object.key.clone(),
            kind: object.kind.clone(),
            object_hash: object.hash.clone(),
        })
        .collect::<Vec<_>>();
    build_prolly_tree_from_entries(entries)
}

/// Full Prolly-tree build from an already-prepared, key-sorted entry list. The
/// reference (oracle) build: `build_prolly_tree` and the dual-commit validation
/// path both route through it so the incremental commit is checked against one
/// canonical chunking. Content-defined boundaries (`should_split_chunk`) give
/// structural sharing across commits via stable per-node hashes.
pub fn build_prolly_tree_from_entries(entries: Vec<GraphTreeEntry>) -> GraphProllyTree {
    let entries_total = entries.len();
    let mut all_nodes = Vec::new();
    let mut current_level = leaf_nodes(entries);
    all_nodes.extend(current_level.iter().cloned());

    let mut level = 1u32;
    while current_level.len() > 1 {
        let parents = parent_nodes(&current_level, level);
        all_nodes.extend(parents.iter().cloned());
        current_level = parents;
        level += 1;
    }

    let root = current_level
        .into_iter()
        .next()
        .unwrap_or_else(empty_leaf_node);
    GraphProllyTree {
        root_hash: root.hash.clone(),
        root_level: root.level,
        entries_total,
        nodes: all_nodes,
    }
}

fn node_content_object(node: &NodeRecord, include_payload: bool) -> GraphContentObject {
    GraphContentObject {
        key: format!("node/{}", node.id),
        kind: GraphObjectKind::Node,
        hash: node.content_hash.clone().unwrap_or_else(|| node.checksum()),
        parent_hashes: node.parent_hashes.clone(),
        payload: include_payload.then(|| serde_json::to_value(node).unwrap_or_else(|_| json!({}))),
    }
}

fn edge_content_object(edge: &EdgeRecord, include_payload: bool) -> GraphContentObject {
    GraphContentObject {
        key: format!("edge/{}", edge.id),
        kind: GraphObjectKind::Edge,
        hash: edge.content_hash.clone().unwrap_or_else(|| edge.checksum()),
        parent_hashes: edge.parent_hashes.clone(),
        payload: include_payload.then(|| serde_json::to_value(edge).unwrap_or_else(|_| json!({}))),
    }
}

fn leaf_nodes(entries: Vec<GraphTreeEntry>) -> Vec<GraphTreeNode> {
    if entries.is_empty() {
        return vec![empty_leaf_node()];
    }
    chunk_by_boundary(entries, |chunk| make_leaf_node(chunk.to_vec()))
}

fn parent_nodes(children: &[GraphTreeNode], level: u32) -> Vec<GraphTreeNode> {
    let descriptors = children
        .iter()
        .map(|child| GraphTreeChild {
            hash: child.hash.clone(),
            first_key: child.first_key.clone(),
            last_key: child.last_key.clone(),
            entries_total: child.entry_count(),
        })
        .collect::<Vec<_>>();
    chunk_by_boundary(descriptors, |chunk| make_parent_node(level, chunk.to_vec()))
}

fn chunk_by_boundary<T: Clone>(
    items: Vec<T>,
    make_node: impl Fn(&[T]) -> GraphTreeNode,
) -> Vec<GraphTreeNode>
where
    T: Serialize,
{
    let mut nodes = Vec::new();
    let mut chunk = Vec::new();
    for item in items {
        chunk.push(item);
        let item_hash = stable_hash(chunk.last().expect("chunk has item"));
        if should_split_chunk(chunk.len(), &item_hash) {
            nodes.push(make_node(&chunk));
            chunk.clear();
        }
    }
    if !chunk.is_empty() {
        nodes.push(make_node(&chunk));
    }
    nodes
}

fn make_leaf_node(entries: Vec<GraphTreeEntry>) -> GraphTreeNode {
    let first_key = entries
        .first()
        .map(|entry| entry.key.clone())
        .unwrap_or_default();
    let last_key = entries
        .last()
        .map(|entry| entry.key.clone())
        .unwrap_or_default();
    let hash = tree_node_hash(0, &first_key, &last_key, &entries, &[]);
    GraphTreeNode {
        hash,
        level: 0,
        first_key,
        last_key,
        entries,
        children: Vec::new(),
    }
}

fn make_parent_node(level: u32, children: Vec<GraphTreeChild>) -> GraphTreeNode {
    let first_key = children
        .first()
        .map(|child| child.first_key.clone())
        .unwrap_or_default();
    let last_key = children
        .last()
        .map(|child| child.last_key.clone())
        .unwrap_or_default();
    let hash = tree_node_hash(level, &first_key, &last_key, &[], &children);
    GraphTreeNode {
        hash,
        level,
        first_key,
        last_key,
        entries: Vec::new(),
        children,
    }
}

fn empty_leaf_node() -> GraphTreeNode {
    make_leaf_node(Vec::new())
}

fn tree_node_hash(
    level: u32,
    first_key: &str,
    last_key: &str,
    entries: &[GraphTreeEntry],
    children: &[GraphTreeChild],
) -> String {
    #[derive(Serialize)]
    struct TreeNodeHashInput<'a> {
        protocol_version: &'static str,
        level: u32,
        first_key: &'a str,
        last_key: &'a str,
        entries: &'a [GraphTreeEntry],
        children: &'a [GraphTreeChild],
    }

    stable_hash(TreeNodeHashInput {
        protocol_version: VERSIONED_GRAPH_PROTOCOL_VERSION,
        level,
        first_key,
        last_key,
        entries,
        children,
    })
}

fn build_commit(
    tree: &GraphProllyTree,
    branch: &str,
    parent_commits: Vec<String>,
    author: &str,
    message: &str,
    timestamp_unix_ms: u128,
    graph_version: u64,
) -> GraphCommit {
    #[derive(Serialize)]
    struct CommitHashInput<'a> {
        protocol_version: &'static str,
        tree_hash: &'a str,
        branch: &'a str,
        parent_commits: &'a [String],
        author: &'a str,
        message: &'a str,
        timestamp_unix_ms: u128,
        graph_version: u64,
        objects_total: usize,
        compiler_version: &'static str,
    }

    let commit_hash = stable_hash(CommitHashInput {
        protocol_version: VERSIONED_GRAPH_PROTOCOL_VERSION,
        tree_hash: &tree.root_hash,
        branch,
        parent_commits: &parent_commits,
        author,
        message,
        timestamp_unix_ms,
        graph_version,
        objects_total: tree.entries_total,
        compiler_version: GRAPH_PACK_COMPILER_VERSION,
    });

    GraphCommit {
        commit_hash,
        tree_hash: tree.root_hash.clone(),
        branch: branch.to_string(),
        parent_commits,
        author: author.to_string(),
        message: message.to_string(),
        timestamp_unix_ms,
        graph_version,
        objects_total: tree.entries_total,
        compiler_version: GRAPH_PACK_COMPILER_VERSION.to_string(),
    }
}

fn should_split_chunk(len: usize, hash: &str) -> bool {
    if len >= PROLLY_MAX_ENTRIES {
        return true;
    }
    len >= PROLLY_MIN_ENTRIES
        && hash
            .as_bytes()
            .last()
            .map(|byte| matches!(byte, b'0' | b'4' | b'8' | b'c'))
            .unwrap_or(false)
}

fn clean_or_default(value: Option<String>, default: &str) -> String {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

impl GraphTreeNode {
    fn entry_count(&self) -> usize {
        if self.level == 0 {
            self.entries.len()
        } else {
            self.children.iter().map(|child| child.entries_total).sum()
        }
    }
}

fn resolve_repository_target(repository: &GraphVersionRepository, target: &str) -> Option<String> {
    repository
        .refs
        .iter()
        .find(|reference| reference.name == target)
        .map(|reference| reference.commit_hash.clone())
        .or_else(|| {
            repository
                .packs
                .iter()
                .find(|pack| pack.commit.commit_hash == target)
                .map(|pack| pack.commit.commit_hash.clone())
        })
        .or_else(|| {
            repository
                .commits
                .iter()
                .find(|commit| commit.commit_hash == target)
                .map(|commit| commit.commit_hash.clone())
        })
}

fn repository_tree_for_pack(
    repository: &GraphVersionRepository,
    pack: &CompiledGraphPack,
) -> Option<GraphProllyTree> {
    if !pack.tree.nodes.is_empty() {
        return Some(pack.tree.clone());
    }
    let mut nodes = Vec::new();
    collect_repository_tree_nodes(repository, &pack.tree.root_hash, &mut nodes)?;
    nodes.sort_by(|a, b| {
        a.level
            .cmp(&b.level)
            .then_with(|| a.first_key.cmp(&b.first_key))
            .then_with(|| a.last_key.cmp(&b.last_key))
            .then_with(|| a.hash.cmp(&b.hash))
    });
    Some(GraphProllyTree {
        root_hash: pack.tree.root_hash.clone(),
        root_level: pack.tree.root_level,
        entries_total: pack.tree.entries_total,
        nodes,
    })
}

fn collect_repository_tree_nodes(
    repository: &GraphVersionRepository,
    hash: &str,
    out: &mut Vec<GraphTreeNode>,
) -> Option<()> {
    let node = repository.tree_nodes.get(hash)?.clone();
    for child in &node.children {
        collect_repository_tree_nodes(repository, &child.hash, out)?;
    }
    out.push(node);
    Some(())
}

fn graph_snapshot_from_repository(
    repository: &GraphVersionRepository,
    pack: &CompiledGraphPack,
) -> Option<GraphSnapshot> {
    if !pack.objects.is_empty() {
        return graph_snapshot_from_pack(pack);
    }
    let entries = repository_tree_entries(repository, &pack.tree.root_hash)?;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for entry in entries {
        let object = repository.objects.get(&entry.object_hash)?;
        let payload = object.payload.clone()?;
        match object.kind {
            GraphObjectKind::Node => nodes.push(serde_json::from_value(payload).ok()?),
            GraphObjectKind::Edge => edges.push(serde_json::from_value(payload).ok()?),
        }
    }
    nodes.sort_by(|a: &NodeRecord, b| a.id.cmp(&b.id));
    edges.sort_by(|a: &EdgeRecord, b| a.id.cmp(&b.id));
    Some(GraphSnapshot {
        version: pack.commit.graph_version,
        nodes,
        edges,
    })
}

fn repository_tree_entries(
    repository: &GraphVersionRepository,
    hash: &str,
) -> Option<Vec<GraphTreeEntry>> {
    let node = repository.tree_nodes.get(hash)?;
    if node.level == 0 {
        return Some(node.entries.clone());
    }
    let mut entries = Vec::new();
    for child in &node.children {
        entries.extend(repository_tree_entries(repository, &child.hash)?);
    }
    Some(entries)
}

fn graph_snapshot_from_pack(pack: &CompiledGraphPack) -> Option<GraphSnapshot> {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for object in &pack.objects {
        let payload = object.payload.clone()?;
        match object.kind {
            GraphObjectKind::Node => nodes.push(serde_json::from_value(payload).ok()?),
            GraphObjectKind::Edge => edges.push(serde_json::from_value(payload).ok()?),
        }
    }
    nodes.sort_by(|a: &NodeRecord, b| a.id.cmp(&b.id));
    edges.sort_by(|a: &EdgeRecord, b| a.id.cmp(&b.id));
    Some(GraphSnapshot {
        version: pack.commit.graph_version,
        nodes,
        edges,
    })
}

#[derive(Clone, Debug)]
enum MergeRecord {
    Node(NodeRecord),
    Edge(EdgeRecord),
}

impl MergeRecord {
    fn key(&self) -> String {
        match self {
            Self::Node(node) => format!("node/{}", node.id),
            Self::Edge(edge) => format!("edge/{}", edge.id),
        }
    }

    fn kind(&self) -> GraphObjectKind {
        match self {
            Self::Node(_) => GraphObjectKind::Node,
            Self::Edge(_) => GraphObjectKind::Edge,
        }
    }

    fn hash(&self) -> String {
        match self {
            Self::Node(node) => node.content_hash.clone().unwrap_or_else(|| node.checksum()),
            Self::Edge(edge) => edge.content_hash.clone().unwrap_or_else(|| edge.checksum()),
        }
    }
}

enum MergeDecision {
    Resolved {
        selected: Option<GraphMergeSide>,
        record: Option<MergeRecord>,
        reason: String,
    },
    Conflict {
        reason: String,
    },
}

fn merge_records_by_key(snapshot: &GraphSnapshot) -> BTreeMap<String, MergeRecord> {
    let mut records = BTreeMap::new();
    for node in &snapshot.nodes {
        let record = MergeRecord::Node(node.clone());
        records.insert(record.key(), record);
    }
    for edge in &snapshot.edges {
        let record = MergeRecord::Edge(edge.clone());
        records.insert(record.key(), record);
    }
    records
}

fn merge_record_kind(
    base: Option<&MergeRecord>,
    ours: Option<&MergeRecord>,
    theirs: Option<&MergeRecord>,
) -> GraphObjectKind {
    ours.or(theirs)
        .or(base)
        .map(MergeRecord::kind)
        .unwrap_or(GraphObjectKind::Node)
}

fn resolve_merge_record(
    base: Option<&MergeRecord>,
    ours: Option<&MergeRecord>,
    theirs: Option<&MergeRecord>,
    strategy: &GraphMergeStrategy,
    min_confidence_delta: f64,
) -> MergeDecision {
    let base_hash = base.map(MergeRecord::hash);
    let ours_hash = ours.map(MergeRecord::hash);
    let theirs_hash = theirs.map(MergeRecord::hash);

    if ours_hash == theirs_hash {
        return MergeDecision::Resolved {
            selected: ours.map(|_| GraphMergeSide::Ours),
            record: ours.cloned(),
            reason: "same_result".to_string(),
        };
    }
    if base_hash == ours_hash {
        return MergeDecision::Resolved {
            selected: theirs.map(|_| GraphMergeSide::Theirs),
            record: theirs.cloned(),
            reason: "theirs_only_change".to_string(),
        };
    }
    if base_hash == theirs_hash {
        return MergeDecision::Resolved {
            selected: ours.map(|_| GraphMergeSide::Ours),
            record: ours.cloned(),
            reason: "ours_only_change".to_string(),
        };
    }

    match strategy {
        GraphMergeStrategy::PreferOurs => MergeDecision::Resolved {
            selected: ours.map(|_| GraphMergeSide::Ours),
            record: ours.cloned(),
            reason: "strategy_prefer_ours".to_string(),
        },
        GraphMergeStrategy::PreferTheirs => MergeDecision::Resolved {
            selected: theirs.map(|_| GraphMergeSide::Theirs),
            record: theirs.cloned(),
            reason: "strategy_prefer_theirs".to_string(),
        },
        GraphMergeStrategy::AutoConfidence => {
            resolve_confidence_merge(ours, theirs, min_confidence_delta).unwrap_or_else(|| {
                MergeDecision::Conflict {
                    reason: "both_sides_changed".to_string(),
                }
            })
        }
        GraphMergeStrategy::Manual => MergeDecision::Conflict {
            reason: "manual_resolution_required".to_string(),
        },
    }
}

fn resolve_confidence_merge(
    ours: Option<&MergeRecord>,
    theirs: Option<&MergeRecord>,
    min_confidence_delta: f64,
) -> Option<MergeDecision> {
    let (Some(MergeRecord::Edge(ours)), Some(MergeRecord::Edge(theirs))) = (ours, theirs) else {
        return None;
    };
    let (side, edge) = resolve_auto_confidence_edge(ours, theirs, min_confidence_delta)?;
    Some(MergeDecision::Resolved {
        selected: Some(side),
        record: Some(MergeRecord::Edge(edge)),
        reason: "higher_edge_confidence".to_string(),
    })
}

pub fn resolve_auto_confidence_edge(
    ours: &EdgeRecord,
    theirs: &EdgeRecord,
    min_confidence_delta: f64,
) -> Option<(GraphMergeSide, EdgeRecord)> {
    let delta = (ours.effective_confidence() - theirs.effective_confidence()).abs();
    if delta <= min_confidence_delta.max(0.0) {
        return None;
    }
    if ours.effective_confidence() > theirs.effective_confidence() {
        Some((GraphMergeSide::Ours, ours.clone()))
    } else {
        Some((GraphMergeSide::Theirs, theirs.clone()))
    }
}

fn snapshot_from_merge_records(version: u64, records: Vec<MergeRecord>) -> GraphSnapshot {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for record in records {
        match record {
            MergeRecord::Node(node) => nodes.push(node),
            MergeRecord::Edge(edge) => edges.push(edge),
        }
    }
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    edges.sort_by(|a, b| a.id.cmp(&b.id));
    GraphSnapshot {
        version,
        nodes,
        edges,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::graph_store::{
        EdgeRecord, GraphMutation, GraphMutationBatch, GraphSnapshot, NodeRecord,
    };

    #[test]
    fn compiler_builds_stable_tree_independent_of_record_order() {
        let ada = NodeRecord::new("node:ada", ["Person"], json!({"name": "Ada"}));
        let bob = NodeRecord::new("node:bob", ["Person"], json!({"name": "Bob"}));
        let edge = EdgeRecord::new("edge:knows", "node:ada", "KNOWS", "node:bob", json!({}));
        let first = GraphSnapshot {
            version: 2,
            nodes: vec![ada.clone(), bob.clone()],
            edges: vec![edge.clone()],
        };
        let second = GraphSnapshot {
            version: 2,
            nodes: vec![bob, ada],
            edges: vec![edge],
        };
        let opts = GraphCompileOptions {
            timestamp_unix_ms: Some(1),
            ..GraphCompileOptions::default()
        };
        let first_pack = compile_graph_pack(&first, opts.clone());
        let second_pack = compile_graph_pack(&second, opts);

        assert_eq!(first_pack.tree.root_hash, second_pack.tree.root_hash);
        assert_eq!(
            first_pack.commit.commit_hash,
            second_pack.commit.commit_hash
        );
        assert_eq!(first_pack.manifest.objects_total, 3);
        assert_eq!(first_pack.capabilities.len(), 2);
    }

    #[test]
    fn diff_reports_added_removed_and_modified_records() {
        let base = GraphSnapshot {
            version: 1,
            nodes: vec![NodeRecord::new(
                "node:ada",
                ["Person"],
                json!({"name": "Ada"}),
            )],
            edges: Vec::new(),
        };
        let target = GraphSnapshot {
            version: 2,
            nodes: vec![
                NodeRecord::new("node:ada", ["Person"], json!({"name": "Ada Lovelace"})),
                NodeRecord::new("node:bob", ["Person"], json!({"name": "Bob"})),
            ],
            edges: Vec::new(),
        };
        let diff = diff_graph_snapshots(&base, &target);

        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.modified.len(), 1);
        assert_eq!(diff.removed.len(), 0);
        assert_eq!(diff.unchanged, 0);
    }

    #[test]
    fn incremental_pack_matches_full_recompile_and_reuses_tree_nodes() {
        let base_nodes = (0..96)
            .map(|idx| {
                NodeRecord::new(
                    format!("node:{idx:03}"),
                    ["Person"],
                    json!({ "name": format!("person-{idx:03}") }),
                )
            })
            .collect::<Vec<_>>();
        let base = GraphSnapshot {
            version: 7,
            nodes: base_nodes,
            edges: Vec::new(),
        };
        let base_pack = compile_graph_pack(
            &base,
            GraphCompileOptions {
                branch: Some("main".to_string()),
                timestamp_unix_ms: Some(1),
                ..GraphCompileOptions::default()
            },
        );
        let batch = GraphMutationBatch::new([GraphMutation::NodeUpsert(NodeRecord::new(
            "node:095",
            ["Person"],
            json!({ "name": "person-095", "status": "updated" }),
        ))]);
        let final_snapshot = apply_graph_mutation_batch(&base, &batch);
        let incremental = compile_graph_pack_incremental(
            &base_pack,
            &batch,
            GraphCompileOptions {
                branch: Some("main".to_string()),
                parent_commits: vec![base_pack.commit.commit_hash.clone()],
                timestamp_unix_ms: Some(2),
                ..GraphCompileOptions::default()
            },
        );
        let full = compile_graph_pack(
            &final_snapshot,
            GraphCompileOptions {
                branch: Some("main".to_string()),
                parent_commits: vec![base_pack.commit.commit_hash.clone()],
                timestamp_unix_ms: Some(2),
                message: Some("incremental graph mutation batch".to_string()),
                ..GraphCompileOptions::default()
            },
        );

        assert_eq!(incremental.changed_object_keys, vec!["node/node:095"]);
        assert_eq!(incremental.pack.tree.root_hash, full.tree.root_hash);
        assert_eq!(incremental.pack.commit.commit_hash, full.commit.commit_hash);
        assert!(
            incremental.reused_tree_nodes > incremental.changed_tree_nodes,
            "single-record update should reuse most Prolly nodes: reused={}, changed={}",
            incremental.reused_tree_nodes,
            incremental.changed_tree_nodes
        );
    }

    #[test]
    fn refs_log_and_checkout_round_trip_snapshot_payloads() {
        let snapshot = GraphSnapshot {
            version: 7,
            nodes: vec![NodeRecord::new(
                "node:ada",
                ["Person"],
                json!({"name": "Ada"}),
            )],
            edges: Vec::new(),
        };
        let pack = compile_graph_pack(
            &snapshot,
            GraphCompileOptions {
                branch: Some("main".to_string()),
                timestamp_unix_ms: Some(1),
                ..GraphCompileOptions::default()
            },
        );
        let update = update_graph_ref(
            GraphVersionRepository::default(),
            pack.clone(),
            Some("main".to_string()),
            Some(2),
        );

        assert_eq!(update.reference.commit_hash, pack.commit.commit_hash);
        let log = graph_version_log(&update.repository, Some("main"));
        assert_eq!(log.commits.len(), 1);

        let checkout = checkout_graph_version(&update.repository, "main").unwrap();
        assert_eq!(checkout.commit.commit_hash, pack.commit.commit_hash);
        assert_eq!(checkout.snapshot.nodes[0].id, "node:ada");
    }

    #[test]
    fn repository_deduplicates_objects_and_tree_nodes_across_commits() {
        let snapshot = GraphSnapshot {
            version: 7,
            nodes: vec![NodeRecord::new(
                "node:ada",
                ["Person"],
                json!({"name": "Ada"}),
            )],
            edges: Vec::new(),
        };
        let first_pack = compile_graph_pack(
            &snapshot,
            GraphCompileOptions {
                branch: Some("main".to_string()),
                timestamp_unix_ms: Some(1),
                ..GraphCompileOptions::default()
            },
        );
        let first = update_graph_ref(
            GraphVersionRepository::default(),
            first_pack,
            Some("main".to_string()),
            Some(2),
        );
        let object_count = first.repository.objects.len();
        let tree_node_count = first.repository.tree_nodes.len();
        assert_eq!(object_count, 1);
        assert!(tree_node_count > 0);
        assert!(first.repository.packs[0].objects.is_empty());
        assert!(first.repository.packs[0].tree.nodes.is_empty());

        let second_pack = compile_graph_pack(
            &snapshot,
            GraphCompileOptions {
                branch: Some("main".to_string()),
                parent_commits: vec![first.reference.commit_hash.clone()],
                timestamp_unix_ms: Some(3),
                ..GraphCompileOptions::default()
            },
        );
        let second = update_graph_ref(
            first.repository,
            second_pack,
            Some("main".to_string()),
            Some(4),
        );

        assert_eq!(second.repository.objects.len(), object_count);
        assert_eq!(second.repository.tree_nodes.len(), tree_node_count);
        let checkout = checkout_graph_version(&second.repository, "main").unwrap();
        assert_eq!(checkout.snapshot.nodes[0].id, "node:ada");
    }

    #[test]
    fn graph_ref_cas_rejects_stale_expected_commit() {
        let snapshot = GraphSnapshot {
            version: 1,
            nodes: vec![NodeRecord::new("node:ada", ["Person"], json!({}))],
            edges: Vec::new(),
        };
        let first_pack = compile_graph_pack(
            &snapshot,
            GraphCompileOptions {
                branch: Some("main".to_string()),
                timestamp_unix_ms: Some(1),
                ..GraphCompileOptions::default()
            },
        );
        let first = update_graph_ref(
            GraphVersionRepository::default(),
            first_pack,
            Some("main".to_string()),
            Some(2),
        );
        let mut changed = snapshot.clone();
        changed.version = 2;
        changed
            .nodes
            .push(NodeRecord::new("node:bob", ["Person"], json!({})));
        let second_pack = compile_graph_pack(
            &changed,
            GraphCompileOptions {
                branch: Some("main".to_string()),
                parent_commits: vec![first.reference.commit_hash.clone()],
                timestamp_unix_ms: Some(3),
                ..GraphCompileOptions::default()
            },
        );

        let conflict = update_graph_ref_cas(
            first.repository,
            second_pack,
            Some("main".to_string()),
            Some("sha256:stale".to_string()),
            Some(4),
        )
        .unwrap_err();

        assert_eq!(conflict.branch, "main");
        assert_eq!(
            conflict.actual_commit_hash.as_deref(),
            Some(first.reference.commit_hash.as_str())
        );
    }

    #[test]
    fn three_way_merge_resolves_non_overlapping_changes() {
        let base = GraphSnapshot {
            version: 1,
            nodes: vec![NodeRecord::new(
                "node:ada",
                ["Person"],
                json!({"name": "Ada"}),
            )],
            edges: Vec::new(),
        };
        let ours = GraphSnapshot {
            version: 2,
            nodes: vec![
                NodeRecord::new("node:ada", ["Person"], json!({"name": "Ada Lovelace"})),
                NodeRecord::new("node:ours", ["Note"], json!({})),
            ],
            edges: Vec::new(),
        };
        let theirs = GraphSnapshot {
            version: 3,
            nodes: vec![
                NodeRecord::new("node:ada", ["Person"], json!({"name": "Ada"})),
                NodeRecord::new("node:theirs", ["Note"], json!({})),
            ],
            edges: Vec::new(),
        };

        let merged = merge_graph_snapshots(&base, &ours, &theirs, GraphMergeOptions::default());

        assert_eq!(merged.status, "clean");
        let snapshot = merged.merged_snapshot.unwrap();
        let ids = snapshot
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["node:ada", "node:ours", "node:theirs"]);
        assert!(merged.conflicts.is_empty());
    }

    #[test]
    fn three_way_merge_confidence_resolves_edge_conflicts() {
        let base_edge =
            EdgeRecord::new("edge:e", "a", "SUPPORTS", "b", json!({})).with_confidence(0.4);
        let ours_edge =
            EdgeRecord::new("edge:e", "a", "SUPPORTS", "b", json!({})).with_confidence(0.9);
        let theirs_edge =
            EdgeRecord::new("edge:e", "a", "SUPPORTS", "b", json!({})).with_confidence(0.6);
        let base = GraphSnapshot {
            version: 1,
            nodes: Vec::new(),
            edges: vec![base_edge],
        };
        let ours = GraphSnapshot {
            version: 2,
            nodes: Vec::new(),
            edges: vec![ours_edge],
        };
        let theirs = GraphSnapshot {
            version: 3,
            nodes: Vec::new(),
            edges: vec![theirs_edge],
        };

        let merged = merge_graph_snapshots(&base, &ours, &theirs, GraphMergeOptions::default());

        assert_eq!(merged.status, "clean");
        let edge = &merged.merged_snapshot.unwrap().edges[0];
        assert_eq!(edge.confidence, Some(0.9));
        assert_eq!(merged.resolved[0].reason, "higher_edge_confidence");
    }

    #[test]
    fn three_way_merge_reports_manual_node_conflict() {
        let base = GraphSnapshot {
            version: 1,
            nodes: vec![NodeRecord::new("node:a", ["Doc"], json!({"title": "base"}))],
            edges: Vec::new(),
        };
        let ours = GraphSnapshot {
            version: 2,
            nodes: vec![NodeRecord::new("node:a", ["Doc"], json!({"title": "ours"}))],
            edges: Vec::new(),
        };
        let theirs = GraphSnapshot {
            version: 3,
            nodes: vec![NodeRecord::new(
                "node:a",
                ["Doc"],
                json!({"title": "theirs"}),
            )],
            edges: Vec::new(),
        };

        let merged = merge_graph_snapshots(&base, &ours, &theirs, GraphMergeOptions::default());

        assert_eq!(merged.status, "conflicted");
        assert!(merged.merged_snapshot.is_none());
        assert_eq!(merged.conflicts.len(), 1);
    }
}
