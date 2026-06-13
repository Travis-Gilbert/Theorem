# Build Checklist: Local Loop (status)

Source plan/checklist: the Local Loop handoff (plan-local-loop, checklist-local-loop).
Branch: `001-local-loop`. Code lives in `rustyredcore_THG/crates/theorem-agentd/`
plus the shared `Job` correspondence in `theorem-harness-core` / `-runtime` /
`rustyred-thg-mcp`.

Status legend: [x] done + verified, [~] partial, [ ] not done.

## Correspondence (job to task)

- [x] CHK001 `source_task_id` + `source_project_id` added to `Job` and
  `JobSubmission` (`theorem-harness-core/src/job.rs`), additive + serde-default so
  existing job nodes deserialize unchanged. Test: `from_submission_carries_source_correspondence`.
- [x] CHK002 Set on capture (capture builds the submission with both ids) and
  reachable for plan-submitted jobs (the `job_submit` MCP tool advertises both
  fields). Tests: `capture_converts_task_to_job_with_source_correspondence`.
- [x] CHK003 `job_list` surfaces the source task id (whole-Job serialization).
  Test: `source_correspondence_round_trips_through_job_list` (runtime).

## Inbound capture (phone to job)

- [x] CHK004 `Agent Queue` TickTick list created (id `6a2911688f08a8907c774531`);
  `capture` only ever reads the configured Agent Queue list.
- [x] CHK005 `capture::run_capture` converts each task mechanically: title->title,
  content->spec_inline, TickTick priority (5/3/else) -> P0/P1/P2.
- [x] CHK006 Stamp job id into content, check the `dispatched` subtask, move to the
  product list. One `ticktick_update_task` carries stamp + subtask; `ticktick_move_task` moves it.
- [x] CHK007 Only the Agent Queue list is read, and a `projectId` guard skips any
  stray task. Tasks elsewhere are never converted.
- [~] CHK008 Acceptance verified offline (deterministic `FakeGateway` test
  `capture_converts_task_to_job_with_source_correspondence`, grounded against the
  real `ticktick_get_project` shape). Live end-to-end (a phone-created task within
  one tick) is the operator's run; a seed task already sits in Agent Queue.

## Milestone relay (run to phone)

- [x] CHK009 `relay::run_relays` relays started, PR-opened, merged, failed onto the
  source task. Milestone prose is model-composed (`ModelClient::compose_line`).
- [x] CHK010 PR-opened is its own line and is guaranteed to carry the PR URL
  (mechanically appended if the model omits it).
- [x] CHK011 Transitions only, deduped by `relay:<milestone>` marker receipts on
  the job thread; there is no per-`job_note` mirror module.
- [x] CHK012 Acceptance offline: `full_sweep_relays_started_pr_merged_and_completes`
  asserts exactly started/pr_opened/merged lines, the PR link present, one content update.

## Completion semantics

- [x] CHK013 Completion fires on merge only (mechanical), not on PR-opened.
- [x] CHK014 Manual completion wins: a task the operator already completed is left
  alone (markers recorded so it is not re-checked). Test: `manual_completion_wins`.
- [x] CHK015 Acceptance offline via the two relay tests.

## Wake path (confirmation, already built)

- [x] CHK016 Confirmed against source: the daemon reaches heads only via
  `job_submit` (capture) and `coordinate` (the existing rule path); `theorem-receiver`
  performs the local launch + set-once start. The charter forbids direct spawn.
- [x] CHK017 Confirmed: a submitted job is launched by the receiver, never spawned
  by the daemon (`receiver.rs` `run_loop_until` -> `start_and_run_job`).

## Burn serving path (the one real gap)

- [x] CHK018 llama-server stays the production loop (config `provider = "openai-compatible"`).
- [ ] CHK019 Burn server behind the openai-compatible seam: needs the external
  `burn-lm` + `CubeK` crates (not in-repo) and a Gemma inference stack. Sequenced
  next per the plan's own build order. The reversible seam (per-config base_url
  swap, llama-server left installed) already exists.
- [x] CHK020 Constrained decoding: `constrained_decoding.rs` compiles the
  `ToolCatalog` into a token-level logit mask (`ToolGrammar::token_mask`), the
  catalog-specific part being the enumerated tool names. 10 tests green, incl. the
  "every prefix of a valid envelope is viable" invariant and the mask projection.
  Wiring it into the Burn sampler is the remaining step once CHK019 lands.
- [ ] CHK021 Import Gemma 4 12B safetensors (llama-burn reference shape): on-disk
  weights are GGUF, not safetensors; needs conversion + the Burn model. Blocked on
  CHK019.
- [~] CHK022 Parity before cutover: the per-config reversible seam is in place;
  parity itself needs a working Burn server (CHK019) + a GPU run. Not achievable in
  this environment (no burn-lm/CubeK/weights/GPU); recorded, not cut.

## Tenant (before multi-user)

- [x] CHK023 `operator_memory_tenant` config; recall + personal-memory tool calls
  routed to it. Default stays "default".
- [x] CHK024 Coordination, jobs, runs, shared substrate stay on default (only
  personal-memory tools are re-tenanted). Test: `personal_memory_tools_route_to_operator_tenant`.
- [x] CHK025 No migration: existing default history is untouched; new personal
  writes are routed.
- [~] CHK026 Acceptance offline (routing test); live recall-from-named-tenant is the
  operator's run after setting the tenant.

## Label factory (the payoff)

- [x] CHK027 Ledger is append-only and never rotated; the mirror is additive.
- [x] CHK028 Each ledger line is mirrored into the graph as a `self_note` receipt
  (`kind = agentd_ledger`), accumulating beside the CC/Codex traces.
- [~] CHK029 Acceptance: the job thread (task via source_task_id, spec_inline,
  receipts/outcome) plus the ledger self_notes form the retrievable trace; verified
  by construction, live retrieval is the operator's run.

## Charter (the daemon's voice)

- [x] CHK030 Charter folded into `ToolCatalog::system_prompt` (principles, not tone).
- [x] CHK031 Disagreement licensed explicitly; charter states the voice is not
  optimized on approval.
- [x] CHK032 Inspectable in the built prompt. Test: `system_prompt_contains_the_charter`.

## Deferred by decision (not this plan)

- Daily digest relay: unchanged, still sequenced after milestone relays.
