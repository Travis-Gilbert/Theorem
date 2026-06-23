# Reconciliation Matrix

**Date:** 2026-05-29. Every row checked against the live tree (file:line where load-bearing). Status legend:

- **BUILT:** shipped in code now.
- **IN FLIGHT:** partially built, owned by an active lane.
- **GREENFIELD (mine):** unbuilt, owned by this reconciliation, has a plan.
- **GREENFIELD (later):** unbuilt, separate track, needs its own plan when sequenced.
- **DESIGNED:** spec exists, no code, captured in an existing plan.
- **DEFERRED:** intentionally sequenced behind offload proof.
- **OWNED-OTHER:** built/in-flight by the offload-cost-test lane; no-touch.

---

## Part 3 (substrate revisions, kernel, phase ordering)

| Claim / requirement | Status | Evidence / destination |
|---|---|---|
| Substrate vs business separated (Commonplace != Theorem Cloud) | BUILT (as doctrine) | Honored across plans; `docs/plans/commonplace/` treats substrate standalone. No code conflict. |
| Shared-room reliability before browser | IN FLIGHT | Coordination kernel shipped; reliability tail (plugin auth drift, tenant semantics) is the offload plan's Phase 0.2/0.3 (Codex lane). |
| Kernel = Git state machine + encode pipeline + Pairformer | PARTIAL | Durability layer = RustyRed-THG + Postgres shadow (BUILT). Encode pipeline exists (BUILT). Pairformer = DESIGNED/plan-only (router training plan, Codex). |
| Datalog before Z3 before e-graphs | BUILT (order honored) | Datalog engine BUILT (`datalog/engine.py`); solver/Z3 engine exists (`solver/`); egraph exists (`egraph/`). Native order followed in the Rust plan (RT-1 Datalog first). |
| Edge naming on thought vectors (SIMILAR_WITHIN_SOURCE etc.) | DESIGNED | Phase 2 vector work; not shipped. |
| Persistent presence over continuous co-presence | BUILT | `PresenceStore` cache primitive (kernel-object-model 3c). |
| Replay capability matrix per tier | DESIGNED | Phase 3 in commonplace plan. |
| vector_source 5-tier enum | DESIGNED | kernel-object-model 3d / Phase 2. |
| Kernel object model (Participant, Room, Event, Intent, Artifact, ValidationReceipt, Decision, Tension, Reflection, Subscription) | BUILT (6) + ANCHOR (1) + CACHE (1) + DESIGNED (3) | `coordination.py:50-64` kinds: Room, Intent, Event, Decision, Tension, Reflection (+ ContinuityPack) BUILT. Participant = Actor anchor node (BUILT). Subscription/Presence = cache (BUILT). Artifact, ValidationReceipt, Subscription.delivery_policy = DESIGNED (`docs/plans/commonplace/kernel-object-model.md` 3d). |

## Part 4 (native roster + Theseus mirror + inference engines as affordances)

| Claim / requirement | Status | Evidence / destination |
|---|---|---|
| Native model roster (Gemma 31B GL-Fusion, Mistral, Jamba, Qwen Coder, DeepSeek-Distill, xLSTM) | DEFERRED | compute-offload plan sequencing item 4: roster stands up after offload proven. Not cut. |
| Five-layer architecture (event substrate / participant interface / thought-vector / compute affordances / synthesis) | PARTIAL | Layer 1 (event substrate) BUILT. Layer 4 (compute affordances) IN FLIGHT (Path 1) + GREENFIELD (Path 2). Layers 2/3/5 DESIGNED/DEFERRED. |
| Theseus mirror (project capabilities as affordances, voice not narrator) | IN FLIGHT | First projection (datalog + probabilistic) shipped as affordances; rest is capability-by-capability future. |
| Part C: inference engines as affordances, Path 1 (PyO3 bridge) | BUILT | `affordances.py` (`build_fact_pack_from_substrate`, `run_datalog_affordance`, `run_probabilistic_*`, content-addressed `AffordanceReceipt`). Commit `d13e6fa4`. |
| Part C: inference engines as affordances, Path 2 (native Rust) | BUILT for hot engines + PROFILE-GATED tail | Datalog, probabilistic, egraph `context_pack`, and evolution archive now have native/parity coverage. Remaining RT-5 ports are profile/use-case gated. Plan: `docs/plans/rust-theorem-symbolic-engines/`. |
| Canonical (Django/Memgraph) vs hot (RustyRed) split | BUILT | Established pattern; shadow-write shipped. |
| All ten engines exist as pure functions with receipts | BUILT | `apps/notebook/inference_engines/{causal,datalog,egraph,evolution,expression,optimizer,probabilistic,proof,simulation,solver}/`. |

## Part 4.1 (compute-offload hypothesis, RunPod cost test, roster)

| Claim / requirement | Status | Evidence / destination |
|---|---|---|
| Compute-offload hypothesis (CPU symbolic cheaper than GPU) | OWNED-OTHER | `docs/plans/compute-offload/implementation-plan.md`. |
| Gate 0 differential-receipt projection | OWNED-OTHER | `benchmark/gate0.py`, `benchmark/differential.py` (commit `57a21034`). No-touch. |
| Gate 1 cost A/B four arms (A/B0/B1/B2/B3) | OWNED-OTHER | `benchmark/arms.py`. No-touch. |
| Benchmark ledger (19-field record) | OWNED-OTHER | `benchmark/ledger.py`, `records.py` (commit `a2d5a701`). No-touch. |
| Pre-registered predictions | OWNED-OTHER | `benchmark/preregistration.py`. No-touch. |
| Cascade arm (CO-2), Reuse arm (CO-3), blind synthesis scoring (CO-4) | OWNED-OTHER / DEFERRED | compute-offload plan CO-2/3/4. No-touch. |
| Native Rust promotion (Path 2) for hot engines, Datalog + probabilistic first | BUILT + EXTENDED | The Rust theorem now covers RT-1 Datalog, RT-2 probabilistic, and RT-5.2a evolution archive. Plan: `docs/plans/rust-theorem-symbolic-engines/`. |
| Revised coding roster (Qwen3-Coder-Next, GLM, etc.) | DEFERRED | Roster-agnostic by the offload plan's own scope note; volatile picks. |
| RunPod serverless infra for the test | OWNED-OTHER | compute-offload lane. |

## From Claude.ai.md (five-axis query planner reframe)

| Axis | Status | Evidence / destination |
|---|---|---|
| Axis 1: symbolic offload | IN FLIGHT (Path 1) + GREENFIELD (Path 2) | affordances.py + the Rust theorem. |
| Axis 2: predicate pushdown | DEFERRED | Optimizer-internal; compute-offload "measure later." |
| Axis 3: operator fusion | DEFERRED | Same. |
| Axis 4: cheap-model cascade | OWNED-OTHER / DEFERRED | compute-offload CO-2 + Pairformer router plan (Codex). |
| Axis 5: computation reuse | OWNED-OTHER / DEFERRED | compute-offload CO-3 (reuse arm, staleness policy). |
| Substrate query planner / Pairformer as operation-level cost router | DESIGNED | `pairformer-tool-router-training-plan` (Codex lane). |

## compute-offload-perspective-2026-05-28-claude.md (the cost-test design)

| Claim / requirement | Status | Evidence / destination |
|---|---|---|
| Gate 0/1/2 design, four arms, pre-registration, differential-first | OWNED-OTHER | This doc IS the design that became `docs/plans/compute-offload/implementation-plan.md`. Entirely the other lane. No-touch. |
| "Convergence is a hypothesis, not a result" discipline | ADOPTED | Carried into this reconciliation: every row checked against code, not prose. |

## rustyweb-substrate-search-reframe-2026-05-27.md (RustyWeb)

| Claim / requirement | Status | Evidence / destination |
|---|---|---|
| RustyWeb V0 boring crawler kernel | ACTIVE-PLAN | No `rustyweb` crate in `rustyredcore_THG/crates/` or the RustyRed workspace yet. `feat/theseus-burst-crawler-scaffold` has seed code, but V0 still needs graph-shaped RustyRed output. Destination: `docs/plans/rusty-red-web/implementation-plan.md`. |
| Graph-yielding external sources (Wikidata/OpenAlex/etc. edge import) | PLANNED-V1 | Not built. Contract lives in `docs/plans/rustyweb-v1-design/`; implementation follows RustyWeb V0. |
| AnswerDraft node type (two-fidelity: extractive Tier A / enriched Tier B) | DESIGNED | `docs/plans/commonplace/kernel-object-model.md` 3d names AnswerDraft as first Artifact subtype, Phase 3. |
| License-tiering / share-alike quarantine | PLANNED-V1 | Not built; preserve the legal constraint in the RustyWeb V1 contract. |
| Sequencing: RustyWeb after coordination-kernel proving ground | SUPERSEDED | Travis reprioritized RustyWeb on 2026-05-29. It is now active while kernel-object gaps can be handed to parallel sessions. |

## plan-review-2026-05-27/ packet

| Recommendation | Status | Evidence / destination |
|---|---|---|
| Add a maturity ladder | ADOPTED | This reconciliation's README ladder. |
| Separate substrate from monetization | ADOPTED | Part 3 doctrine; honored. |
| Define kernel object model | BUILT/DOCUMENTED | `docs/plans/commonplace/kernel-object-model.md`. |
| Make affordance compiler load-bearing | IN FLIGHT | affordances.py is the first instance. |
| Search repair before substrate expansion (fractal expansion / dense embedding) | OPEN | Named as a gate in the source docs; tracked separately (release-week blocker), not in scope here. |
| Validation receipts as a communication primitive | DESIGNED | ValidationReceipt kernel-object gap (commonplace plan). |
| Servo as thesis until evidence | NOTED | Browser is a later track; no kernel dependency. |
| Thought vectors as a separate research track | DEFERRED | Matches the frontier layer. |
| 6-phase implementation plan | ADOPTED | Superseded by `docs/plans/commonplace/implementation-plan.md` (already on disk). |

---

## Net: what is genuinely unbuilt and unowned

After removing everything that is BUILT, IN FLIGHT, OWNED-OTHER, DESIGNED-with-a-home, or DEFERRED-by-design, the residue that needs a new implementation plan now is no longer the Rust theorem hot path; that path is implemented through RT-5.2a.

- **Remaining Rust theorem tail:** profile/use-case-gated RT-5 work only: e-graph generalization when a second rewrite consumer exists, and causal/optimizer/proof/solver/simulation only after real non-stub Python implementations and hot callers exist.

One thing now has an active implementation plan:

- **RustyWeb V0+.** `docs/plans/rusty-red-web/implementation-plan.md`; recover the burst-crawler scaffold, move the real product toward a RustyRed-backed graph crawler, and bridge Index-API through a client.

Everything else in the corpus is already built, already owned by the offload lane, already captured in the commonplace plan, or intentionally deferred behind the offload economics. No requirement was dropped silently; each row above is its accounting.
