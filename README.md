# Theorem

Theorem planning and implementation spine for Rust-native substrate work.

This repository mirrors the programmable Rust projection from the Theseus /
Index-API workspace. It is intentionally narrow: it carries the RustyRed/THG
substrate code, the Python bridge and parity surfaces that exercise it, the
Rust theorem symbolic-engine plan, the Rusty Red Web plans, and the
reconciliation notes that explain the current sequencing.

## Contents

- `rustyredcore_THG/` - RustyRed / THG Rust workspace and PyO3 bridge code.
- `apps/notebook/inference_engines/` - symbolic-engine contracts, Python
  reference engines, native bridge adapters, affordance gates, and tests.
- `apps/notebook/inference_kernel/` - routing and execution layer for native
  versus Python engine strategy.
- `apps/notebook/benchmarks/` - parity and cost gates for native symbolic work.
- `apps/notebook/discovery_runs/` - Rust theorem callers that currently route
  archive and policy-evolution work through the native evolution engine.
- `apps/orchestrate/runtime/map_elites_tick.py` - orchestration tick for native
  MAP-Elites archive throughput.
- `docs/plans/rust-theorem-symbolic-engines/` - Rust Theorem: Native Symbolic Engines, including RT-0 through RT-5 status and parity discipline.
- `docs/plans/rusty-red-web/` - Rusty Red Web / RustyWeb implementation plan.
- `docs/plans/rustyweb-v1-design/` - RustyWeb V1 design and AnswerDraft contract.
- `docs/plans/commonplace-substrate-reconciliation/` - reconciliation notes that route work between Rust theorem, RustyWeb, and kernel-object lanes.
- `Theseus/Theorem.md` - source framing for Theseus as canonical substrate and
  Theorem as the Rust projection/mirror.
- `docs/reference/commonplace-substrate-architecture-part-4-1.md` - source
  architecture context for symbolic compute offload and substrate design.

## Current Direction

Rust theorem hot-path work is implemented through RT-5.2a. Remaining RT-5 ports
stay profile/use-case gated.

RustyWeb is now active work. The first implementation move is to recover the
burst-crawler scaffold as seed code, then build the real product as a RustyRed-
backed graph crawler rather than treating the fetcher scaffold as complete.

## Current Code

- `rustyredcore_THG/` - mirrored Rust theorem projection from Theseus.
- `rustyredcore_THG/crates/rustyred-web/` - first RustyWeb crate. It currently
  contains the V0 fixture crawler kernel: URL canonicalization, streaming
  `a[href]` extraction, BLAKE3 content snapshots, V0 graph node/edge emission,
  and application into a RustyRed `GraphStore`.

## Mirror Rule

Theseus remains the canonical application/runtime workspace. Theorem mirrors the
Rust projection and its bridge contracts so Rust-side work can move at its own
cadence while staying auditable against the source workspace.

Last sync: 2026-05-29. This sync includes the Rust theorem commits for Datalog
rules and byte parity, native Datalog/probabilistic receipts, Gate-0 affordance
coverage, symbolic-side CPU cost measurement, RT-5.2a native evolution parity,
and RustyWeb V1 planning.
