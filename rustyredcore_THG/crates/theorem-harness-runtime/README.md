# theorem-harness-runtime

The GraphStore-backed runtime seam for `theorem-harness-core`: it persists kernel transition receipts as `HarnessRun`/`HarnessEvent` graph nodes with append-chain edges, and adds the Dispatch v2 job board, durable coordination (v1/v2/push), graph-native memory and reasoning bank, skill/engineering packs, work-graph persistence, the governor, and live provider head invocation. Storage stays out of the parity kernel.

## Key API

- Run/event persistence (`event_log.rs`): `load_run`, `load_events`, `append_transition[_from_store]`, `persist_transition_result`, `replay_persisted_run`, `run_node_id`, `event_node_id`.
- Binding persistence (`binding_store.rs`): `persist_binding`, `load_binding`, `append_binding_transition`, `load_scratchpad_revisions`.
- Job board (`job_queue.rs`): `job_submit`, `job_list`, `job_note`, `job_archive`.
- Head invocation (`head_invoker/`): `RealHeadInvoker` (= `ProviderHeadInvoker`) impl of core's `HeadInvoker`; provider profiles (`AnthropicMessages`, `OpenAiChatCompletions`); `CredentialResolver` (`env:<VAR>`); `invoke_mcp_head`.
- Composition/coordination: `run_composed_agent[_with_claims]`, `default_theorem_binding`; coordination v1 (`join_room`, `write_message`/`write_intent`/`write_record`, `read_mentions_for_actor`, `room_status`), v2 Task-Reference Rooms (`resolve_task_ref`, `room_digest`), and `coordination_push` broadcast buses (`subscribe_coordination_room_events`, `wake_targets`).
- Memory/reasoning: `remember_memory`, `recall_memory`, `encode_memory`, `upsert_note` (Obsidian sync), `reasoning_bank`.
- Packs/work-graph/governor: `skill_pack`, `engineering_capability_packs()`, `work_graph_store` (`persist_work_graph`, `claim_task_node_durable`), `governor::govern_turn`, `patch_sequencer`.

Path deps: `rustyred-thg-affordances`, `rustyred-thg-core`, `theorem-harness-core`, `prose-check`, `design-check`. Uses `reqwest` (blocking) and `tokio` (sync, for the broadcast buses).

## Live provider heads

For the API-backed composed-agent binding:

```bash
THEOREM_AGENT_HEADS=deepseek,mistral,qwen,minimax
DEEPSEEK_API_KEY=...   # plus MISTRAL_API_KEY, QWEN_API_KEY or DASHSCOPE_API_KEY, MINIMAX_API_KEY
THEOREM_HEAD_INVOKER=real
```

Override models with `DEEPSEEK_MODEL` / `MISTRAL_MODEL` / `QWEN_MODEL` / `MINIMAX_MODEL`; endpoints with `*_CHAT_URL`. Qwen defaults to the DashScope OpenAI-compatible endpoint and can be overridden with `QWEN_CHAT_URL`. `HeadTransport::Local` uses `THEOREM_LOCAL_OPENAI_URL` (default `http://127.0.0.1:8080/v1/chat/completions`). Production `run_composed_agent` allocates `THEOREM_COMPOSED_AGENT_BUDGET_UNITS` (default 5000).

## Room runner

`agent_runner.rs` is the Harness-native participation layer. It consumes unhandled room mentions for an actor (default `theorem`), calls `run_composed_agent_with_claims`, and writes an alignment-gated contribution/reflection back to the same room. `wake_model` requests should normally mention `@theorem` and carry requested provider heads in `wake_target_head_ids`; the runner threads those head ids into the task and grounding claims before invoking the composed agent.

Hosted `theorem-harness-server` enables this with:

```bash
THEOREM_AGENT_ROOM_RUNNER=1
THEOREM_AGENT_TENANT_SLUG=Travis-Gilbert
THEOREM_AGENT_ROOM_ID=theorem-spreading-nli-completion
THEOREM_AGENT_ACTOR_ID=theorem
THEOREM_AGENT_HEADS=mistral,qwen,deepseek
MISTRAL_API_KEY=...
QWEN_API_KEY=...        # or DASHSCOPE_API_KEY
DEEPSEEK_API_KEY=...
```

Optional operator knobs: `THEOREM_AGENT_BINDING_ID`, `THEOREM_AGENT_RUNNER_INTERVAL_MS`, `THEOREM_AGENT_SURFACE`, `THEOREM_AGENT_REPO`, `THEOREM_AGENT_BRANCH`, `THEOREM_AGENT_TASK`, `THEOREM_AGENT_WORKTREE`, and `THEOREM_AGENT_REPLY_TO_REQUESTER=1`. Per-head visible room actors can be added later by running separate runners with `THEOREM_AGENT_ACTOR_ID=qwen` plus `THEOREM_AGENT_HEADS=qwen`, but the default `@theorem` runner is the composed-agent path.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p theorem-harness-runtime
cargo run -p theorem-harness-runtime --example dump_engineering_packs
```

Tests: `provider_invoker.rs` (offline mock server), `project_anchor_parity.rs` (cross-crate guard), and `live_single_head_smoke.rs` (`#[ignore]`, needs `THEOREM_LIVE_PROVIDER_TEST=1` plus a provider key or `THEOREM_LOCAL_OPENAI_URL`).

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
