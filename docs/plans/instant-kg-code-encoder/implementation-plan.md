# Instant KG Rust code-encoder loop - implementation plan

**Status:** Design / plan. Claude-authored, grounded against live code on 2026-06-05.
**Owner lane:** Claude (this plan + the SessionDelta converter); coordinate with Codex on the two hot files (`code_index.rs`, `rustyred-thg-mcp/src/lib.rs`).
**Scope:** A native Rust loop that turns a code edit into an instantly-served knowledge-graph view: parse the changed files, produce a `SessionDelta`, overlay it on a base graph snapshot, and serve the merged view through the existing `harness_kg_*` surface. No Python in the path.
**Source framing:** the Jun-5 reconciliation ("build the native Rust parser (tree-sitter plus a stack-graphs semantic layer) and wire the existing versioned_graph diff to produce the SessionDelta into instant_kg.rs"). Read the premise correction below before that sentence is taken literally; check `docs/plans/skill-encoder-theorem-port/implementation-plan.md` for the surrounding two-repo split.

## Premise correction (read first)

Grounding the repo inverts most of the task's framing. Three of the four named pieces already exist:

- **`instant_kg.rs` EXISTS** at `rustyredcore_THG/crates/rustyred-thg-core/src/instant_kg.rs` (742 lines). It already defines `SessionDelta` (`:55-69`: `commit_sha`, `changed_files`, `objects: Vec<NodeRecord>`, `edges: Vec<EdgeRecord>`, `tombstoned_object_ids`, `removed_edge_ids`), `CodeKgManifest` (`:20-53`), and `HarnessInstantKg::new(base: GraphSnapshot, manifest, delta)` (`:155-529`) which merges base + delta and serves `status`/`merged_snapshot`/`ppr`/`impact`/`related_objects`/`search`/`explain_edge`/`resolve_symbol_name`. It is wired into MCP as `rustyred_thg_instant_kg_*` (aliased `harness_kg_*`) and into HTTP in `rustyred-thg-server/src/router.rs`.
- **The versioned-graph diff EXISTS** at `rustyredcore_THG/crates/rustyred-thg-core/src/versioned_graph.rs` (1364 lines): `diff_graph_snapshots(base, target) -> GraphVersionDiff` (`:417`). But it emits **hash-only** entries (`GraphDiffEntry { key, kind, old_hash, new_hash }`), not the body-carrying `SessionDelta` that `HarnessInstantKg` consumes.
- **A Rust parser EXISTS** at `apps/theorem-grpc/src/code_index.rs` - but it is **`syn`, not tree-sitter** (`Cargo.toml:37`; `syn::parse_file` + `syn::visit::Visit` at `:1546-1685`). It extracts fns/structs/enums/traits/impls and call/dependency edges, and writes them **straight into the store** as `GraphMutation` batches (`:608-670`), never as a `SessionDelta`.
- **tree-sitter and stack-graphs do NOT exist** anywhere (no Cargo dep, no code). The only semantic resolution today is name-matching across symbols (`infer_symbol_call_edges`, `code_index.rs:1355`).

So the real work is not "create instant_kg.rs and write a parser." It is:

1. The **missing connector**: parser output -> `SessionDelta` (the converter does not exist; `code_index.rs` writes straight to the store).
2. A **delta source**: either a shape adapter over `diff_graph_snapshots` (hash entries -> full records) or a direct delta computed from the parse against the base.
3. **Feeding** `HarnessInstantKg` from a live parse (today its delta comes only from request payloads / tests, `:609`).
4. As the one genuinely-new capability: an **optional stack-graphs semantic layer** for cross-file name resolution. tree-sitter is only required if non-Rust languages must be parsed.

## What exists (reuse, do not rebuild)

| Piece | Location | What it already does | Reuse as |
|---|---|---|---|
| `SessionDelta` | `rustyred-thg-core/src/instant_kg.rs:55` | The delta shape: objects, edges, tombstones, removed edges | The converter's output type |
| `HarnessInstantKg` | `instant_kg.rs:155` | Merge base snapshot + delta; serve ppr/impact/search/explain_edge | The served session view (feed it) |
| `CodeKgManifest` | `instant_kg.rs:20` | repo_id / commit_sha / encoder_version / encoded_files | The session provenance header |
| `harness_kg_*` MCP + HTTP | `rustyred-thg-mcp/src/lib.rs:769-871`, `rustyred-thg-server/src/router.rs` | Serve the instant KG over MCP and HTTP | The query surface (already live) |
| `diff_graph_snapshots` | `versioned_graph.rs:417` | Hash-based diff between two snapshots | Provenance / version-vs-version; not the SessionDelta source |
| `syn` Rust extractor | `theorem-grpc/src/code_index.rs:1546` | Parse Rust -> symbol nodes + call/dep edges | The parse step (emit records instead of store writes) |
| CodeCrawlerService verbs | `theorem-grpc/src/code_service.rs` | IngestCodebase / SearchCode / CodeContext / etc. | The gRPC surface; add a session-delta path beside the commit path |

## The gap to build

The loop, end to end (each arrow is the work; bracketed items already exist):

- `[syn parse changed files]` -> per-file `NodeRecord`/`EdgeRecord` symbols + call/dep edges
- those records -> **`SessionDelta` (NEW converter)** against a base snapshot, including tombstone detection
- `[HarnessInstantKg::new(base, manifest, delta)]` -> merged session view
- `[harness_kg_* verbs]` -> served instantly, no full re-ingest
- (optional, additive) stack-graphs resolution between parse and records, for cross-file binding

## Architecture and seam decisions

- **Crate boundary (decided):** the converter lives in `apps/theorem-grpc/` (where `syn` already is) and emits `rustyred-thg-core`'s `SessionDelta`. Do **not** add `syn` to `rustyred-thg-core` - core stays parser/ML-framework-free, consistent with the substrate discipline. Put the converter in a **new** module `apps/theorem-grpc/src/session_delta.rs`, not inline in the Codex-active `code_index.rs`, to avoid hot-file collision; have it call `code_index.rs`'s existing extraction functions.
- **Delta source (decided):** compute the `SessionDelta` **directly from the parse against the base snapshot**, not via `diff_graph_snapshots`. The parser already produces the full records `SessionDelta` needs; the hash-only diff would force a re-fetch of records from the target snapshot. Keep `diff_graph_snapshots` for version-vs-version provenance, not as the delta source. Tombstone detection: symbols present in the base for a changed file but absent from the new parse become `tombstoned_object_ids`; call/dep edges present in base but not re-emitted become `removed_edge_ids`.
- **Overlay-before-commit (decided):** the loop produces an **uncommitted** `SessionDelta` overlaid on the base via `HarnessInstantKg` (the "session" view). It does not write to the store. The existing `code_index.rs` unconditional ingest stays as the separate "commit" path; the new loop is the "edit -> instant view" path. This matches what `HarnessInstantKg` already models.
- **stack-graphs (phased):** Phase 1 reuses the working `syn` name-match resolution (ship the loop without stack-graphs). Phase 2 adds stack-graphs for cross-file/import/shadowing binding. This keeps the biggest scope driver out of the critical path.
- **No third representation:** reuse `code_index.rs`'s `NodeRecord`/`EdgeRecord` symbol output. Do not fork a new code-graph schema alongside the existing `CodeSymbolRecord` (theorem-grpc) and `NodeRecord`-as-symbol (core).

## Slices (IK)

Each slice is independently shippable and validatable; IK-0..IK-3 deliver the loop on the existing `syn` parser with no new parser and no stack-graphs.

### IK-0 - SessionDelta-from-parse converter
New module `apps/theorem-grpc/src/session_delta.rs`. A function that runs the existing `syn` extraction over a set of changed files and assembles a `core::SessionDelta { objects, edges, tombstoned_object_ids, removed_edge_ids }` against a supplied base `GraphSnapshot`, instead of writing straight to the store. Reuses `code_index.rs`'s `rust_reference_index` / `RustCallCollector` / `RustTypeCollector`.
**Acceptance:** a fixture repo change set (one file added, one modified, one symbol deleted) yields the correct `SessionDelta`; unit-tested in the theorem-grpc crate; tombstones and removed edges are correct against the base.

### IK-1 - instant_kg feed + manifest
A constructor/entry that takes the code-encoder `SessionDelta` plus a `CodeKgManifest` carrying `commit_sha`, `encoder_version` (the `syn` parser version), and `encoded_files`, and builds the served session via `HarnessInstantKg::new`. If a new core entry point is needed it is a thin addition to `instant_kg.rs`; otherwise the existing `new` suffices and only the manifest construction is new.
**Acceptance:** `HarnessInstantKg` merges the parse delta over a base snapshot and serves `search` / `ppr` / `impact` / `explain_edge` over the session view; the manifest reports the real commit + encoder version.

### IK-2 - direct delta + tombstone semantics
Implement the direct-against-base delta computation chosen above (no `diff_graph_snapshots` dependency): re-parse the changed files, diff the per-file symbol/edge set against the base, and emit additions/modifications/tombstones/removed-edges. Dangling-edge cleanup follows the existing `HarnessInstantKg` merge rules.
**Acceptance:** deleting a symbol in a changed file tombstones it in the session view; a removed call edge is dropped; an added symbol appears; a renamed symbol shows as tombstone + add.

### IK-3 - the live loop (session reingest)
Wire IK-0..IK-2 to a session trigger: on a git working-set change (or an explicit reingest call), produce the `SessionDelta` from the parse and serve it through the existing `instant_kg_reingest` / `harness_kg_*` verbs, scoped to the session - feeding them from the parser rather than from request payloads. Add a CodeCrawlerService verb or reuse `RecordUseReceipt`/reingest as the entry.
**Acceptance:** editing a Rust file and reingesting updates the served instant KG (search/ppr/explain_edge reflect the edit) without a full codebase re-ingest; the base graph is untouched (overlay only).

### IK-4 (optional, additive) - stack-graphs semantic layer
Add cross-file name resolution so call/dependency edges bind to the correct definition across files, modules, and imports, replacing or augmenting the name-match in `infer_symbol_call_edges`. Use the `stack-graphs` crate; its grammars are tree-sitter-based, so this is also where tree-sitter enters - and only here, and only if multi-language parsing is also wanted.
**Acceptance:** a call to a function imported from another module resolves to that module's symbol, not a same-named local; shadowed names resolve to the in-scope binding. Profile-gated; off by default until proven.

## Coordination and hazards

- `apps/theorem-grpc/src/code_index.rs` and `rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs` are Codex-active hot files (the multi-source-search broker touches the MCP file; code_index is the code-search engine). IK-0 deliberately lands in a **new** `session_delta.rs` that *calls into* `code_index.rs` rather than editing it inline. Claim the seam in the coordination room before any edit that touches the hot files (IK-1's possible `instant_kg.rs` entry, IK-3's MCP/gRPC verb).
- Keep `rustyred-thg-core` free of `syn` and tree-sitter; the parser stays in `theorem-grpc`.
- This is the same duplicate-module risk the repo CLAUDE.md and the skill-encoder plan both flag: do not fork a third code-graph representation.

## Open questions (Travis / Codex)

1. **stack-graphs: hard requirement or Phase-2 enhancement?** The working `syn` layer already produces a usable call/dep graph by name-matching. Cross-file/import/shadowing binding is the value stack-graphs adds. This is the single biggest scope driver; the plan defaults it to IK-4 (additive, gated).
2. **Multi-language now, or Rust-only v1?** `syn` is Rust-only. tree-sitter (via stack-graphs grammars) is the multi-language path. Default: Rust-only for IK-0..IK-3.
3. **Overlay-only, or eventually commit through refs?** The plan keeps the session delta an in-memory overlay. If a session should be promotable to a committed graph version, that is an explicit IK-5 wiring `update_graph_ref` (out of scope here).
4. **Manifest provenance source:** `encoder_version` = the `syn` crate version + a converter schema version? `commit_sha` from `git rev-parse`? Decide the exact provenance the manifest records.

## Sequencing

IK-0 -> IK-1 -> IK-2 -> IK-3 ship the instant code-KG loop on the existing `syn` parser, no new parser and no stack-graphs. IK-4 (stack-graphs, and tree-sitter only if multi-language is required) is additive and profile-gated, taken on evidence. The loop is useful at IK-3; IK-4 makes its edges semantically precise.
