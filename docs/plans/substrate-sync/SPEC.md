# SPEC: Substrate sync — two-layer local↔hosted harness state convergence

**Status:** proposed (brainstormed 2026-06-29)
**Predecessor:** `docs/plans/theorem-desktop/phase-3-sync.md` (job-003)
**Related:** `docs/plans/obsidian-sync/README.md` (same connection pattern, different sink)

## 0. Why this spec exists

The local Theorem node (`rustyred-thg-server` over a RedCore data dir on the
SSD) starts empty. The canonical harness memory — every cross-surface
encode, coordination event, hippo node, epistemic shadow, etc. — lives in
the Railway-hosted `rustyredcore-theorem-production` tenant. Claude.ai,
Codex, and the existing Obsidian plugin all read FROM Railway. The local
node today carries no real substrate.

`docs/plans/theorem-desktop/phase-3-sync.md` already specifies a Prolly
version-pack sync mechanism for this. This spec **subsumes** that plan and
adds a second, near-real-time layer on top so the local node can carry the
substrate as a live peer, not a periodic mirror.

## 1. Scope

| In scope | Out of scope |
|---|---|
| Bidirectional convergence between local node and Railway for one tenant | Multi-tenant routing on the local node (one tenant per local node) |
| Live (sub-second) propagation of structured ops, presence, awareness, text edits across Codex / Claude / other peers | Interactive merge UI for unresolved conflicts |
| Periodic (5–30s) Prolly version-pack rounds as the convergence floor | Billing / tier gating (defer to existing `sync_enabled` config flag) |
| Offline-safe writes via a local Valkey-backed outbox | A graph view UI (standing fence per phase-3-sync) |
| Bootstrap of an empty local node from Railway's current head | Schema translation (both ends are RedCore) |
| Distribution: each user runs against their own Railway tenant | Cloud-multi-tenant Railway provisioning |

## 2. The three roles

Three concerns, each held to exactly its job. Not three parallel stores.

### Role 1: Prolly version packs — the store, the floor, the truth

Inherits everything from `docs/plans/theorem-desktop/phase-3-sync.md`. The
mechanism: compile local state to a content-addressed pack, pull Railway's
head pack, three-way merge with the configured strategy (see Role 3),
push the merged pack to Railway, checkout locally. Idempotent on content
hash, so an interrupted round just re-runs and converges.

**This is the durable versioned truth.** It is also the offline-merge
engine, the audit log, the branch/time-travel machinery, and the ONLY
place that does convergence. If a stream message is dropped, the next
Prolly round catches it; if the merge round happens at all, both sides
land at the same content hash.

MCP verbs both ends already speak:

| Verb | Used for |
|---|---|
| `rustyred_thg_graph_version_compile` | snapshot current state to pack |
| `rustyred_thg_graph_version_diff` | O(delta) delta between two refs |
| `rustyred_thg_graph_version_merge` | three-way merge with strategy |
| `rustyred_thg_graph_version_checkout` | apply pack as current state |
| `rustyred_thg_graph_version_log` | history of refs |
| `rustyred_thg_graph_version_ref` | named pointers (heads, tags) |

Default cadence: every 5 seconds with local activity, every 30 seconds
idle, plus on app launch and manual trigger.

### Role 2: Valkey event stream — the low-latency tail

Every committed mutation on either side publishes a thin event to the
tenant's stream (the harness's existing `stream_publish` / `stream_read`
verbs ride Valkey under the hood on the local side, and the Railway side
already publishes mutation events through the same verbs). Subscribers on
the other side apply the mutation eagerly so a Codex edit on Railway lands
in the local node in < 1s without waiting on the next Prolly round.

**This is freshness, not convergence.** Held to exactly that job:

- Stream events MAY be dropped, reordered, or duplicated. Prolly catches
  every one of those failure modes at the next round.
- Stream MUST NOT be the system of record. The store is Prolly; the
  stream is a tail.
- On reconnect after offline, the stream simply resumes from the saved
  cursor — but the freshness gap fills via the next Prolly round, not
  by replaying historic stream events.

If the stream is unavailable, the system degrades to "convergence within
N seconds" instead of "freshness within 1 second." It does not lose data.

### Role 3: CRDT merge policy on contested keys (inside Prolly)

A small set of mutable values on the graph genuinely collide during
offline reconcile — a tag set on a memory atom, a "last-edited-by"
register, a counter, a soft-decay confidence score. For these, the Prolly
three-way merge calls a CRDT-style resolver instead of `auto_confidence`'s
plain text-style merge. Concretely:

- **LWW register** for single mutable scalars (e.g., a status field).
  HLC-stamped; latest stamp wins. Bounded loss; deterministic.
- **OR-set** for tag-like multi-value fields. Adds and removes
  reconcile cleanly without a "tombstone vs add" ordering bug.
- **Delta-CRDT** if a specific field's surface grows complex enough
  (e.g., a structured op log on a shared scope) — only when the simpler
  options aren't enough.

**CRDT is policy, not a parallel store.** The data still lives in the
Prolly graph; only the merge function for these specific fields changes.
A field that doesn't need CRDT semantics uses the default merge
(`auto_confidence`). The CRDT surface grows by adding more fields to
the registry, not by adding a parallel CRDT system.

This deliberately leaves `theorem-copresence`'s SubstratePeer untouched
in scope. SubstratePeer's CRDT machinery — HLC-stamped StructuredOps,
yrs text regions, awareness via working_log — remains the right tool
for live multi-peer co-editing within one scope (already proven, already
shipped). It is not extended across the local↔Railway sync wire here.

### How the three roles compose

```
                          Railway harness
                        /                \
   Prolly pack push                   Prolly pack pull
   (Role 1, periodic)                 (Role 1, periodic)
                        |                |
                        v                v
                     +----------------------+
                     |    Sync daemon       |
                     |  Role 1 driver +     |
                     |  Role 2 tail         |
                     +----------------------+
                        ^                ^
                        |                |
   stream_publish (Role 2)          stream_subscribe (Role 2)
   per local mutation               per remote mutation
                        |                |
                        v                v
                  Local node (RustyRed)
                  - apply stream events eagerly (Role 2)
                  - merge round resolves contested keys
                    via CRDT-aware fields (Role 3)
                        ^
                        |
                  Local agents (Codex, Claude Desktop via .mcp.json,
                                theorem-proxy ambient memory, watcher)
```

Convergence is Role 1's job. Freshness is Role 2's job. Conflict
semantics on contested keys is Role 3's job. Each holds only what it can
guarantee.

## 3. Components to build

### 3.1 Sync daemon (`apps/theorem-substrate-sync` - new standalone crate)

A small Rust process. Owns the connection to Railway and orchestrates the
three roles. Standalone Cargo crate (bare `[workspace]` like
`apps/theorem-proxy`), not a `rustyredcore_THG` member, so it builds
independently.

Responsibilities:

- Open and maintain the outbound HTTPS connection to Railway's `/mcp`.
- Authenticate with a Railway tenant token (separate from Anthropic OAuth;
  same Bearer-token-in-`Authorization`-header pattern as `apps/obsidian-sync`).
- Drive Role 1: periodic Prolly rounds via `rustyred_thg_graph_version_*` verbs.
- Drive Role 2: subscribe to Railway's tenant stream, apply each event as
  a local mutation; publish local mutations to the stream.
- Manage the Valkey outbox (see 3.3) and cursors (see 3.4).
- Surface receipts (rounds completed, events applied, conflicts logged
  by Role 3) through a small HTTP status endpoint for `theorem-proxy
  doctor` and any desktop UI.

The daemon is a **client** of two MCP servers: the local node
(`127.0.0.1:8380/mcp`) and Railway (`https://...up.railway.app/mcp`). It
holds no graph state of its own.

### 3.2 Stream tail wiring (Role 2)

The wire for low-latency mutation propagation is the harness's existing
stream verbs (`stream_publish` / `stream_subscribe` / `stream_read`),
ridden in both directions:

- **Local → Railway**: a post-commit hook on the local node
  (`rustyred-thg-core::hooks`) serializes each committed mutation as a
  stream event and publishes it to the tenant stream on Railway.
- **Railway → local**: the daemon holds a subscription with a saved
  cursor; pulls events since the cursor, applies each as a local
  mutation, advances the cursor.

Events are flat mutation envelopes (op kind, node/edge id, property delta,
HLC stamp), not CRDT deltas. They MAY be lost, reordered, or duplicated;
Role 1 catches all three failure modes at the next round. The data log's
content-hash dedup handles the duplicate case at apply time so a
re-applied event no-ops.

No SubstratePeer is instantiated across this wire. The stream is the
transport; the local node is the consumer; the apply path is the same
write surface a local agent uses.

### 3.3 CRDT merge registry (Role 3, inside Prolly)

A registry that tells the Prolly three-way merge how to resolve specific
contested fields when both sides diverged offline. Lives in the
`rustyred-thg-core` merge path; no separate store.

The registry holds entries shaped like:

```
{ node_label, field_name, strategy: LwwRegister | OrSet | DeltaCrdt }
```

When `graph_version_merge` reaches a contested field, it consults the
registry. A field with no entry uses the default (`auto_confidence`).

Initial registry shape (small on purpose; grow only on evidence):

| Field | Strategy | Why |
|---|---|---|
| `MemoryItem.tags` | OR-set | tag adds/removes from multiple surfaces must compose without tombstone-order bugs |
| `MemoryItem.status` | LWW register (HLC) | enum state; latest decision wins |
| `MemoryItem.confidence` | LWW register (HLC) | scalar with monotonic-ish updates |

Anything more complex (e.g., a structured op log on a shared scope) is
out of scope for the first cut. The escape hatch is delta-CRDT for that
specific surface; the registry shape allows it without redesign.

### 3.4 Valkey-backed outbox + cursors

The local stream-publish path is not a fire-and-forget on a remote socket.
Each event is pushed to a local Valkey list `sync:outbox:<tenant>` first
(durable across restart). A drainer in the sync daemon reads from the
head and publishes to Railway. On success, pops. On network failure,
backoff + retry. Terminal failures surface in the daemon status endpoint.

Cursors and last-known state:

- `sync:cursor:<tenant>` — last applied HLC vector from Railway's stream.
- `sync:last_round:<tenant>` — receipt of the last Prolly round.
- `sync:last_head:<tenant>` — Railway's last-known head ref, for diff
  base on the next round.

All in Valkey because the daemon is otherwise stateless. Restart-safe.

## 4. Bootstrap

A fresh local node (`nodes_total: 0`) onboards in one shot:

1. Daemon connects to Railway, calls `rustyred_thg_graph_version_ref(tenant, name="head")`
   to learn the current head.
2. Calls `rustyred_thg_graph_version_compile` on Railway against that head, downloads
   the pack.
3. `rustyred_thg_graph_version_checkout` on the local node with the pack as state.
4. Stores `sync:last_head` and the matching HLC cursor in Valkey.
5. Starts Role 2 stream subscription from the cursor.
6. Role 1 rounds resume on the configured interval.

No partial-state window: bootstrap is atomic on the version-pack apply.

## 5. Distribution model

This ships as a feature of the local stack. For other users adopting
Theorem:

- Each user runs against their own Railway tenant (or another hosted
  RustyRed instance — the protocol is the same).
- Daemon config:
  - `--railway-url` (or `THEOREM_SYNC_REMOTE_URL`).
  - `--tenant-token` (file or env; per-tenant auth on Railway side).
  - `--tenant` (the tenant slug, e.g. `Travis-Gilbert`).
- Default `sync_enabled = false` (matches phase-3-sync deliverable D4).
  The local stack is fully functional offline; sync is opt-in.
- The launcher (`apps/theorem-proxy/scripts/start-proxied-session.sh`) gets
  one additional optional spawn step that starts the sync daemon if
  `THEOREM_SYNC_ENABLED=1`.

## 6. Phases (independently shippable, organized by role)

Each phase delivers one role's responsibility and is independently
falsifiable. Roles 1 and 2 ship independently; Role 3 attaches to Role 1.

### Role 1 (Prolly rounds) — the floor

| Phase | Scope | Acceptance |
|---|---|---|
| **1.1** | Sync daemon scaffold + Railway auth + status endpoint | `theorem-substrate-sync doctor` reports `connected` against Railway with a valid token; `disconnected` with no token |
| **1.2** | Prolly round at 30s interval (compile → diff → merge → push → checkout) | A memory written locally appears in hosted recall after one round, and vice versa (phase-3-sync acceptance criterion 1) |
| **1.3** | Bootstrap atomicity | Cold local node onboards from Railway in one shot; no half-state visible to local readers |
| **1.4** | Tunable interval (5s active, 30s idle, manual trigger) | Idle-no-traffic round produces an empty diff with no version bump |
| **1.5** | Tier seam: `sync_enabled` config + launcher integration | With `sync_enabled = false`, zero version-pack traffic occurs (phase-3-sync acceptance criterion 4) |

1.1–1.5 implement the phase-3-sync plan with the daemon split into its
own crate. Role 1 alone delivers a usable system (5s freshness floor).

### Role 2 (stream tail) — the freshness

| Phase | Scope | Acceptance |
|---|---|---|
| **2.1** | Local-side post-commit hook publishes mutation events to the outbox | A locally committed mutation lands in `sync:outbox:<tenant>` within 100ms |
| **2.2** | Outbox drainer publishes to Railway's stream | A Codex session reading Railway's tenant within 1s sees the mutation |
| **2.3** | Remote-side subscription applies incoming events as local mutations | A Codex edit committed on Railway lands in the local node in < 1s (95th percentile) |
| **2.4** | Cursor durability + reconnect resume | Daemon restart resumes from saved cursor; no event applied twice (dedup by content hash) |
| **2.5** | Stream-unavailable degradation | If the stream is unreachable, the system continues to converge through Role 1 rounds; no data loss; status endpoint reports `stream: disconnected` |

Role 2 makes the system feel live. Without it, the user-visible behavior
is "memory shows up within 5 seconds" instead of "within 1 second."

### Role 3 (CRDT registry) — the merge policy

| Phase | Scope | Acceptance |
|---|---|---|
| **3.1** | Merge registry surface in `rustyred-thg-core` + default `auto_confidence` fallback | Initial registry empty: behavior identical to phase-3-sync; merges go through `auto_confidence` |
| **3.2** | LWW register strategy for `MemoryItem.status` and `MemoryItem.confidence` | Concurrent updates to the same status from both sides converge to the latest HLC stamp; both originals retained in the version log |
| **3.3** | OR-set strategy for `MemoryItem.tags` | Concurrent tag adds and removes from both sides reconcile without tombstone-order bugs (specific failing case: add-then-remove on local concurrent with re-add on Railway converges to {tag}) |

3.1 alone is a no-op delivery (the registry exists, nothing is in it).
3.2 and 3.3 add the two strategies that actually need them today. Anything
delta-CRDT-shaped is gated behind future evidence.

## 7. Acceptance test (end-to-end)

One falsifiable scenario that exercises all three roles. If any step
fails, the failed role points at the bug.

1. Start fresh local node (`nodes_total: 0`).
2. Start sync daemon, `sync_enabled = true`, against your Railway tenant.
3. **Bootstrap (Role 1)**: bootstrap completes atomically; `nodes_total`
   now matches Railway's head and no half-state is visible at any point.
4. **Stream tail (Role 2)**: in a separate session (claude.ai or Codex
   against Railway) commit a mutation; the local node reflects it in
   < 1s, without waiting on a Prolly round.
5. **Round convergence (Role 1)**: locally `upsert_note` a memory; within
   one round interval Railway's `harness_kg_status` reports it; remove on
   Railway and within one round it's gone locally.
6. **Stream-down degradation (Role 2)**: block the stream verbs (firewall
   the publish path). Local writes still converge on the next round;
   status endpoint reports `stream: disconnected`; no data loss.
7. **Offline outbox (Role 2)**: stop the daemon, write locally, restart
   the daemon; the queued write reaches Railway on next stream
   round-trip, and the next Prolly round confirms convergence.
8. **CRDT merge on contested key (Role 3)**: with Role 3.3 shipped, on
   two disconnected sides, add tag `A` locally and re-add `A` on Railway
   after a remove; reconnect and run one round. The merged state shows
   `tags = {A}`, not `{}`. Both individual histories are reachable in the
   version log.

If all eight pass, all three roles are wired. If 4 fails but 5 passes,
Role 2 (stream) is broken. If 5 fails but 4 passes, Role 1 (rounds) is
broken. If 8 fails, Role 3 (CRDT registry) is wrong or unwired.

## 8. Non-goals (named, not deferrals)

- Multi-tenant routing on the local node. Per-user single tenant is the
  product shape.
- Interactive merge UI. Auto_confidence merges + the CRDT registry
  resolve everything programmatically; unresolved cases log as branches,
  surfaced as a count, never block a round.
- A parallel CRDT store. CRDT is a merge policy on specific fields
  applied **inside** the Prolly merge, not a separate store. The data is
  always in the Prolly graph.
- SubstratePeer across the local↔Railway wire. SubstratePeer remains the
  right tool for live multi-peer co-editing within one scope (text
  regions, structured ops with genuine concurrency); it is not used as
  the sync transport here. The stream is the transport.
- Billing or tier-payment flow. The seam exists; the tier is config-only.
- Replacing the markdown corpus seed (`seed-node.py`) — orthogonal, can
  coexist as a one-time pre-sync seed for offline-first users.

## 9. References

- `docs/plans/theorem-desktop/phase-3-sync.md` (job-003) — the original
  plan this spec subsumes and extends.
- `docs/plans/obsidian-sync/README.md` — same outbound-poll pattern,
  different sink.
- `crates/theorem-copresence/` — SubstratePeer + crdt::join_delta /
  diff_since + yrs text-region exchange.
- `rustyredcore_THG/crates/rustyred-thg-core/` — graph_version_* MCP verbs.
- `rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs:17249` — read-only
  tools/list assertion for the MCP surface both ends speak.
- `apps/theorem-proxy/scripts/start-proxied-session.sh` — the launcher
  the daemon spawn integrates into.
- `apps/obsidian-sync/` — reference implementation for the
  outbound-only HTTPS + per-tenant token + three echo guards pattern.
