# Commonplace Substrate: Plan-vs-Code Reconciliation

**Status:** Reconciliation complete. Index + verdict.
**Date:** 2026-05-29
**Author:** Claude Code (Opus 4.8), harness plan mode.
**Input corpus:** `Theseus/CommonPlace/Commonplaces needs/` (gitignored vault export): Part 3, Part 4, Part 4.1, `From Claude.ai.md` (five-axis reframe), `compute-offload-perspective-2026-05-28-claude.md`, `rustyweb-substrate-search-reframe-2026-05-27.md`, and the `plan-review-2026-05-27/` packet.
**Method:** map then code, never doc-sweep (per `Index-API/docs/codebase-map.md`). Every claim below was checked against the live tree, not against other prose.

---

## Why this folder exists

Travis asked to reconcile the "Commonplaces needs" plan corpus against what is actually built, write implementation plans, and flagged that "all of Rust's theorem in plan 4 and 4.1 still needs to be built, the most critical part." He also noted another Claude Code session and a Codex agent already own the offload cost test, and that Codex will help implement after reconciliation.

This folder is the reconciliation. It does three things:

1. Maps every input-doc claim to its true status: built, planned-elsewhere, greenfield, or owned-by-another-lane (`reconciliation-matrix.md`).
2. Records the maturity ladder and the no-touch lane boundary so the three agents do not collide.
3. Points each genuinely-unbuilt requirement at a plan, existing or new.

## The one-paragraph verdict

The plans lag the code. Most of the near-term layer that Parts 4 and 4.1 describe as "next" is already built or in flight: the inference-engine affordances (`affordances.py`), the entire compute-offload cost-test stack (`benchmark/`: gate0, differential runner, ledger, arms, pre-registration, report), the coordination kernel (seven durable `MemoryAtom` kinds plus three anchor nodes plus presence/subscription cache), and the inline compute MCP (`rustyred_thg_algorithm_*_inline`). The native-Rust promotion of symbolic engines (Path 2 in Part 4 Part C) is no longer greenfield for the hot engines: Datalog, probabilistic, and the MAP-Elites/evolution archive path are implemented with parity gates. Its detailed plan remains `docs/plans/rust-theorem-symbolic-engines/`.

## The maturity ladder (adopted from the plan-review packet, validated against code)

The plan-review packet asked for an explicit maturity ladder so the corpus would stop mixing maturity levels in one lane. Applied to the reconciled state:

- **Operational now (built or in flight):** coordination kernel + gossip protocol; inference-engine affordances (Path 1); compute-offload cost-test stack; inline compute MCP; native graph algorithms (PPR/PageRank/components/communities) over tenants and inline.
- **Implemented, with remaining profile-gated tail:** the Rust theorem (native symbolic engines, Path 2) now covers Datalog, probabilistic, and RT-5.2a evolution archive. Remaining RT-5 work is profile/use-case gated. Plan: `docs/plans/rust-theorem-symbolic-engines/`.
- **Active now:** RustyWeb as Rusty Red Web, a RustyRed-backed crawler and substrate-native search engine. Main still has no RustyWeb crate, but `feat/theseus-burst-crawler-scaffold` contains salvageable seed code. Plan: `docs/plans/rusty-red-web/implementation-plan.md`.
- **Designed, not shipped (kernel-object gaps):** Participant capability profile, first-class Artifact, ValidationReceipt, Subscription `delivery_policy`, AnswerDraft. Already captured in `docs/plans/commonplace/kernel-object-model.md` Section 3d and the commonplace `implementation-plan.md`. Reconciled, not re-planned here.
- **Research frontier (deferred by design):** native model roster (Part 4 Part A), thought-vector capture + cross-model translation (Part 4 Layer 3, Part 3 Section 2.1), xLSTM lingua franca, cascade/reuse/pushdown/fusion axes 2 to 5 of the five-axis planner (Part 4.1 / `From Claude.ai.md`). Deferred per the compute-offload plan's own sequencing item 4 ("six warm containers contradict the cost discipline until offload is proven"). Not cut; sequenced.

## The no-touch lane boundary

Three agents are active on one shared tree. To prevent collisions:

- **Offload cost test lane (other Claude Code session + Codex):** owns `apps/notebook/inference_engines/benchmark/` entirely and `docs/plans/compute-offload/implementation-plan.md`. The Rust theorem plan does not touch these.
- **Co-owned:** `apps/notebook/inference_engines/affordances.py` (Codex). The Rust theorem plan's only edit is one engine-factory seam (RT-3.2), coordinated.
- **Rust theorem lane (this reconciliation):** `rustyredcore_THG/src/bgi.rs`, the `*/native.py` bridges, `inference_kernel/native_strategy.py`, the parity harness, and the MCP symbolic tools. Full list in `rust-theorem-symbolic-engines/implementation-plan.md` Section 9.

All commits use explicit pathspecs (never bare `git commit`) because the index is shared.

## Files

- `README.md` (this file): index, verdict, ladder, boundary.
- `reconciliation-matrix.md`: claim-by-claim mapping of all seven input docs to built / planned-elsewhere / greenfield / owned-by, with file:line evidence and the destination plan for each unbuilt item.
- `credit-model.md`: Commonplace credit-estimation product note moved out of the repo root so pricing policy stays with the Commonplace plan corpus.

## Where the work goes next

1. The Rust theorem hot path is implemented through RT-5.2a. Continue only profile/use-case-gated RT-5 work there: e-graph generalization when a second rewrite consumer exists, or real non-stub ports when they become hot.
2. The kernel-object gaps already have a home (`docs/plans/commonplace/`); no new plan needed, only execution when sequenced.
3. RustyWeb is now front-of-queue work. Use `docs/plans/rusty-red-web/implementation-plan.md`; treat the feature-branch burst crawler as seed code, not the finished graph-native service.
4. The frontier layer stays deferred until the offload economics land, per the existing compute-offload sequencing.
