# Rust Theorem: Native Symbolic Engines (Path 2)

**Status:** RT-0 through RT-4 complete; RT-5.2a evolution archive native path added.
**Date opened:** 2026-05-29
**Owners:** Claude Code (plan author + Rust engine impl), Codex (co-implementer, invited after reconciliation per Travis).
**Spec sources:** `Theseus/CommonPlace/Commonplaces needs/commonplace-substrate-architecture-part-4.md` Part C ("Path 2, native Rust") and `part-4-1.md` Part A + sequencing ("Native Rust promotion (Path 2) for hot engines. Datalog and probabilistic first").

---

## What this is

The "Rust theorem" is the native-Rust promotion of the symbolic inference engines, living in `rustyredcore_THG/src/bgi.rs` (engine identifiers `rust-datafrog-core` and `egraph-theorem`). Part 4 Part C names two paths for projecting the ten Python inference engines into the substrate:

- **Path 1 (PyO3 bridge, Python engine core unchanged).** Already shipped for Datalog and probabilistic as `apps/notebook/inference_engines/affordances.py`. Owned by the compute-offload lane (Codex + the other Claude Code session). Not this plan.
- **Path 2 (native Rust, same receipt contract).** Reimplement the hot engines in Rust so the substrate computes symbolic work without a Python forward pass, returning receipts byte-identical to the Python reference. This is the greenfield. This is this plan.

This plan does NOT touch the compute-offload cost test (`apps/notebook/inference_engines/benchmark/`) or the affordance wrappers (`affordances.py`) beyond a single coordinated engine-factory seam. See `implementation-plan.md` Section 9 (Lane boundary).

## Why it is the critical part

1. Path 1 already proved the projection shape works (content-addressed `AffordanceReceipt`, substrate fact packs, Gate 0 differential runner). The remaining economic claim from Part 4.1 ("CPU symbolic compute is ~2 orders cheaper than GPU inference, and not supply constrained") only pays off when the symbolic work runs on the cheap native path, not a Python interpreter loop. Path 2 is where the cost-offload thesis turns from "correct" into "cheap."
2. The original native scaffold was shallow and partly wrong (2 of 10 Datalog rules, and those 2 diverged from Python semantics). RT-0 through RT-4 fixed that baseline; RT-5.2a now extends the same parity discipline to MAP-Elites archive throughput.
3. It carried a latent production bug worth fixing immediately (see `implementation-plan.md` Section 2, R2); that is why the plan keeps Python as the reference and requires byte-parity before native dispatch.

## Files in this plan

- `README.md` (this file): orientation and scope.
- `implementation-plan.md`: the build spine. Checklist IDs `RT-0` through `RT-5`, parity contracts, phasing, tests, rollout, lane boundary, risks.
- `rt5-cold-engines-analysis.md`: decision support for e-graph generalization and the colder engine ports. Updated after the evolution archive port.

## Reconciliation context

This plan is one output of the broader reconciliation in `docs/plans/commonplace-substrate-reconciliation/`. That folder maps every claim in the "Commonplaces needs" plan corpus (Parts 3, 4, 4.1, the five-axis reframe, the RustyWeb reframe, the plan-review packet) to built / planned-elsewhere / greenfield, and records the no-touch boundary against the offload-cost-test lane. Read it first if you want the whole picture; read this file if you are implementing the Rust engines.
