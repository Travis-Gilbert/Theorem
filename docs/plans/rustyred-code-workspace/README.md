# RustyRed as Code Workspace: build plan

The execution plan for `NORTH-STAR-RUSTYRED-CODE-WORKSPACE.md` (a companion to
`docs/plans/rustyred-multimodel/`, Area E embedded mode). Where E0 made the
folder tree the agent's working filesystem, this turns that filesystem into an
editable, runnable git working tree that a user and their agents write code
inside, with the full RustyRed intelligence stack (graph, vector, code graph,
epistemic, versioned history) live over every file.

This README is the navigation truth and the loop register. Order of truth: this
map, then the unit specs (`W0`-`W6`), then the actual code. The unit specs are
grounded against the live tree (read 2026-06-21, eight parallel readers over the
seven subsystems); the `file:line` anchors and divergence flags inside them are
the real seam, not aspiration.

## The headline "done" sentence

> A user runs the embedded engine locally, imports a git checkout into its
> filesystem, and edits and runs that code with their agents inside RustyRed. The
> real toolchain reads and writes the same tree, every file is a live graph node,
> indexing and graph and epistemic maintenance happen as a side effect of the
> write, and only genuine semantic queries (what calls this, find me similar,
> what is downstream, what is contradicted) stay tool calls.

## The thesis the plan is built to prove: the tool-call collapse

When a file changes inside the engine, three things must stay true: its bytes are
stored, its intelligence is fresh (embedding, code-graph symbols and edges,
epistemic and staleness signals), and the change is durable and versioned. Sort
by who does the work:

- File read and write becomes a filesystem operation, not MCP (W0, W6).
- Maintenance (embed, symbols, edges, staleness) becomes a write side effect, not
  MCP (W1, W4).
- Only the genuine semantic query stays a tool call (already built: the Area A
  GraphQL surface).

The measurable payoff (fewer round trips, fewer tokens, lower latency) is itself a
unit acceptance criterion, not a slogan: W1 ships a before/after round-trip count
on a fixed editing task.

## The seven-layer model, mapped to owners

| Layer | What it is | Plan home | Status |
|---|---|---|---|
| 1. Remote truth: GitHub | push, pull, PRs, sharing | W2 | to build |
| 2. Local durable VCS: real git | `gix` repo; objects in `DiskObjectStore` | W2 | to build (no git lib exists yet) |
| 3. Live working tree | materialized dir near-term, FUSE endgame | W0 (build), W3 (run), W6 (FUSE) | W0/W3 to build, W6 deferred |
| 4. Semantic overlay on write | CodeCrawler symbols/edges + embedding + staleness | W1, W4 | to build (encoder exists, on-write + incremental edges do not) |
| 5. Semantic versioning | graph-version Prolly-tree commit/branch/diff/merge | (none) | already built, the plan consumes it |
| 6. Execution: real OS processes | materialize-to-run via the receiver | W3 | to build (receiver spawns children; no materialize layer) |
| 7. Collaboration | code surface adapter into `theorem-copresence` | W5 | to build (deferred behind a shared working tree) |

Layer 5 has no unit because it is done: `versioned_graph.rs` already gives
content-addressed `compile_graph_pack` / `checkout_graph_version` /
`diff_graph_snapshots` / `merge_graph_snapshots` (3-way, confidence-resolved) /
`update_graph_ref_cas`, with `node_to_content_object` / `edge_to_content_object`
round-trips the north star references. The plan uses it as the layer-4 (knowledge)
VCS; git (W2) is the layer-2/3 (code) VCS. Two version-control systems for two
kinds of state: graph-version cannot represent a git history GitHub will accept
(its `GraphCommit.graph_version` counter feeds the commit hash), so git is a
complement, not a substitute.

## What unblocks what (the dependency DAG)

```
            W0  import + workspace seam   (the shared front door)
           /  \         \
          /    \          \
        W1      W3          W2
   on-write   exec       git-as-truth
   overlay    bridge     (gix + GitHub)
        \       |           /
         \      |          /
          \     |         /
           the proven loop: edit (fs) -> overlay maintained (no MCP) ->
           query semantics (MCP) -> run (sandbox) -> commit -> push
                   |                    |
                  W4                   W5                   W6
          real code embedding   collaboration adapter   FUSE endgame
          (quality of overlay)  (presence over the      (collapse the
                                 shared tree)            materialize copy)
```

Edges, with the concrete reason:

- **W0 precedes everything.** Both W1 (on-write overlay) and W3 (execution) need a
  working tree that exists; the import path that builds it, and the workspace seam
  that exposes the engine's private store/doc-tree/object-store, are W0.
- **W0 precedes W1** (on-write needs files-as-nodes to already land via the import
  path) and **W0 precedes W3** (materialize-to-run needs a tree to materialize).
- **W2 is parallel to W1/W3** (git-as-truth underlies versioning and the remote;
  it does not block the in-engine edit-index loop, and the edit loop does not block
  it). They converge at "commit, push".
- **W4 sharpens W1** (real encoder swaps the placeholder; the on-write hook is the
  same, only the embedding function changes). W4 can land before or after W1.
- **W5 follows the proven loop** (a code presence adapter needs a single shared
  working tree to be present on).
- **W6 (FUSE) is the endgame** that collapses W0/W3's materialize copy once the
  loop is demonstrably working; it is the layer-3 upgrade, deferred by design.

## The source-of-truth decision (resolved, announce-not-ask)

The north star left three options open. Decision, with the reason, so W0/W3 are not
blocked on a fork:

- **Near-term: DocTree-primary, materialize-to-run.** The `DocTree` is the source
  of truth; the relevant subtree is materialized into a real sandbox directory to
  build and run; toolchain edits sync back into the `DocTree`. It reuses
  CodeCrawler's existing clone-to-temp primitive (`stage_repo_for_ingest`) as the
  materialize step and makes the whole loop demonstrable with pieces already in the
  tree. This is W0 + W3.
- **Endgame: FUSE, a single copy.** Mount the `DocTree` as a real POSIX filesystem
  so the toolchain and RustyRed edits land in the same place with no sync. This is
  W6, deferred until the materialize loop is proven (the north star's own arc).
- **Rejected: sandbox-primary with a watcher.** It makes RustyRed a read-mostly
  mirror, which weakens the "edit inside RustyRed" framing the product leads with.
  Not pursued.

## Boundaries that keep it honest (carried into every unit)

- **Not an OS.** An application-level filesystem stores, edits, versions, and
  reasons over code; it does not execute code. Execution is a real process against
  a real OS directory (W3), the standard cloud-dev pattern (durable store plus
  ephemeral sandbox plus sync/mount). The differentiator is that the store is also
  a graph/vector/epistemic engine.
- **The WASM host is a separate sandbox.** The Wasmtime plugin host cannot run a
  native `cargo build`. "Move the sandbox inside RustyRed" means the embedded
  engine owns the lifecycle of real-process execution (W3), not that execution runs
  in-process.
- **Code is not CRDT-merged.** Code collaborates through git (W2) plus a presence
  layer (W5); CRDT stays for the typed objects and free text it already serves in
  `theorem-copresence`. W5 is awareness, not merge.
- **Reactive indexing fires on source, not artifacts.** A build writes thousands of
  transient files; re-embedding a compiler's intermediate output makes builds
  unusable. Source is indexed and read-write; `target/`, `node_modules/`, and the
  like go to throwaway disk and are not indexed (W1 and W3 both enforce this).

## Loop scorecard

Readiness, not estimates. Order units by readiness; a unit is GATED only when a
real external constraint blocks it, named in the Gate column.

| Unit | Title | Status | Gate |
|---|---|---|---|
| W0 | Import path + workspace seam | spec ready | none; reuses `stage_repo_for_ingest_with_credential` + a batch-persist DocTree path |
| W1 | CodeCrawler-on-write + incremental edges | spec ready | depends on W0; the incremental-edge strategy is the one real algorithm to design |
| W2 | Git-as-truth (`gix`) + GitHub remote | spec ready | none for local git; remote reuses the existing `GithubApp` auth; new dep `gix` |
| W3 | Execution bridge (materialize-to-run) | spec ready | depends on W0; reuses `run_proof` + `SandboxRuntime`; isolation choice is open (W3 resolves it) |
| W4 | Real code embedding | spec ready | encoder/model choice (offline candle vs hosted); the seam is in place |
| W5 | Collaboration adapter (presence over code) | deferred | follows the proven loop (needs W0+W3); a `file:line:col` cursor model is new |
| W6 | FUSE endgame | deferred | follows a proven materialize loop; heaviest; platform-specific (macFUSE/fuse3) |

## Coordination posture

`rustyred-thg-core` (`doc_tree.rs`, `versioned_graph.rs`, `graph_store.rs`) is a
hot, Codex-active crate. The lazy-and-safe lane choice, mirroring how E0 landed:

- **New code lands in a new crate where possible.** W0's importer, W2's git layer,
  and W3's execution bridge should be a new `apps/` crate (or crates) path-depping
  into core, not edits inside core. The one unavoidable core touch is the workspace
  seam (W0): a few public accessors on `Engine` and possibly a `defer_persist` flag
  on the DocTree write path. Keep that edit minimal and announce it before editing.
- **Commit with explicit pathspec only** (`git commit -- <paths>`), never a bare
  commit; the index can carry another head's staged files.
- **Read the room before editing a shared file.** The harness `coordinate` /
  `stream_read` surface is the channel; git history is the fallback.

## Open questions the units resolve (so they are not left dangling)

1. **Engine accessor seam.** The `Engine` owns `SharedStore<RedCoreGraphStore>`, the
   `DocTree`, and the `DiskObjectStore` as private fields with no accessors. Every
   later layer needs store/tree/object access. W0 resolves this: a small public
   workspace facade on `Engine`, not a fork. Anti-pattern to avoid: a parallel
   second engine.
2. **Batch import without per-file re-serialization.** `fs_write` re-serializes the
   whole `doc-tree.json` per write (confirmed). Importing thousands of files this
   way is O(N) full serializations. W0 adds a build-then-persist-once path.
3. **Incremental call-edges.** `infer_symbol_call_edges` is a whole-graph name-match
   pass (confirmed). Single-file edits cannot re-run the global pass. W1 designs the
   incremental strategy (a persistent name-bucket index + recompute only the touched
   buckets).
4. **Git objects: shared store or own DB.** Whether the local git repo's objects
   live in the `DiskObjectStore` from day one or git keeps its own object database
   with convergence later. W2 resolves this (recommendation: git owns its own
   `.git/objects` first, converge later; both are content-addressed, so convergence
   is a later optimization, not a prerequisite).
5. **Execution isolation.** Containerized child processes vs a lighter sandbox,
   given the receiver already spawns children and already has an `OpenSandboxRuntime`
   HTTP-sidecar backend. W3 resolves it (reuse the `SandboxRuntime` trait; the
   first backend is the existing sidecar, a local-process backend is the dev path).
