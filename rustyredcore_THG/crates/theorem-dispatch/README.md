# theorem-dispatch

Postgres hot-execution queue for Dispatch v2. The THG board stays canonical for coordination; this crate owns only hot execution state: claim leases, retries, completion, and dead-letter visibility. Lib name `theorem_dispatch`.

## Key API

- `DispatchQueue`: `connect(database_url)`, `from_pool(PgPool)`, `migrate`, `submit(job, priority)`, `claim_next(worker_id, head, lease)`, `claim_next_for_heads`, `renew_lease`, `complete`, `fail(job_id, FailureClass, error) -> JobState`, `reap() -> ReapReport`, `state_counts()`. Claims use `FOR UPDATE SKIP LOCKED` ordered by `(priority, not_before, created_at, job_id)`; retry backoff is `least(900, greatest(30, attempts*30))` seconds.
- `model.rs`: `Head { Claude, Codex, Either }`, `JobState { Pending, Claimed, Running, Done, Failed, Dead }`, `FailureClass { Retryable, Fatal }`, `Job`, `ClaimedJob` (`into_harness_job`), `ReapReport`, `StateCount`; harness bridges `Job::from_harness`/`into_harness_submission`. Errors: `DispatchError`.

Embeds `migrations/0001_dispatch_jobs.sql` via `include_str!` and runs it on connect. Path dep: `theorem-harness-core`. Other: `sqlx 0.8` (postgres), `time`, `thiserror`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p theorem-dispatch
```

The offline test parses harness timestamps. The live acceptance test (claim/reap/retry/heartbeat) self-skips unless `THEOREM_DISPATCH_TEST_DATABASE_URL` points at a Postgres.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
