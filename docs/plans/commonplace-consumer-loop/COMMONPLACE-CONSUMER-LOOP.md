# CommonPlace: the full product, as an agent loop

This replaces the foundation-only version. Every engine dependency the earlier
draft named is landed, so the gating is removed and the foundation units are ready
now. Beyond the foundation this maps the whole product, because most of the whole
product already exists in Theseus, the harness, and the engine, and CommonPlace is
the consumer projection that wires them to a personal-data object model. Each unit
carries observable acceptance so a head knows when it is done.

## The target
CommonPlace as a portable, auto-organizing, multi-agent personal database with
interchangeable front ends. Three layers held apart on purpose:
- The database is portable, self-hosted on Railway, AWS, Azure, or local, or
  hosted, and it speaks one API.
- The API is the universal seam: anything that speaks it plus a key is a client.
- The front ends are interchangeable: the CommonPlace app, Notion through a
  connector, or a custom UI. Multi-agent collaboration is the app's first-class
  feature; everything else is swappable.

The promise is that you put anything in and it organizes itself with no button,
the system is intelligent over everything you saved, agents work alongside you in
it, and your data is portable and moves with you.

## Dependencies are landed
The engine pieces this builds on, the GraphQL surface, the document and folder
tier, the visual subsystem, the lite engine, and the sync substrate, are landed.
There is no engine gate left. The only ordering is internal: the object model
comes before the things that read it, the API comes before the clients of it.
Take any ready unit.

## How to work the loop
- Take a ready unit, build it to its acceptance, verify against the running
  engine, record the result through the harness, take the next ready unit.
- Announce footprints through the harness room and reconcile concrete edits there.
  Heads work in parallel where units do not share files.
- Heavy compute, embedding and extraction, runs on RunPod. Do not use Modal.
- This doc lives in the repo at `docs/plans/` so the loop reads it in-tree.

---

## Foundation: the data layer

### F1 the consumer object model
The personal-database object model, generic enough to store anything and
structured enough to organize and query. Home: a `commonplace` module over
`rustyred-thg-catalog`.
- `Item`, the universal unit: `{ id, kind (file | note | link | image | doc |
  ...), title, body, source, residency (local | synced | hosted), tags,
  collections, embedding_ref, classification, created_at, updated_at }`. A `File`
  is an `Item` whose body is a content-addressed blob in `DiskObjectStore`.
  `Collection` is a named grouping, auto or manual. `Tag` is a label.
- Residency is a field on every `Item` and is the hook the sync layer reads.

Acceptance: an `Item` of any kind writes and reads back with its metadata,
residency, tags, and collections; a `File` resolves its blob by content hash; a
`Collection` returns its items.

### F2 the auto-structuring ingest pipeline
The no-button auto-organize, the product's core. Anything ingested is embedded,
classified, filed, and linked automatically. Home: `commonplace/ingest.rs`.
- `ingest(input)` detects the type, embeds it (a text embedder for text and
  extracted document text, SigLIP for images), classifies it by comparing the
  embedding to existing collection and tag label embeddings, creating a new
  collection when confidence is low and a cluster has formed, files it into the
  folder tree and its collection, links it to its nearest neighbors in the graph,
  and writes the `Item`.
- For unstructured documents, the law-firm case, it extracts candidate fields
  (entities, dates, types), resolves near-duplicate entities by embedding
  similarity so the same client spelled three ways becomes one entity, then
  classifies and files. This is auto-structuring, not a sort.
- It runs on ingest with no user action. Batch work runs on RunPod.

Acceptance: a dropped document or image is embedded, classified into a collection,
filed into the folder tree, linked to similar items, and made similarity-searchable
with no user action; a near-duplicate entity resolves to the existing one.

**Implementation Notes:**
- Implemented in `rustyredcore_THG/crates/commonplace/src/ingest.rs` as `IngestPipeline`.
- Considered: call external text/SigLIP services directly vs define a local `Embedder` seam.
- Chose: deterministic local `DeterministicEmbedder` for acceptance and offline-first runs, with `Embedder` ready for RunPod-backed text/image embedders.
- The pipeline detects input kind, stores blobs for binary/image items, writes top-level item `embedding` properties for the engine vector index, creates or reuses auto collections from label embeddings, writes `SIMILAR_TO` edges, stores folder-path metadata, and resolves near-duplicate `Entity` nodes through canonicalization plus embedding similarity.
- Validation: `cargo test -p commonplace` and `cargo clippy -p commonplace --all-targets --no-deps -- -D warnings`.

### F3 the interoperability API seam
The universal connection point, so any front end or self-hosted instance talks to
the database with the same API. Home: the consumer schema profile in the GraphQL
surface plus key auth.
- The GraphQL surface exposes the F1 object model as a consumer profile: queries
  for items, collections, and search; mutations for ingest and edit. Per-user or
  per-instance API keys; a client connects with an instance URL and a key.

Acceptance: an external client with a URL and a key reads and writes items; pointing
it at a different instance URL connects to that instance's data; an invalid key is
rejected.

---

## Intelligence: Theseus over the personal store
Mostly wiring existing endpoints to the F1 object model.

### I1 ask over your store
Point the unified retrieve, graph plus vector plus FTS with RRF, at the consumer
store, so a question answers from everything you saved, with provenance.

Acceptance: a natural-language question returns an answer grounded in the user's
items, each claim traceable to the item it came from.

### I2 proactive surfacing
Briefing and graph-weather over the user's data: what is new, what connects, what
is unresolved, surfaced without being asked.

Acceptance: a briefing call returns recent and newly connected items and open
threads drawn from the user's store.

### I3 epistemic surfacing
Run the claim and tension layer over the user's own notes, so contradictions and
unresolved tensions in what they saved are surfaced.

Acceptance: two items asserting conflicting things produce a flagged tension the
user can open.

### I4 discovery
Propose connections the user did not make and hypotheses across their items.

Acceptance: a discovery call returns candidate links between items that are not yet
connected, ranked.

---

## Capture: the input surface

### CAP1 quick capture
Capture from everywhere: a web clipper browser extension, a phone share-sheet, a
voice-in path, and an email-in address. Each capture flows straight into F2.

Acceptance: a clipped page, a shared item, a voice note, and a forwarded email each
arrive as items and auto-structure with no further action.

### CAP2 source auto-ingest
Connect a source, files, mail, or bookmarks, and have it flow in and auto-structure
continuously.

Acceptance: connecting a source backfills its contents as items and new entries
arrive and auto-structure on their own.

---

## Agents: the first-class differentiator
Mostly wiring the harness into a personal workspace.

### AG1 agents in the store
Agents that research into the store, answer from it, and act on it, drafting from
the user's items.

Acceptance: an agent ingests and organizes new material into the store and answers
a question from it in the same session.

### AG2 multi-agent collaboration
Several agents collaborating in one personal workspace through the harness, with the
user present in the same space.

Acceptance: two agents work the same workspace, announce footprints, and reconcile
without clobbering each other.

### AG3 MCP access
Any model reaches the store over MCP, so the workspace is not tied to one frontend
model.

Acceptance: an external model connects over MCP and reads and writes items.

---

## Views: the app surface
New UI over existing renderers.

### V1 the graph view
Scene OS over the consumer store: see your knowledge as a graph and navigate it.

Acceptance: the store renders as a navigable graph; selecting a node opens the item.

### V2 rich item views
Per-type views: a note editor, an image viewer, a PDF viewer, a link preview.

Acceptance: each item kind opens in a view appropriate to it and edits persist.

### V3 smart collections and timeline
Auto-updating saved queries, recents, and a timeline.

Acceptance: a saved query updates as matching items arrive; a timeline orders items
by date.

### V4 the console
The power-user surface over the API for direct query and bulk action.

Acceptance: a user runs a query and a bulk operation from the console and sees the
result reflected in the store.

---

## Interoperability: no lock-in

### X1 connectors
Notion, one-way mirror first and bidirectional later, and Obsidian. The
bidirectional version, editing in either and both converging, is the hard kind,
conflict resolution and schema mapping and rate limits, and is a later refinement.

Acceptance: items appear in a connected Notion database one-way with a documented
mapping, within Notion's API limits; Obsidian vaults round-trip.

### X2 import and export
Plain markdown and JSON in and out, so data enters and leaves cleanly.

Acceptance: a markdown or JSON export reimports without loss.

---

## Sync and trust
The networked build.

### S1 per-item residency and selective sync
Residency drives the policy: local-only items never leave the device, synced items
replicate to the hosted instance and converge after edits, hosted items live in the
cloud. The sync layer between the lite local engine and the hosted instance is the
real build, cursor-based and last-write-wins per the existing versioning, respecting
residency.

Acceptance: a local-only item never appears on the hosted instance; a synced item
appears on both and converges after an edit on either; changing residency moves the
item.

### S2 end-to-end encryption
Synced data is end-to-end encrypted, so the hosted tier cannot read it.

Acceptance: synced item contents are unreadable on the server without the user's
key.

---

## The moat: the epistemic web
The outer build.

### M1 collective intelligence
Opt-in sharing of structure, not raw data, so a user's graph benefits from others'
without exposing their content.

Acceptance: with sharing on, a user receives structural signal derived from the
wider graph; with it off, nothing of theirs leaves.

### M2 shared spaces
Collaborative CommonPlace spaces, several people in one workspace.

Acceptance: two users share a space and both see and edit its items, with the
multi-agent layer available in it.

### M3 GL-Fusion frontends
GL-fused models as the preferred intelligence frontends over the user's graph, the
paid tier.

Acceptance: a GL-fused model answers over the user's store as a selectable
intelligence frontend.

---

## What is new versus what is wiring
- New build: the data layer (F1, F2), the capture surface (CAP1, CAP2), the app UI
  (V1 to V4), the connectors (X1, X2), the sync and trust layer (S1, S2), and the
  moat (M1 to M3).
- Mostly wiring existing capability to the personal object model: the intelligence
  layer (I1 to I4 over Theseus), the agent layer (AG1 to AG3 over the harness), and
  the graph view (V1 over Scene OS).
- The foundation, F1 to F3, is the near-term slice and unblocks everything else.
