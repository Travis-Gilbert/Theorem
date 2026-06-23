# A green suite with descriptive test names is not acceptance coverage; read the test bodies

**Kind:** anti_pattern
**Captured:** 2026-06-14
**Session signature:** `claude:travisgilbert@Traviss-Laptop:b944c683`
**Domain tags:** verification, testing, rust, frontier

## Trigger

Verifying Codex's frontier against the handoff's six acceptance criteria, `cargo test`
reported 123 passing. Two test names looked like they covered the hardest criteria, but
the bodies did not: `next_batch_claims_each_url_once` calls `next_batch(1)` twice
SEQUENTIALLY (proves a claimed node leaves the frontier, not AC2's "two concurrent
workers never double-fetch under load"), and `ppr_prioritizer_ranks_central_node`
asserts only `scores[b] > 0.0` (not AC4's "PPR produces a different order than depth").
AC5 (fetcher swap) and AC6 (queryable mid-crawl) had no test at all. Green plus
plausible names masked three partial criteria and one missing one.

## Rule

When verifying an implementation against a spec's acceptance criteria, map each
criterion to its asserting test and READ that test's body - never accept a test name as
proof. Specifically distrust: a "claims/locks once" test that never spawns concurrent
actors (a concurrency claim needs a multi-thread runtime and N racing tasks), and a
"ranks/scores X" test whose assertion is `> 0` rather than a relative ordering. Close
the gaps with new tests in `tests/` (external integration tests are non-colliding when
another agent owns `src/`). Here that meant four tests proving AC2 (8 workers / 200 URLs
/ no double-claim), AC4 (PPR ranks the deep-central hub above shallow nodes while depth
ranks it below), AC5 (two link orderings -> identical graph), and AC6 (mid-crawl
neighbors + PPR).

## Evidence

- `next_batch_claims_each_url_once`: body is `next_batch(1).await` twice +
  `assert!(second.is_empty())` - single-threaded.
- `ppr_prioritizer_ranks_central_node`: body asserts `scores[&b] > 0.0` only; a uniform
  PPR would also pass it.
- New `tests/frontier_acceptance.rs`: 4 tests, all green; full crate suite 127 passing
  (109 lib + 12 fixture + 2 parity + 4 acceptance).

## Encoded in

- `docs/learnings/2026-06-14-green-tests-are-not-acceptance-coverage.md` (this file)
