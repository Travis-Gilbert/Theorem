# Harness Rust Port Claims

Date: 2026-06-01
Coordination mode: hybrid. The live harness room/intent/presence tools are
responding again, but direct THG mirror writes still report tenant resolution
degradation. Keep this file and path-scoped commit messages as the durable
coordination fallback until substrate mirroring is clean.

## Current Claims

| Actor | Status | Files | Notes |
|---|---|---|---|
| Codex | done for MCP exposure slice | `rustyredcore_THG/crates/theorem-harness-core/**`; `rustyredcore_THG/crates/theorem-harness-runtime/**`; `rustyredcore_THG/crates/rustyred-thg-mcp/**`; `rustyredcore_THG/Cargo.lock`; `docs/plans/harness-rust-port/CLAIMS.md`; `docs/plans/harness-rust-port/parity/**`; `docs/plans/harness-rust-port/parity-context/**` | Rust `theorem-harness-core` now ports the pure state machine, replay/fork helpers, toolgraph toolkit selector, context-web bounded pack compiler/policy core, pure affordance registry/receipt contract, Pairformer session metrics, federated structural-signal privacy helpers, the pure MapArtifact compiler, and memory preparation contracts. `theorem-harness-runtime` adds the spec's GraphStore-backed event-log seam and native direct-coordination substrate with room membership, live intents, durable presence, direct messages, and mentions. `rustyred-thg-mcp` now exposes those native coordination tools over MCP with read-only/write-mode gating and GraphStore-backed round-trip coverage. |
| Claude Code | in progress for HTTP exposure | `apps/theorem-harness-server/**`; `docs/plans/harness-rust-port/ios-transport-handoff.md` | Owns the standalone Axum HTTP transport lane over `theorem-harness-runtime`, including run listing/detail and presence/intent endpoints for the iOS handoff. Codex should not start or edit this lane unless coordination changes. |
| Claude Code | done for fixture slices | `docs/plans/harness-rust-port/parity/**`; `docs/plans/harness-rust-port/parity-toolgraph/**`; `docs/plans/harness-rust-port/parity-context/**` | Generated Python reference fixtures from `Index-API/apps/orchestrate/runtime/state_machine.py`, `toolgraph.py`, and `context_web/{contracts,policy}.py`; Codex extended the state-machine corpus to 25 scenarios / 260 steps and consumed the toolgraph/context corpora read-only for the Rust ports. |

## Git Protocol

- Use path-scoped commits only.
- Do not create a second harness crate. The Phase 1 crate is
  `rustyredcore_THG/crates/theorem-harness-core`.
- If editing a claimed file, update this table first in the same path-scoped
  commit or coordinate in the commit message.
- Keep unrelated dirty state (`CLAUDE.md`, `Product.MD`,
  `rustyredcore_THG/crates/reconstruction-engine/Cargo.lock`) out of this slice
  unless Travis explicitly asks to include it.

## Immediate Acceptance Target

- `cargo test -p theorem-harness-core` passes from `rustyredcore_THG/`.
- `cargo test -p theorem-harness-runtime` passes from `rustyredcore_THG/`.
- `cargo test -p rustyred-thg-mcp` passes from `rustyredcore_THG/`.
- `cargo clippy -p theorem-harness-runtime --all-targets --no-deps -- -D warnings`
  passes from `rustyredcore_THG/`.
- `cargo clippy -p rustyred-thg-mcp --all-targets --no-deps -- -D warnings`
  passes from `rustyredcore_THG/`.
- `python3 docs/plans/harness-rust-port/parity/generate_fixtures.py --check`
  regenerates byte-identical fixtures.
- `python3 docs/plans/harness-rust-port/parity-toolgraph/generate_toolkit_fixtures.py --check`
  regenerates byte-identical toolkit fixtures.
- `python3 docs/plans/harness-rust-port/parity-context/generate_context_fixtures.py --check`
  regenerates byte-identical context pack fixtures.
- The Rust parity test replays legal and illegal transition sequences through
  the Rust port, comparing `state_hash_before`, `state_hash_after`, status,
  sequence number, and guard codes against the Python reference output.
- The Rust toolgraph parity test compares the catalog plus compiled toolkit
  outputs against the Python reference corpus.
- The Rust context-web parity test compares bounded pack output, generated
  artifact policy, token ledger, source mix, edge/path filtering, validation,
  and evaluation output against the Python reference corpus.
- The Rust affordance registry test covers the eleven Python projection
  affordance IDs and the content-addressed `AffordanceReceipt` envelope. Engine
  execution wrappers remain runtime/native-engine work, not core-crate IO.
- The Rust session-metrics tests cover Pairformer mode normalization, JSONL
  loading, completed-session summaries, and Welch-z mode comparison.
- The Rust federated-signal tests cover recursive raw-content rejection, receive
  normalization, coarse privacy buckets, and structural patch projection without
  importing Django.
- The Rust map-artifact tests cover stable map IDs, scope resolution, CodebaseMap
  baseline entries, applied delta upsert/remove behavior, ToolMap metadata,
  markdown/json export, and artifact summaries.
- The Rust memory-contract tests cover typed hydration handles, recall policy
  normalization, nested recall previews, full contract parsing, Python-style
  immutable/editable truthiness, active status defaults, and serialization shape.
- The Rust runtime coordination tests cover room membership persistence,
  `COORDINATION_MEMBER_OF` graph edges, live intent replacement and filtering,
  durable presence heartbeat/end records, direct message persistence,
  `COORDINATION_MENTIONS` graph edges, consume-on-read mention inbox behavior,
  and `RedCoreGraphStore` reopen behavior for all coordination primitives.
- The Rust MCP coordination tests cover native tool listing, read-only/write
  gating, room join, presence heartbeat/readback, intent write/readback,
  coordinate receipt shape, pending mention reads, consume-on-read semantics,
  and room message reads through the MCP server surface.
