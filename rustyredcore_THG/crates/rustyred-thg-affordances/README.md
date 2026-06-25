# rustyred-thg-affordances

Connector-as-substrate learning registry: MCP connector tools become first-class `Affordance` graph nodes, and the substrate learns which to reach for from accumulated outcomes (PPR over `SERVED_TASK`/`PRODUCED_OUTCOME`/`SEQUENCED_WITH` edges plus time-decayed fitness), scoped per agent by a `CapabilityScope`. Not a passthrough aggregator: selection compounds with use because it rides the same graph the rest of the substrate learns on. Structural sibling of the LoRA adapter catalog (`rustyred-thg-adapters`).

## Key API

- Types (`types.rs`): `Affordance`, `AffordanceGraphStore`, `CapabilityScope`, `ConnectorManifest`, `ToolManifest`, `SelectionRequest`, `InvocationRecordRequest`; id helpers (`affordance_node_id`, `connector_node_id`, `task_type_node_id`); labels `AFFORDANCE_LABEL`/`CONNECTOR_LABEL`/`TASK_TYPE_LABEL`/`INVOCATION_RECEIPT_LABEL`; edges `OFFERS`/`SERVED_TASK`/`PRODUCED_OUTCOME`/`SEQUENCED_WITH`.
- Registry (`registry.rs`): `register_connector[_with_target]`, `upsert_affordance`, `register_builtin_affordances`, `register_theseus_app_affordances`. Registration is idempotent (preserves fitness/embeddings/outcomes).
- Selection (`selection.rs`): `select_affordances` (PPR seeded at the task-type node, scaled by time-decayed fitness; zero-training warm start), `select_affordances_by_embedding`.
- Outcomes (`outcomes.rs`): `record_invocation` (writes receipt plus edges plus EWMA fitness in one), `effective_affordance_fitness_from_node`.
- Training (`training.rs`): `export_affordance_training_view`, `register_pairformer_artifact`, `pairformer_validation_gate`.

Path deps: `rustyred-thg-core`, `theorem-harness-core` (the affordance vocabulary plus the pairformer A/B validation gate).

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-affordances
```

Tests under `src/tests/` (registry, outcomes, selection, training). No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
