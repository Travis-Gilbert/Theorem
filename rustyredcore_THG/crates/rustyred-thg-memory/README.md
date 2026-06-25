# rustyred-thg-memory

Graph-native memory over `GraphStore`: PPR-seeded recall under a token budget, consolidation into summary nodes, decay to archive, bitemporal validity and contradiction handling, and the cold-storage eviction/rehydration tier.

## Key API

- Operations: `recall(store, MemoryRecallInput) -> RankedMemories`, `consolidate(store, ConsolidateInput) -> ConsolidateOutput`, `decay(store, DecayInput) -> DecayOutput`.
- Validity / contradiction: `invalidate_memory_edge(store, edge_id, invalid_at_ms)`, `invalidate_on_contradiction(store, &EdgeRecord, &ContradictionPolicy)`, `recall_valid_time(store, RecallQuery)`. Types `ContradictionPolicy`, `Contradiction`, `RecallQuery`.
- Cold tier (the storage spine): `ColdTier` (`new(Box<dyn ColdObjectStore>, Box<dyn ColdIndex>)`, `in_memory()`), `evict_decayed(store, &mut ColdTier, DecayInput) -> EvictionReport`, `rehydrate(store, &mut ColdTier, id) -> bool`, `recall_with_cold_tier(...)`, `park_scope`/`unpark_scope`. Eviction reads the `EvictionFrontier` coldest-first (O(k log n), never the O(n) scan); residency changes are version-neutral, so the PPR cache stays warm. `evict_decayed` is the durable superset of `decay`.
- Plugin: `MemoryPlugin`, `builtin_memory_plugin_registry()` (registers `memory.recall` / `memory.consolidate` / `memory.decay`).
- Similarity (`similarity.rs`): `compute_memory_similarity_edges` writes `MEMORY_SIMILAR` edges via a `MemoryEmbedder` (deterministic `HashEmbedder` default; SBERT/GL-Fusion swap behind the trait).
- Anchor-id contract: `project_anchor_node_id`, `project_membership_edge_id` (byte-parity with `theorem-harness-runtime`, guarded by a cross-crate test).

Path dep: `rustyred-thg-core`. The eviction mechanism (`evict_node`/cold index/`EvictionFrontier`) lives in core; this crate owns the policy.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-memory
```

Tests: `tests/cut5_acceptance.rs`, `tests/similarity_acceptance.rs`, `tests/storage_spine_acceptance.rs`. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
