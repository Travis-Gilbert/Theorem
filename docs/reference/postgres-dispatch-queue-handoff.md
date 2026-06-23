# Postgres Dispatch Queue for the Local Agent Loop: Execution Handoff

Target repo: `Travis-Gilbert/Theorem`
Crates touched: `rustyredcore_THG/crates/theorem-receiver`, `rustyredcore_THG/crates/theorem-agentd`, `apps/theorem-harness-server`
Always `git pull` before editing.

---

## Decision summary (named choices are requirements)

1. **Postgres (sqlx) is the hot dispatch store, not Redis.** Durable by default, transactional (claim + result + state flip in one transaction = exactly-once), and SQL-inspectable, which matters because you will debug this board by hand. Redis's only edge is throughput, irrelevant at tens of jobs a day.

2. **The job is mirrored, not moved.** The canonical coordination and provenance record stays in the existing RustyRed Dispatch v2 board (`job_submit`/`job_list`/`job_note`/`job_archive`). Postgres holds only hot execution state. Postgres is the source of truth for execution state; RustyRed is the source of truth for coordination. This is the hot/canonical split already used for the crawl frontier.

3. **Wake reuses what exists.** The harness server already has `push_router` + `RoomBus` + SSE + `spawn_wake_listener`. The receiver wakes on that SSE and then runs the SKIP LOCKED claim against Postgres. A periodic SKIP LOCKED poll (for example every 5 seconds) is the backstop so no job is ever stranded if a wake is missed. Postgres LISTEN/NOTIFY is not required; the existing SSE plus a poll covers it. Jobs inserted directly (for example from a database console, see Manual submission below) do not pass through `job_submit`, so they fire no SSE; the poll is what catches them.

4. **No secrets in the job payload.** Matches the existing `IngestJobRequest` rule (the request is persisted to a durable mirror node). Tokens and credentials are resolved at spawn time by the worker, never stored on the job.

5. **Library: sqlx with a bespoke `dispatch_jobs` table.** Maximum control over the state machine, the most inspectable surface, easy THG mirror, and `sqlx` is already in the workspace. Swap targets, if wanted later: `pgmq` to make the queue itself a pgrx extension (also serves the customize-Postgres interest), or `apalis-postgres` for a worker framework. Neither is needed for this.

---

## Schema

```sql
create type dispatch_state as enum ('pending','claimed','running','done','failed','dead');
create type dispatch_head  as enum ('claude','codex','either');

create table dispatch_jobs (
    job_id          text primary key,            -- same id as the THG Dispatch v2 job node
    title           text not null,
    repo            text,                         -- owner/repo
    spec_ref        text,                         -- repo path or harness doc_id (no inline secrets)
    spec_inline     text,                         -- inline spec text when no ref
    target_head     dispatch_head not null default 'either',
    priority        smallint not null default 100, -- lower = sooner
    state           dispatch_state not null default 'pending',
    not_before      timestamptz not null default now(),
    claimed_by      text,                         -- receiver/agent id
    claimed_at      timestamptz,
    lease_expires_at timestamptz,
    attempts        smallint not null default 0,
    max_attempts    smallint not null default 3,
    result          jsonb,                        -- pr url, summary, error
    source_task_id  text,                         -- TickTick task id for milestone relay
    created_at      timestamptz not null default now(),
    updated_at      timestamptz not null default now()
);

-- the claim query reads only eligible rows in priority order
create index dispatch_jobs_claimable
    on dispatch_jobs (priority, not_before)
    where state = 'pending';

-- the reaper reads only leased rows
create index dispatch_jobs_leased
    on dispatch_jobs (lease_expires_at)
    where state in ('claimed','running');
```

---

## The claim (atomic, SKIP LOCKED)

One statement: select the next eligible job, flip it to `claimed`, set the lease. The state flip and the lease set are in the same statement so there is never a claimed-but-unleased window.

```sql
update dispatch_jobs
set state = 'claimed',
    claimed_by = $1,
    claimed_at = now(),
    lease_expires_at = now() + $2::interval,   -- e.g. '10 minutes'
    attempts = attempts + 1,
    updated_at = now()
where job_id = (
    select job_id from dispatch_jobs
    where state = 'pending'
      and not_before <= now()
      and (target_head = $3 or target_head = 'either')  -- $3 = this worker's head
    order by priority, not_before
    for update skip locked
    limit 1
)
returning *;
```

`for update skip locked` guarantees two concurrent receivers never claim the same job. Returns zero rows when nothing is eligible; the worker then waits for the next wake or poll tick.

## The lease reaper

A periodic sweep (for example every 30 seconds) requeues jobs whose worker died (lease expired) and dead-letters jobs past `max_attempts`.

```sql
-- requeue expired leases that still have attempts left
update dispatch_jobs
set state = 'pending', claimed_by = null, claimed_at = null,
    lease_expires_at = null, updated_at = now()
where state in ('claimed','running')
  and lease_expires_at < now()
  and attempts < max_attempts;

-- dead-letter expired leases out of attempts
update dispatch_jobs
set state = 'dead', updated_at = now()
where state in ('claimed','running')
  and lease_expires_at < now()
  and attempts >= max_attempts;
```

A long-running job renews its lease (`update ... set lease_expires_at = now() + interval, state='running'`) on a heartbeat shorter than the lease, so the reaper never reclaims a job that is still progressing.

---

## Files

```
rustyredcore_THG/crates/theorem-dispatch/         (new crate, shared by receiver + agentd)
  src/lib.rs        DispatchQueue: connect, submit, claim_next, renew_lease, complete, fail, reap
  src/model.rs      Job, JobState, Head, FailureClass
  migrations/0001_dispatch_jobs.sql

rustyredcore_THG/crates/theorem-receiver/         (drain loop, spawns the agent)
rustyredcore_THG/crates/theorem-agentd/           (relays milestones; can also drain)
apps/theorem-harness-server/                       (job_submit writes Postgres + THG + announces)
```

## Signatures (theorem-dispatch)

```rust
pub struct DispatchQueue { pool: sqlx::PgPool }

pub struct Job {
    pub job_id: String,
    pub title: String,
    pub repo: Option<String>,
    pub spec_ref: Option<String>,
    pub spec_inline: Option<String>,
    pub target_head: Head,
    pub source_task_id: Option<String>,
}

pub enum Head { Claude, Codex, Either }
pub enum FailureClass { Retryable, Fatal }

impl DispatchQueue {
    pub async fn connect(database_url: &str) -> Result<Self>;
    pub async fn submit(&self, job: Job, priority: i16) -> Result<String>;          // also mirror to THG (caller)
    pub async fn claim_next(&self, worker_id: &str, head: Head, lease: Duration) -> Result<Option<ClaimedJob>>;
    pub async fn renew_lease(&self, job_id: &str, lease: Duration) -> Result<()>;    // heartbeat
    pub async fn complete(&self, job_id: &str, result: serde_json::Value) -> Result<()>;  // state=done, in a txn
    pub async fn fail(&self, job_id: &str, class: FailureClass, error: serde_json::Value) -> Result<()>;
    pub async fn reap(&self) -> Result<ReapReport>;
}
```

## The drain loop (theorem-receiver)

The receiver is a local daemon holding one long-lived `PgPool` to the dispatch Postgres (Railway or Neon) plus a subscription to the harness SSE. Loop:

1. Wait on either an SSE wake or a poll tick (5s).
2. `claim_next(worker_id, head, lease)`. If `None`, go to 1.
3. Mirror the claim to the THG job node (`job_note` start_session_ref) so the board shows it running. Set state to `running` in Postgres and start the lease heartbeat.
4. Spawn the agent for the claimed head: `claude -p` with acceptEdits, or `codex` with workspace-write. Feed it `spec_ref`/`spec_inline`. Confirm the receiver's existing spawn entry point and reuse it; do not add a second spawn path.
5. On success: `complete(job_id, {pr_url, summary})`, mirror to THG (`job_note` + `job_archive` or a done state), and relay a milestone to TickTick if `source_task_id` is set.
6. On failure: `fail(job_id, class, error)`. Retryable increments attempts and returns the job to `pending` with backoff (`not_before = now() + backoff`); fatal or out-of-attempts goes to `dead`. Mirror to THG.

`agentd` can run the same drain loop for the Gemma-relay path, or stay a pure milestone relay; either way it shares `theorem-dispatch`.

## The submit path (harness server)

`job_submit` (the existing Dispatch v2 tool) gains a Postgres write alongside its current THG write:

1. Upsert the THG Dispatch v2 job node (current behavior, the canonical coordination record).
2. `DispatchQueue::submit` inserts the `dispatch_jobs` row with the same `job_id`.
3. Announce over the existing `RoomBus`/SSE so a listening receiver wakes immediately.

The two writes are not two-phase committed. Postgres is authoritative for execution state, THG for coordination. Mirror after each Postgres transaction commits, and let the reaper sweep reconcile any drift.

## Manual submission from a database console (Neon)

If the dispatch Postgres is hosted on Neon, the console SQL editor doubles as a job-submission surface: insert a row and the receiver runs it.

- The receiver connects on Neon's **direct** endpoint, not the `-pooler` one. PgBouncer in transaction mode disables session-scoped features including LISTEN/NOTIFY, so the pooled string cannot carry a wake and is the wrong endpoint for one stable long-lived connection. Wrap the direct connection in a client-side pool so it reconnects cleanly across Neon's scale-to-zero.
- A console `INSERT` bypasses `job_submit`, so it fires no SSE wake and creates no THG coordination node at submit time. The receiver's poll catches it, and the drain loop's claim step (`job_note` to THG) backfills the coordination node at claim time. For lower latency than the poll interval, add a trigger on `dispatch_jobs` that `NOTIFY`s on insert and have the receiver `LISTEN` (works only on the direct endpoint).
- A console insert must satisfy the table constraints: `job_id` (any unique id), `title`, and `target_head`. Keep a saved query in the console as the submission template.
- A receiver watching the database (poll or held LISTEN) keeps Neon's compute from scaling to zero. That is expected and fine for a dispatch database; do not rely on scale-to-zero while the loop is running.

```sql
insert into dispatch_jobs (job_id, title, repo, spec_ref, target_head, priority)
values (gen_random_uuid()::text, 'fix the thing', 'Travis-Gilbert/Theorem', 'docs/plans/x/HANDOFF.md', 'either', 100);
```

---

## Acceptance criteria (observable)

1. Submit N jobs; the receiver drains them; each `claude -p`/`codex` job runs exactly once. Verified by asserting no `job_id` is spawned twice under two concurrent receivers.
2. Kill the receiver mid-job. The lease expires, the reaper returns the job to `pending`, a restarted receiver claims it, and the job completes. No lost or permanently stuck jobs.
3. A failing job retries up to `max_attempts` then dead-letters, visible as a row with `state = 'dead'`.
4. The board is inspectable: `select state, count(*) from dispatch_jobs group by state` shows the live distribution at any moment.
5. Job state mirrors to the THG Dispatch v2 node; `coordination_context` reflects the same job as running/done.
6. With SSE connected, a submitted job is claimed within wake latency. With SSE disconnected, the 5s poll still claims it. No job is stranded in either case.
7. A long job renewing its lease on a heartbeat is never reclaimed by the reaper while it is still running.

---

## Cautions and dependencies

- The local receiver connects to the dispatch Postgres (Railway or Neon) over the network. Use one long-lived `PgPool` per receiver; do not open a connection per claim. On Neon, use the direct endpoint, not the pooler (see Manual submission).
- The claim must set `lease_expires_at` in the same statement as the state flip (shown above). A two-step claim-then-lease has a crash window where a job is claimed but unleased and the reaper cannot recover it.
- Mirror to THG is best-effort and eventually consistent. Do not block a Postgres commit on the THG write; mirror after commit and reconcile on the reaper sweep.
- No secrets in `spec_ref`/`spec_inline`/`result`. Resolve credentials at spawn time, matching the existing `IngestJobRequest` no-secrets-in-the-request rule.
- `database_url` for the receiver is an env var on the local machine, not committed. The harness server reads its own from the dispatch Postgres service binding (the Railway Postgres binding, or the Neon connection string).

## Sequencing

Build `theorem-dispatch` (schema, claim, lease, reaper) first with a fake spawn that just sleeps and reports, and prove acceptance criteria 1, 2, 3, 4, 6, 7 against it. Then wire the real `claude -p`/`codex` spawn in `theorem-receiver` by reusing its existing entry point. Then add the Postgres write to `job_submit` and the THG mirror. The RustyRed-native claim primitive that would let you drop Postgres later is out of scope here and gated on hardening the claim path against the audit's TOCTOU and mutex findings.

## Implementation receipt (2026-06-17)

Implemented:

- Added `theorem-dispatch` with the `dispatch_jobs` migration, `sqlx` `PgPool`, atomic `FOR UPDATE SKIP LOCKED` claim, lease renewal, completion, retry/dead-letter failure handling, reaper, and state counts.
- Added optional external `job_id` support to THG `JobSubmission` so manual Postgres rows and THG board threads can share identity.
- Wired `theorem-receiver` to use the Postgres queue when `THEOREM_DISPATCH_DATABASE_URL` is set, with 5s poll backstop, reaper cadence, heartbeat renewal, board backfill, set-once `job_note` start guard, and existing spawn path reuse.
- Added harness-server `POST /harness/jobs` and `GET /harness/jobs/counts`; the submit route writes the THG board, mirrors the same job to Postgres, then publishes a wake message through the existing `RoomBus`/SSE path.
- Added Postgres mirroring to the product MCP `job_submit` path when `THEOREM_DISPATCH_DATABASE_URL` is configured.

Validation:

- `cargo check -p theorem-dispatch -p theorem-receiver`
- `cargo test -p theorem-dispatch`
- `cargo test -p theorem-harness-core job`
- `cargo test -p theorem-harness-runtime job_queue`
- `cargo test -p theorem-receiver`
- Local temporary Postgres: `THEOREM_DISPATCH_TEST_DATABASE_URL=... cargo test -p theorem-dispatch live_postgres_acceptance_claim_reap_retry_counts_and_heartbeat -- --nocapture`
- `cargo test -p rustyred-thg-mcp job_submit`
- `cargo test` in `apps/theorem-harness-server`
- `git diff --check`

Boundary:

- `theorem-agentd` remains a milestone relay; the shared drain implementation is in `theorem-receiver`.
- A later broad `cargo check -p rustyred-thg-mcp -p rustyred-thg-server` was blocked by unrelated dirty work in `rustyred-thg-core/src/versioned_graph.rs` (`IncrementalGraphPack` missing `commit_cost`). The dispatch-specific check still passed after that surfaced.
