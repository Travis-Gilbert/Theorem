# Dispatch Queue: push verbs + a job lineup, dispatched through the harness

**Repo:** Travis-Gilbert/theorem
**Audience:** Claude Code + Codex, building as one agent
**Status:** ready to build
**Plan home:** docs/plans/dispatch-queue/
**Supersedes:** the GitHub-Actions draft committed at cba0df0a. That draft reused spawn_session and the theorem-handoff workflow as the dispatch backend, which is the architecture decided against on 2026-06-04. This version implements the decided design.

## Decision record

The GitHub Actions pipeline (push, relay, repository_dispatch, claude-code-action) was built and proven green end to end, then rejected as the control plane: PATs, OAuth token secrets, relay workflows, and runner complexity for what is locally a one-line spawn. The decided architecture is the harness-local-session design with the receiver note approved 2026-06-04:

- The queue lives in the GraphStore. Execution happens on the machine where the Claude Code and Codex apps are installed, using their existing logins. Zero new credentials: the CLAUDE_CODE_OAUTH_TOKEN machinery existed only because GitHub runners have no login.
- The local receiver role is light and idle until pinged. It holds only an outbound connection to the cloud harness, pulls the brief with a network call, and spawns the CLI. It does not run the RustyRed engine locally: no vector index, no PPR, no BM25, no embedders. Option A, primary: the receiver as a capability of the local RustyRed node. Option B, fallback: a standalone listener binary. Deployment is docker run with a restart policy or launchd; Kubernetes is ruled out.
- The RunPod serverless executor remains a shelved fallback for two cases only: multi-user execution (other users' jobs must run on their own keys) and machine-asleep dispatch. Out of scope here.

## What already exists, do not rebuild

- Run lifecycle in GraphStore: `harness_append_transition`, `harness_run` replay.
- Coordination room: intents, messages, records, mentions with `delivery=wake`.
- Multi-head run execution handoff (docs/plans/multi-head-run-execution/HANDOFF.md) defines TaskNode, the INTRA-run work graph. The Job introduced here sits ABOVE runs: one job is one spec, one session, one run. Do not merge Job into TaskNode.
- `spawn_session` and `.github/workflows/theorem-handoff.yml` stay in the codebase as an untouched legacy remote lane. This slice does not call them.

## Deliverable 1: Job node in GraphStore

New typed node `Job` in the rustyredcore_THG GraphStore, beside RunState:

```rust
Job {
    job_id: String,              // "job-" + ulid
    kind: JobKind,               // ImplementSpec | Feature | Edit | App | Investigation
    title: String,
    spec_ref: String,            // repo path (docs/plans/x/HANDOFF.md) or harness doc_id
    repo: String,                // "Travis-Gilbert/theorem" etc.
    branch: Option<String>,      // default: job/{job_id}
    priority: Priority,          // P0 | P1 | P2
    target_head: TargetHead,     // ClaudeCode | Codex | Either
    status: JobStatus,           // Queued | Claimed | Running | PrOpen | Verifying | Done | Failed | Cancelled
    submitted_by: String,        // actor_id
    submitted_at: String,
    claimed_by: Option<String>,  // receiver id
    claimed_at: Option<String>,
    closed_at: Option<String>,
    session_ref: Option<String>, // run_id
    pr_ref: Option<String>,      // PR number or branch ref
    idempotency_key: String,     // default: hash(spec_ref + title)
    notes: Option<String>,
}
```

Edges: `JOB_FOR_SPEC` to the spec doc node when spec_ref is a doc_id; `DISPATCHED_AS` to the run node; `PRODUCED` to the PR or branch record. Every status transition appends a graph event so the lifecycle is replayable.

## Deliverable 2: push verbs (MCP tools on the harness server)

- `job_submit { title, spec_ref, repo, kind, priority?, target_head?, notes?, idempotency_key? }` creates Job{Queued}, returns job_id. A duplicate idempotency_key returns the existing job_id and creates nothing.
- `queue_status { repo?, status? }` returns jobs ordered by priority then submitted_at.
- `job_cancel { job_id }` moves Queued (or Claimed-not-yet-running) to Cancelled.
- `job_promote { job_id, priority }` reorders.
- `job_claim { receiver_id, lanes, repos }` atomically pops the highest-priority Queued job matching the receiver's lanes and configured repos, marks it Claimed. CAS semantics: under concurrent claims exactly one caller wins a given job; the loser receives the next job or empty.
- `job_complete { job_id, outcome, pr_ref?, receipts? }` closes to Done or Failed and writes a fitness outcome receipt.

All verbs are ordinary MCP tools, callable from claude.ai, Claude Code, Codex, or any connected surface. A running interactive session may also call job_claim directly; hand-started sessions are queue consumers too.

## Deliverable 3: the receiver (replaces the GitHub dispatcher)

New crate `theorem-receiver` under rustyredcore_THG/crates/, buildable two ways: a standalone binary (Option B) and a feature of the node binary (Option A, primary). Verify exact placement against the workspace before scaffolding.

Startup:
- Detect lanes: `which claude`, `which codex`. Register only what is present.
- Load receiver config (TOML): map of repo to local worktree path, claim interval (default 20s), capacity (default 1 concurrent job per repo).
- Open an outbound connection to the cloud harness: attempt job_claim on the interval, and immediately after any job completes. Bearer token + tenant_slug on every call, the same client config as the 2026-06-05 auth fix. SSE wake on the jobs channel is a named follow-up, gated on the tenant-scoped push fix in push.rs (owned by Claude Code); until it lands, polling is the mechanism. Outbound only, either way: no inbound port, no tunnel.

On claim:
- Spawn the head as a child process in the repo worktree.
  - Claude lane: `claude -p "<intent>" --permission-mode acceptEdits`. Verify the exact flag set against the installed CLI version at build time.
  - Codex lane: `codex exec "<intent>"`.
- Spawn environment: inherit the user environment, then strip `ANTHROPIC_API_KEY`. The CLI's own subscription login is the auth. An API key in the child env silently wins precedence and bills metered rates.
- Intent template: "Implement {spec_ref} fully as written. This is {job_id}. Work on branch job/{job_id}. When done: push the branch, open a PR with the local gh login if present, and call job_complete with the outcome, pr_ref, and receipts. Do not expand scope beyond the spec."

During and after:
- The session reports through the existing run lifecycle. The receiver captures the child exit code and the tail of stdout as fallback receipts.
- If the child exits without job_complete, the receiver closes the job Failed with the exit receipt.
- If gh is not authenticated, the pushed branch is the review artifact; job_complete carries the branch ref as pr_ref.

Billing and policy notes, bake as comments in the receiver source:
- From 2026-06-15, `claude -p` on a subscription draws from the separate monthly Agent SDK credit bucket. Finite. Log a per-job usage line so draw is measurable.
- Solo use on the owner's own repos is sanctioned individual use. The moment a job belongs to another user, it must execute on that user's own key; that is the shelved RunPod lane, never the personal subscription login.

## Deliverable 4: chat surface behavior

From claude.ai or any MCP client, the whole gesture is: commit a handoff, call job_submit with its path. Status checks are queue_status. No UI in this slice.

## Bootstrap note

The receiver itself is built by a hand-started session; the queue cannot dispatch before the receiver exists. The first dispatched job is job-001 below.

## Acceptance criteria

1. job_submit from claude.ai creates a Job visible in queue_status, ordered correctly across priorities.
2. With a receiver running on a machine where Claude Code is installed and docs/plans/<x>/HANDOFF.md committed: job_submit leads, with no further human action and zero GitHub Actions, runners, PATs, or stored OAuth tokens, to a local `claude` process whose prompt contains the spec_ref and job_id, work pushed to branch job/{job_id}, and the Job reaching PrOpen or Done via job_complete.
3. A receiver without `codex` on PATH never claims Codex-lane jobs; they stay Queued.
4. job_cancel on a Queued job prevents execution; the event log shows the transition.
5. Two jobs submitted out of priority order (P1 then P0) execute in priority order under capacity 1.
6. Under concurrent job_claim calls from two receivers, each Queued job is won exactly once.
7. A child process exiting without job_complete closes the job Failed with the exit code receipt.
8. A duplicate idempotency_key returns the original job_id and creates nothing.
9. The receiver at idle holds no engine state: no vector index, no embedders, listener-scale memory footprint.

## Fences, not in this slice

- No GitHub Actions, no runners, no PATs, no stored OAuth tokens.
- No recurring or cron jobs; single-shot only.
- No auto-merge; humans merge.
- No web UI.
- No new storage; Jobs live in the existing GraphStore.
- No Kubernetes.
- The RunPod executor is out of scope (shelved multi-user and machine-asleep fallback).

## Security

- Verbs are available only on authenticated MCP surfaces (bearer + tenant).
- The receiver executes only repos present in its local config map; a job for an unmapped repo is never claimed.

## First job after this is built

job-001: spec_ref docs/plans/theorem-desktop/HANDOFF.md, kind App, priority P0, target_head Either. The desktop app, Dia rebuild as phase one.
