# RustyRedCore PyO3 Modernization

## Goal

Modernize `rustyredcore_THG/src/` so it is no longer a catch-all old bridge
crate. The root crate should be one of two things:

- a thin Python ABI wrapper around contracts owned by current Rust crates, or
- a pure native accelerator whose Python boundary is only marshalling.

Runtime ownership belongs in the current substrate crates: pure harness
contracts in `theorem-harness-core`, graph/runtime persistence in
`theorem-harness-runtime` and `rustyred-thg-mcp`, and database behavior in
`rustyred-thg-core` plus its server crates.

## Current Classification

| Module | Classification | Direction |
| --- | --- | --- |
| `lib.rs` | ABI registry | Keep thin; only module registration and Python-visible names. |
| `cmh.rs` | Compatibility wrapper | Done: pure CMH hashes now live in `theorem-harness-core::cmh`; this file delegates. |
| `adapters.rs` | Runtime/store boundary | Done: tenant/data-dir resolution now has a config seam with current Theorem envs first and legacy envs preserved. |
| `bgi.rs` | Mixed pure contract + wrapper | Partial: deterministic JSON/hash receipt contracts now live in `theorem-harness-core::bgi`; engine-backed calls stay downstream. |
| `push_ppr.rs` | Pure accelerator | Keep here unless a Rust-native caller needs the exact kernel elsewhere. Preserve Python parity. |
| `search_kernel.rs` | Pure accelerator | Keep here; add edge-case tests around URL and scoring invariants. |
| `graph_export.rs` | Pure transformation accelerator | Keep here; pure helper functions now sit behind the PyO3 wrappers. |
| `thg.rs` | Thin core wrapper | Keep here unless a richer Python SDK supersedes it. |

## Execution Plan

1. Make the CMH split real.
   - Move `cmh_body_hash`, `cmh_atom_id_v1`, and
     `cmh_handoff_state_hash_v1` into `theorem-harness-core::cmh`.
   - Keep `theseus_native.cmh_*` ABI stable by delegating from `src/cmh.rs`.

2. Harden the ABI boundary.
   - Add a Python export-contract test for all top-level functions and the
     `theseus_native.adapters` submodule.
   - Keep old parity tests as byte-level oracles.

3. Pull pure helper logic behind wrappers.
   - Extract pure graph-export helpers so Rust unit tests can validate behavior
     without needing Python/Numpy.
   - Add Rust tests for search-kernel edge cases and stable ranking ties.

4. Modernize `adapters.rs`.
   - Extract an adapter store config/factory seam.
   - Support current env names such as `THEOREM_TENANT_SLUG` and
     `RUSTYRED_THG_TENANT_SLUG` without breaking existing adapter envs.
   - Preserve legacy `RUSTYRED_THG_ADAPTER_DATA_DIR`,
     `RUSTY_RED_DATA_DIR`, `RUSTYRED_THG_PRODUCT_DATA_DIR`, and
     `RUSTYRED_THG_ADAPTER_DEFAULT_TENANT`.
   - Status: implemented with focused Rust tests for env precedence,
     fallback behavior, and tenant discovery.

5. Move BGI contracts out of the bridge.
   - Start with deterministic hash and receipt-summary functions.
   - Leave heavy or Python-shape-specific context-pack extraction until golden
     fixtures prove parity.
   - Status: stable JSON hash, fact-pack row hash, egraph/datalog receipt
     summaries, and receipt compaction now live in `theorem-harness-core::bgi`.
     Engine-backed functions still delegate to `rustyred-thg-core` or `egg`.

## Validation

Run focused checks from `rustyredcore_THG/` after each slice:

- `cargo test -p theorem-harness-core cmh`
- `cargo test -p theorem-harness-core bgi`
- `cargo test -p rustyredcore_thg cmh`
- `cargo test -p rustyredcore_thg bgi`
- `cargo test -p rustyredcore_thg graph_export`
- `cargo test -p rustyredcore_thg search_kernel`
- `python -m pytest tests/test_export_contract.py`
- Existing byte oracles: `tests/test_cmh_parity.py`, `tests/test_bgi_parity.py`,
  `tests/test_datalog_derivation_parity.py`, `tests/test_evolution_parity.py`,
  and `tests/test_parity.py` when the wheel is installed.
