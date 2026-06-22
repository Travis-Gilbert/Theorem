# W1: CodeCrawler-on-write + incremental edges

The layer that collapses the agent's bookkeeping tool calls. A file write inside
the engine maintains the code-graph symbols, call/dependency edges, and the code
embedding as a side effect, so the agent never calls "reindex" or "update the
graph." Only the genuine semantic query (what calls this, find similar) stays MCP.

Dependency edges: **W0 precedes W1** (on-write maintenance needs files-as-nodes
already landing via the import path and the workspace seam to reach the store).
**W4 sharpens W1** (real encoder swaps the placeholder embedder; the hook is the
same). W1 is independent of W2 and W3.

## Thesis

Per-file symbol indexing on write is nearly free, because the file is already a
graph node and the encoder is per-file. The expensive part is the call-edge layer:
`infer_symbol_call_edges` is a whole-graph name-match pass (confirmed by reading
source), so a single-file edit cannot just re-run it. W1's real work is the
incremental-edge strategy. Everything else (symbol extraction on write, embedding
on write) reuses hooks that already exist behind a flag.

## What already exists (reuse, do not rebuild)

- **The reactive substrate.** Post-commit graph hooks already exist
  (`rustyred-thg-core/src/hooks.rs`) and code ships two handlers behind
  `THEOREM_CODE_HOOKS`:
  - `incremental_centrality_hook` at
    `rustyredcore_THG/crates/rustyred-thg-code/src/code_hooks.rs:55`: bounded-BFS
    localized PPR (`MAX_NEIGHBORHOOD=5000`, `EXPANSION_DEPTH=2`) warming a
    `centrality` property on reachable symbols, idempotent under an epsilon, fired on
    `CodeSymbol` upsert or `CALLS`/`DEPENDS`/`DECLARES` edge change.
  - `incremental_embed_hook` at
    `rustyredcore_THG/crates/rustyred-thg-code/src/code_embed_hook.rs:34`:
    deterministic 64-dim FNV1a bag-of-tokens embedding on `signature`/`snippet`/
    `search_text` change, idempotent under an epsilon.
  These are the model for the on-write maintenance pattern, already wired into
  `start_code_kg_dispatcher`.
- **The per-file encoder.** `indexed_file_from_loaded(config, loaded) -> IndexedFile`
  at `rustyred-thg-code/src/lib.rs:3628`. For Rust it overlays `rust_reference_index`
  (`:4136`, a `syn` AST walk populating `call_names`/`dependency_names` on
  `IndexedSymbol`, `:2213`); for other languages it falls back to a line-symbol token
  encoder with `parser_backed=false`.
- **The edge inference and its caps.** `infer_symbol_call_edges(files, config)`
  at `:3836` builds `symbols_by_name: HashMap<name, Vec<EdgeTargetRef>>` from all
  files, then per symbol matches its observed names (parser call/dep names, or body
  tokens) and calls `push_symbol_edges` (`:3924`), which enforces
  `EDGE_NAME_BUCKET_CAP = 24` (reject names with too many targets, the common-name
  guard) and `EDGE_TARGETS_PER_NAME_CAP = 8` (fan-out cap), dedupes by `symbol_id`,
  and forbids self-loops.
- **Incremental reindex (D4).** A content-hash SPLIT already skips unchanged files
  and reconstructs carried `IndexedFile`s, then infers edges over fresh+carried so
  the edge graph is generation-identical to a full ingest. W1 reuses this machinery;
  the gap is that it still re-infers over the whole carried set, not just the touched
  name buckets.

## The gap W1 closes (the real algorithm)

`infer_symbol_call_edges` rebuilds `symbols_by_name` from scratch and re-infers all
edges on every ingest. A single changed symbol forces a re-scan of all names and all
bodies. Automatic at the symbol level is easy (the encoder is per-file); automatic
at the edge level needs an incremental strategy.

The strategy:

1. **Persist the name-bucket index in the store.** Today `symbols_by_name` is an
   ephemeral `HashMap` rebuilt per ingest. W1 materializes it as a queryable inverse
   index `name -> [symbol_id, file_path, line]` in the graph (a `SymbolName` node, or
   a side index keyed by name), updated incrementally on each symbol upsert.
2. **Recompute edges only for the touched buckets.** On a single-file edit producing
   a changed symbol set `C` and a set of newly-observed names `N`:
   - For each symbol in `C`: recompute its outgoing edges by looking up its observed
     names against the persistent index (O(names-in-C) lookups, not O(all-symbols)).
   - For each name in `N`: recompute incoming edges from any symbol whose body/parser
     names reference it (the reverse direction), bounded by the same caps.
   - Leave all other edges untouched. Upsert handles idempotency for re-emitted
     edges within a bucket.
3. **Wire it as a post-commit hook**, the same shape as `incremental_centrality_hook`:
   a `CodeSymbol` upsert (from `fs_write` of a source file) fires the
   incremental-edge handler, which reads the persistent name index and rewrites only
   the affected buckets. Coalesce per dispatch group so a batch import fires once.

This is the only new algorithm in the whole plan. It must produce an edge graph
identical to a full `infer_symbol_call_edges` run over the same tree (the D4
generation-identity property, extended to single-file granularity).

## What to build

- **W1.1: on-write symbol extraction.** A post-commit hook (or extension of the
  existing code-KG dispatcher) that, on a `File` node write of a source file, runs
  `indexed_file_from_loaded` for just that file and upserts its `CodeSymbol` nodes.
  Reuse the source/artifact filter from W0 so a `target/` write never triggers it.
- **W1.2: persistent name-bucket index.** Materialize `name -> [symbol refs]` in the
  store, maintained on `CodeSymbol` upsert/delete. This is the data structure the
  incremental edge pass queries.
- **W1.3: incremental edge handler.** The hook that recomputes only the touched
  buckets per W1's strategy, fired on symbol change, coalesced per batch.
- **W1.4: embedding on write.** Wire `incremental_embed_hook` (already built) into
  the on-write path so a source-file edit refreshes the code embedding. (W4 swaps the
  64-dim FNV1a function for a real encoder; W1 just connects the existing hook.)
- **W1.5: the tool-call collapse measurement.** A test/bench that runs a fixed
  editing task (open a repo, edit K files, query "what calls X" M times) two ways:
  the imperative MCP path (read/write/reindex/update-graph as explicit tool calls)
  and the on-write path (fs writes, maintenance as side effect, only the queries as
  MCP), and reports the round-trip and (proxy) token delta. This is the thesis made
  measurable, and it is an acceptance criterion, not a slogan.

## Acceptance criteria

1. A Rust test imports a fixture repo (via W0), edits a single source file that adds a
   call to an existing symbol, and asserts the new `CALLS` edge appears, the unrelated
   edges are untouched, and the resulting edge set is byte-identical to a full
   `infer_symbol_call_edges` run over the post-edit tree (single-file generation
   identity).
2. The incremental edge pass touches only the affected name buckets: a counter on
   bucket recomputation shows O(names-in-changed-file), not O(all-names-in-repo), for
   a single-file edit in a multi-hundred-symbol fixture.
3. Editing a source file refreshes its code embedding via the on-write hook (the
   `embedding` property changes; idempotent on a whitespace-only edit that does not
   change tokens).
4. The collapse measurement (W1.5) runs green and reports a strictly lower MCP
   round-trip count for the on-write path on the fixed task; the number is recorded in
   the unit notes (the north star's "fewer round trips" claim, proven on a fixture,
   not a deployment).
5. `cargo test -p rustyred-thg-code` green; the on-write path is off by default behind
   the existing `THEOREM_CODE_HOOKS` flag (additive, no behavior change when off);
   changed files clippy-clean.

## Divergences and risks to surface (not bury)

- **The cap is per-generation global.** `EDGE_NAME_BUCKET_CAP = 24` rejects a name's
  edges across the whole graph if it has too many targets. In a growing monorepo, a
  legitimate name (e.g. `Handler` with 50 impls) gets dropped. The incremental index
  inherits this; if it bites, per-module/per-namespace scoping is a named follow-up,
  not part of W1.
- **Parser fallback is silent.** `rust_reference_index` returns an empty map on a
  `syn` parse error, so a syntactically-broken file (common mid-edit) falls back to
  body tokenization with no error surfaced. On-write over a half-typed file must not
  poison the index; the hook should treat a parse failure as "keep the prior symbols"
  rather than wiping them. The tree-sitter parsing pass (separate spec) raises parse
  quality feeding all of this.
- **High-churn symbols re-fire PPR.** `incremental_centrality_hook` fires on every
  call-edge change; a hot symbol (called by many) re-triggers localized PPR per edit.
  Coalescing per dispatch group helps; a cross-generation centrality cache is a named
  follow-up.
- **D4 reconstruction cost.** Carried-file reconstruction queries `CodeFileText` per
  file; a 10k-file repo with 1k carried files is 1k store reads per reindex. The
  incremental path should avoid full reconstruction for a single-file edit (that is
  the whole point), reading only the changed file plus the touched buckets.
