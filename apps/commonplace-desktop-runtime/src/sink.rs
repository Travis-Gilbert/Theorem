//! [`CommonplaceIngestSink`]: the production [`ChangeSink`] that lands settled
//! change sets in a durable commonplace graph inside the RustyRed sidecar.
//!
//! This realizes ambient-layer handoff Part A deliverable 1 (the ingest path)
//! and deliverable 4 (the change-to-provenance record). The watcher (slice 1) is
//! READ-ONLY on the working tree; this sink writes ONLY into the sidecar
//! (`<root>/.rustyred`, always ignored), never tracked files -- the canonical-git
//! boundary holds.
//!
//! For each settled change set it:
//! 1. Collects `Created`/`Modified` changes for UTF-8 text files (binary and
//!    non-UTF-8 files are skipped this slice; `Removed` is noted as deferred).
//! 2. Reads each file's current bytes and builds one
//!    [`IngestInput::document`](commonplace::IngestInput::document) keyed by a
//!    [`SourceRef`] of `(source = "file", external_id = <relative path>)`. The
//!    source ref makes re-ingest of the same path idempotent: the commonplace
//!    ingest pipeline (A3) reuses the existing item id, so an edit updates the
//!    SAME item -- provenance across edits, not a duplicate.
//! 3. Ingests the whole batch in one [`IngestPipeline::ingest_batch`] call.
//! 4. Appends a minimal change-set lineage node (deliverable 4): a
//!    `CommonplaceChangeSet`-labelled node capturing the settled paths and their
//!    content hashes, linked back to the previous change-set node by a
//!    `FOLLOWS` edge so the change history forms an auditable chain.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use commonplace::{
    content_hash, Collection, Commonplace, IngestInput, IngestPipeline, IngestReceipt, Item,
    SourceRef, COLLECTION_LABEL,
};
use rustyred_thg_core::{
    now_ms, DiskObjectStore, EdgeRecord, GraphStore, GraphStoreResult, NodeQuery, NodeRecord,
    RedCoreGraphStore, RedCoreOptions,
};
use serde_json::json;

use crate::{ChangeKind, ChangeSet, ChangeSink, FileChange, Result, WatchConfig};

/// The concrete durable commonplace the sidecar owns: a file-backed RedCore graph
/// plus a disk blob store. Read routes and the ambient writer share ONE of these
/// (one process, one graph, no AOF contention -- a second handle on the same AOF
/// would conflict).
pub type SidecarCommonplaceStore = Commonplace<RedCoreGraphStore, DiskObjectStore>;

/// The source name stamped on every file-derived item's [`SourceRef`]. The
/// external id is the file's path relative to the watched root, so re-ingesting
/// the same path updates the same item (A3 idempotency / provenance across edits).
pub const FILE_SOURCE: &str = "file";

/// Node label for a settled change-set lineage record (deliverable 4).
pub const CHANGE_SET_LABEL: &str = "CommonplaceChangeSet";
/// Edge from a change-set node to the change-set that preceded it, newest ->
/// older, so the lineage is a walkable chain.
pub const FOLLOWS_EDGE: &str = "FOLLOWS";
/// Stable id of the node that always points at the latest change-set node, so
/// the head is an O(1) lookup instead of a scan.
const LINEAGE_HEAD_ID: &str = "commonplace:changeset:head";
/// Label for the single lineage-head pointer node.
const LINEAGE_HEAD_LABEL: &str = "CommonplaceChangeSetHead";

/// What a single change contributed to an ingest pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestedPath {
    /// Path relative to the watched root (the item's `SourceRef` external id).
    pub relative_path: String,
    /// `sha256:<hex>` of the file's bytes at ingest time.
    pub content_hash: String,
    /// The id of the commonplace item the path was ingested as.
    pub item_id: String,
}

/// What one change set produced after the sink applied it. Returned by
/// [`CommonplaceIngestSink::ingest_change_set`] so callers (and tests) can see
/// exactly what was ingested, what lineage node was written, and what was
/// deferred. The blocking [`ChangeSink::apply`] path drops this.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IngestOutcome {
    /// One entry per file ingested in this pass (text `Created`/`Modified`).
    pub ingested: Vec<IngestedPath>,
    /// Paths skipped because they were not readable UTF-8 text (binary, deleted
    /// mid-batch, or a non-UTF-8 file). Deferred for a later slice.
    pub skipped: Vec<PathBuf>,
    /// Paths from `Removed` changes, noted but not yet processed (deferred to a
    /// later slice; removal handling needs durable node delete + tombstoning).
    pub removed_deferred: Vec<PathBuf>,
    /// The id of the lineage node written for this change set, if any files were
    /// ingested. `None` when nothing was ingested (no lineage node is written).
    pub change_set_node_id: Option<String>,
}

/// A [`ChangeSink`] that ingests settled change sets into a durable commonplace
/// graph in the RustyRed sidecar. Owns the graph store, blob store, and the
/// ingest pipeline for the lifetime of the watcher.
pub struct CommonplaceIngestSink {
    commonplace: Commonplace<RedCoreGraphStore, DiskObjectStore>,
    pipeline: IngestPipeline,
    root: PathBuf,
    /// The watched root canonicalized at open time, if it resolves. The watcher
    /// (FSEvents on macOS) reports canonicalized paths (`/private/var/...` for a
    /// `/var/...` symlink), which do not share the configured `root` prefix; so a
    /// path is made relative against BOTH `root` and this canonical root,
    /// keeping the item's `SourceRef` a clean relative path (and idempotent)
    /// instead of an absolute one.
    root_canonical: Option<PathBuf>,
}

impl CommonplaceIngestSink {
    /// Open the sink over the sidecar configured by `config`. The durable graph
    /// lives at `<sidecar>/graph` and content-addressed blobs at
    /// `<sidecar>/blobs`; both are inside `<root>/.rustyred`, which the watcher
    /// always ignores, so the sink never triggers itself.
    pub fn open(config: &WatchConfig) -> Result<Self> {
        let graph_dir = config.sidecar_dir.join("graph");
        let blob_dir = config.sidecar_dir.join("blobs");
        std::fs::create_dir_all(&graph_dir)?;
        std::fs::create_dir_all(&blob_dir)?;

        // `GraphStoreError` does not implement `std::error::Error`, so map it
        // into this crate's boxed error explicitly (its `Debug` carries the
        // code + message).
        let store = RedCoreGraphStore::open(&graph_dir, RedCoreOptions::default())
            .map_err(|error| format!("open sidecar graph: {error:?}"))?;
        let blobs = DiskObjectStore::open(&blob_dir)
            .map_err(|error| format!("open sidecar blobs: {error:?}"))?;
        Ok(Self {
            commonplace: Commonplace::new(store, blobs),
            pipeline: IngestPipeline::default(),
            root: config.root.clone(),
            // `canonicalize` requires the path to exist; the watched root does by
            // the time the sink opens. Tolerate failure (keeps the configured root).
            root_canonical: config.root.canonicalize().ok(),
        })
    }

    /// Borrow the underlying commonplace store (for queries / verification).
    pub fn commonplace(&self) -> &Commonplace<RedCoreGraphStore, DiskObjectStore> {
        &self.commonplace
    }

    /// Mutably borrow the underlying commonplace store. The ambient passes (slice
    /// 3) write provenance into this same durable sidecar store after ingest, so
    /// they need mutable access without opening a second store handle.
    pub fn commonplace_mut(&mut self) -> &mut Commonplace<RedCoreGraphStore, DiskObjectStore> {
        &mut self.commonplace
    }

    /// Ingest one settled change set and write its lineage record, returning a
    /// detailed [`IngestOutcome`]. This is the testable core of [`apply`].
    ///
    /// [`apply`]: ChangeSink::apply
    pub fn ingest_change_set(&mut self, change_set: &ChangeSet) -> GraphStoreResult<IngestOutcome> {
        let mut outcome = IngestOutcome::default();
        let mut inputs: Vec<IngestInput> = Vec::new();
        // Parallel to `inputs`: the (relative_path, content_hash) for each one,
        // so the post-ingest receipt can be paired back to its path for lineage.
        let mut pending: Vec<(String, String)> = Vec::new();

        for change in &change_set.changes {
            match change.kind {
                ChangeKind::Removed => {
                    outcome.removed_deferred.push(change.path.clone());
                }
                ChangeKind::Created | ChangeKind::Modified => {
                    match self.prepare_input(change) {
                        Some((input, relative_path, hash)) => {
                            inputs.push(input);
                            pending.push((relative_path, hash));
                        }
                        None => outcome.skipped.push(change.path.clone()),
                    }
                }
            }
        }

        if inputs.is_empty() {
            return Ok(outcome);
        }

        let receipts: Vec<IngestReceipt> =
            self.pipeline.ingest_batch(&mut self.commonplace, inputs)?;
        for ((relative_path, content_hash), receipt) in pending.into_iter().zip(receipts) {
            outcome.ingested.push(IngestedPath {
                relative_path,
                content_hash,
                item_id: receipt.item.id,
            });
        }

        let node_id = self.write_lineage_node(&outcome.ingested)?;
        outcome.change_set_node_id = Some(node_id);
        Ok(outcome)
    }

    /// Build the ingest input for one `Created`/`Modified` change, or `None` if
    /// the path is not readable UTF-8 text (skip binary / non-UTF-8 / vanished).
    fn prepare_input(&self, change: &FileChange) -> Option<(IngestInput, String, String)> {
        // A path that resolves to a directory or vanished between the debounce
        // window and now is not an ingestable file; skip it.
        let bytes = std::fs::read(&change.path).ok()?;
        let text = String::from_utf8(bytes).ok()?;
        let relative_path = self.relative_path(&change.path);
        let hash = content_hash(text.as_bytes());
        let source_ref = SourceRef::new(FILE_SOURCE, relative_path.clone());
        let input = IngestInput::document(relative_path.clone(), text).with_source_ref(source_ref);
        Some((input, relative_path, hash))
    }

    /// The path relative to the watched root, as a forward-slash string so the
    /// `SourceRef` external id is stable across runs and platforms. Tries the
    /// configured root and the canonicalized root (the watcher reports
    /// canonicalized paths on macOS), falling back to the lossy absolute path only
    /// if the change is genuinely outside the root.
    fn relative_path(&self, path: &Path) -> String {
        let relative = path
            .strip_prefix(&self.root)
            .ok()
            .or_else(|| {
                self.root_canonical
                    .as_ref()
                    .and_then(|canonical| path.strip_prefix(canonical).ok())
            })
            .unwrap_or(path);
        relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Write the change-set lineage node (deliverable 4) and link it to the prior
    /// head, returning the new node id. The node records the settled paths and
    /// their content hashes; a `FOLLOWS` edge points at the previous change-set
    /// node so the history is a walkable chain, and the head pointer is advanced.
    fn write_lineage_node(&mut self, ingested: &[IngestedPath]) -> GraphStoreResult<String> {
        let now = now_ms();
        let node_id = format!("commonplace:changeset:{now:x}-{}", ingested.len());
        let previous = self.current_lineage_head();

        let paths = ingested
            .iter()
            .map(|entry| {
                json!({
                    "relative_path": entry.relative_path,
                    "content_hash": entry.content_hash,
                    "item_id": entry.item_id,
                })
            })
            .collect::<Vec<_>>();
        let record = NodeRecord::new(
            node_id.clone(),
            [CHANGE_SET_LABEL],
            json!({
                "recorded_at_ms": now,
                "path_count": ingested.len(),
                "paths": paths,
                "previous": previous,
            }),
        );
        self.commonplace.store_mut().upsert_node(record)?;

        if let Some(previous_id) = &previous {
            let edge = EdgeRecord::new(
                format!("follows:{node_id}:{previous_id}"),
                &node_id,
                FOLLOWS_EDGE,
                previous_id,
                json!({}),
            );
            self.commonplace.store_mut().upsert_edge(edge)?;
        }

        // Advance the head pointer so the next change set finds this one in O(1).
        let head = NodeRecord::new(
            LINEAGE_HEAD_ID,
            [LINEAGE_HEAD_LABEL],
            json!({ "latest": node_id, "updated_at_ms": now }),
        );
        self.commonplace.store_mut().upsert_node(head)?;

        Ok(node_id)
    }

    /// The id of the latest change-set node, if any. Read via the head pointer
    /// node through a label+id query so it works uniformly over the trait surface
    /// (the inherent `RedCoreGraphStore::get_node` shadows the trait method).
    fn current_lineage_head(&self) -> Option<String> {
        let head = GraphStore::query_nodes(
            self.commonplace.store(),
            NodeQuery::label(LINEAGE_HEAD_LABEL).with_limit(1),
        )
        .into_iter()
        .next()?;
        head.properties
            .get("latest")
            .and_then(|value| value.as_str())
            .map(str::to_string)
    }
}

impl ChangeSink for CommonplaceIngestSink {
    fn apply(&mut self, change_set: ChangeSet) {
        // The watcher runs `apply` on its own thread with no return channel; a
        // store error here must not poison the watch loop, so it is swallowed
        // after surfacing on stderr. Richer error routing (a degraded-state
        // receipt back to the runtime) is a later-slice concern.
        if let Err(error) = self.ingest_change_set(&change_set) {
            // `GraphStoreError` has no `Display`; its `Debug` carries code + message.
            eprintln!("commonplace-desktop-runtime: ingest failed: {error:?}");
        }
    }
}

/// A cloneable, thread-safe handle to the ONE durable [`CommonplaceIngestSink`]
/// the local instance owns.
///
/// This is the seam that makes the local-first connection path work: the ambient
/// watcher writes through this handle (it is itself a [`ChangeSink`]) and the
/// control endpoint's data routes ([`ControlState`](crate::ControlState)) read
/// through a CLONE of the same handle. Because both go through one
/// `Arc<Mutex<..>>` over a single `RedCoreGraphStore`, a watcher ingest is
/// immediately visible to a subsequent read, and there is never a second process
/// (or second handle) opening the same AOF -- the architecture the slice requires
/// ("one process, one graph, no AOF contention").
///
/// Reads are short critical sections (a query then a clone of owned data), so a
/// read never holds the lock across an `await`.
#[derive(Clone)]
pub struct SharedSink {
    inner: Arc<Mutex<CommonplaceIngestSink>>,
}

impl SharedSink {
    /// Wrap an open sink in a shared handle. The watcher takes one clone (to
    /// write) and the control endpoint takes another (to read).
    pub fn new(sink: CommonplaceIngestSink) -> Self {
        Self {
            inner: Arc::new(Mutex::new(sink)),
        }
    }

    /// Open the durable sink over `config`'s sidecar and wrap it in a shared
    /// handle in one step (the common case).
    pub fn open(config: &WatchConfig) -> Result<Self> {
        Ok(Self::new(CommonplaceIngestSink::open(config)?))
    }

    /// Lock the sink, exposing the inner [`CommonplaceIngestSink`] for a critical
    /// section. Recovers from a poisoned lock (a prior holder panicked mid-op)
    /// rather than cascading the panic: the durable store is the source of truth
    /// and the next write re-establishes consistency.
    ///
    /// Used by the ambient runtime to run one ingest+passes cycle atomically, and
    /// by tests to inspect the live store. Do NOT hold the returned guard across an
    /// `await`.
    pub fn lock(&self) -> MutexGuard<'_, CommonplaceIngestSink> {
        self.inner.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    /// List items (newest first), capped at `limit`. Mirrors the commonplace
    /// consumer API's `items` query: it reads `Commonplace::all_items` and then
    /// applies a sane ceiling so a large store does not return unbounded JSON.
    pub fn list_items(&self, limit: usize) -> GraphStoreResult<Vec<Item>> {
        let sink = self.lock();
        let mut items = sink.commonplace().all_items()?;
        // Newest first by update time; an ambient edit bumps `updated_at_ms`, so
        // the most recently touched files surface at the top of the list.
        items.sort_by_key(|item| std::cmp::Reverse(item.updated_at_ms));
        items.truncate(limit);
        Ok(items)
    }

    /// One item by id (mirrors the consumer API's `item` query: `get_item`).
    pub fn get_item(&self, id: &str) -> GraphStoreResult<Option<Item>> {
        self.lock().commonplace().get_item(id)
    }

    /// Every collection (mirrors the consumer API's `collections` query exactly:
    /// `query_nodes(label = Collection)` then `get_collection` per id).
    pub fn list_collections(&self) -> GraphStoreResult<Vec<Collection>> {
        let sink = self.lock();
        let cp = sink.commonplace();
        // UFCS to the trait method: `RedCoreGraphStore`'s inherent `query_nodes`
        // shadows the trait one with a `Result`-returning signature (CLAUDE.md's
        // inherent-vs-trait shadow), so call `GraphStore::query_nodes` explicitly
        // to get the plain `Vec<NodeRecord>` the consumer API uses.
        let ids: Vec<String> = GraphStore::query_nodes(
            cp.store(),
            NodeQuery::label(COLLECTION_LABEL).with_limit(usize::MAX),
        )
        .into_iter()
        .map(|node| node.id)
        .collect();
        let mut collections = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(collection) = cp.get_collection(&id)? {
                collections.push(collection);
            }
        }
        Ok(collections)
    }

    /// Similarity search over items, capped at `k` (mirrors the consumer API's
    /// `search` query: the REAL commonplace vector search,
    /// [`IngestPipeline::search`], over the engine embedding index the ambient
    /// ingest populates -- not an invented text scan). Returns the hit items paired
    /// with their similarity score, score-descending.
    pub fn search(&self, query: &str, k: usize) -> GraphStoreResult<Vec<(Item, f32)>> {
        let sink = self.lock();
        let cp = sink.commonplace();
        let hits = IngestPipeline::default().search(cp, query, k)?;
        let mut results = Vec::with_capacity(hits.len());
        for (id, score) in hits {
            if let Some(item) = cp.get_item(&id)? {
                results.push((item, score));
            }
        }
        Ok(results)
    }
}

impl ChangeSink for SharedSink {
    fn apply(&mut self, change_set: ChangeSet) {
        // Delegate to the inner sink under the shared lock, preserving its
        // swallow-after-stderr contract (the watcher loop must never be poisoned).
        self.lock().apply(change_set);
    }
}
