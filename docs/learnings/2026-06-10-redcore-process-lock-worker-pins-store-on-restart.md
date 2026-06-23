# RedCore's process-global dir lock + a worker holding the store Arc breaks in-process restart

**Kind:** postmortem
**Captured:** 2026-06-10
**Session signature:** `claude-code:travisgilbert@Theorem:73b72efc`
**Domain tags:** rust, thg, redcore, graphstore, concurrency, testing

## Trigger

A durable-jobs test that simulates a restart by `drop(first)` then
re-`try_new_at(same_dir)` panicked at the reopen `.unwrap()`:

```
called `Result::unwrap()` on an `Err` value: CodeIndexError {
  code: "redcore_lock_unavailable",
  message: "RedCore data directory /var/.../theorem-code-index-store-... is
            already open in this process" }
```

Two facts combined to cause it:

1. `RedCoreGraphStore::open` registers the data dir in a process-global
   `static REDCORE_PROCESS_LOCKS: OnceLock<Mutex<BTreeSet<PathBuf>>>`
   (`graph_store.rs:1896`). The path is removed only when the store's
   `RedCoreDirectoryLock` drops, i.e. when the last `Arc<Mutex<RedCoreGraphStore>>`
   strong ref drops. Opening the same dir twice in one process is a hard error.
2. The ingest worker thread's `run_ingest_job` upgraded the `Weak` store to a
   strong `Arc` and kept it in scope THROUGH `record_event(CommitDone)` and
   `record_event(Finished)`. So when the test observed `Finished` and dropped
   the runtime, the worker still held a strong store ref -> the dir lock was
   not released -> the reopen failed.

## Rule

In a background worker that upgrades a `Weak<Mutex<RedCoreGraphStore>>`, scope
the strong `Arc` to the smallest critical section (e.g. just the
`commit_batch`), never across subsequent event recording or idle waits, so a
dropped runtime releases the RedCore directory lock promptly. And: any test
that simulates a process restart in ONE process must retry the reopen on
`redcore_lock_unavailable` (a real restart is a fresh process where the
process-global lock never conflicts; the in-process simulation can race the
prior worker's transient store handle).

## Evidence

- `rustyredcore_THG/crates/rustyred-thg-core/src/graph_store.rs:1896` —
  `REDCORE_PROCESS_LOCKS`; `:2761` — `impl Drop for RedCoreDirectoryLock`.
- `rustyredcore_THG/crates/rustyred-thg-code/src/ingest_jobs.rs` —
  `run_ingest_job` commit block now scopes the upgraded `Arc`.
- `rustyredcore_THG/crates/rustyred-thg-code/src/lib.rs` —
  `open_runtime_with_retry` test helper; commit `73b72efc`.

## Encoded in

- `docs/learnings/2026-06-10-redcore-process-lock-worker-pins-store-on-restart.md` (this file)
