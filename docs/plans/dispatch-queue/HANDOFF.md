# Dispatch Queue: push verbs + a job lineup for chat-triggered builds

**Repo:** Travis-Gilbert/theorem
**Audience:** Claude Code + Codex, building as one agent
**Status:** ready to build
**Plan home:** docs/plans/dispatch-queue/

## Why this exists

Travis is the throughput bottleneck: specs accumulate faster than he can manually start sessions. This slice adds a standing queue of jobs (specs to implement, features, edits, apps, investigations) plus push verbs callable from any MCP surface, so that enqueueing work from claude.ai chat produces a building session and a PR with no terminal involvement. Travis's role compresses to: decide, then review PRs.

## What already exists, do not rebuild

- `spawn_session` MCP tool fires `repository_dispatch` to `.github/workflows/theorem-handoff.yml`, which runs Claude Code non-interactively with the intent as its prompt, pushes work, and opens a PR. This is the push mechanism. Reuse it as the dispatch backend.
- Run lifecycle in GraphStore: `harness_append_transition`, `harness_run` replay.
- Coordination room: intents, messages, records, mentions with `delivery=wake`.
- Multi-head run execution handoff (docs/plans/multi-head-run-execution/HANDOFF.md) defines TaskNode, the INTRA-run work graph. The Job introduced here sits ABOVE runs: one job is one spec, one session, one run. Do not merge Job into TaskNode. A dispatched job's session may decompose into TaskNodes; that is the existing spec's territory.

## Deliverable 1: Job node in GraphStore

New typed node `Job` in the rustyredcore_THG GraphStore, beside RunState:

```rust
Job {
    job_id: String,              // "job-" + ulid
    kind: JobKind,               // ImplementSpec | Feature | Edit | App | Investigation
    title: String,
    spec_ref: String,            // repo path (docs/plans/x/HANDOFF.md) or harness doc_id
    repo: String,                // "Travis-Gilbert/theorem" etc.
    branch: Option<String>,      // default: repo default branch
    priority: Priority,          // P0 | P1 | P2
    target_head: TargetHead,     // ClaudeCode | Codex | Either
    status: JobStatus,           // Queued | Dispatched | Running | PrOpen | Verifying | Done | Failed | Cancelled
    submitted_by: String,        // actor_id
    submitted_at: String,
    dispatched_at: Option<String>,
    closed_at: Option<String>,
    session_ref: Option<String>, // run_id and/or workflow run id
    pr_ref: Option<String>,
    idempotency_key: String,     // default: hash(spec_ref + title)
    notes: Option<String>,
}
```

Edges: `JOB_FOR_SPEC` to the spec doc node when spec_ref is a doc_id; `DISPATCHED_AS` to the run node; `PRODUCED` to the PR record. Every status transition appends a graph event so the lifecycle is replayable.

## Deliverable 2: push verbs (MCP tools on the harness server)

- `job_submit { title, spec_ref, repo, kind, priority?, target_head?, notes?, idempotency_key? }` creates Job{Queued}, returns job_id. A duplicate idempotency_key returns the existing job_id and creates nothing.
- `queue_status { repo?, status? }` returns jobs ordered by priority then submitted_at.
- `job_cancel { job_id }` moves Queued (or Dispatched-not-yet-running) to Cancelled.
- `job_promote { job_id, priority }` reorders.
- `job_claim { actor_id, head }` is PULL mode: atomically pops the highest-priority Queued job matching the head, marks it Dispatched with session_ref bound to the caller's run. CAS semantics: under concurrent claims exactly one caller wins a given job; the loser receives the next job or empty.
- `job_complete { job_id, outcome, pr_ref?, receipts? }` closes to Done or Failed and writes a fitness outcome receipt.

All verbs are ordinary MCP tools, so they are callable from claude.ai, Claude Code, Codex, or any connected surface.

## Deliverable 3: dispatcher (PUSH mode)

- Runs inside the harness runtime as an event-triggered task (on job_submit and on any job closing) with a periodic sweep as fallback.
- Capacity policy, named choice: 1 concurrent dispatched job per repo at first; raise only after observed clean merges.
- Action: pop the highest-priority Queued job with target_head ClaudeCode or Either, then fire the existing repository_dispatch path with this intent template:

  "Implement {spec_ref} fully as written. This is {job_id}. Open a PR whose description references {job_id}. Record receipts to the run. Do not expand scope beyond the spec."

- Status tracking: Dispatched on fire; Running on the first run event; PrOpen when the session reports it via job_complete or when the periodic sweep finds the PR through the GitHub API; Verifying when a verify record appears; Done on merge signal or job_complete.
- Codex lane: jobs with target_head Codex stay Queued for PULL mode until `.github/workflows/codex-handoff.yml` exists. That workflow, mirroring theorem-handoff with the Codex CLI, is a deliverable of this slice; once present, the dispatcher treats Codex jobs as dispatchable.

## Deliverable 4: chat surface behavior

From claude.ai or any MCP client, the whole gesture is: commit a handoff, call job_submit with its path. Status checks are queue_status. No UI in this slice.

## Acceptance criteria

1. job_submit from claude.ai creates a Job visible in queue_status, ordered correctly across priorities.
2. With docs/plans/<x>/HANDOFF.md committed, job_submit leads, with no further human action, to a theorem-handoff workflow run whose prompt contains the spec_ref and job_id, and a PR whose body references the job_id; the Job reaches PrOpen.
3. job_cancel on a Queued job prevents dispatch; the event log shows the transition.
4. Two jobs submitted out of priority order (P1 then P0) dispatch in priority order under cap=1.
5. Under concurrent job_claim calls, each Queued job is won exactly once (CAS verified by test).
6. job_complete writes an outcome receipt; replay shows the full lifecycle Queued through Done.
7. A duplicate idempotency_key returns the original job_id and creates nothing.

## Fences, not in this slice

- No recurring or cron jobs; single-shot only. A cron lane is a named follow-on.
- No auto-merge; humans merge PRs.
- No web UI.
- No new storage; Jobs live in the existing GraphStore.

## Security

- Verbs are available only on authenticated MCP surfaces.
- Dispatch is restricted to repos that already carry the theorem-handoff workflow, the Anthropic GitHub App, and the token secret.

## First job after this is built

job-001: spec_ref docs/plans/theorem-desktop/HANDOFF.md, kind App, priority P0, target_head Either. The desktop app, Dia rebuild as phase one.
