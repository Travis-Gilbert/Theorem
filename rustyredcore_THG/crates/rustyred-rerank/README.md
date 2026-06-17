# rustyred-rerank

Reranker scorer implementations for RustyRed membrane admission.

## What it is

Reranker-backed [`Scorer`](rustyred_membrane::Scorer) implementations.

v1 keeps model execution behind [`CrossEncoder`]. The hot-path default is a
SequenceClassification-style single forward pass. Causal-LM rerankers can be
benchmarked elsewhere, but they do not sit on the default gate path.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-rerank
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
