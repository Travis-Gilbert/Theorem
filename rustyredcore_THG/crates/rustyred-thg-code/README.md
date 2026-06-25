# rustyred-thg-code

Transport-independent code parsing runtime: parse source repos into code graph records (symbols, files, calls, deps) written into the caller's `RedCoreGraphStore`, with on-write hooks (centrality, embeddings, epistemic), a context-pack membrane, and the code-KG surfaces. Independent of tonic/MCP/HTTP, so every transport calls the same parser.

## Key API

- Ingest/index: `ingest_codebase_in_store`, `reindex_codebase_in_store`, `ingest_codebase_from_url_in_store`, `index_source_file_write_in_store` (the on-write lane), `build_code_mutations`, `collect_code_files`. `IngestCodebaseInput` carries the opt-in `materialize_symbol_name_index` flag.
- Records: `CodeSymbolRecord`, `CodeHitRecord`, `CodeGraphEdgeRecord`.
- Search/list: `search_code_in_store`, `list_repos_in_store`.
- Plugin: `CodeParsingPlugin` (impl `RustyRedPlugin`; ingest/reindex/search/context_pack/recognize/explore/explain), `builtin_code_plugin_registry`, `start_code_kg_dispatcher`.
- Labels/edges: `CODE_REPO_LABEL`/`CODE_FILE_LABEL`/`CODE_SYMBOL_LABEL`/`CODE_SYMBOL_NAME_LABEL`; `CONTAINS_FILE`/`DECLARES_SYMBOL`/`CALLS_SYMBOL`/`DEPENDS_ON_SYMBOL`/`SYMBOL_NAME_TARGET`.

## Modules

- `code_hooks`: graph mutation hooks (incremental centrality; auto-wired via `THEOREM_CODE_HOOKS`).
- `code_embed_hook`: keep `CodeSymbol` embeddings fresh through the `rustyred-code-embedding` seam.
- `code_epistemic_hook`: instant structural epistemic pass (drift, contradiction).
- `context_pack`: code-arm membrane (rank by warm centrality plus task PPR, admit to budget, persist overflow).
- `ensure`: SHA-keyed idempotent repo entry. `ingest_jobs`: async ingest jobs with event stream (in-memory). `map_projection`: PPR-ranked codebase map plus markdown. `repo_fetch`: shallow clone to quarantined temp dirs, size/time-capped, never executes cloned code.

Path deps: `rustyred-code-embedding`, `rustyred-membrane` (feature `graph-store`), `rustyred-rerank`, `rustyred-thg-core`. Other: `ignore`, `rayon`, `syn`. Features `code-embedding-http` and `code-embedding-local` forward to the embedding crate.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-code
```

Tests: `tests/code_kg_hooks.rs`; `tests/centrality_latency.rs` is a `#[ignore]` timing benchmark.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
