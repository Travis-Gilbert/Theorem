# W4: Real code embedding

The quality swap that makes the semantic overlay genuinely useful. The embedding in
`fs_write` is a deterministic placeholder; W4 replaces it with a real code encoder so
"find me similar" and "what is structurally like this" return real answers.

Dependency edges: **W4 sharpens W1** (same on-write hook, different embedding
function); it can land before or after W1, and is independent of W0/W2/W3. The seam
is already in place, so W4 is a focused swap, not a new layer.

## Thesis

Two placeholder embedders exist and both are stand-ins:

- `fs_write` files get a 16-dim hash embedding (`FILE_EMBEDDING_DIM = 16`,
  `apps/rustyred-embedded/src/lib.rs:386`).
- The code-symbol on-write hook embeds with a 64-dim FNV1a bag-of-tokens function
  (`incremental_embed_hook`, `rustyred-thg-code/src/code_embed_hook.rs:34`).

Both are deterministic and offline (good for tests and determinism), but neither
captures code semantics. W4 swaps the embedding function behind the existing
designate-vector seam; the graph node, the vector index, and the on-write hook are
all unchanged.

## What to build

- **Current green slice (W4.1): shared selectable seam.** `rustyred-code-embedding`
  now provides the shared `CodeEmbedder` contract and `CodeEmbeddingConfig`
  selector. `hash` remains the deterministic offline default; `http` is a
  feature-gated hosted-encoder path with mock endpoint coverage; `local` is a
  feature-gated Candle/BGE model path using `BAAI/bge-small-en-v1.5`.
- **An `Embedder` seam with selectable backends**, mirroring the pattern already used
  elsewhere in the tree (jobintel's `Embedder` has `hash` / `http` / `bge` variants;
  commonplace has a `DeterministicEmbedder` seam). For W4:
  - `hash` (the current placeholder, kept as the offline default and the test path).
  - `http` (a hosted code encoder, base-URL-swappable; the Theseus SBERT-style swap).
  - `local` (a feature-gated Candle encoder following the `bge` precedent in
    jobintel; defaults to `BAAI/bge-small-en-v1.5`, D=384, overridable with
    `RUSTYRED_CODE_EMBED_LOCAL_MODEL`).
- **Wire it into both write paths**: the `fs_write` file embedding and the code-symbol
  hook call the same `Embedder` seam, so a deployment chooses one encoder and both the
  file-level and symbol-level vectors are real. This is green for the shared seam:
  `rustyred-embedded` file nodes use the selected `CodeEmbedder`, and the
  CodeCrawler `incremental_embed_hook` plus instant epistemic warm-up use the same
  seam for `CodeSymbol` vectors.
- **Dimension as config, not a constant.** `FILE_EMBEDDING_DIM = 16` is hardcoded; W4
  now reads the dimension from the chosen encoder so the vector designation matches.
  The no-config hash defaults preserve the old 16-dim File and 64-dim CodeSymbol
  behavior; configured encoders can choose a different dimension.
- **Explicit re-embed batch for existing workspace files.**
  `Engine::reembed_files(prefix)` scans the durable DocTree subtree, recomputes
  `File.embedding` with the currently configured embedder, bulk-upserts File
  nodes, and does not fire post-file-write hooks. Re-designating a vector
  property with a new dimension now rebuilds the vector index so stale vectors
  from the old dimension are not searched.
- **A late-interaction multimodal path is a named follow-up, not W4.** The north star
  mentions a multimodal late-interaction path for visually-rich or non-code documents
  in the same tree; W4 scopes to code text. The multimodal path is its own unit when a
  visual model is in play (gated, like the multimodel Area D0/E2 units).

## Acceptance criteria

1. A test embeds two semantically similar code snippets and two dissimilar ones; the
   real encoder ranks the similar pair closer than the dissimilar pair (a property the
   hash placeholder fails). This is covered for the selectable seam by
   `configured_code_embedder_controls_symbol_dimension_and_similarity` with an
   injected semantic fixture embedder. The local BGE crate path also has an ignored
   live smoke (`local_bge_embedder_loads_and_prefers_related_code`) that downloads or
   loads the model and checks that ranking; that smoke is green in the Codex
   worktree.
2. The `Embedder` seam is selectable: the default offline `hash` path keeps every
   existing test green (no behavior change when the real encoder is not configured),
   and the `http` path is exercised against a mock endpoint behind
   `--features http`. The shared crate also carries an ignored hosted-endpoint
   oracle, `live_http_embedder_endpoint_prefers_related_code`, gated on
   `RUSTYRED_CODE_EMBED_URL` and optional `RUSTYRED_CODE_EMBED_DIM` /
   `RUSTYRED_CODE_EMBED_TIMEOUT_SECONDS`. The `local` Candle path is implemented behind
   `--features local` and stays opt-in so default builds do not pull Candle or model
   weights.
3. The vector designation dimension follows the chosen encoder (a non-16-dim encoder
   produces a correctly-dimensioned designation and `vectorSearch` returns it). Green
   for File vectors through `engine_file_embedding_dimension_follows_configured_embedder`
   and for CodeSymbol vectors through
   `configured_code_embedder_controls_symbol_dimension_and_similarity`.
4. `cargo check --features <encoder>` green; the default build does not pull the heavy
   model dependency (feature-gated); changed files clippy-clean. Green for the HTTP
   feature path in the shared crate and both consumers; green for the local feature
   path in the shared crate plus the `rustyred-embedded` and `rustyred-thg-code`
   feature passthroughs.

## Still open

- Live `http`: run `live_http_embedder_endpoint_prefers_related_code` against an
  actual hosted endpoint, not only the mock contract. The ignored oracle compiles;
  the live endpoint proof remains gated on `RUSTYRED_CODE_EMBED_URL`.

## Divergences and risks to surface (not bury)

- **Determinism vs quality trade-off.** The hash embedder is deterministic and makes
  byte-parity tests possible; a real encoder is not byte-stable across versions. Keep
  `hash` as the test/offline default so the test suite stays deterministic, and gate
  the real encoder behind config/feature so CI does not depend on a model.
- **Re-embedding the existing corpus.** Swapping the encoder invalidates existing
  vectors. A re-embed pass over an already-imported tree is needed when the encoder
  changes; make it an explicit batch operation (reuse the W0 batch path), not an
  implicit per-write migration.
- **The seam already exists in sibling crates.** Do not invent a new `Embedder`
  abstraction; lift the established `hash`/`http`/`bge`-style seam (jobintel,
  commonplace) so the pattern stays one shape across the workspace.
