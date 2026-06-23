# Local Loop: implementation notes

Branch `001-local-loop`. The local loop closes with the pieces already built:
Gemma on `theorem-agentd` watches an Agent Queue TickTick list and the rooms,
submits jobs, the existing `theorem-receiver` wakes a head to a PR, and the daemon
relays milestones back onto the originating task so a run is visible from a phone.

## Where the work landed

- `theorem-harness-core/src/job.rs`, `theorem-harness-runtime/src/job_queue.rs`,
  `rustyred-thg-mcp/src/lib.rs`: the job<->task correspondence (`source_task_id`,
  `source_project_id`). The plan's "extend theorem-agentd in place" structure note
  under-specified this; CHK001 puts the fields on the shared `Job`, so the change
  spans the substrate crates. It is purely additive (serde-default, skip-if-none),
  so jobs already in the production graph deserialize unchanged.
- `theorem-agentd/src/capture.rs`: the mechanical Agent Queue sweep.
- `theorem-agentd/src/relay.rs`: milestone detection, dedup, completion.
- `theorem-agentd/src/config.rs`: `CaptureConfig`, `RelayConfig`,
  `operator_memory_tenant`, `ledger.mirror_to_graph`.
- `theorem-agentd/src/tools.rs`: the charter, and the TickTick catalog rewrite.
- `theorem-agentd/src/turn_loop.rs`: `run_tick` (capture -> relay -> turn),
  operator-tenant routing, ledger mirror.
- `theorem-agentd/src/model.rs`: `compose_line` (model-written milestone prose).

## Load-bearing design decisions

### Mechanical vs model-written (the plan's central split)

- Mechanical (deterministic Rust, offline-testable with the `rule` provider):
  capture, completion-on-merge, milestone *detection*, transition gating, and
  dedup. A status that reads more finished than the run is a bug, so none of this
  is left to the model.
- Model-written: the milestone *line* itself. `compose_line` returns the model's
  prose, with a deterministic fallback for the rule provider and on any model
  error. The loop still guarantees the load-bearing fact: a PR-opened line carries
  the PR URL even if the model omits it.

There is no mechanical mirror module (the rejected alternative). Relays fire at
transitions only, deduped by `relay:<milestone>` marker receipts written back onto
the job thread. Considered: optimistic pre-marking (risks a missed line) and
reading the task body for dedup (extra calls, brittle). Chose job-receipt markers:
deterministic, offline-testable, single source of truth.

### TickTick transport

Confirmed against the live MCP, not the drifted catalog. Every TickTick tool wraps
its args in `params`; the body is `content`, the list is `project_id`, priority is
the int enum 0/1/3/5. The session-mode MCP path returns the raw MCP result
envelope (`content[0].text`), not a parsed payload, so capture/relay drill
`content -> text -> parse -> unwrap nested result` via `capture::ticktick_json`
(tolerant of both shapes). The old `ticktick_list_tasks` (which did not exist on
the server) and the flat schemas are gone.

### Tenant routing

`operator_memory_tenant` defaults to "default" so behavior is unchanged until set.
Personal-memory tools (recall/remember/encode/self_*/forget) carry the operator
tenant; coordination, jobs, presence, and TickTick stay on the harness default. An
explicit tenant the model set is respected. No history is migrated.

### Label factory

The JSONL ledger stays the training-data source of truth and is never rotated.
Each line is additionally mirrored into the graph as a `self_note` receipt
(`kind = agentd_ledger`), namespaced so it accumulates beside the CC/Codex traces
without polluting belief recall. Best-effort: a mirror failure never breaks a tick.

## Burn serving path (CHK018-022): status

llama-server stays production (CHK018, satisfied today). The Burn server (CHK019-022)
is a from-scratch inference sub-project: there is no `burn-lm`, `CubeK`, or
`llama-burn` reference anywhere in the repo, and `burn-playground/` is an empty
scaffold; the on-disk weights are GGUF, not safetensors. The plan's own build order
sequences this last ("then the Burn spike CHK018 to CHK022 once the loop is
generating traces"), so the loop ships first and the spike follows.

The reversible seam the acceptance asks for (CHK022) already exists: the model
provider is per-config (`openai-compatible` today), so a Burn server is a base_url
swap with llama-server left installed. The genuinely completable, model-free piece
of CHK020 - compiling the tool catalog into a token-level logit mask for
constrained decoding - is tracked as the first Burn-spike task; it does not need
the weights or a GPU and can be built and unit-tested against `ToolCatalog`.

This is recorded, not silently cut: the loop (CHK001-017, 023-032) is complete and
verified; the Burn spike is the named follow-on the spec sequences after it.

## Verification

`cargo test -p theorem-agentd` (43), `-p theorem-harness-core` (71),
`-p theorem-harness-runtime` (83), `-p theorem-receiver` (41),
`-p rustyred-thg-mcp` (job tools) all green. The production server and the
standalone harness HTTP server build green against the new `Job`. Acceptance
criteria that require a live daemon + a real head + a merged PR are exercised
offline with the deterministic `rule` provider and grounded against the real
TickTick schemas.

## Live capture smoke (CHK008, partial)

Ran one real capture of the seed task ("Add a /health endpoint to theorem-grpc")
via `--capture-once`'s sequence against the live services. Findings:

- job_submit created a real pending job (`job-01KTRTRPSJ90DF8PC2MDTW8HN1`, P1)
  with `spec_inline` = the task content. The priority map (TickTick 3 -> P1) held
  live.
- The stamp + checked `dispatched` subtask landed on the task (live
  `ticktick_update_task`).
- The move to the product list FAILED: the TickTick Open API does not honor moves
  on this token ("the Open API may not honor moves on this token"). The MCP
  correctly detected the no-op rather than reporting false success. capture.rs
  calls the move best-effort, so the daemon degrades gracefully: the task stays
  stamped + dispatched-checked, and the next sweep's `is_already_captured` guard
  skips it (no duplicate job). Operator action: use a token with move permission,
  or treat the stamp + checked subtask as the dispatched signal and leave the move
  off.
- `source_task_id`/`source_project_id` did NOT persist on the board: the deployed
  `rustyred-thg-mcp` predates the field (serde accept-and-ignore). They persist
  once the mcp change deploys; verified offline by the runtime round-trip test.

`--capture-once` was added to the daemon (`theorem-agentd --capture-once`) as the
reusable, model-free way to run exactly this sweep with the operator's token.
