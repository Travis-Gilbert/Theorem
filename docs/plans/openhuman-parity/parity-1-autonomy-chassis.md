# Parity 1: the autonomy chassis (subconscious on dispatch v2)

Date: 2026-06-08. Plan home: docs/plans/openhuman-parity/. Builds on dispatch-v2.md (41681f93) and PARITY-MAP.md (32c2f8b1). Grounded by reading OpenHuman's `src/openhuman/subconscious/` (GPLv3; comprehension only, no code ported).

## What OpenHuman actually built (grounded)

A SQLite-backed loop, driven by their `heartbeat` domain, that on each tick: loads due tasks, builds a "situation report" from the memory tree under a token budget, calls an LLM to evaluate each task, then per task decides `act` / `escalate` / `noop`, and separately emits up to five observation-only `reflection` cards. Tasks are system-seeded (three defaults) plus user-added, with recurrence `once | cron:<expr> | pending`. Provider routing is local-first (Ollama/LM Studio) with cloud fallback. Write intent is gated by keyword heuristics: read-only tasks run analysis-only, and a recommended write action raises an `UnapprovedWrite` escalation for user approval. `last_tick_at` is a restart-durable cutoff so the situation report only reads memory rows newer than the last successful tick. An overlap guard (generation counter) lets a newer tick supersede an in-flight one. Notably, their `decision_log.rs` is retained but not wired into the live path, dead code kept for future dedup.

## The Theorem mapping

Almost every part is composition over primitives that already exist. Net-new is small.

| OpenHuman mechanism | Theorem mechanism |
|---|---|
| heartbeat loop driving ticks | a recurring tick job on dispatch v2, cadence via not_before plus a recurrence field |
| situation report from the memory tree | the launcher's context packet, specialized graph-native: PPR over fresh quarantine and active subgraphs, node hotness deltas since last tick, open intents |
| SubconsciousTask in SQLite (seeded + user) | durable Task nodes, system-seeded templates plus user-added, with an enable toggle |
| act / escalate / noop decision | act = job_submit a real job; escalate = an approval card; noop = a receipt |
| UnapprovedWrite escalation + approval | the affordance approval-card path just validated (the confirmed-passthrough gate) |
| keyword write-intent heuristics | the existing action-tier / RiskMode rail; no keyword matching |
| last_tick_at cutoff in a KV table | a graph time-cursor (bi-temporal: rows newer than last successful tick) |
| generation-counter overlap guard | the launcher's set-once started_at; no separate counter |
| reflections (observation cards) | observation-only digest cards posted to the agent-space room feed |
| decision_log.rs (shelved dead code) | job receipts, which are the live decision log for free |

## Deliverables

### D1: the tick job
A recurring job on dispatch v2 that the receiver launches on cadence (not_before plus a recurrence field, `once | cron:<expr> | pending`). Its session reads the situation report, evaluates each enabled task, and emits a decision per task. The tick is itself a normal job, so it replays and carries receipts like any other. Reuse the scheduled lane; do not build a scheduler.

### D2: the situation report (graph-native)
Specialize the launcher's context-packet composer into a tick situation report assembled under a token budget, sections in priority order: open intents and presence, fresh `open_web_unverified` quarantine since the cursor, node hotness deltas (activity since last tick), PPR seeded from the user's active subgraphs, recently consolidated summaries, and recent reflections rendered as anti-double-emit context. Truncate the tail when over budget. This is the part that beats their flat-tree assembly: the report is grounded in the graph, not a section dump.

### D3: the task model
Durable Task nodes: `title`, `source` (system | user), `recurrence`, `enabled`, `last_run_at`, `next_run_at`. Seed three default templates on init, each reusing an existing engine ability:
- scan fresh quarantine for claims that contradict active beliefs (epistemic filter)
- summarize what changed in the graph since yesterday (consolidation), surfaced as a morning digest card
- re-check monitored pages for changed claims (bi-temporal monitor_page)
A committed task-list file is the HEARTBEAT.md analog; its entries are these templates with an enable toggle. System tasks cannot be deleted, only disabled.

### D4: the decision contract
Per task the tick emits exactly one of:
- `act`: compose a spec and job_submit a real job (self-dispatch). The follow-up runs as its own session with its own receipts.
- `escalate`: raise an approval card through the affordance approval path proven this session (confirmed passthrough). Approving executes at full permission; dismissing closes it. This is their UnapprovedWrite gate, met by the gate we already have.
- `noop`: write a receipt and move on.
Write-intent classification routes through the action-tier / RiskMode rail: read-only analysis runs free, any write action escalates. Do not reimplement keyword heuristics.

### D5: dedupe and overlap
A graph time-cursor replaces `last_tick_at`: the situation report reads only rows newer than the last successful tick, advanced only on success so a failed or superseded tick re-reads the same window. Overlap is handled by the launcher's set-once started_at marker on the tick job; no separate generation counter.

### D6: reflections and the decision log
Reflections are observation-only digest cards posted to the agent-space room feed, capped per tick, with `source_refs` resolved to frozen receipts or pinned node versions for provenance. They never auto-write or auto-spawn. The decision log is the job receipt stream, which is durable and queryable already; do not add a separate decision-log store.

## Acceptance criteria

1. A recurring tick job launches on its cadence and produces a situation report grounded in the graph (quarantine, hotness, PPR, intents), not a flat dump.
2. For a task whose evaluation says act, the tick job_submits a real job with a composed spec, and that job runs as its own session.
3. For a write-intent task, the tick raises an approval card; approving it executes the work, dismissing it does not.
4. A read-only analysis task runs without an approval card.
5. The situation cursor advances only on a successful tick; a forced failure leaves the cursor in place and the next tick re-reads the same window.
6. Three system task templates seed on init, are disable-able, and are not deletable; a user can add a task.
7. Reflections post as observation-only cards to the room feed, capped per tick, each carrying resolved provenance, and none auto-spawn a job.
8. Every tick decision (act, escalate, noop) appears in the job receipt stream.

## Fences

- Reuse dispatch v2, the scheduled lane, the graph, the risk rail, the approval card, and receipts. Build no new scheduler, no new store, no new decision log.
- The coordination layer is untouched.
- Reflections are observation-only; no auto-write, no auto-spawn.
- GPLv3: comprehension only, no ported code, structures, or prompts.
- No time estimates, no em dashes.

## Where it rides

Harness runtime owns the tick job and the task nodes; the launcher's context-packet composer is extended into the situation report; the risk rail gates write intent; the approval card is the affordance path; the room feed carries reflections; receipts are the decision log.
