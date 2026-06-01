# Harness Rust Port Claims

Date: 2026-06-01
Coordination mode: Git fallback. The live harness `presence`, `mentions`, and
`coordination_room` endpoints returned Django 500s during this session, so this
file and commit messages are the coordination channel until substrate writes
recover.

## Current Claims

| Actor | Status | Files | Notes |
|---|---|---|---|
| Codex | done for current slice | `rustyredcore_THG/crates/theorem-harness-core/**`; `rustyredcore_THG/crates/theorem-harness-runtime/**`; `docs/plans/harness-rust-port/CLAIMS.md`; `docs/plans/harness-rust-port/parity/**` | Rust `theorem-harness-core` now ports the pure state machine, replay/fork helpers, and toolgraph toolkit selector. `theorem-harness-runtime` adds the spec's GraphStore-backed event-log seam while keeping persistence out of the parity kernel. |
| Claude Code | done for slice | `docs/plans/harness-rust-port/parity/**`; `docs/plans/harness-rust-port/parity-toolgraph/**` | Generated Python reference fixtures from `Index-API/apps/orchestrate/runtime/state_machine.py` and `toolgraph.py`; Codex extended the state-machine corpus to 25 scenarios / 260 steps and consumed the toolgraph corpus read-only for the Rust port. |

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
- `python3 docs/plans/harness-rust-port/parity/generate_fixtures.py --check`
  regenerates byte-identical fixtures.
- `python3 docs/plans/harness-rust-port/parity-toolgraph/generate_toolkit_fixtures.py --check`
  regenerates byte-identical toolkit fixtures.
- The Rust parity test replays legal and illegal transition sequences through
  the Rust port, comparing `state_hash_before`, `state_hash_after`, status,
  sequence number, and guard codes against the Python reference output.
- The Rust toolgraph parity test compares the catalog plus compiled toolkit
  outputs against the Python reference corpus.
