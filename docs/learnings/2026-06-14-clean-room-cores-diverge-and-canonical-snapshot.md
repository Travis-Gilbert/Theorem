# The two clean-room cores diverge; CRDT tests need canonical_snapshot

**Kind:** gotcha
**Captured:** 2026-06-14
**Session signature:** `cc-session-84baa4a7-e608-46a4-9a5c-748b0572c3b2`
**Domain tags:** rust, thg, crdt, testing, clean-room

## Trigger

Two distinct surprises while writing CRDT acceptance tests:

1. A graph-sync restart test used `join_delta(&mut RedCoreGraphStore, ...)` and
   failed to compile: `the trait bound RedCoreGraphStore: GraphStore is not
   satisfied` — in the DB repo's `rustyred-core`. But Theorem's
   `rustyred-thg-core` RedCoreGraphStore DOES implement GraphStore (CLAUDE.md
   even documents it). The two clean-room sibling cores diverge on trait impls.

2. A property test comparing two replicas via raw `store.snapshot()` would have
   failed on per-replica `version` counter drift even when the graphs were
   logically converged. Codex had already added a `canonical_snapshot` helper
   (zeros `version`/`content_hash`/`parent_hashes`) for exactly this.

## Rule

Do not assume the two clean-room cores (Theorem `rustyred-thg-core` vs DB
`rustyred-core`) expose identical trait impls — `RedCoreGraphStore: GraphStore`
holds in Theorem but NOT in the DB core, so generic `join_delta`/`diff_since`
won't take a DB-core RedCoreGraphStore (use InMemoryGraphStore there). When
asserting CRDT convergence, compare through `canonical_snapshot` (strips version
counters and content hashes), never raw `snapshot()` equality.

## Evidence

- `cargo test -p rustyred-server`: `error[E0277]: the trait bound RedCoreGraphStore: GraphStore is not satisfied` (a33 test).
- DB `rustyred-core/src/lib.rs` exports `RedCoreGraphStore` but its `graph_store.rs` does not `impl GraphStore for RedCoreGraphStore`.
- `crdt/merge.rs` tests use `canonical_snapshot(left.snapshot())` for replica equality.

## Encoded in

- `docs/learnings/2026-06-14-clean-room-cores-diverge-and-canonical-snapshot.md` (this file)
