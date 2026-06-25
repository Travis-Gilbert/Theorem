# theorem-harness-core

The pure-logic harness kernel: a guarded run state machine, content-addressed state hashing, deterministic replay/fork, permission-aware toolgraph selection, the composed-agent binding plane, and the dispatch Job domain. No storage, no network; parity-tested against Python reference corpora. This is why the sibling crates depend on it: it has no intra-workspace path deps.

## Key API

- State machine: `apply_transition(state: Option<RunState>, input: TransitionInput) -> Result<TransitionResult, HarnessError>` (all guards enforced here). `RunState`, `TransitionInput`, `EventState`, `GuardViolation`, `Payload`.
- State hashing: `hash_run_state(&RunState)`, `stable_value_hash(&Value)`, `empty_state_hash()` (canonical-JSON SHA256).
- Replay/fork: `replay_events`, `replay_run`, `fork_run`, `fork_events`.
- Toolgraph: `select_tools(task_type, scope)`, `compile_task_toolkit(...) -> CompiledToolkit`, `normalize_permissions`, `ToolContract`.
- Jobs: `Job`, `JobSubmission`, `JobReceipt`, `idempotency_key_for`, `TargetHead { Claude, Codex, Either }`, `Priority { P0, P1, P2 }`.
- Composed-agent plane: `AgentBinding`, `apply_binding_transition`, `evaluate_publication`, `composition_hash`, `MIN_CONSENSUS_HEADS`; `AgentHeadRegistry`; `HeadInvoker` trait plus `FakeHeadInvoker`; `run_intra_agent_loop_with_invoker`.
- Work graph (multi-head CAS): `WorkGraph`, `TaskNode`, `ClaimLease`, `claim_task_node`, `spawn_verify_node`, `submit_verify_receipt`, `HeadFitness`, `next_for_head`.
- Budget/policy/memory: `check_contribution_budget`, `Constitution`, `compile_map_artifact`, `PrepareMemoryBank`, `ContextManager`, `AffordanceContract`, `receive_federated_signal`, `SessionMetricsState`.

Modules: `state_machine`, `types`, `state_hash`, `replay`, `toolgraph`, `job`, `agent_binding`, `agent_head_registry`, `alignment`, `head_invocation`, `intra_agent_loop`, `budget`, `constitution`, `work_graph`, `work_graph_verify`, `head_fitness`, `scheduler`, `affordances`, `map_artifacts`, `memory_contracts`, `provider_head_adapter`, `context_manager`, `context_web`, `federated_signals`, `session_metrics`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p theorem-harness-core
```

Tests under `tests/` (state machine, registries, intra-agent loop, map artifacts, memory contracts, parity, toolgraph parity). No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
