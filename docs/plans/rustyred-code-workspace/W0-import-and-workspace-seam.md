# W0: Import path + workspace seam

The shared front door. A real git checkout becomes a working tree the embedded
engine indexes, and the engine grows the minimal public seam every later layer
needs. Everything else in the plan (W1 on-write, W2 git, W3 execution) depends on
this unit landing first.

Dependency edges: **W0 precedes W1, W3** (they need a working tree and store
access that only W0 provides). **W0 is independent of W2** (git-as-truth can be
built in parallel; they converge at "commit, push").

## Thesis

The north star is "mostly assembly plus wiring over pieces that already exist."
W0 is where the assembly starts: it reuses CodeCrawler's existing
clone-to-temp primitive as the materialize/import step, walks that tree into the
`DocTree` as files-as-nodes, and persists once instead of per file. The one
genuinely new design is small: the `Engine` hides its store, doc-tree, and object
store as private fields, so W0 opens a narrow, honest workspace seam rather than
forking the engine.

## What already exists (reuse, do not rebuild)

- `stage_repo_for_ingest_with_credential(input, url, credential) -> (input, clone_ms, Option<FetchedRepo>)`
  at `rustyredcore_THG/crates/rustyred-thg-code/src/lib.rs:874`. Shallow-clones a
  repo into a temp dir, rewrites `input.repo_path` to the clone, auto-excludes
  `.git`, and returns the `FetchedRepo` RAII handle so the caller owns the temp-dir
  lifetime. This is the import seed; it already accepts a `GitCredential` (the
  credential plumbing W2 reuses for private repos).
- `FetchedRepo { path, keep }` at
  `rustyredcore_THG/crates/rustyred-thg-code/src/repo_fetch.rs:86`: RAII temp dir,
  auto-cleans on drop unless `keep` is taken.
- `Engine::open(data_dir, EmbeddedConfig)` at `apps/rustyred-embedded/src/lib.rs:163`
  already recovers `RedCoreGraphStore` (AOF replay) and the `DocTree` (from
  `doc-tree.json`) on restart. Import builds on top of an open engine.
- `Engine::fs_write(path, content) -> content_hash` at `lib.rs:256`: writes a
  `File` graph node keyed by path, with content hash and a 16-dim embedding, and
  persists the tree. Files-as-nodes is already the contract; the import just drives
  it in bulk.
- `DocTree::put_body(path, body, tier, created_ms, gist, object_store) -> DocEntry`
  at `rustyred-thg-core/src/doc_tree.rs:164`: the inline/overflow write under
  `fs_write`. Inline below threshold (zstd), overflow to `DiskObjectStore`;
  maintains the `previous_hashes` version chain.
- `DiskObjectStore::put_document_bytes` / `get_document_bytes` at
  `rustyred-thg-core/src/object_store.rs:155` / `:166`, content-addressed
  `sha256:` over raw bytes (`content_hash_bytes` at `:232`).
- `SharedStore::with_store(|store| ...)` at
  `rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs:677`: the in-process handle
  that yields `&mut RedCoreGraphStore` between GraphQL calls. The seam the importer
  reaches the durable store through.

## The gaps W0 closes

1. **No batch-import path.** `fs_write` calls `persist_doc_tree`
   (`apps/rustyred-embedded/src/lib.rs:317`) on every write, which re-serializes the
   entire `OrdMap` to `doc-tree.json` (confirmed: north star's claim is accurate).
   Importing N files is O(N) full serializations. W0 adds a build-then-persist-once
   path.
2. **No workspace accessors on `Engine`.** `Engine` owns `SharedStore<RedCoreGraphStore>`,
   the `DocTree` (behind `RefCell`), and the `DiskObjectStore` as private fields
   (`lib.rs:147`, `:154`, `:155`) with no public accessor. W1 (on-write hooks), W2
   (git scanning blobs), and W3 (materialize) all need this. W0 opens a narrow seam.
3. **No tree enumeration.** `DocTree` exposes only `range_prefix(prefix)` and the
   engine exposes only `fs_ls(prefix)`; there is no "list all paths" for a git
   layer or a full materialize. W0 adds an iterator on the workspace seam.

## What to build

A new crate `apps/rustyred-workspace` (path-depping into `rustyred-thg-code`,
`rustyred-thg-core`, and `rustyred-embedded`), plus the one minimal core/embedded
touch for the seam. New code in a new crate is the low-collision lane (mirrors how
E0 landed `apps/rustyred-embedded`).

### W0.1: the workspace seam on `Engine`

Add to `apps/rustyred-embedded/src/lib.rs` a small public facade, not a fork:

- `Engine::with_store(|store: &mut RedCoreGraphStore| -> R) -> R` (delegates to the
  inner `SharedStore::with_store`).
- `Engine::with_doc_tree(|tree: &DocTree| -> R) -> R` and a `_mut` variant guarded so
  it cannot be nested inside a borrow that already holds the `RefCell` (the existing
  borrow panic risk, called out in grounding).
- `Engine::object_store() -> &DiskObjectStore`.
- `Engine::list_paths(prefix: &str) -> Vec<String>` (full enumeration, the missing
  iterator).

Anti-pattern to avoid: a parallel second engine, or a global accessor that bypasses
the `RefCell` discipline. The seam is a handful of borrowed-closure accessors on the
existing `Engine`, nothing more.

### W0.2: batch import (build-then-persist-once)

In `apps/rustyred-workspace`:

- `import_checkout(engine: &Engine, repo: &Path, opts: ImportOptions) -> ImportReceipt`.
  Walks the materialized tree (respecting `.gitignore` and an explicit
  source/artifact filter, the boundary from the README: index source, skip
  `target/`, `node_modules/`, `.git/`), and for each file writes the body + `File`
  node into the engine via a single deferred-persist batch.
- Add a `defer_persist` mode to the engine's write path so the importer drives N
  `put_body` + node upserts and persists the `doc-tree.json` once at the end. This
  is the one place where the batch path needs an embedded-engine change beyond the
  seam: either a public `Engine::fs_write_batch(items)` that suppresses
  `persist_doc_tree` until the end, or a `BatchGuard` that defers persistence on drop.
  Prefer `fs_write_batch` (explicit, no Drop-order surprises).
- Reuse `stage_repo_for_ingest_with_credential` to produce the materialized tree
  from a URL (or accept a local path directly for the local-checkout case).

### W0.3: round-trip and restart proof

The import is durable across restart (the engine already rehydrates the DocTree and
the store), and the imported tree is queryable: `fs_read` returns the bytes,
`fs_ls`/`list_paths` enumerates, and each file is a `File` graph node reachable via
the existing GraphQL `graphNode` / `vectorSearch` surface.

## Acceptance criteria

1. A Rust test opens an engine over a temp dir, imports a small fixture checkout
   (tens of files, at least one larger-than-inline-threshold file so the overflow
   path is exercised), and asserts: every source file is readable via `fs_read`,
   `list_paths` enumerates exactly the imported source paths (and none of the
   excluded `target/` / `.git/` paths), and each file is a `File` graph node found
   by label query.
2. The batch import persists `doc-tree.json` exactly once (assert via a write-count
   probe on the persist path, or by timing/serialization-count instrumentation), not
   once per file. This is the concrete proof the O(N)-serialization defect is gone.
3. After dropping and re-opening the engine over the same data dir, the full
   imported tree rehydrates: `fs_read` of a previously imported file returns the
   same bytes, and its `File` node is present (the E0.2 restart-rehydration guarantee
   extended to a bulk import).
4. The workspace seam compiles and is exercised: a test reaches the durable store via
   `Engine::with_store`, reads a `File` node, and the `DocTree` via `with_doc_tree`,
   without the importer holding a private field.
5. `cargo check` clean across `apps/rustyred-workspace` and the touched embedded
   crate; changed files clippy-clean. The new crate links no server framework (single
   embedded discipline: verify with `cargo tree` that it pulls in no axum/tonic).

## Divergences and risks to surface (not bury)

- **`RefCell` borrow discipline.** The `DocTree` is behind a `RefCell`; the importer
  must not call `fs_write`/`with_doc_tree_mut` from inside a closure already
  borrowing the tree, or it panics. The batch API must take the borrow once and hold
  it for the batch, not re-enter.
- **Persist durability mismatch.** `persist_doc_tree` is a synchronous `fs::write`
  while the store uses AOF; a crash mid-batch (after store writes, before the single
  persist) can leave graph `File` nodes without their `doc-tree.json` entry. The
  batch path should persist the tree before reporting success, and a recovery probe
  should reconcile orphaned `File` nodes on next open (named follow-up if not in W0).
- **Embedding placeholder rides along.** Imported files get the 16-dim hash embedding
  (`FILE_EMBEDDING_DIM = 16`, `lib.rs:386`). That is fine for W0 (vector-searchable at
  all); W4 swaps it for a real code encoder. Do not block W0 on embedding quality.
- **The materialize copy is real disk.** `stage_repo_for_ingest` clones to a temp
  dir; the import reads from there. For the local-checkout case the user already has
  a tree, so the importer should accept a local path and skip the clone (the clone is
  only for the URL case).
