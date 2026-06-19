# Context Artifact - Stream-Based Coordination (substrate slice)

**Harness run:** `multihead:unix_ms:1781845659652` (tenant `Travis-Gilbert`, actor `claude-code`)
**Context Brief signature:** `a3443ba455b836b3e86831371e3acdb23eec64f1768be17dcff1b9f78b2c6f69`
**Repo / branch:** `Theorem` @ `Travis-Gilbert/stream-based-coordination` (cut from `origin/main` `069c6c7`)
**Worktree:** `Creative/Website/Theorem-jobintel-main-ship` (the `Creative/Website/Theorem` main worktree is untouched)

## Decision

Coordination moves from turn-start room polling to an append-only, cursor-read
event stream. The native primitive lives in **`rustyred-thg-core/src/stream.rs`**
and is embedded in **`ThgState`** alongside `runs`/`contexts`/`patches`. CRDT
stays on the graph (`rustyred-thg-core/src/crdt/`); streams carry communication
and awareness, not shared state.

### Why this layering
- Durable per-`(actor, stream)` cursors require persistence, and
  `StoreBackedThgExecutor` only persists `ThgState`. Embedding the stream store
  there gives durable cursors for free and makes every publish advance the
  `state_hash`, exactly like `runs`/`contexts`/`patches`.
- The stream log is the **append-only special case of `ordered.rs`**: where
  `OrderedIndex` keys an `imbl::OrdMap` by score, a stream keys an
  `imbl::OrdMap<u64, StreamEvent>` by a monotonic ordering token and reads the
  tail after a cursor with the same range-after machinery. `imbl` throughout
  keeps `ThgState` clones copy-on-write.

### Confirmed against the post-commit tree
- Target crate is `rustyred-thg-core` (the spec's `rustyred-thg-core`, confirmed
  on `main`, **not** the standalone `RustyRed-Graph-Database`, and not the older
  `jobintel-main-ship` checkout which is 45 commits behind `main`).
- `ordered.rs` exists on `main` and is the structure the stream is analogous to.
- `sanitize_tenant_segment` (`graph_store.rs`) is a **reversible percent-encoder**
  (`pct_` prefix); it never collapses a non-empty tenant to a default. So the
  spec's "empty tenant rejected rather than defaulted" is enforced by an explicit
  empty/whitespace rejection in `resolve_stream_key`; no new scope resolver is
  added.

## What was built (this slice)

| File | Change |
|---|---|
| `crates/rustyred-thg-core/src/stream.rs` | **New.** `StreamEvent`, `StreamUrgency` (`info`/`ask`/`block`), `StreamStore` (append-log + cursors + subscriptions + pending-ping queue), `resolve_stream_key`, full unit tests. |
| `crates/rustyred-thg-core/src/state.rs` | `ThgState.streams: StreamStore` (`#[serde(default)]`). |
| `crates/rustyred-thg-core/src/commands.rs` | 5 `ThgCommand` variants + `from_name`/`name` -> `RUSTYRED_THG.STREAM.{PUBLISH,READ,SUBSCRIBE,UNSUBSCRIBE,MENTIONS}`. |
| `crates/rustyred-thg-core/src/executor.rs` | 5 handlers (writes via `state_mut()`), dispatch arms, command-level integration test. |
| `crates/rustyred-thg-core/src/lib.rs` | `pub mod stream;` + re-exports. |

### Observable command surface (the tool contract, at the core level)
- `RUSTYRED_THG.STREAM.PUBLISH` `(tenant, stream, actor, kind, payload, urgency=info, target_actor?)` -> `{event_id, ordering_token, stream_key, urgency, pinged}`
- `RUSTYRED_THG.STREAM.READ` `(actor, tenant, streams[]?, advance=true, limit?)` -> `{events, new_cursors, count, advanced}`
- `RUSTYRED_THG.STREAM.SUBSCRIBE` / `UNSUBSCRIBE` `(actor, tenant, stream)` -> `{subscriptions}`
- `RUSTYRED_THG.STREAM.MENTIONS` `(actor, tenant, advance=true)` -> `{mentions, count, drained}` - the tenant-scoped drain seam for the warm-head Stop-hook / cold-head wake.

### Semantics decisions
- **Ordering token:** global monotonic `StreamStore.seq`; every event id is unique
  and each per-stream subsequence is strictly increasing. A single `&mut`
  executor serializes appends into a total order, no merge step.
- **Subscription = attend now:** a first-time subscriber's cursor is initialized
  to the stream head, so it receives future events, not the full backlog.
  Re-subscribe resumes from the stored cursor. Historical replay needs a future
  cursor-override API; this slice intentionally exposes delta reads only.
- **Ping = publish with `ask`/`block` + `target_actor`:** lands on the stream AND
  enqueues a `(stream_key, ordering_token)` ref to the target's pending-ping
  queue, bypassing subscription attention. `MENTIONS` drains it.

## Acceptance criteria -> coverage

All five are covered by tests in `stream.rs` (primitive) and replayed through the
command surface in `executor.rs`:

1. Offline-for-N-turns delta after cursor, in order, no miss/dup -> `offline_head_pulls_exact_delta_after_cursor`.
2. Ping appears in target's mention drain -> `ping_lands_in_targets_mention_drain` + command test.
3. Concurrent publishers get distinct tokens, single total order, no merge -> `concurrent_publishers_get_distinct_tokens_in_total_order`.
4. Publish+read share a stream under a tenant; empty tenant rejected -> `tenant_scope_shares_stream_and_rejects_empty` + command test.
5. Subscribe/unsubscribe changes read deltas; ping reaches an unsubscribed target -> `attention_controls_reads_but_ping_bypasses_it`.

## Verify

```bash
cd rustyredcore_THG
cargo test -p rustyred-thg-core --lib stream     # 12 tests
cargo test -p rustyred-thg-core --lib            # 197 pass, 0 fail
cargo clippy -p rustyred-thg-core --lib          # completes; baseline warnings remain outside stream.rs
```

## Explicit scope boundary - remaining integration (NOT a silent cut)

This slice delivers the native primitive + the observable core command surface +
the ping/mention seam, all tested. The following wiring is required to make
streams the *default coordination transport across the running system* and is
deliberately surfaced for consent rather than assumed:

1. **MCP tool names.** Expose `stream_publish` / `stream_read` /
   `stream_subscribe` / `stream_unsubscribe` in
   `crates/rustyred-thg-mcp/src/lib.rs` as thin wrappers over the
   `RUSTYRED_THG.STREAM.*` commands, beside the existing `coordination_*` tools.
2. **Tenant normalizer at the boundary.** Have the MCP / runtime layer pass a
   tenant already canonicalized through `theorem-harness-runtime::tenant::normalize_tenant_slug`
   before calling in (the core layer reuses `sanitize_tenant_segment` and rejects
   empty).
3. **Wake bridge.** Connect the pending-ping queue / `STREAM.MENTIONS` drain to
   `theorem-receiver/src/wake.rs` (+ `coordination_push.rs` `RoomEventBus`) so a
   cold head's ping triggers the existing spawn/courier wake and a warm head's
   ping drains on the Stop hook.
4. **Migration of room-poll readers.** Move `read_intents_for_room` /
   `read_messages_for_room` consumers onto cursor reads; keep durable records as
   graph nodes that *also* publish a stream event.
