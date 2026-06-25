# rustyred-rerank

Reranker-backed `Scorer` implementations for membrane admission, including the lexical cross-encoder seam. v1 keeps model execution behind a `CrossEncoder`; the hot-path default is a single SequenceClassification forward pass.

## Key API

- `RerankScorer` (impl `rustyred_membrane::Scorer`), `ArmWeights` (`web()`/`code()`/`balanced()` presets), `ListwiseRankScorer`, `stamp_listwise_rank`.
- `cross_encoder`: `CrossEncoder` trait, `LexicalCrossEncoder`, `HttpCrossEncoder`, `SequenceClassificationModel`, `select_small_cpu_sequence_classifier`; listwise `ListwiseReranker`/`HttpListwiseReranker`/`NoopListwiseReranker`; benchmarking (`BenchmarkLedger`, `ModelBenchmark`); model-id consts `BGE_RERANKER_V2_M3`, `GTE_RERANKER_MODERNBERT_BASE`, `JINA_RERANKER_V3`.

Modules: `lib.rs`, `cross_encoder.rs`. Path dep: `rustyred-membrane`. The HTTP cross-encoder/listwise reranker uses blocking reqwest.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-rerank
```

`tests/ordering_quality.rs` plus inline tests. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
