# Inject infra as a typed seam that refuses to fabricate, for verification-first pipelines

**Kind:** rule
**Captured:** 2026-06-12
**Session signature:** `claude:travisgilbert@Traviss-Laptop:b944c683`
**Domain tags:** encode, testing, architecture

## Trigger

Building the receipt runner (the slice-4 keystone) for a gate whose entire job is
to SCORE `UseReceipt` rows: `benchmarks.py::rust_held_out_validation_gate` flips
`canonical_ready` based purely on the receipts it is handed. The obvious
"make-it-testable" move — a stub executor that returns success receipts — would
let `canonical_ready` go true on fabricated outcomes, silently defeating the
verification spine the slice exists to build. The encode pipeline already encodes
this discipline in its own docstring ("The function does not run an agent. It
scores receipts from real baseline and treatment runs"); a fabricating stub would
quietly violate it while looking like a passing test.

## Rule

When an organ depends on live LLM/agent infra you cannot run in-session, model the
infra as a typed injected seam (a `Protocol`). Provide a deterministic impl for
tests (ScriptedExecutor / fake Analyst) AND a live impl that RAISES rather than
returns fake data without real configuration. The deterministic impl makes the
pipeline real and testable today; the raising live impl keeps the gate from ever
scoring fabricated evidence. Never stub-return plausible data into a
verification-first path — the failure is invisible because the test still passes.

## Evidence

- `apps/notebook/encode/receipt_runner.py` (Index-API main @ 3bb58031): `Executor`
  protocol; `AgentExecutor.execute` raises `NotImplementedError` without an
  injected `run_task`; `ScriptedExecutor` carries the deterministic/replay path.
- `apps/notebook/encode/distill.py` (Index-API main @ 480a05fa): `Analyst` /
  `Consolidator` injected; the live LLM tier is the pluggable seam (Q9), the
  provenance lint rejects uncited claims so no fabricated citation survives.
- Mirrors `benchmarks.py:514` docstring "does not run an agent. It scores receipts".

## Encoded in

- `docs/learnings/2026-06-12-inject-infra-seam-that-refuses-to-fabricate.md` (this file)
