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

- **An `Embedder` seam with selectable backends**, mirroring the pattern already used
  elsewhere in the tree (jobintel's `Embedder` has `hash` / `http` / `bge` variants;
  commonplace has a `DeterministicEmbedder` seam). For W4:
  - `hash` (the current placeholder, kept as the offline default and the test path).
  - `http` (a hosted code encoder, base-URL-swappable; the Theseus SBERT-style swap).
  - `local` (a feature-gated candle code encoder, e.g. a code-specialized model,
    following the `bge` feature-gate precedent in jobintel).
- **Wire it into both write paths**: the `fs_write` file embedding and the code-symbol
  hook call the same `Embedder` seam, so a deployment chooses one encoder and both the
  file-level and symbol-level vectors are real.
- **Dimension as config, not a constant.** `FILE_EMBEDDING_DIM = 16` is hardcoded; W4
  reads the dimension from the chosen encoder so the vector designation matches.
- **A late-interaction multimodal path is a named follow-up, not W4.** The north star
  mentions a multimodal late-interaction path for visually-rich or non-code documents
  in the same tree; W4 scopes to code text. The multimodal path is its own unit when a
  visual model is in play (gated, like the multimodel Area D0/E2 units).

## Acceptance criteria

1. A test embeds two semantically similar code snippets and two dissimilar ones; the
   real encoder ranks the similar pair closer than the dissimilar pair (a property the
   hash placeholder fails). This is the concrete "the overlay is now useful" proof.
2. The `Embedder` seam is selectable: the default offline `hash` path keeps every
   existing test green (no behavior change when the real encoder is not configured),
   and the `http`/`local` paths are exercised against a mock/feature-gate.
3. The vector designation dimension follows the chosen encoder (a non-16-dim encoder
   produces a correctly-dimensioned designation and `vectorSearch` returns it).
4. `cargo check --features <encoder>` green; the default build does not pull the heavy
   model dependency (feature-gated); changed files clippy-clean.

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
