# SPEC: Source Intake and Routing

Execution handoff for Claude Code. Grounded in the committed tree of
`Travis-Gilbert/Theorem` as read on 2026-06-22, not from memory.

## What this is

The front of the object lifecycle. An object in CommonPlace enters from a
source, gets organized, and gets acted on. The organize stage and the act stage
are built or designed. The enter stage is the unspecced piece, and it is the one
that makes the other two real, because there is nothing to organize or act on
until Gmail and Notion and Outlook are feeding the graph.

This spec builds that front: connecting external sources, pulling their data in
on a scoped basis, and routing what arrives into the existing classifier. It
also makes the two-tier organize boundary explicit, promotes tasks to
first-class graph nodes, and wires the residue to delegation.

## Grounding (what already exists)

Read these before writing anything. Paths are under
`rustyredcore_THG/crates/`.

The `commonplace` crate is the consumer projection (plan units F1 and F2).

- `commonplace/src/item.rs`: the universal `Item`. Fields that matter here:
  `kind: ItemKind` (`File | Note | Link | Image | Doc | Other(String)`),
  `title`, `body: ItemBody` (`Empty | Inline{text} | Blob{content_hash,
  byte_len, mime}`), `source: Option<String>`, `residency: Residency`
  (`Local | Synced | Hosted`), `tags`, `collections` (edge-canonical),
  `embedding: Option<Vec<f32>>` (a top-level node property the substrate vector
  index picks up on write), `classification: Option<String>`, `created_at_ms`,
  `updated_at_ms`, and `extra: Map<String, Value>` (the arbitrary-JSON escape
  hatch). Note that `source` already rides on every item.
- `commonplace/src/ingest.rs` (plan unit F2, the tier-one classifier):
  - `IngestInput { title, body: IngestBody, source: Option<String>, residency,
    tags }` where `IngestBody` is `Text{text, kind} | Link{url, text} |
    Binary{bytes, mime, kind}`. This is the no-button capture contract, and it
    is the universal target every source maps onto.
  - `IngestPipeline<E: Embedder>` with `collection_threshold: 0.58`,
    `similarity_threshold: 0.62`, `entity_threshold: 0.86`. Its `ingest` path
    embeds the input, chooses or creates a collection by cosine to each
    collection's `label_embedding` (gated by `collection_threshold`), writes the
    `Item`, writes `SIMILAR_TO` edges, and resolves entities.
  - `classify_item(commonplace, item) -> Classification` ranks every live
    collection by cosine to the item's stored embedding, best first, and is
    deliberately NOT gated by `collection_threshold`. Its doc comment: the
    caller applies its own ceiling. `Classification::confidence()` returns the
    top score, `Classification::best()` the top candidate. This is the tier-one
    confidence signal and the seam the two-tier boundary plugs into.
  - `Embedder` trait (`dimension`, `embed_text`, `embed_image`). The default
    `DeterministicEmbedder` keeps the suite offline and repeatable; the module
    doc notes production text, SigLIP, and RunPod embedders implement the same
    seam.
  - `EmbeddingGraphStore` trait (designate and search the item vector index),
    implemented for both `InMemoryGraphStore` and `RedCoreGraphStore`.
- `commonplace/src/store.rs`: the `Commonplace<S, B>` facade. Methods you will
  call: `put_item`, `get_item`, `all_items`, `items_by_kind`,
  `create_collection`, `get_collection`, `add_to_collection`, `add_similarity`,
  `collection_items`, `tag_item`, `item_tags`, plus `store()` / `store_mut()` /
  `blobs()`. Labels: `ITEM_LABEL`, `COLLECTION_LABEL`, `TAG_LABEL`,
  `ENTITY_LABEL`. Edges: `IN_COLLECTION_EDGE`, `HAS_TAG_EDGE`,
  `SIMILAR_TO_EDGE`, plus the entity edge. Properties: `EMBEDDING_PROPERTY`
  (`embedding`), `LABEL_EMBEDDING_PROPERTY` (`label_embedding`). The store is
  generic over `GraphStore`, so it runs in-memory for tests, durably on
  `RedCoreGraphStore`, or against a server.
- `commonplace/src/collection.rs`: `Collection` with `CollectionKind`
  (`Manual` user-made, `Auto` coined by F2 when a cluster forms).

The connector gateway is the hub (plan: the harness MCP surface).

- `rustyred-thg-mcp/src/connector_gateway.rs`. Its module doc defines two spoke
  classes that stay separate. Federated-MCP spokes are external MCP servers
  whose `tools/list` becomes tenant-scoped `Affordance` nodes, re-exposed
  through the `search` / `describe` / `invoke` meta-tools in this file (the act
  side: send-an-email and the like). Ingestion spokes write source data into the
  graph through verified mappings and do not add callable tools. The GitHub App
  webhook receiver is the named example.
- The federated-MCP side is fully built here: `search_payload` (selection by
  `select_affordances`, PPR plus fitness), `describe_payload`, `invoke_payload`
  (`invoke_affordance` from `rustyred-thg-connectors`, `InvokePolicy::DryRun`
  or `InvokePolicy::FireAllowlist`, with a per-affordance `writeback_policy`).
- `rustyred-thg-connectors` is the outbound MCP transport that carries those
  federated spokes (`register_connector`, sync and tokio-free).

What does not exist yet, verified by searching the tree: a generalized
ingestion-spoke framework. The gateway doc references the class and names the
GitHub example, but there is no ingestion-spoke crate or trait in
`rustyredcore_THG`. The ingestion side is per-source and aspirational. This spec
builds the first generalized version.

Two honest divergences carried from the code. First, `lib.rs` already records
that F1 lands graph-native over `GraphStore` rather than over
`rustyred-thg-catalog`; the catalog stays the home for tenant, key, and billing
rows (plan unit F3), which is where source credentials belong. Second, `source`
is present on `Item` and `IngestInput` today but is never read during
classification; routing by source does not exist yet. Deliverable B1 is what
makes it load-bearing.

## The settled decision

Ingestion is scoped, not full. A connected mailbox is a firehose, so a spoke
pulls only what its scope admits (labels or folders, a recency window, type
filters), both to keep the noise down and to hold the privacy line the Claude
Connectors Directory enforces. Every spoke in this spec carries a scope.

---

## Layer A: the ingestion-spoke framework and the sources

### A1. The source-spoke framework

New crate `rustyred-thg-intake`, sibling to `rustyred-thg-connectors`. Same
convention as the rest of the substrate: sync and tokio-free, generic over the
`GraphStore` the `commonplace` store wraps. It depends on `commonplace` (for the
store and `IngestInput`) and on `rustyred-thg-core`. Placement is a fork (see
the end); the rationale for a crate is that it mirrors the connectors-versus-mcp
split and keeps `commonplace` a pure data layer.

The trait:

```rust
/// A source of items: an external service that CommonPlace pulls from.
/// Implementations map the service's native records onto IngestInput, the
/// universal capture contract. They never call back into the agent.
pub trait SourceSpoke {
    /// Stable source identifier, written onto Item.source (for example
    /// "gmail", "notion", "linear"). Source-agnostic organizing keys off
    /// content, so this string is a routing signal, not a classifier.
    fn source_id(&self) -> &str;

    /// Pull the records the scope admits that changed after the cursor.
    /// Returns the records plus the advanced cursor. Pagination lives inside
    /// the implementation; the driver sees one bounded page at a time.
    fn fetch(
        &self,
        scope: &SourceScope,
        cursor: &SourceCursor,
    ) -> SourceResult<SourcePage>;

    /// Map one native record onto the universal capture contract.
    /// This is the only source-specific shaping the rest of the system sees.
    fn to_ingest_input(&self, record: &SourceRecord) -> SourceResult<IngestInput>;
}
```

Supporting types: `SourceRecord { external_id: String, raw: Value, fetched_at_ms:
i64 }` (the native record plus its stable id in the source), `SourcePage {
records: Vec<SourceRecord>, next: SourceCursor, exhausted: bool }`, and
`SourceResult` over a `SourceError` enum (`Auth`, `Transport`, `RateLimit`,
`Mapping`).

The driver:

```rust
/// Runs one spoke against one tenant's store: fetch the scoped delta, map each
/// record to IngestInput, ingest the batch, advance and persist the cursor,
/// and record source-ref idempotency so a re-run updates in place.
pub fn sync_source<S, B>(
    commonplace: &mut Commonplace<S, B>,
    spoke: &dyn SourceSpoke,
    scope: &SourceScope,
    cursor: SourceCursor,
    pipeline: &IngestPipeline<impl Embedder>,
) -> SourceResult<SyncReport>
where
    S: EmbeddingGraphStore,
    B: BlobStore;
```

`SyncReport { source_id, fetched, ingested, updated, skipped, next_cursor,
receipts: Vec<IngestReceipt> }`. The driver loops pages until `exhausted`,
mapping and ingesting each page through the batch path (A4), deduplicating by
source ref (A3).

Acceptance: a test spoke backed by an in-memory fixture, run through
`sync_source` against `InMemoryGraphStore`, lands its records as `Item`s with the
right `source`, files them through the real `IngestPipeline`, and returns a
`SyncReport` whose counts match the fixture. Re-running with the same cursor
ingests zero and updates the changed ones, never duplicating.

### A2. Scoped fetch configuration

```rust
/// What a spoke is allowed to pull. Scoped by construction.
pub struct SourceScope {
    /// Named containers in the source (Gmail labels, Notion databases,
    /// Outlook folders, Linear teams). Empty means the spoke's documented
    /// default container, never the whole account.
    pub containers: Vec<String>,
    /// Only records touched at or after this instant. Paired with the cursor
    /// for incremental runs; on a first run it bounds the backfill.
    pub since_ms: Option<i64>,
    /// Hard cap on records per sync, so a first connect cannot stall on a
    /// decade of history.
    pub max_records: Option<u32>,
    /// Optional per-source record-type filter (for example Gmail "is:unread"
    /// or Linear "state:open"), passed through opaquely to the spoke.
    pub filters: Vec<String>,
}
```

`containers` empty resolves to a documented per-source default that is a subset,
not the account. Acceptance: a spoke given a scope with two containers and a
`since_ms` fetches only records in those containers after that instant, and a
`max_records` of N returns at most N with a cursor that resumes where it
stopped.

### A3. Incremental sync and idempotency

Re-fetching the same record updates the same `Item` rather than minting a new
one. Identity is `(source_id, external_id)`.

Add a `source_ref` to the item as a first-class field on `Item` in `item.rs`:
`source_ref: Option<SourceRef>` where `SourceRef { source: String, external_id:
String }`. Persist it as a node property and add a store lookup in `store.rs`:

```rust
/// The item that came from this exact source record, if one exists.
pub fn item_by_source_ref(
    &self,
    source: &str,
    external_id: &str,
) -> GraphStoreResult<Option<Item>>;
```

Implement it as a `query_nodes(NodeQuery::label(ITEM_LABEL)
.with_property("source_ref_key", json!(key)))` where `source_ref_key` is the
stable `format!("{source}:{external_id}")` string, written alongside
`source_ref` so the property filter is a single exact match.

The cursor: `SourceCursor` is an opaque per-source token (`{ token: String,
updated_at_ms: i64 }`). The driver persists it per `(tenant, source)`. Where it
persists is the catalog (plan unit F3, `rustyred-thg-catalog`), keyed with the
credential row, since that is the tenant-scoped durable store; the in-memory
test path keeps it in a map.

Acceptance: ingesting a record, then ingesting a changed version of the same
`(source, external_id)`, leaves exactly one `Item`, updated, with its
`SIMILAR_TO` and collection edges reconciled rather than doubled.

### A4. Batch ingest

The current `IngestPipeline::ingest` reads `all_items()` on every call to write
similarity edges, which is a full-store scan per item and quadratic across a
connected source's volume. Add a batch path that snapshots prior items once:

```rust
impl<E: Embedder> IngestPipeline<E> {
    /// Ingest a batch, amortizing the prior-items scan and the vector-index
    /// designation across the whole batch. Similarity is computed against the
    /// snapshot plus items earlier in the same batch.
    pub fn ingest_batch<S, B>(
        &self,
        commonplace: &mut Commonplace<S, B>,
        inputs: Vec<IngestInput>,
    ) -> GraphStoreResult<Vec<IngestReceipt>>
    where
        S: EmbeddingGraphStore,
        B: BlobStore;
}
```

Acceptance: ingesting a batch of N inputs performs one prior-items snapshot and
one index designation, not N of each, and the resulting graph is identical to
ingesting the same inputs one at a time through `ingest`.

### A5. The universal-plus-curated contract

`IngestInput` is the universal target. Two ways to reach it.

Curated spokes are concrete `SourceSpoke` implementations with rich auth, scope,
and incremental sync, one per first-class source (A6). You write the mapping.

The universal path is a `MappedSpoke` that consumes a contract descriptor plus
credentials and derives the record-to-`IngestInput` mapping, so any source whose
contract maps onto the item contract becomes a spoke without bespoke code:

```rust
/// A spoke whose mapping is data, not code. Any source describable by a
/// contract (an OpenAPI or GraphQL endpoint, an MCP server's resources, or a
/// user-supplied field mapping) becomes an ingestion spoke.
pub struct MappedSpoke {
    source_id: String,
    contract: SourceContract,
    transport: Box<dyn RecordTransport>,
}

pub enum SourceContract {
    /// Field paths from the source record onto IngestInput fields.
    FieldMap(IngestFieldMap),
    /// An OpenAPI or GraphQL descriptor plus the response path to the records.
    Schema(SchemaDescriptor),
    /// An MCP server's resources or read tools as the record source.
    Mcp(McpResourceDescriptor),
}
```

`IngestFieldMap` names which source field becomes `title`, which becomes the
body and whether it is text or a blob, which becomes `kind`, and which fields
become `tags`. The point in the doc: organizing is source-agnostic because tier
one classifies by content, so a `MappedSpoke` source organizes identically to a
curated one. Breadth is nearly free because the database does the organizing.

Note the relationship to the gateway. An MCP server can be both an ingestion
spoke here (its resources become `Item`s) and a federated-MCP spoke in
`connector_gateway.rs` (its action tools become `Affordance`s for delegation).
Same server, two roles, the two classes the gateway already separates. The
`Mcp` contract variant reuses `rustyred-thg-connectors` for the handshake.

Acceptance: a `MappedSpoke` built from a `FieldMap` over a JSON fixture ingests
records into correctly typed `Item`s with no source-specific Rust, and the
filed result is indistinguishable from a curated spoke's output for the same
content.

### A6. First-class spokes

Five curated `SourceSpoke` implementations: GSuite, Gmail, Outlook, Notion,
Linear. GitHub is already the ingestion exemplar referenced by the gateway, so
it is the pattern to mirror, not rebuild. For each, the spec names four things,
and the implementation fills them:

- Auth: the credential kind and where the token lives (the catalog credential
  row, F3).
- Scope dimensions: what `SourceScope.containers` and `filters` mean for this
  source (Gmail labels and search operators, Notion databases, Outlook folders,
  Linear teams and states, GSuite Drive folders).
- Mapping: which native fields become `title`, `body`, `kind`, `source`, and
  `tags`. A Gmail message becomes a `Note`-kinded or `Doc`-kinded item with the
  subject as title and the body inline; an attachment becomes a `File` item with
  a `Blob` body through `commonplace.blobs()`. A Linear issue becomes a `Task`
  (Layer C). A Notion page becomes a `Doc`.
- Cursor: the source's incremental token (Gmail historyId, Notion
  last_edited_time, Linear updatedAt, Microsoft Graph delta link).

Acceptance per spoke: against a recorded-response fixture, the spoke fetches a
scoped page, maps it to `IngestInput`, and the driver files it. No live network
in tests.

---

## Layer B: routing and the two-tier boundary

### B1. Source as a routing signal

`source` is stored and unused today. Make it load-bearing in two layers that
compose, with content classification staying primary.

Explicit rules first. A `RoutingRule { source: String, container_match:
Option<String>, collection: String }` lets a user pin a source-and-container to a
collection, for example a Gmail label to a collection or a Linear team to a
project collection. A matching rule hard-routes: the item files to that
collection regardless of cosine. Rules live in the catalog (tenant-scoped, F3)
and are read at ingest.

A soft source prior second. When no rule matches, extend `classify_item` so a
small additive prior favors collections that already hold items from the same
source, so a Linear issue tends to land with other Linear issues without
overriding a strong content signal. The prior is a bounded boost on the cosine
score, not a replacement. Keep `classify_item`'s ungated ranking intact and add
the prior as a separate, documented term so the core stays source-agnostic.

Acceptance: an item whose source-and-container matches a rule files to the rule's
collection even when its top cosine points elsewhere. With no rule, two items of
equal content but different sources rank their shared-source collection slightly
higher, and a strong content match still wins over the prior.

### B2. The two-tier boundary

This is the cost moat made explicit. Tier one is the engine: one embedding and a
cosine ranking over collections, no model call, on every item. Tier two is the
agent, and it touches only what tier one declines. The dial between them is a
confidence ceiling applied to `Classification::confidence()`, the signal that is
already sitting in `ingest.rs` waiting for a caller.

Add an organize decision in `commonplace` (a new module
`commonplace/src/organize.rs`):

```rust
pub enum OrganizeDecision {
    /// Tier one is confident and unambiguous. File silently.
    AutoFiled { collection_id: String, confidence: f32 },
    /// Tier one filed it but the call is close enough to show for review.
    /// Reversible, low-stakes, lands in "organized today".
    FiledForReview { collection_id: String, confidence: f32 },
    /// Tier one declined. This is the bounded "needs you" set: the tier-two
    /// queue, optionally carrying an agent suggestion.
    NeedsYou { candidates: Vec<ClassificationRank>, reason: NeedsYouReason },
}

pub enum NeedsYouReason {
    /// Top score below the file line.
    LowConfidence,
    /// Top two scores within the ambiguity margin.
    Ambiguous,
    /// No collection has a label embedding to match against yet.
    NoCandidates,
}

pub fn decide<S, B>(
    commonplace: &Commonplace<S, B>,
    item: &Item,
    policy: &OrganizePolicy,
) -> GraphStoreResult<OrganizeDecision>
where
    S: EmbeddingGraphStore,
    B: BlobStore;
```

`OrganizePolicy { auto_ceiling: f32, review_floor: f32, ambiguity_margin: f32 }`.
Three bands fall out: at or above `auto_ceiling` and unambiguous goes
`AutoFiled`; between `review_floor` and `auto_ceiling` goes `FiledForReview`;
below `review_floor`, or top-two within `ambiguity_margin`, goes `NeedsYou`. The
bands map onto the auto-organize surface already designed in
`SPEC-COMMONPLACE-AUTO-ORGANIZE`: `FiledForReview` populates organized-today,
`NeedsYou` populates the bounded needs-you set. The needs-you set is literally
the set of items tier one declined, which is the tier-two queue.

The moat lives in this boundary: tier one runs on all items with no model call,
and the agent budget is spent only on `NeedsYou`. A competitor running a model
per item burns money at the volume this handles for free.

Acceptance: a high-cosine unambiguous item returns `AutoFiled` and no agent is
invoked; a mid-band item returns `FiledForReview` and is filed and listed; a
low or ambiguous item returns `NeedsYou` with its ranked candidates and reason.
Running `decide` over a thousand-item batch makes zero model calls.

The two cut points (`auto_ceiling`, `review_floor`) and the `ambiguity_margin`
are the trust-versus-precision dial and are yours to set (fork at the end). The
existing `collection_threshold` of 0.58 is the current auto-file gate and is the
natural starting point for `review_floor`; `auto_ceiling` is a higher bar for
filing silently.

---

## Layer C: the object model extension and the act seam

### C1. Tasks as first-class graph nodes

A task is an `Item` in the same graph as emails, people, and collections, using
each store for what it is good at, which is the multi-model database earning its
keep.

Promote `ItemKind::Task` in `item.rs` (out of `Other("task")`, since tasks get
first-class edges and scalars). Structure is edges, filterable state is indexed
scalars, content is the document body, and the embedding drives classification.

Edges (new types in `store.rs`, with the existing edges):

- `SUBTASK_OF` (Task to Task), so subtask rollup and progress are a reverse
  traversal.
- `DEPENDS_ON` (Task to Task), so "what blocks this" is a traversal.
- `ABOUT` (Task to any Item or Entity it concerns), so "what is this task about"
  walks to the related email, person, or document for free. This is the
  load-bearing one and the reason tasks live in the unified graph rather than a
  task table.
- `WORKED_BY` (Task to an agent-run node), so a delegated task points at its run.
- Source provenance reuses `source_ref` from A3 rather than a new edge.

Scalars as top-level node properties so the relational layer answers filter
queries without loading bodies: `status`, `priority`, `due_at_ms`. Promote these
to first-class fields on `Item` (fork: first-class fields versus documented keys
in `extra`; first-class is recommended for index clarity). With them indexed,
"what is due today" is `query_nodes(label(ITEM_LABEL)
.with_property("kind", "task"))` filtered by a `due_at_ms` range, and the
progress denominator (done subtasks over total) is a `SUBTASK_OF` reverse
traversal plus a `status` count.

Content and artifacts stay in the body (`ItemBody::Inline` for the description,
`Blob` for produced artifacts), addressed by the node. The embedding stays on
the node, so a task auto-files through tier one like anything else.

A task query surface in `store.rs` or `organize.rs`: open tasks, tasks due in a
window, and subtask progress rollup for a parent.

Acceptance: a task created with two subtasks and one dependency answers its
progress by traversal, answers "due today" by a scalar range query, and answers
"what is this about" by an `ABOUT` traversal to the item it came from. Tier one
classifies it into a collection by content, same path as a note.

### C2. The volume-absorber seam to delegation

The tier-two residue is where the agent acts, which is the volume-absorber story:
intake floods, the engine sorts, delegation absorbs, the surface bounds. This
deliverable is the seam, not a redesign of delegation (the `organizeAction` plus
run-event-binding design stands).

A `NeedsYou` item can carry an agent suggestion (draft, delegate, develop). When
accepted, it fires a federated-MCP affordance through the existing gateway path
in `connector_gateway.rs`: `invoke_payload` to `invoke_affordance` with
`InvokePolicy::FireAllowlist`, and the affordance's `writeback_policy` lands the
result on the object (the drafted reply attaches to the email, the delegated
task's run populates its subtasks and artifacts). The act side is already built;
this wires the residue to it.

Add a batch-absorb entry: hand the agent a whole connected stream's `NeedsYou`
residue to process, rather than one item at a time, so a first connect of a busy
mailbox is cleared down to the bounded set that genuinely needs a human. This
reuses the dispatch board and `theorem-receiver` already in the tree.

Acceptance: accepting a suggestion on a `NeedsYou` item invokes the correct
affordance with writeback and the result is readable on the object afterward.
Handing a residue batch to the absorb path clears the auto-decidable remainder
and leaves only the human-needed items in the needs-you set.

---

## Open forks

These are real decisions left to you, not defaults to assume.

1. Placement of the spoke framework: a new `rustyred-thg-intake` crate
   (recommended, mirrors connectors-versus-mcp) or a module inside
   `commonplace`.
2. The two cut points and the ambiguity margin in `OrganizePolicy`
   (`auto_ceiling`, `review_floor`, `ambiguity_margin`). The trust-versus-
   precision dial. `review_floor` near the existing 0.58 is the natural start.
3. Task scalars as first-class `Item` fields (recommended) versus documented
   keys in `extra`.
4. Source-signal strength: explicit rules hard-route (recommended) and the soft
   source prior is a bounded boost. How large the boost is, and whether a rule
   may also force a brand-new collection, are yours.
5. `ItemKind::Task` promoted to a first-class variant (recommended) versus left
   as `Other("task")`.

## Cleanup noticed in passing

`store.rs` defines `MENTIONS_EDGE` as `"MENTIONS"` while `ingest.rs` writes
`MENTIONS_ENTITY_EDGE` as `"MENTIONS_ENTITY"`. Align them when touching the
model so entity traversals are not split across two edge types.
