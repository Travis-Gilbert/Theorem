# rustyred-thg-ml

Shared graph-tensor and message-passing primitives for the THG learned organs, plus a ColPali-style multi-vector retrieval tier.

## Key API

- Core (`lib.rs`): `GraphTensorBatch`, `ScatterAggregationPath` (`BurnScatterAdd` / `FixedPointAtomicCompatible` / `NativeFloatAtomicFastPath`), `choose_scatter_aggregation_path`, `MessageAggregator`, `FixedPointAggregator`, `aggregate_messages_fixed_point`, `validate_messages`. Feature `burn` adds `aggregate_messages_burn`, `BurnAggregator`.
- Multi-vector (`multivector.rs`): `MultiVectorEmbeddingSet`, `BinaryMultiVectorSet`, `MaxSimScorer`, `exact_maxsim_score`, `binary_hamming_maxsim_score`, `quantize_sign_bits`, `rank_exact_maxsim`, `recall_against_exact_top_k`, `storage_costs`.
- Producer (`producer.rs`): `MultiVectorProducer`, `HashingMultiVectorProducer` (deterministic default), `project_multivector_tiers`. Feature `colpali-candle` adds `CandleColPaliProducer` (real model load).
- Benchmark (`benchmark.rs`): `run_fixture_benchmark`.

Path dep: `rustyred-thg-core`.

## Features

`default = []`. `burn` (`dep:burn`), `cubecl` (`burn` plus `dep:cubecl`), `colpali-candle` (Candle plus tokenizers/hf-hub/image).

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-ml
```

Tests are inline (`lib`, `multivector`, `producer`, `benchmark`). No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
