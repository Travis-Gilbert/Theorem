# Grounded Skill API plus Native Fractal Expansion

Status: first Rust slice implemented.

## Decision

Build these specs as one loop with two surfaces:

- Grounded Skill API: packages already-grounded encoder output as an open Agent Skills folder.
- Native fractal expansion: grows the corpus in Rust by exhausting graph frontier search, reaching the web, and ingesting admitted web state as lower-trust graph records.

The default embedder model for this slice is `qwen3-embedding-8b`.

## Shipped Slice

- `rustyred-thg-adapters::grounded_skill` defines the portable skill folder contract: `SKILL.md`, executable script file, and `theorem.provenance.json`.
- `rustyred-thg-fractal` is a new workspace crate. Its fixture runner composes `rustyred-web` crawl output with `rustyred-thg-core` graph writes.
- Fractal expansion has no graph-only terminal state. If no web seed can be derived or supplied, the run errors with `fractal_web_seed_required`.
- Admitted web graph records are annotated with `trust_tier = open_web_unverified`, `quarantine = true`, a confidence ceiling, tenant id, run id, and embedder model.
- Graph-derived frontier seeds are tenant-scoped; one tenant's substrate pages cannot seed another tenant's web expansion.

## Next Implementation Edges

- Replace fixture web pages with the live RustyWeb reach path.
- Add cross-encoder rerank admission before ingest.
- Expose `rustyred-thg-fractal` through `rustyred-thg-mcp`.
- Add the API/SPA route that calls the grounded skill builder after retrieval and encoding.
- Backfill semantic embeddings into the graph using the 8b embedder, then register HNSW vector views for the skill and fractal corpora.

## Validation

- `cargo test -p rustyred-thg-adapters -p rustyred-thg-fractal`
- `git diff --check`
