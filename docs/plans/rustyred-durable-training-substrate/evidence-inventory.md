# Evidence Inventory

**Date:** 2026-06-02

This inventory records what was visible from the local Theorem and Index-API
checkouts during the planning pass.

## User Decision

Travis clarified that RustyRed itself is a durable database when RedCore AOF is
enabled. The current direction is:

- Do not deploy Neo4j for this slice.
- Use RustyRed RedCore as Theorem's durable primary graph.
- Turn on AOF and require a persistent volume.
- Give RustyRed the Theseus S3 training artifacts, especially reasoning traces.
- Design explicit training snapshots and writeback contracts so GNNs,
  paraphramers, adapters, rerankers, and router models can train from the
  graph and write evaluated outputs back.

## Runtime Availability

| Probe | Result |
|---|---|
| `NEO4J_*` env keys | none visible in shell |
| `MEMGRAPH_*` env keys | none visible in shell |
| `THG_*` / `RUSTYRED_*` env keys | none visible in shell |
| local Bolt listener on `:7687` | none found |
| Docker daemon | unavailable from this shell |
| Theorem harness KG tenant `theorem` | zero objects, zero edges |
| Theorem context compiler | returned HTTP 500 in the earlier pass |

No live Neo4j or Memgraph inspection was possible from this environment. The
new plan no longer depends on deploying Neo4j.

## RustyRed Durability Evidence

| Path | Finding |
|---|---|
| `rustyredcore_THG/crates/rustyred-thg-core/src/graph_store.rs` | `RedCoreDurability` supports `none`, `aof_everysec`, `aof_always`, and `snapshot_only`. |
| `rustyredcore_THG/crates/rustyred-thg-core/src/graph_store.rs` | `RedCoreOptions::default()` uses `aof_everysec` and snapshots every 1,000 writes. |
| `rustyredcore_THG/crates/rustyred-thg-core/src/graph_store.rs` | `RedCoreGraphStore::commit_batch()` stages writes, persists before publish, then updates the in-memory graph. |
| `rustyredcore_THG/crates/rustyred-thg-core/src/graph_store.rs` | `graph_snapshot()` exposes the current graph view and `snapshot_now()` forces a snapshot boundary. |
| `rustyredcore_THG/crates/rustyred-thg-server/src/config.rs` | Default storage mode is embedded unless configured otherwise. |
| `rustyredcore_THG/crates/rustyred-thg-server/src/config.rs` | `RUSTY_RED_STRICT_ACID=true` requires embedded mode, `aof_always`, `single_writer`, and `serializable`. |

Working conclusion: RustyRed is sufficient as the durable graph substrate for
this slice if deployed in embedded RedCore mode with AOF and a mounted volume.

## Existing Training And Adapter Evidence

| Path | Finding |
|---|---|
| `rustyredcore_THG/crates/rustyred-thg-adapters/src/types.rs` | `LoraAdapter` records adapter id, tenant, base model SHA, S3 URI, training object ids, version, and fitness. |
| `rustyredcore_THG/crates/rustyred-thg-adapters/src/upsert.rs` | Adapter writeback creates `TRAINED_ON` edges to training objects and `DERIVED_FROM` lineage edges. |
| `rustyredcore_THG/crates/rustyred-thg-adapters/src/routing.rs` | Adapter routing can use PPR over training edges or embedding similarity. |
| `Index-API/apps/notebook/graph_kernel/kuzu_export/reasoning_traces.py` | Reasoning trace export bridge exists. |
| `Index-API/apps/notebook/graph_kernel/analytics_bridge/manifests.py` | Planned training sources include search sessions, reasoning traces, attention logs, ask feedback, training pairs, and privacy manifest. |
| `Index-API/docs/runtime/memgraph-training-exports.md` | Memgraph-backed graph training exports already exist in Theseus. |
| `Index-API/apps/notebook/management/commands/export_gnn_data.py` | Supports graph training export and S3 upload paths. |
| `Index-API/apps/notebook/management/commands/export_kge_triples.py` | Writes KGE triples and maps. |
| `Index-API/apps/notebook/management/commands/export_memgraph_arrow_training.py` | Streams graph data to Arrow/Parquet and optional TSV. |

Working conclusion: the training ecosystem exists, but its source graph should
be extended to RustyRed snapshots for this lane. The adapter catalog already
models trained artifacts as graph records, so GNN/paraphramer/reranker outputs
should follow that pattern.

## Experiential Memory Evidence

| Path | Finding |
|---|---|
| `Index-API/apps/notebook/models/learning.py` | `ImprovementEpisode` stores model trajectories, outcomes, reviewer status, and training disposition for future adapter training. |
| `Index-API/apps/orchestrate/models/postmortem.py` | `Postmortem` stores failure-derived lessons with origin patch/run links and tags. |
| `Index-API/apps/orchestrate/registry/learnings.py` | `Learning` captures durable rules, methods, postmortems, anti-patterns, and gotchas from real sessions. |
| `Index-API/mcp_server_theorem/tools/workspace.py` | `encode` saves feedback, solutions, and postmortems as typed memory with outcome metadata and graph fitness. |
| `Theorem/rustyredcore_THG/crates/theorem-harness-runtime/src/memory.rs` | Native memory accepts `encode`, `feedback`, `solution`, and `postmortem` kinds. |

Working conclusion: this is the corpus family that should become Theorem's
durable RustyRed graph content.

## Local `gnn_export/` Inventory

Base path:
`/Users/travisgilbert/Tech Dev Local/Creative/Website/Index-API/gnn_export`

Manifest:

- `schema_version`: `v1`
- `generator`: `export_gnn_data --include-features --upload`
- `exported_at`: `2026-04-25T03:02:24.311751+00:00`
- graph snapshot hash: `ac4a691900bdd954`
- object count: 144,275
- edge count: 275,515
- max object timestamp: `2026-04-22T16:07:40.900437+00:00`
- max edge timestamp: `2026-04-22T16:07:41.846164+00:00`

Key files:

| File | Shape or count |
|---|---:|
| `entity_map.tsv` | 144,275 rows |
| `triples.tsv` | 275,515 rows |
| `relation_map.tsv` | 27 rows |
| `node_features.npy` | 144,275 x 384 float32 |
| `edge_features.npy` | 275,515 x 71 float32 |
| `gnn_geomoe_embeddings.npz` | 144,275 x 128 embeddings |
| `community_labels.json` | graph labels |
| `contrastive_edges.json` | contrastive supervision |
| `typed_paths.npy` | 50,000 x 5 int64 |
| `path_masks.npy` | 50,000 x 5 float32 |
| `sha_to_object_id.json` | 144,275 keys |

Training metadata:

- model: `geomoe-rich-v1`
- architecture: `geomoe_mixed_curvature`
- embedding dimension: 128
- S3 key: `gnn-export/gnn_geomoe_embeddings.npz`

Working conclusion: this is a strong seed corpus for RustyRed artifact
registration and for validating the RustyRed training-export shape.

## Local `kge_embeddings/` Inventory

Base path:
`/Users/travisgilbert/Tech Dev Local/Creative/Website/Index-API/kge_embeddings`

Current metadata:

- `triple_count`: 52
- `entity_count`: 33
- `relation_count`: 10
- model: `RotatE`
- embedding dimension: 64
- trained at: `2026-03-10T02:08:10Z`

Mixed-size files also exist:

| File | Observation |
|---|---|
| `temporal_triples.tsv` | 152,258 lines |
| `temporal_profiles.json` | about 20 MB |
| `sha_to_object_id.json` | about 5.8 MB |

Working conclusion: treat `kge_embeddings/` as stale or mixed-provenance until
its metadata is reconciled with its larger files.

## Missing Pieces

- Production RustyRed import/registration for live S3 training artifacts.
- Production RustyRed-native reasoning trace normalization.
- Production RustyRed training snapshot exporter.
- Feature/label/split schemas for GNN, KGE, paraphramer, reranker, and router
  training.
- Trainer launch wiring from immutable RustyRed export manifests.
- Evaluated model writeback beyond the current LoRA adapter catalog.
- Promotion gates for active trained artifacts.

## Implemented Fixture Slice

| Path | Finding |
|---|---|
| `rustyredcore_THG/crates/rustyred-thg-adapters/src/training_substrate.rs` | Adds fixture import, training snapshot manifest export, model artifact writeback, training labels, and training edge constants over `GraphStore`. |
| `rustyredcore_THG/crates/rustyred-thg-adapters/src/tests/training_substrate_test.rs` | Proves RedCore `aof_always` persistence, `snapshot_now()`, reopen, export manifest counts, and fake paraphramer writeback. |
| `rustyredcore_THG/crates/rustyred-thg-adapters/src/lib.rs` | Re-exports the training-substrate API for downstream Theorem callers. |

Validation:

- `cargo test -p rustyred-thg-adapters`: 7 passed.
