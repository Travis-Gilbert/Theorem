# RustyRed's vector index is TurboVec (`turbovec::IdMapIndex`), not HNSW — and a hardcoded `knn_strategy = "filtered_hnsw"` trace label is false on two counts

**Kind:** gotcha
**Captured:** 2026-06-20
**Session signature:** `claude-code:travisgilbert (review + fix multimodal-planner-unify)`
**Domain tags:** vector-search, turbovec, hnsw, trace-receipts, honest-labels, graph_store, rustyred-thg-core

## Trigger

`SPEC-MULTIMODAL-PLANNER-UNIFY` said "run filtered HNSW," and the implementation hardcoded `trace.knn_strategy = Some("filtered_hnsw")` chosen purely from a candidate-count threshold — with `knn_overfetch_rounds = max(1)` set cosmetically and NO actual traversal, adaptive `ef`, or overfetch loop behind it. RustyRed has no HNSW: `crates/rustyred-thg-core/src/graph_store.rs` backs `VectorIndex` with `turbovec::IdMapIndex` (`turbovec: Option<IdMapIndex>`, `VectorIndex::search` returning `Vec<(String, f32)>`), falling back to brute force for unsupported embedding dims. So the receipt named a nonexistent engine AND a mechanism that wasn't implemented — a doubly misleading label.

## Rule

Trace/receipt fields (`knn_strategy`, `score_source`, etc.) must name what ACTUALLY executed and the ACTUAL engine. Verify the engine by grepping the index type in the store (`grep -n turbovec crates/rustyred-thg-core/src/graph_store.rs`), never by trusting a spec's assumed name. When the executor (here a `ModalityResolver`) decides the strategy, have it RETURN the strategy + round count so the caller records the truth — never set a strategy label from a parameter/threshold the executor doesn't honor. Honest labels for this path: `exact_over_candidates` (scored C exactly), `filtered_overfetch` (approx top-k intersected with C, with the real round count), `index_topk` (no filter); and `score_source` `hop_distance` (not `ppr_or_hop_distance`) when hop distance is what ran.

## Evidence

- `crates/rustyred-thg-core/src/graph_store.rs`: `use turbovec::IdMapIndex;`, `struct VectorIndex { turbovec: Option<IdMapIndex>, ... }`, test `vector_index_uses_turbovec_for_supported_embedding_dimension`.
- The cosmetic label survived its own acceptance test because the test only asserted `knn_strategy == "filtered_hnsw"` for a large candidate set (it set the label by the same threshold), so it proved nothing about a real HNSW/overfetch path.
- After the fix the strategy + `knn_overfetch_rounds` come from the resolver's actual loop; the tightened AC drives a filter that removes the most-similar rows so overfetch genuinely iterates (asserts `>= 2` rounds).
