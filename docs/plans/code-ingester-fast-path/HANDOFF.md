# Code Ingester Fast-Path Handoff

Execution register. Read CONVENTIONS.md first. Named choices here are requirements, not examples. The spec fixes load-bearing decisions and observable acceptance criteria; how you implement within them is yours. Where the current source contradicts this spec, flag it in the PR rather than silently working around it. Do not downgrade scope, do not manufacture blockers, and verify end to end before calling anything done.

## Grounding

Verified against `Travis-Gilbert/Theorem` on 2026-06-08 by reading source:

- `apps/theorem-grpc/src/code_service.rs` (SHA 8f6b4220): the gRPC `CodeCrawlerService`. `ingest_codebase` is a blocking unary call; request carries `repo_path`, not `repo_url`.
- `apps/theorem-grpc/src/code_index.rs` (SHA a072fb48): 235-byte shim, re-exports `rustyred_thg_code`. Parsing and graph ops now live in the crate so adapters can share one tenant-store implementation.
- `rustyredcore_THG/crates/rustyred-thg-code/src/lib.rs` (SHA 7640075c): the runtime, plugin, parser, and the prepare/commit pipeline.
- `rustyredcore_THG/crates/rustyred-thg-code/src/repo_fetch.rs` (SHA 8b93a771): CA-1 URL to local clone.

Builds on the code-graph plugin design (harness memory `doc_d4eaca42f004d8ce`) and the resolved decision that `theorem-grpc` is the reference worker writing into the THG tenant substrate.

## Two corrections to prior framing, do not re-derive the old version

- There is no embedding step in the ingest path today. Search is purely lexical (`score_hit` over symbol nodes loaded into memory). Embeddings are a later slice.
- The second-store split is largely resolved. Parsing moved into `rustyred-thg-code`, and a `CodeParsingPlugin` writes into `context.store`. The one residue to verify is D6.

## Diagnosis, ranked by cost

1. **Quadratic call-edge inference.** `infer_symbol_call_edges` in `lib.rs` has two branches. Rust symbols take a real `syn` AST path (`rust_reference_index`). Every non-Rust symbol takes the `else` branch: for each symbol it iterates every distinct symbol name in the repo and calls `body_references_name(symbol.body, name)`. Cost scales as `symbols x distinct_names x body_length`. A repo with a large TS/JS/Python surface puts most symbols on this branch. This is the dominant cost.

2. **Edge fan-out explosion.** `push_symbol_edges` emits an edge to every symbol that shares a name across the whole repo. A common name (`new`, `build`, `handle`, `get`) links to every match in the repo. This is both slow and a graph-quality problem (noise edges).

3. **Full file text stored in the graph.** `build_code_mutations` writes `"text": file.text` on every `CodeFile` node, up to `max_file_bytes` (1MB) x `max_files` (2500), inside a single `commit_batch`. Inflates commit size, store memory, and every node load.

4. **Synchronous unary RPC on one store lock.** `CodeIndexRuntime` holds `Arc<Mutex<RedCoreGraphStore>>` for the entire parse and commit. The harness MCP deadline is 120s. The heavy path blows the client deadline and blocks all other ops on that store for the duration. This is why a follow-up `search` on the same tenant also timed out: it waited on the same lock while the first ingest kept running server-side.

5. **Deadline mismatch.** `repo_fetch` caps clone at 20s, the `theorem-grpc` comment assumes a 30s deadline, the harness MCP enforces 120s.

## Not the cause

- The clone is not the bottleneck. CA-1 (`repo_fetch.rs`) already shallow-clones with `--depth 1 --single-branch --no-tags` into a quarantined tempdir, caps at 512 MiB and 20s, never executes the tree, and drops it after parse.
- No embedding step exists in ingest.
- gRPC is the right transport, with the wrong request shape.

## Deliverables

Each deliverable: the file, the change, the observable acceptance criterion.

### D1. Async ingest: submit plus stream

`apps/theorem-grpc/src/code_service.rs`, the proto, and `rustyred-thg-code`.

Change `ingest_codebase` and `reindex_codebase` from a blocking unary that returns the result, to a job submission that returns a `job_id` immediately, plus a server-streaming `WatchIngest(job_id) -> stream IngestEvent`. A poll variant `GetIngestStatus(job_id) -> IngestStatus` covers clients that do not stream.

Run the heavy work (clone, walk, parse, mutation build) on a worker task off the request path. Hold the store `Mutex` only for the final `commit_batch`, not for parse.

`IngestEvent` variants carry: `clone_done{ms}`, `walk_done{files_found}`, `parse_progress{done, total}`, `commit_done{graph_version}`, `finished{IngestCodebaseOutput}`, `failed{code, message}`. The existing `IngestStageTimings` rides on the final event.

Acceptance: submitting a 2500-file repo returns a `job_id` in under one second. A concurrent `search` on the same tenant returns during the parse rather than blocking. The stream emits stage events in order and ends with a `finished` or `failed` event carrying stage timings.

### D2. Inverted-index edge inference, with fan-out caps

`rustyred-thg-code/src/lib.rs`, `infer_symbol_call_edges` and `push_symbol_edges`. Highest leverage.

Replace the non-parser-backed branch. Tokenize each symbol body once into a set of identifier tokens. For each token that is also a known symbol name (lookup in the existing `symbols_by_name` map), emit a `CALLS_SYMBOL` edge. This turns `symbols x distinct_names x body_length` into `symbols x body_tokens`.

Add fan-out control so common names do not explode the graph: skip names whose `symbols_by_name` bucket exceeds a frequency threshold (a name that resolves to dozens of symbols is not a useful edge), and cap targets-per-name. Keep the Rust `syn` path unchanged.

Acceptance: edge-inference time on a repo with roughly 20k symbols drops from minutes to sub-second, shown by the `mutation_ms` stage timing on a fixture. Edge counts on a fixture are a superset of the old output minus substring false positives (token matching is stricter than `body_references_name`). No name resolves to more targets than the cap.

### D3. File text out of the hot graph

`rustyred-thg-code/src/lib.rs`, `build_code_mutations` and `code_context_with_store`.

Stop writing `"text": file.text` onto the `CodeFile` node. Store file contents in a content-addressed side record keyed by the existing `content_hash` (a separate label or a separate store column not loaded by symbol or file node queries). `code_context_with_store` reads context from that side record by hash instead of from the file node.

Acceptance: `search` and `explore` node loads no longer deserialize file text. The `commit_batch` byte size on a fixture drops materially. `code_context` still returns correct line windows.

### D4. Incremental reindex via content_hash

`rustyred-thg-code/src/lib.rs`, the reindex path, and the seam already described in the `build_code_mutations` doc comment.

`reindex` currently re-parses every file. Use the `content_hash` already computed per file: load the prior generation's file hashes, skip unchanged files (carry their symbol nodes forward to the new generation), parse only changed or new files, tombstone removed files. Keep the mutation sequence compatible with `session_delta.rs` so a single-file edit produces an overlay `SessionDelta` rather than a full commit.

Acceptance: reindex of an unchanged repo parses zero files and completes in under one second. Reindex after editing one file parses exactly one file. The instant-KG delta path and full ingest share the same mutation builder.

### D5. Parallel, gitignore-aware walk

`rustyred-thg-code/src/lib.rs`, `collect_code_file_candidates`.

Replace the manual recursive `fs::read_dir` walk with the `ignore` crate's parallel `WalkBuilder`. It respects `.gitignore` and `.ignore` for free (so `node_modules`, `target`, `dist` drop without a hardcoded list) and parallelizes the walk. Keep the extension allowlist, the size cap, and the binary sniff.

Acceptance: the walk respects the repo's `.gitignore`. `walk_ms` drops on a large tree. The candidate set excludes gitignored paths.

### D6. Finish the gRPC to tenant-store route, verify first

`apps/theorem-grpc/src/main.rs`, `engine.rs`, `code_service.rs`.

`CodeIndexRuntime::try_new` opens its own RedCore at `code_index_data_dir()`. Confirm whether the MCP `compute_code` ingest writes into the THG tenant substrate or into this private store. If private, route the gRPC adapter through the `CodeParsingPlugin` path so ingest writes into the tenant `RedCoreGraphStore` that `search` and `explore` read.

Acceptance: after an ingest, `compute_code search` over the same tenant returns hits. The store that ingest writes and the store that search reads are the same store.

### D7. Deadline alignment, deadline-free heavy path

`repo_fetch.rs`, `theorem-grpc` service config, harness MCP `compute_code`.

With D1 the heavy path has no client deadline. Keep the 20s clone cap. Add a server-side parse budget that commits partial progress and returns a clear `budget_exceeded` status with counts rather than a transport `Cancelled`. Reconcile the documented 30s comment with the 120s MCP timeout.

Acceptance: a large repo no longer surfaces a transport `Cancelled / Timeout expired`. It streams progress and ends with `finished` or a `budget_exceeded` status carrying partial counts.

## Future slices, not this handoff

- tree-sitter and tree-sitter-graph per-language encoders retire the line-regex symbol extraction and the heuristic edge inference, and add scoped cross-file name resolution. D2's inverted index is the bridge until this lands. The line-regex parser misses arrow functions, methods, and exports.
- Embeddings: a background pass on Modal GPU using Qwen3-Embedding-4B at 2560-dim, `vector_designate(CodeSymbol.embedding)`, never inline in the ingest RPC. `code_map` becomes KNN-seeded PPR.
- SCIP-style monikers for node IDs so cross-repo `CALLS_SYMBOL` edges resolve.
- Versioned snapshot per commit SHA via `versioned_graph`.

## Out of scope, do not regress

- One store: the THG tenant substrate. No return to a second private code store.
- The schema: `CodeRepository`, `CodeFile`, `CodeSymbol`, and the `CONTAINS_FILE`, `DECLARES_SYMBOL`, `CALLS_SYMBOL`, `DEPENDS_ON_SYMBOL` edge types.
- Clone-safety invariants: shallow clone, size cap, parse-only never execute, per-repo quarantine, drop after parse.
