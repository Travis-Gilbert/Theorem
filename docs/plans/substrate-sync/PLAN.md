# PLAN: Substrate sync — implementation checklist

**Spec:** [`docs/plans/substrate-sync/SPEC.md`](./SPEC.md)
**Predecessor:** [`docs/plans/theorem-desktop/phase-3-sync.md`](../theorem-desktop/phase-3-sync.md)
**Status:** open (no PT row complete; coordination mirror degraded — see §6)

## Executive summary

Land a three-role local↔Railway substrate sync: Prolly version-pack rounds
as the durable convergence floor, a stream tail riding the existing harness
`stream_publish` / `stream_read` / `stream_subscribe` verbs for low-latency
freshness, and a small CRDT merge registry inside the existing Prolly merge
path for the handful of contested fields that need it. Ships as a new
standalone crate (`apps/theorem-substrate-sync`) plus targeted additions to
`rustyred-thg-core` (merge registry, post-commit hook event emission) and
the launcher (`apps/theorem-proxy/scripts/start-proxied-session.sh`). No
new transport, no parallel CRDT store, no MCP verbs added.

## Production goal

| Lens | What "done" looks like |
|---|---|
| **User-visible** | A memory written on any surface (claude.ai, Codex on Railway, local Claude Desktop) is reflected on every other surface within 5s for routine state and within 1s for stream-eligible state. Cold local node onboards from Railway in one bootstrap call. |
| **System** | New crate `theorem-substrate-sync` running as a sidecar of the local stack; outbound HTTPS connection to Railway's `/mcp`; local writes flow through the existing `rustyred-thg-core::hooks` surface into a Valkey outbox; a drainer publishes to Railway. |
| **Data** | Both ends converge on the same Prolly head ref every round. Contested fields (tags, status, confidence) merge via the registry's CRDT strategies. Version log on both ends preserves the unmerged sides for audit. |
| **Operational** | Tier seam: `sync_enabled = false` by default; zero version-pack traffic when off. Status endpoint reports `{connected, last_round, last_event, stream}` for the doctor command. Restart-safe via Valkey cursors and outbox. |

## Grounding corrections to the spec

Two corrections required after live-repo verification; the checklist below
uses the corrected forms.

| Spec assertion | Live state | Resolution |
|---|---|---|
| MCP verbs named `graph_version_compile` / `_diff` / etc. | Names are `rustyred_thg_graph_version_*` per `rustyred-thg-mcp/src/lib.rs:1446-1561` | Plan rows use the live names |
| New crate `apps/theorem-harness-sync` | `apps/obsidian-sync/manifest.json` already reserves id `theorem-harness-sync` | New crate renamed to `apps/theorem-substrate-sync` |

A small spec patch follows the plan landing; not blocking.

## Checklist

### Role 1 — Prolly rounds (the floor)

| ID | Subject | Files touched | Acceptance | Validation | Risk |
|---|---|---|---|---|---|
| **PT-001** | Scaffold `apps/theorem-substrate-sync` standalone crate with bin + lib + axum status endpoint | `apps/theorem-substrate-sync/{Cargo.toml,src/main.rs,src/lib.rs,src/status.rs}`; bare `[workspace]` like `apps/theorem-proxy` | `theorem-substrate-sync --version` prints version; `GET /status` returns `{connected: false, sync_enabled: false}` | `cargo check --manifest-path apps/theorem-substrate-sync/Cargo.toml` clean; `curl 127.0.0.1:<port>/status` returns the JSON | Crate boundaries: do not pull into `rustyredcore_THG/Cargo.toml` workspace; copy `apps/theorem-proxy/Cargo.toml`'s pattern |
| **PT-002** | Railway auth: load tenant token from file (env override), inject as `Authorization: Bearer` on every outbound MCP call | `apps/theorem-substrate-sync/src/railway_client.rs`; default token path `~/.theorem-substrate-sync/tenant-token`; pattern lifted from `apps/obsidian-sync/src/` request shape | `theorem-substrate-sync doctor` reports `connected` against Railway with a valid token; `disconnected` with no token; `token-invalid` with a bad token | Integration test using a fake HTTP server (axum test harness) confirms the Authorization header is sent; live smoke against Railway with a real token | Tenant-token format change: token loader is a thin trait, swappable for an alternative (env var) |
| **PT-003** | Prolly round driver: compile→diff→merge→push→checkout via `rustyred_thg_graph_version_*` verbs at 30s interval | `apps/theorem-substrate-sync/src/round.rs`; consumes MCP verbs at `rustyred-thg-mcp/src/lib.rs:1446` (compile), `:1460` (diff), `:1481` (ref), `:1518` (log), `:1529` (checkout), `:1543` (merge); merge strategy `auto_confidence` from `rustyred-thg-core/src/versioned_graph.rs:2059` | `harness_kg_status` on Railway reports a locally-written `MemoryItem` within one round interval; the reverse holds (Railway-written item appears locally within one round) | Two-node integration test: spin a local node + a fake Railway node (in-memory `GraphStore`), write on one, run one round, assert convergence; record receipt in JSON | The current MCP verb signatures (line refs above) may require schema reads — write a small wrapper that asserts response shape on each verb at startup |
| **PT-004** | Atomic bootstrap from an empty local node | `apps/theorem-substrate-sync/src/bootstrap.rs` invokes `rustyred_thg_graph_version_ref` then `_compile` then local `_checkout`; touches `rustyred-thg-core/src/versioned_graph.rs` only if a public bootstrap helper is missing | Cold local node (`nodes_total: 0`) onboards in one shot; no readers ever see partial state; `nodes_total` matches Railway's head after bootstrap | Integration test: fresh in-memory local node, run bootstrap, observe `nodes_total` jumps atomically from 0 to N; intermediate reads either see 0 or see N | Apply-mid-bootstrap race: take a write lock on the local store for the duration of the checkout; rollback to the prior head ref on failure |
| **PT-005** | Tunable round interval — 5s when local activity, 30s idle, manual trigger via status endpoint | `apps/theorem-substrate-sync/src/scheduler.rs`; `local_activity` signal from the post-commit hook (see PT-007) | Idle-no-traffic round produces an empty Prolly diff and no version bump; busy interval drops to 5s within 1s of a local mutation; `POST /trigger` runs one round on demand | Unit test on the scheduler with a fake clock; integration test that posts a write, asserts the next round fires within 5s; doctor command shows the current interval | Thrash from chatty hooks: debounce the activity signal (notify-debouncer-full pattern from `apps/commonplace-desktop-runtime`) |
| **PT-006** | Tier seam: `sync_enabled` config flag + launcher integration | `apps/theorem-substrate-sync/src/config.rs`; new env `THEOREM_SYNC_ENABLED` read by `apps/theorem-proxy/scripts/start-proxied-session.sh` to spawn the daemon under EXIT trap | With `sync_enabled = false` (default), zero version-pack traffic occurs; the daemon exits cleanly; the launcher does not spawn it | Two launcher runs: one with `THEOREM_SYNC_ENABLED=1` shows the daemon process; one without shows no process and no outbound HTTPS to Railway | Default-off discipline: an integration test asserts no outbound socket opens when the flag is off |

### Role 2 — stream tail (freshness)

| ID | Subject | Files touched | Acceptance | Validation | Risk |
|---|---|---|---|---|---|
| **PT-007** | Post-commit hook in `rustyred-thg-core::hooks` emits each committed mutation as a flat event into the Valkey outbox `sync:outbox:<tenant>` | `rustyred-thg-core/src/hooks.rs` (extend `MutationEvent` consumer to include a new SubstrateSyncHook); `apps/theorem-substrate-sync/src/outbox.rs` | A local `upsert_note` lands in `LLEN sync:outbox:travis-gilbert` within 100ms; event shape preserves op kind, node id, property delta, HLC stamp | Unit test on `rustyred-thg-core` hooks with a fake Valkey client; integration test does the full local write → outbox read round-trip | Hook registration order: register after the merge-registry hook (PT-012) so registry-resolved property writes get the post-resolve value |
| **PT-008** | Outbox drainer publishes events to Railway via `stream_publish` (MCP verb at `rustyred-thg-mcp/src/lib.rs:1689`); pops on success, exponential backoff on failure | `apps/theorem-substrate-sync/src/drainer.rs` | A queued local write reaches Railway within 1s on a healthy connection; on connection failure, the item stays in the outbox; on terminal auth failure, the status endpoint reports `outbox: blocked` | Integration test with a flaky fake Railway: assert the item is published exactly once after recovery; smoke against Railway confirms `stream_read` on the other side sees the event | Idempotency: each event carries a content-hash key; if Railway's stream rejects a duplicate, drainer pops without retry |
| **PT-009** | Daemon `stream_subscribe` on Railway's tenant stream; apply incoming events as local mutations through the local node's MCP | `apps/theorem-substrate-sync/src/subscriber.rs`; consumes `stream_subscribe` at `:1703` and `stream_read` at `:1698` | A Codex edit on Railway lands in the local node in under 1s (95th percentile, healthy network) | End-to-end timed test: publish on Railway, measure local-apply latency over 100 events; assert P95 < 1000ms; doctor reports `stream: connected` with last event timestamp | Self-loop: the event our PT-008 drainer published returns through subscribe; content-hash dedup at the local apply path makes it a no-op |
| **PT-010** | Cursor persistence in Valkey + reconnect resume | `apps/theorem-substrate-sync/src/cursor.rs`; keys `sync:cursor:<tenant>`, `sync:last_round:<tenant>`, `sync:last_head:<tenant>` | Daemon restart resumes from saved cursor; no event applied twice (dedup by content hash); reconnect after disconnect picks up the gap | Restart test: stop daemon mid-stream, restart, count applied events; assert exact-once on the timeline | Cursor staleness: if the saved cursor is older than Railway's stream retention, fall back to Role 1 round on next tick; surface as a status warning |
| **PT-011** | Stream-down degradation: when stream verbs are unreachable, status reports `stream: disconnected` but Role 1 rounds continue; no data loss | `apps/theorem-substrate-sync/src/status.rs`; reconcile signal between subscriber and round driver | Block the stream path (firewall the publish call): local writes still converge to Railway via the next Role 1 round; status endpoint correctly reports `stream: disconnected` | Integration test: kill the fake stream server, observe drainer/subscriber back off, observe round driver continues to converge state, observe status accuracy | Backoff thrash: cap exponential backoff at 30s; expose as `stream_retry_after_ms` in status |

### Role 3 — CRDT merge registry (policy inside Prolly)

| ID | Subject | Files touched | Acceptance | Validation | Risk |
|---|---|---|---|---|---|
| **PT-012** | `MergeRegistry` surface in `rustyred-thg-core` with default `auto_confidence` fallback (no behavior change on day one) | `rustyred-thg-core/src/merge_registry.rs` (new); wire-in at `rustyred-thg-core/src/versioned_graph.rs` near `resolve_auto_confidence_edge` (`:2059`) | An empty registry produces identical merge results to the predecessor — phase-3-sync acceptance criterion 2 still holds | `cargo test -p rustyred-thg-core versioned_graph` unchanged; new unit test creates an empty registry, runs the existing merge fixtures, asserts byte-equal output | Default safety: the registry's `Default` impl IS empty (no entries) so an unwired call site falls through to `auto_confidence` |
| **PT-013** | LWW register strategy for `MemoryItem.status` and `MemoryItem.confidence` | `rustyred-thg-core/src/merge_registry.rs::Strategy::Lww`; registry entry registration in the merge entrypoint | Concurrent edits to the same `status` on both sides converge to the latest HLC stamp; the unmerged version is reachable through the version log per existing `versioned_graph` history | Integration test: two-side divergent writes to `status`, run a merge round, assert the winning value matches the higher HLC stamp; assert both originals are reachable in `_log` | HLC clock skew: the local node's HLC is generated by the same `rustyred-thg-core` HLC source as Railway's; verify both ends agree on the same monotonic ordering in a fixture |
| **PT-014** | OR-set strategy for `MemoryItem.tags` | `rustyred-thg-core/src/merge_registry.rs::Strategy::OrSet`; registry entry registration | Concurrent tag adds and removes from both sides reconcile without tombstone-order bugs; the SPEC §7 step 8 falsifying case (add-then-remove on local concurrent with re-add on Railway) converges to `{A}` not `{}` | Integration test with the exact SPEC §7 step 8 scenario; assert final tag set is `{A}` | OR-set state size: each tag carries an add-set + tombstone-set; bound the tombstone retention to avoid unbounded growth (one-line policy: tombstones older than the last merged ref drop) |

### End-to-end + housekeeping

| ID | Subject | Files touched | Acceptance | Validation | Risk |
|---|---|---|---|---|---|
| **PT-015** | Wire the SPEC §7 8-step acceptance test as an integration test in the new crate | `apps/theorem-substrate-sync/tests/end_to_end.rs` | All 8 steps from SPEC §7 pass; on failure, the failing step number maps to the role per SPEC §7's diagnostic note | `cargo test --manifest-path apps/theorem-substrate-sync/Cargo.toml --test end_to_end` is green | Test flake from clocks: use a fixed seed for HLC stamps in the harness; bound timing assertions to generous percentiles |
| **PT-016** | Doc-drift refresh: add new crate to CLAUDE.md, refresh README sync line, run `scripts/check-doc-drift.sh --refresh` | `CLAUDE.md` (crate table), `README.md` (Last sync), `.harness/code-kg-manifest.json` via the script | `scripts/check-doc-drift.sh` reports `0 undocumented` after the run; CLAUDE.md table includes `theorem-substrate-sync` | Run the script; assert clean output | Skipping this lands as a doc-drift gate failure on the next session start |

## Validation (end-to-end)

The SPEC §7 acceptance test is the gating proof. Concretely:

```
cd apps/theorem-substrate-sync
cargo test --test end_to_end -- --nocapture
```

Each step prints which role it exercises. A failed step points at the role
to debug per the SPEC §7 diagnostic mapping.

## Non-goals (mirrored from SPEC §8 for execution-time reconciliation)

- No multi-tenant routing on the local node.
- No interactive merge UI.
- No parallel CRDT store; CRDT semantics live inside the Prolly merge.
- No SubstratePeer extended across the local↔Railway wire.
- No billing or tier-payment flow.
- No replacement of `seed-node.py` (orthogonal).

## How `/harness mode=execute` reconciles against this plan

- Each PT row's `Acceptance` column is the ONLY exit gate for that row.
  Marking complete without a concrete verification (integration test pass,
  doctor output, status JSON shape) is a planning bug.
- Rows are ordered by execution-safe dependency: PT-001 → PT-002 → PT-003
  is a hard chain (daemon scaffold → auth → round driver); Role 2 rows
  (PT-007+) depend on PT-001 only; Role 3 rows (PT-012+) are independent of
  the daemon entirely and can land in `rustyred-thg-core` first.
- `cargo check -p <crate>` and `cargo clippy -p <crate> --all-targets
  --no-deps -- -D warnings` are the per-row narrow validators per the
  active rust-engineering directive.
- Deferring any row requires a concrete one-line reason added to the
  matching `.harness/checklist.json` entry's `deferred_reason` field, not
  a silent skip.

## Degraded state

`Coordination Context: remote_unavailable` was reported on this run. The
planning skill specifies that this plan's `.harness/checklist.json` is
mirrored to the coordination substrate; that mirror is currently
**deferred until coordination is reachable**. The local
`.harness/checklist.json` is the source of truth meanwhile.

## References

- SPEC: [`docs/plans/substrate-sync/SPEC.md`](./SPEC.md)
- Predecessor plan: [`docs/plans/theorem-desktop/phase-3-sync.md`](../theorem-desktop/phase-3-sync.md)
- Reference impl for outbound poll pattern: [`apps/obsidian-sync/`](../../../apps/obsidian-sync/)
- MCP verb registry: `rustyred-thg-mcp/src/lib.rs:1446-1703`
- Merge engine: `rustyred-thg-core/src/versioned_graph.rs:2059`
- Hook surface: `rustyred-thg-core/src/hooks.rs`
- Launcher integration point: `apps/theorem-proxy/scripts/start-proxied-session.sh`
- Naming-collision avoidance: `apps/obsidian-sync/manifest.json` (reserved id)
