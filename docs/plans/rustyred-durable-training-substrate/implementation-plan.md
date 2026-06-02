# RustyRed Durable Training Substrate

**Status:** planning slice from local repo audit and Travis correction on
2026-06-02.

**Decision:** Do not deploy Neo4j for this slice. Theorem should use RustyRed
RedCore as the durable primary graph, with AOF enabled and a persistent volume.
Theseus S3 training artifacts and reasoning traces should be imported or
registered directly into RustyRed, then RustyRed should provide immutable
training snapshots and receive trained artifacts back as graph records.

**Implementation slice landed:** `rustyred-thg-adapters` now exposes a
training-substrate proof surface:

- `register_training_fixture`: writes a tenant, three objects, two reasoning
  traces, trace steps, one postmortem, one S3 artifact, one training pack, one
  paraphrase pair, one GNN export node, and one LoRA adapter into a `GraphStore`.
- `export_training_snapshot`: emits a deterministic manifest from a frozen
  `GraphSnapshot`, including graph version, snapshot hash, counts, selected
  labels, edge types, privacy tiers, and fixture feature schema.
- `register_model_artifact`: writes a trained model artifact, evaluation
  receipt, `TRAINED_ON`, `EVALUATED_BY`, and optional `PROMOTED_TO_ACTIVE`
  edges back into RustyRed.
- The end-to-end test opens RedCore with `aof_always`, writes the fixture,
  forces `snapshot_now()`, reopens from disk, exports the training view, and
  writes back a fake paraphramer model.

**Operator slice landed:** `rustyred-thg-adapters` now also exposes a runnable
Theorem training seam:

- `theorem_training_run fixture`: seeds the durable RedCore training fixture.
- `theorem_training_run export`: forces a RedCore snapshot and writes
  `manifest.json`, `graph_snapshot.json`, and `runpod_input.json`.
- `theorem_training_run writeback`: reads a trainer-produced
  `ModelArtifactInput` JSON and records the model artifact plus evaluation
  receipt in the same RedCore store.
- `theorem_training_run smoke`: runs the full local RedCore substrate loop for
  preflight validation.
- `scripts/submit_theorem_training_runpod.py`: submits `runpod_input.json` to a
  configured RunPod endpoint, but refuses to launch unless remote manifest and
  snapshot URIs are provided. This prevents accidentally sending local `/tmp`
  paths to a serverless worker.

Current RunPod account state, checked on 2026-06-02: the available endpoints are
GL-Fusion or model-serving endpoints, not a Theorem RustyRed trainer. The next
RunPod step is a dedicated Theorem/RustyRed training endpoint that consumes the
snapshot contract and returns `ModelArtifactInput`.

## Direct Answer

RustyRed does not need to be "trained" in the way a model is trained. It is the
durable graph substrate. But yes, we need a special training design around it:

- A stable schema for training-relevant nodes and edges.
- Immutable snapshot/export contracts for GNNs, paraphramers, rerankers,
  adapters, and router models.
- Feature, label, split, privacy, and provenance manifests.
- Trainer orchestration outside RustyRed, on Modal/RunPod/Theseus worker
  surfaces.
- Writeback contracts for embeddings, model artifacts, adapter records,
  evaluation receipts, and fitness updates.

The important distinction:

```text
AOF durability
  protects the live RustyRed graph

training snapshot
  freezes a versioned view of that graph for a trainer

model writeback
  records what was trained, what data it used, how it evaluated, and how the
  graph should use it
```

## Goal

Make Theorem's graph/training stack:

```text
Theseus sources
  S3 training artifacts, reasoning traces, graph exports, Memgraph/Postgres
  ledgers, harness events, postmortems, ImprovementEpisodes
        |
        | direct import, artifact registration, trace normalization
        v
RustyRed RedCore durable graph
  AOF + snapshots, GraphStore records, vectors, traces, adapters,
  model artifacts, training manifests, graph-version receipts
        |
        | immutable snapshot/export
        v
Training jobs
  GNN/KGE, paraphramer, reranker, Pairformer/tool-router, Graph-LoRA/adapters
        |
        | evaluated writeback
        v
RustyRed RedCore durable graph
  updated embeddings, adapter catalog, model artifact nodes, fitness edges,
  evaluation receipts, active-model designations
```

Memgraph remains Theseus's research/canonical knowledge graph. RustyRed becomes
Theorem's durable experiential/procedural graph and computational layer.
Neo4j is not deployed for this architecture.

## What AOF Gives Us

The current RustyRed code already supports the durability shape this plan needs:

- `RedCoreDurability` supports `aof_everysec`, `aof_always`, `snapshot_only`,
  and `none`.
- `RedCoreOptions::default()` uses `aof_everysec`.
- `RUSTY_RED_STRICT_ACID=true` requires embedded mode, `aof_always`,
  `single_writer`, and `serializable`.
- `RedCoreGraphStore::commit_batch()` stages mutations, persists before publish,
  then swaps the staged graph into memory.
- `RedCoreGraphStore::graph_snapshot()` exposes the current graph view.
- `RedCoreGraphStore::snapshot_now()` can force a snapshot boundary.

For deployment:

```bash
RUSTY_RED_MODE=embedded
RUSTY_RED_DATA_DIR=/data/rusty-red
RUSTY_RED_REQUIRE_VOLUME=true
RUSTY_RED_DURABILITY=aof_always
RUSTY_RED_CONCURRENCY=single_writer
RUSTY_RED_TXN_ISOLATION=serializable
RUSTY_RED_STRICT_ACID=true
```

This makes RustyRed durable enough to be Theorem's primary graph for this lane.
Training still needs explicit dataset snapshots so trainers do not read a
moving live graph.

## Durable Graph Content

RustyRed should store or reference these families:

- `ImprovementEpisode`: task, model trajectory, reviewer status, outcome,
  training disposition.
- `Postmortem`: failure, origin run/patch, repair pattern, tags.
- `Learning`: durable rule, method, anti-pattern, gotcha, session evidence.
- `ReasoningTrace`: task, context pack, steps, tool calls, retrieved evidence,
  intermediate claims, outcome, reviewer disposition, downstream training use.
- `Artifact`: S3 URI or local path, content hash, byte size, export family,
  privacy tier, source run, graph hash.
- `TrainingPack`: grouped artifacts for a run or model family.
- `TrainingExport`: immutable exported dataset view from a RustyRed graph
  version.
- `ModelArtifact`: trained model, adapter, paraphramer, scorer, or embedding
  pack with evaluation state.
- `EvaluationReceipt`: metrics, holdout set, failure notes, promotion decision.

Large files stay in S3 or local artifact storage. RustyRed stores provenance,
manifest metadata, hashes, vectors selected for hot retrieval, and graph
relationships.

## Trainable Surfaces

### GNN And KGE

Required design:

- Node and edge type taxonomy.
- Feature builders for text embeddings, graph metrics, timestamps, confidence,
  privacy tier, and outcome labels.
- Negative sampling rules.
- Temporal train/validation/test splits.
- Export formats: Arrow/Parquet for rows, TSV for triples, NPZ for dense
  arrays, JSON manifest for provenance.
- Snapshot hash and graph version written into every export.
- Writeback for `EmbeddingPack`, selected node vector properties, and
  `EvaluationReceipt`.

RustyRed already has vector designations and search. The missing piece is the
RustyRed-native training exporter, not the in-memory graph math.

### Paraphramer

Required design:

- Extract source text, paraphrase target, evidence-preservation constraints,
  style/voice target, and privacy tier from reasoning traces and artifacts.
- Keep trace links so the paraphramer can learn from successful rewrites and
  failure postmortems.
- Evaluate semantic preservation, citation preservation, hallucination rate,
  and downstream retrieval effect.
- Write back `ParaphramerModel`, `TrainingPack`, and `EvaluationReceipt`.

The paraphramer should not train from raw blobs alone. Reasoning traces are the
valuable supervision because they tell the model why a phrasing worked.

### Reranker, Pairformer, Tool Router

Required design:

- Turn retrieval sessions, chosen tools, skipped results, clicked results,
  validation outcomes, and final task success into examples.
- Preserve the context pack and graph neighborhood used at decision time.
- Train outside RustyRed, then write back scorer/model artifacts plus fitness
  edges.
- Keep an A/B or shadow-eval receipt before promotion.

### LoRA And Adapter Catalog

The repo already has a useful pattern in `rustyred-thg-adapters`:

- `LoraAdapter` is stored as a graph node.
- `TRAINED_ON` links adapters to training objects.
- `DERIVED_FROM` links adapter lineage.
- Fitness updates and supersession are graph records.
- Adapter search can use PPR over training edges or embeddings.

Extend this pattern to GNN, paraphramer, reranker, and router artifacts instead
of creating a separate registry.

## Implementation Phases

### Phase 0: Durability Baseline

Acceptance criteria:

- RustyRed starts in embedded RedCore mode with a mounted volume.
- AOF is enabled as `aof_always` for strict mode.
- `/ready` passes.
- A small graph write survives process restart.
- `snapshot_now()` produces a training boundary.

### Phase 1: Direct Artifact And Trace Import

Build import/registration for:

- Theseus S3 training artifacts.
- Local `gnn_export/` and `kge_embeddings/` inventories.
- Reasoning traces and trace attributions.
- Harness events and memory records.
- Postmortems, learnings, and ImprovementEpisodes.

Acceptance criteria:

- Dry run reports counts by artifact family, trace family, and source path.
- Imports are idempotent by stable id and content hash.
- Large matrices are referenced by artifact nodes, not blindly expanded into
  every graph node.
- Reasoning traces are queryable by task, toolchain, evidence source, outcome,
  and downstream training pack.

### Phase 2: RustyRed Training Snapshot Exporter

Build a snapshot/export command over RustyRed:

```text
RustyRed graph version -> frozen training export
```

Export outputs:

- `nodes.parquet`
- `edges.parquet`
- `entity_map.tsv`
- `relation_map.tsv`
- `triples.tsv`
- `node_features.npy` or `.npz`
- `edge_features.npy` or `.npz`
- `reasoning_traces.jsonl`
- `paraphrase_pairs.jsonl`
- `ranking_pairs.jsonl`
- `manifest.json`

Acceptance criteria:

- Manifest includes graph version, snapshot hash, source graph status, export
  time, privacy tier, selected labels, selected edge types, and feature schema.
- Export is deterministic for the same graph snapshot.
- Export rejects mixed-dimension vectors and stale artifact references.
- Export can run in dry-run mode without writing artifacts.

### Phase 3: Training Jobs

Wire training jobs to consume RustyRed exports:

- GNN/KGE trainer consumes node/edge features, triples, and graph splits.
- Paraphramer consumes reasoning traces and paraphrase pairs.
- Reranker/Pairformer/tool-router trainers consume retrieval/tool/outcome
  examples.
- Adapter/LoRA trainers consume selected objects, traces, and Graph-LoRA packs.

Acceptance criteria:

- Tiny smoke trains on a fixture export.
- Full run can be launched from a manifest without reading the live graph.
- Trainer records dataset hash and source graph version.
- Training fails closed when privacy or temporal-alignment checks fail.

### Phase 4: Evaluated Writeback

Write trained outputs back into RustyRed:

- `ModelArtifact` nodes with S3 URI, model type, base model, dataset hash,
  graph version, and manifest version.
- `EmbeddingPack` or vector-property updates for selected labels.
- `EvaluationReceipt` nodes.
- `TRAINED_ON`, `DERIVED_FROM`, `EVALUATED_BY`, `SUPERSEDES`,
  `PROMOTED_TO_ACTIVE`, and `FAILED_EVAL` edges.

Acceptance criteria:

- No model becomes active without an evaluation receipt.
- Writeback is idempotent by model artifact id and dataset hash.
- Existing adapter catalog behavior remains compatible.
- RustyRed vector/full-text/query paths can use newly written embeddings or
  active-model designations.

### Phase 5: Training Cadence

After receiving new training data:

1. Import/register artifacts and traces into RustyRed.
2. Force or record a stable graph snapshot.
3. Export training views from that graph version.
4. Run targeted training jobs.
5. Evaluate on holdouts and task replay.
6. Write back artifacts and receipts.
7. Promote only the models that pass gates.

Acceptance criteria:

- A single command or runbook executes the cadence for a fixture tenant.
- The cadence can be resumed from any manifest id.
- Every active trained artifact can be traced back to a RustyRed graph version
  and artifact hashes.

## What To Avoid

- Do not deploy Neo4j for this slice.
- Do not treat AOF as a training export. AOF is a mutation log; trainers need a
  stable snapshot with feature and label manifests.
- Do not train directly from mutable live graph reads.
- Do not write huge arrays into every node when an artifact reference plus
  selected hot vectors is enough.
- Do not promote trained artifacts without evaluation receipts.
- Do not lose temporal alignment. Corpus rows, graph exports, embeddings, and
  reasoning traces must refer to compatible graph versions.

## Immediate Next Slice

1. Update this plan into the active architecture source of truth. Done for this
   slice.
2. Add a RustyRed fixture import with:
   - three objects
   - two reasoning traces
   - one postmortem
   - one S3 training artifact manifest
   - one adapter artifact
   Done in `register_training_fixture`.
3. Open a RedCore store with `aof_always`, write the fixture, call
   `snapshot_now()`, reopen it, and verify the fixture survives. Done in
   `durable_training_fixture_exports_and_writeback_survive_redcore_reopen`.
4. Add a dry-run training exporter that emits counts and a manifest from the
   fixture graph version. Done in `export_training_snapshot`.
5. Add a tiny paraphrase-pair and GNN-export fixture from the same graph
   version. Done in the fixture graph.
6. Register a fake trained artifact back into RustyRed with `TRAINED_ON` and
   `EVALUATED_BY` edges. Done in `register_model_artifact`.
