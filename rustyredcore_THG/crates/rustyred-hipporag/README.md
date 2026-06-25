# rustyred-hipporag

HippoRAG 2 candidate generation with graph-native RAPTOR hubs. A retrieval layer: it indexes passages/phrases into the THG graph, builds first-class `Hub` nodes from graph communities, and returns `rustyred_membrane::Candidate` values for the shared membrane gate. Reranking and admission live in the sibling crates.

## Key API

- Indexing: `index_passage(&mut S, passage_id) -> IndexStats`, `index_passage_with_embedder` (async).
- RAPTOR hubs: `build_summary_tree_for_region`, `build_summary_tree_with_embedder`, `summary_tree_hook`, `SummaryTreeHooksPlugin`, `RaptorPolicy`.
- Retrieval: `retrieve(&S, HippoQuery) -> Vec<Candidate>`, `retrieve_with_trace`, `retrieve_with_embedder`, `retrieve_with_query_vector`. `HippoQuery<'a>` (`text`, `top_k`, `include_hubs`), `RetrievalTrace`. Uses `rustyred_thg_core::personalized_pagerank` seeded from matched phrase/hub nodes (query-specific PPR; the trace flags `ran_query_ppr` / `ran_global_ppr` / `warm_centrality_reads`).
- Embedding seam: `HippoTextEmbedder`.
- Schema: `HubNode`, `PhraseNode`, labels `LABEL_PAGE`/`LABEL_PHRASE`/`LABEL_HUB`, edges `CONTAINS`/`RELATES`/`SYNONYM`/`SUMMARIZES`/`HUB_PARENT`.

Modules: `schema`, `indexing`, `raptor`, `retrieve`, `embedding`. Path deps: `rustyred-membrane`, `rustyred-thg-core`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-hipporag
```

`tests/spec_acceptance.rs` plus inline tests. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
