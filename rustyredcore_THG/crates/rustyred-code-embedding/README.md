# rustyred-code-embedding

Shared code-embedding seam for the file-write and CodeCrawler symbol paths: one `CodeEmbedder` trait with three backends, two of them feature-gated.

## Key API

- `CodeEmbedder: Send + Sync` (`embed_code(&str) -> Result<Vec<f32>, CodeEmbeddingError>`, `dimension()`, `name()`).
- `CodeEmbeddingConfig`: `hash(dim)`, `http(url, dim)`, `local(dim)`, `from_env_or_hash(default_hash_dim)`, `build() -> Arc<dyn CodeEmbedder>`.
- `CodeEmbeddingKind { Hash, Http, Local }`.
- Impls: `HashCodeEmbedder` (always built; FNV-1a token hash, L2-normalized), `HttpCodeEmbedder` (feature `http`), `LocalCodeEmbedder` (feature `local`).
- Free fns: `hash_code_embedding(text, dim)`, `cosine_similarity`, `extract_embedding_vector(&Value)` (parses `embedding` / `embeddings[0]` / `data[0].embedding`).
- `DEFAULT_REAL_CODE_EMBEDDING_DIM = 384`. Env: `RUSTYRED_CODE_EMBEDDER`, `RUSTYRED_CODE_EMBED_URL`, `RUSTYRED_CODE_EMBED_DIM`, `RUSTYRED_CODE_EMBED_TIMEOUT_SECONDS`, `RUSTYRED_CODE_EMBED_LOCAL_MODEL`.

## Features

`default = []`. `http` (blocking reqwest, rustls). `local` (Candle plus `BAAI/bge-small-en-v1.5`, D=384, CPU, CLS pooling plus L2; loaded from HF hub/cache, dimension-checked against the model config).

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-code-embedding
```

Default tests are offline (hash determinism, near-vs-far ordering, response-shape extraction, feature-gating). The `http` and `local` features each add one `#[ignore]` live test.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
