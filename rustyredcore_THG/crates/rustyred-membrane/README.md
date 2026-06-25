# rustyred-membrane

The shared context admission and eviction membrane. The context window is treated as a cache over graph-resident nodes: callers generate candidates, a `Scorer` ranks them, and one gate admits a budgeted subset while converting overflow into recoverable handles.

## Key API

- Always available: `fill_to_budget(...) -> Admission`, `Admission`, `Handle` (`gate.rs`); `Candidate`, `Scorer`, `ScoreContext`, `EpistemicFeatures`, `SourceArm`, `DEFAULT_MMR_LAMBDA`, `DEFAULT_REDUNDANCY_PENALTY` (`scorer.rs`); `MembraneReceipt`, `Source` (`receipt.rs`); compaction-side eviction `page_back[_with_scorer]`, `CompactionFitnessScorer`, `CompactionWeights` (`compaction.rs`).
- Behind the `graph-store` feature (`recover.rs`): `admit_to_budget`, `context_fetch`, `emit_receipt`, `persist_deferred`, `DEFERRED_CONTEXT_LABEL`. The default build is pure cache mechanics with no graph dependency; the feature gates the lossless overflow recovery against `rustyred-thg-core`.

Admission and eviction share one gate; consumers include HippoRAG, RustyWeb, and fractal expansion.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-membrane
cargo test -p rustyred-membrane --features graph-store
```

Tests are inline (`gate`, `recover`, `compaction`). No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
