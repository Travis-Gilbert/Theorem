# rustyred-thg-adapters

LoRA adapter catalog and routing over RustyRedCore-THG graph records, plus the inference organs (Pairformer, HOT temporal, EdgeMPNN), the training substrate/runner, situation search, and grounded-skill output. Stays above `rustyred-thg-core`: it reuses core graph records, stores, and PPR while keeping adapter routing and fitness out of the core executor.

## Binary

`theorem_training_run` (`src/bin/theorem_training_run.rs`): a training-orchestration CLI with subcommands `fixture`, `gnn-import`, `hot-fixture`, `hot-train`, `export`, `writeback`, `smoke`, `runpod`.

## Module groups

- Catalog/routing: `types` (`AdapterGraphStore`, `LoraAdapter`, `AdapterRef`), `upsert`, `routing` (`find_adapters_for`, `find_adapters_by_query_embedding`), `fitness` (record/supersede/decay), `commands`.
- Reflexive inference (advisory, no mutation): `reflexive`, `reflexive_executor`.
- HOT temporal: `hot` (deterministic reference scorer, `run_hot`); `hot_burn`, `hot_cubecl` (feature-gated).
- Pairformer: `pairformer` (`run_pairformer`); `burn_pairformer`, `pairformer_cubecl` (feature-gated).
- EdgeMPNN: `edge_mpnn` (NBFNet-style); `burn_mpnn` (feature-gated).
- Admission/retrieval: `standing_pass` (one hook-facing contract over multiple `StandingGenerator`s), `situation_search`.
- Training: `training_substrate`, `training_runner`. Product output: `grounded_skill` (open-agent-skills packing).

Re-exports message-passing primitives from `rustyred-thg-ml`. Path deps: `rustyred-thg-core`, `rustyred-thg-ml`.

## Features

`default = []`. `pairformer-burn-cubecl` pulls Burn 0.21 and CubeCL 0.10 and gates the `burn_*`/`*_cubecl` modules. That feature needs rustc 1.92 or newer (above the crate's declared MSRV); the default build is the deterministic reference path and builds on 1.85.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-adapters
```

Tests under `src/tests/` cover the catalog, routing, organs, standing pass, training, and situation search; two files are feature-gated behind `pairformer-burn-cubecl`. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
