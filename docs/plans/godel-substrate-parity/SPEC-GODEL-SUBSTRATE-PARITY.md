# SPEC-GODEL-SUBSTRATE-PARITY: Self-Modification Substrate for the Harness

Status: Planning
Author: Claude (technical partner)
Audience: Travis, then Claude Code / Codex for execution
Format: Orchestrate (BGI3), no duration estimates, no em dashes
Parity source: Travis-Gilbert/Theseus docs/plans/2026-04-07 SPEC-GODEL-BUILD-ORDER (Batch 0 and Batch 3) and SPEC-GODEL-PATTERNS

---

## Executive Summary

- Goal: give Theorem's harness the substrate that makes self-modification safe, so it can tune its own configuration space (routing weights, thresholds, pipeline order, ranking features) under a discipline that only keeps a change when the change is attributable, statistically real, non-oscillating, and reversible.
- The Theseus build-order spec's central insight is that seven self-modification patterns collapse into three primitives plus two guarantees: an improvement-rate tracker, an attribution table, and a shadow evaluator, gated by safety invariants and rollback. This spec ports those, not the seven patterns.
- The reason to do this in the harness rather than port Theseus: the harness already has the hard parts. `session_metrics::compare_modes` is a baseline-versus-candidate comparator with a Welch z-score and a 90 percent confidence gate. `replay::fork_run` and `fork_events` fork a run and replay it deterministically, and `compare_runs` diffs two runs. `versioned_graph` branches, checks out, diffs, and rolls back graph state. `head_fitness` already runs an attribution-to-config bandit loop (node receipts update Laplace-smoothed per-head fitness) with an epsilon-explore floor that preserves disagreement. The substrate exists at the metric, run, and graph levels; this spec composes it into the closing-loop discipline.
- The load-bearing constraint, held throughout: the system modifies its configuration space, never its own Rust source. The composite metric it optimizes is itself fixed and not self-modifiable, because a system that can tune its own success metric optimizes the metric, not the work.
- Out of scope: the seven patterns themselves (they ride on this substrate and are separate specs), source-code self-editing, self-modification of the composite metric, and concurrent multi-loop modification.

## Current Condition

What exists on `main` in `Travis-Gilbert/Theorem` under `rustyredcore_THG/crates/theorem-harness-core/src`:

- `session_metrics.rs`: `SessionMetricsState` per session (input and output tokens, tool calls, `task_completion: bool`, `task_category`, `workstream_id`). `load_jsonl_metrics` reads a JSONL stream. `summarize_pairformer_ab` buckets completed sessions by category and mode. `compare_modes(baseline, candidate)` returns a baseline-versus-candidate comparison with `welch_z`, a `token_reduction`, and `confidence_90_bar_met` (both arms at least 50, z at least 1.645, reduction positive). This is the outcome signal and the significance gate.
- `replay.rs`: runs are event-sourced. `replay_events(events)` rebuilds state by folding `apply_transition` over ordered events. `fork_run(run, through_step_id, actor)` and `fork_events(events, through_event_seq, actor)` fork a run and replay its prefix. `compare_runs(before, after)` diffs two runs into added, removed, and changed steps. Steps carry kinds (`tool_call`, `observation`, `validation`) and provenance (`replayed_from_step_id`).
- `head_fitness.rs`: `HeadFitness` with `FitnessCounter` (Laplace-smoothed `accepted/total` rate, neutral 0.5 cold start) keyed by `(node_type, head)`, updated by `NodeResult::{Accepted, Rejected}` receipts from verification. `route(node_type, explore_token)` is an epsilon-explore bandit (default 15 percent explore) that preserves a per-head floor so disagreement does not die. The explore token is caller-supplied so runs replay deterministically.
- `budget.rs`: a token-bucket budget primitive (the rate-limiter shape).
- `constitution.rs` and `alignment.rs`: the values and drift surface.
- `federated_signals.rs`: cross-session feedback signals.
- `versioned_graph.rs` (in `rustyred-thg-core`): content-addressed Prolly-tree graph versioning. `GraphSnapshot`, `diff_graph_snapshots` returning added, removed, modified, unchanged, `checkout_graph_version`, `merge_graph_snapshots`, `compile_graph_pack_incremental`. Graph-level isolation, diff, and rollback.
- `Candidate` (in `rustyred_membrane`) carries an `epistemic` sub-struct with `support_ratio` and `source_reliability`. Two of the anti-conspiracy fitness traits are already first-class fields.

Gap (the whole of this spec):

1. No composite metric. The axes exist separately (`task_completion`, tokens, tool calls); there is no single versioned composite the discipline can track and gate on, and no fitness-trait safety sub-score.
2. No improvement-rate tracker. `compare_modes` is a point-in-time A/B; there is no first or second derivative over a window, and no non-oscillation check.
3. No generalized attribution. `head_fitness` attributes outcomes to heads only; nothing attributes a composite delta to a config delta, a skill, a tool, or a context atom.
4. No shadow-eval orchestrator. The primitives (fork, replay, compare, branch, checkout) exist; nothing strings them into "fork a corpus of past runs, replay under a config variant, score the composite delta, return a verdict."
5. No anti-conspiracy fitness-trait gate. The traits are not yet a single check that a self-modification must not degrade.
6. No config-delta ledger with inverses. Nothing records an applied config change with its inverse for single-command rollback.
7. No closing-loop gate. Nothing composes attribution, shadow eval, and rate stability into the Invariant 12 decision.

## Goal

- System behavior:
  - A proposed config delta (for example, a routing weight change, a threshold, a pipeline reordering, a ranking-feature toggle) is evaluated on a shadow before it is applied: a corpus of past runs is forked and replayed under the variant, the composite delta is measured, and a verdict is returned with the significance bar attached.
  - A delta is applied permanently only when its contribution is attributable to it (the attribution table credits it with a positive composite delta), statistically real (`confidence_90_bar_met`), and non-oscillating over a window (the rate tracker shows stability). Otherwise it is reverted.
  - Any applied delta has a single-command rollback through the config ledger and `checkout_graph_version`.
  - Only one delta is under evaluation at a time, so attribution stays clean. The rate limiter (token bucket) bounds how fast loops close and decays the budget when recent closures earned no composite gain.
  - No self-modification degrades a fitness trait (root depth, source independence, support ratio, claim specificity, temporal spread). A delta that collapses source independence is rejected even if it improves the productivity composite.
- Data and model changes:
  - New modules in `theorem-harness-core`; no schema change beyond a config-delta ledger (append-only, with inverses).
  - The composite metric is versioned and fixed in v1, not self-modifiable.
- What must not regress:
  - Existing `session_metrics`, `replay`, `head_fitness`, and `versioned_graph` behavior and tests.
  - Run determinism (every new decision point takes a caller-supplied token, matching the `head_fitness` and replay discipline).

## Context Stack

| Context | Source | Trust | Why it matters |
| --- | --- | --- | --- |
| Parity source | Theseus SPEC-GODEL-BUILD-ORDER (Batch 0, Batch 3), SPEC-GODEL-PATTERNS | high | the three primitives, Invariant 12, the safety invariants, rollback |
| Outcome signal and significance gate | `theorem-harness-core/src/session_metrics.rs` (`compare_modes`, `welch_z`, `confidence_90_bar_met`, `load_jsonl_metrics`) | high | the composite is built over these axes; the 90 percent bar is the antidote to a gameable signal |
| Run fork, replay, diff | `theorem-harness-core/src/replay.rs` (`fork_run`, `fork_events`, `replay_events`, `compare_runs`) | high | run-level shadow-eval isolation and comparison |
| Attribution precedent | `theorem-harness-core/src/head_fitness.rs` (`FitnessCounter`, `NodeResult`, epsilon-explore floor) | high | the attribution-to-config loop to generalize; the anti-monoculture floor is the source-independence precedent |
| Graph isolation and rollback | `rustyred-thg-core/src/versioned_graph.rs` (`GraphSnapshot`, `diff_graph_snapshots`, `checkout_graph_version`, `merge_graph_snapshots`) | high | graph-level shadow-eval and single-command rollback |
| Rate limiter | `theorem-harness-core/src/budget.rs` | high | bound how fast loops close; decay budget on no-signal closures |
| Fitness-trait fields | `rustyred_membrane` `Candidate.epistemic` (`support_ratio`, `source_reliability`) | high | two traits already exist as fields to read |
| Values and drift | `theorem-harness-core/src/constitution.rs`, `alignment.rs` | medium | where a fitness-trait degradation gate belongs alongside existing guardrails |
| Federated feedback | `theorem-harness-core/src/federated_signals.rs` | medium | a future cross-session signal source for the composite; not required in v1 |

## Delegation Map

| Work type | Route to | Why |
| --- | --- | --- |
| Composite metric module | execute mode | named axes and weights are requirements; the metric must be fixed and versioned |
| Improvement-rate tracker | execute mode | derivative and non-oscillation math over the existing JSONL metrics |
| Generalized attribution | execute mode + validator | generalize `head_fitness`; the credit-assignment correctness is the validator's concern |
| Shadow-eval orchestrator | execute mode + validator | composes fork, replay, compare; the corpus-replay fidelity is the validator's concern |
| Fitness-trait gate | execute mode | the five traits as a single check; two are existing fields |
| Config-delta ledger and rollback | execute mode | append-only with inverses; reuse `checkout_graph_version` |
| Closing-loop gate | execute mode + validator | the Invariant 12 composition; this is the safety-critical seam |

## Action Rail

| Action | Risk | Validator | Approval | Route |
| --- | --- | --- | --- | --- |
| Define the composite metric, fixed and versioned, not self-modifiable | low (read-only over existing axes) | unit test: composite is deterministic and stable for a fixed input | confirm the axis weights with Travis | execute |
| Build the improvement-rate tracker over a window | low | unit test: derivatives and oscillation flag on synthetic series | none | execute |
| Generalize attribution from heads to config keys | medium (credit assignment) | unit test: a known-good delta gets positive credit, a no-op gets neutral | none | execute |
| Build the shadow-eval orchestrator over fork plus replay plus compare | medium (corpus replay) | the orchestrator reproduces a known run's outcome on replay; compare matches | none | execute + validator |
| Add the fitness-trait gate (the five anti-conspiracy traits) | low | unit test: a source-independence-collapsing delta is rejected | none | execute |
| Add the config-delta ledger with inverses and single-command rollback | medium (state mutation) | unit test: apply then rollback restores prior state byte-identically | none | execute |
| Compose the closing-loop gate (Invariant 12) | high (this is the safety seam) | a delta closes only when attributable and significant and non-oscillating; an oscillating delta never closes | confirm the gate logic with Travis before enabling any live closure | execute + validator |
| Enable a live self-modification loop | deferred | n/a | explicit, not this slice | not this slice |

## Checklist

| ID | Task | Grounding | Route | Acceptance criteria | Validation | Risk | Status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| GS-01 | Add `HarnessComposite` (new module, e.g. `metrics_composite.rs`): a fixed weighted sum over normalized axes derived from `SessionMetricsState` (task completion rate, token efficiency, tool-call efficiency), plus a separate fitness-trait safety sub-score. Versioned via a `composite_version` constant. Named choice: the composite is not self-modifiable; the safety sub-score is kept separate from the productivity score, never averaged in. | `session_metrics.rs` axes; Theseus 7-axis IQ as the shape | execute | `composite(metrics)` is deterministic; changing `composite_version` is the only way the formula changes; safety sub-score is reported separately. | unit test on a fixed metrics fixture | low | planned |
| GS-02 | Add an improvement-rate tracker (e.g. `improvement_rate.rs`): over a sliding window of composite values, compute the first derivative (trend), the second derivative (acceleration), and an oscillation flag (sign-change frequency over the window). Reuse `load_jsonl_metrics` for the series source. | `session_metrics.rs` `load_jsonl_metrics`; Theseus rate tracker | execute | On a monotone series, oscillation flag is false and trend is positive; on a flip-flop series, oscillation flag is true. | unit test on synthetic series | low | planned |
| GS-03 | Add generalized attribution (e.g. `attribution.rs`): generalize `head_fitness::FitnessCounter` from `(node_type, head)` to `(config_key, outcome)`. For each completed run, read the run trace (`replay::replay_run` steps) and the composite delta, and credit the config keys that participated. Use leave-one-out over recent runs or the Laplace-smoothed counter, matching the `head_fitness` discipline. | `head_fitness.rs` `FitnessCounter`; `replay.rs` step trace; `session_metrics.rs` composite | execute + validator | A config key present only in runs with positive composite delta accrues positive credit; a key present equally in positive and negative runs accrues neutral. | unit test with seeded runs | medium | planned |
| GS-04 | Add the shadow-eval orchestrator (e.g. `shadow_eval.rs`): given a config delta and a corpus of past runs (or graph snapshot), fork and replay the corpus under the variant (`replay::fork_events` plus `replay_events`, or a `versioned_graph` branch via `checkout_graph_version`), compute the composite on the replayed outcomes, and return a verdict using `session_metrics::compare_modes` for the significance gate and `replay::compare_runs` for the step diff. | `replay.rs` fork and replay and compare; `versioned_graph.rs` branch and checkout; `session_metrics.rs` `compare_modes` | execute + validator | Replaying a known corpus under a no-op delta yields `insufficient_change`; under a known-good delta yields a positive composite delta with the significance bar reported. | integration test on a fixture corpus | medium | planned |
| GS-05 | Add the fitness-trait gate (e.g. `epistemic_fitness.rs`, named to avoid collision with `head_fitness.rs`): the five anti-conspiracy traits (root depth, source independence, support ratio, claim specificity, temporal spread) computed over the graph state a delta would produce. Read `support_ratio` and `source_reliability` from `Candidate.epistemic` where available; compute the rest from graph structure. A delta that degrades any trait below its prior value is rejected. | `Candidate.epistemic` (`support_ratio`, `source_reliability`); `head_fitness` anti-monoculture floor as the source-independence precedent; Theseus anti-conspiracy traits | execute | A delta that collapses source independence (concentrates support on one source) is rejected even when it raises the productivity composite. | unit test with a source-collapsing delta | low | planned |
| GS-06 | Add the config-delta ledger (e.g. `config_ledger.rs`): an append-only log of applied config deltas, each with its inverse, plus the graph version checked out at apply time. `rollback(delta_id)` applies the inverse and `checkout_graph_version` to the prior version. | `versioned_graph.rs` `checkout_graph_version`; Theseus applied-delta log | execute | Apply a delta then `rollback(delta_id)` restores the prior config and graph version byte-identically. | unit test: apply then rollback round-trip | medium | planned |
| GS-07 | Add the rate limiter on loop closing: wrap loop closure in a `budget.rs` token bucket; decay the budget when the last closures earned no composite gain (read from the attribution table). | `budget.rs`; Theseus rate limiter with decay-on-no-signal | execute | After several closures with no composite gain, the budget shrinks and further closures are throttled. | unit test on the bucket decay | low | planned |
| GS-08 | Add the closing-loop gate (e.g. `loop_gate.rs`): compose GS-03, GS-04, GS-05, GS-02 into the Invariant 12 decision. A delta closes (is applied permanently) only when attribution credits it positively (GS-03), shadow eval passes the 90 percent bar (GS-04), the rate tracker shows non-oscillation over the window (GS-02), and no fitness trait degrades (GS-05). One delta at a time (a single in-flight lock). | all prior items; Theseus Invariant 12 | execute + validator | An oscillating delta never closes; a delta failing the significance bar never closes; a fitness-degrading delta never closes; a delta passing all four closes and is logged in the ledger. | integration test exercising each rejection path | high | planned |

## Test Strategy

- Preflight: `cargo build -p theorem-harness-core` after each module; `cargo build` workspace after the `versioned_graph` and `Candidate` reads are wired.
- Unit:
  - Composite determinism and stability (GS-01); the safety sub-score is reported separately, never folded into productivity.
  - Rate tracker: monotone series gives no oscillation and positive trend; flip-flop series gives oscillation true (GS-02).
  - Attribution: positive credit for a delta present only in positive runs, neutral for an ambiguous key (GS-03).
  - Fitness gate: a source-collapsing delta is rejected (GS-05).
  - Ledger: apply then rollback restores prior state byte-identically (GS-06).
  - Rate limiter decay on no-gain closures (GS-07).
- Integration:
  - Shadow-eval reproduces a known run outcome on replay and reports the significance bar (GS-04).
  - Closing-loop gate exercises every rejection path and the one acceptance path (GS-08).
- Determinism: every new decision point (explore, sampling, tie-break) takes a caller-supplied token, matching `head_fitness` and `replay`, so any loop replays.
- Regression: rerun `session_metrics`, `replay`, `head_fitness`, and `versioned_graph` tests after each block.
- Static and lint: `cargo clippy -p theorem-harness-core` on touched modules.

## Production Gates

- [ ] Tests pass or failures are explained.
- [ ] No live self-modification loop is enabled in this slice (the substrate ships inert; enabling a loop is a separate, approved step).
- [ ] The composite metric is fixed and versioned; nothing in this slice lets the system change it.
- [ ] Every applied delta has a working single-command rollback (GS-06).
- [ ] The fitness-trait gate is wired into the closing-loop gate; no delta closes without passing it.
- [ ] Determinism preserved (caller-supplied tokens at every decision point).
- [ ] Observability: the shadow-eval verdict, the attribution credit, and the rate-tracker oscillation flag are all recorded per evaluated delta.
- [ ] Rollback and revert path proven (apply then rollback round-trip test green).
- [ ] Docs and codebase map updated; parity source cited.
- [ ] Final report reconciles every checklist item.

## Epistemic Ledger

| Primitive | Entry | Evidence | Confidence | Action |
| --- | --- | --- | --- | --- |
| Claim | The shadow-eval and rollback substrate already exists at three levels. | `session_metrics::compare_modes`, `replay::fork_run`/`fork_events`/`compare_runs`, `versioned_graph` branch and checkout and diff are all present. | high | compose, do not build from scratch. |
| Claim | The attribution-to-config loop already exists in miniature. | `head_fitness` updates Laplace-smoothed fitness from `NodeResult` receipts and routes with an epsilon-explore floor. | high | generalize the same `FitnessCounter` pattern from heads to config keys. |
| Claim | The 90 percent significance gate is the antidote to a gameable metric. | `compare_modes` requires both arms at least 50, z at least 1.645, reduction positive. | high | reuse it verbatim in the closing-loop gate. |
| Tension | The composite metric must be stable enough to gate a non-oscillation window, yet the harness has no IQ-equivalent today. | the axes exist (`task_completion`, tokens, tool calls) but are not fused; Theseus had a defined 7-axis composite. | medium | GS-01 defines a fixed composite over existing axes; treat its stability as the prerequisite and measure oscillation before trusting any gate. |
| Tension | Self-modification target. | jcode ships a self-dev mode that edits its own source; the Gödel spec rejects that. | high | the substrate modifies config space only; new logic goes through the Feature DSL (separate spec), never source generation. This is the load-bearing line. |
| Assumption | The five fitness traits are computable over a candidate graph state. | two (`support_ratio`, `source_reliability`) are existing `Candidate.epistemic` fields; the other three are structural. | medium | GS-05 computes the structural three from graph topology; confirm their definitions against the Theseus fitness implementation before tuning thresholds. |
| Gap | The benchmark headwind (EvolMem found memory agents failed to beat the base model; StructMemEval found flat beats hierarchical) means the gains may live in the noise. | prior research this conversation. | medium | the shadow-eval-before-commit discipline is the mitigation: a delta that does not show an attributable, significant gain never closes. Treat a long run of no-close verdicts as a real signal, not a bug. |

## Explicit Non-Goals and Deferrals

| Item | Why deferred | Risk | Follow-up |
| --- | --- | --- | --- |
| The seven patterns (P1, P2, P3, M1, M2, M3, M4) | they ride on this substrate; each is its own spec | none | spec them onto the initiatives already in flight (tool governor, ReasoningBank, ranking) after the substrate lands |
| Source-code self-editing | the safety story depends on an enumerable config space | high if violated | new logic goes through the Feature DSL spec, AST not source |
| Self-modification of the composite metric | a system that tunes its own metric optimizes the metric | high if violated | never; the metric is versioned by humans |
| Concurrent multi-loop modification | clean attribution requires one in-flight delta | medium | revisit only after single-loop closure is proven stable |
| Enabling a live loop | the substrate ships inert; enabling is a separate approved step | high if rushed | a follow-up once GS-08 is validated and a first safe config dimension is chosen |

## Execution Instructions

- Build the read-only pieces first: GS-01 (composite) and GS-02 (rate tracker), both pure over existing metrics, both safe to land and review in isolation.
- Then the evaluators: GS-03 (attribution), GS-04 (shadow eval), GS-05 (fitness gate). Each composes existing primitives and is independently testable.
- Then the state and safety: GS-06 (ledger and rollback), GS-07 (rate limiter), and finally GS-08 (the closing-loop gate), which is the safety seam and lands last.
- Ship the substrate inert. Do not enable a live self-modification loop in this slice.
- Preserve: existing `session_metrics`, `replay`, `head_fitness`, `versioned_graph` behavior and tests, and run determinism.
- Run after each block: `cargo test -p theorem-harness-core`, `cargo clippy -p theorem-harness-core`, workspace build after the cross-crate reads.

## Recommended First Step

Land GS-01 plus GS-02 in one pass: define `HarnessComposite` over the `session_metrics` axes with a separate fitness-trait safety sub-score and a `composite_version` constant, and the improvement-rate tracker with the oscillation flag, each with a unit test. That is the smallest slice that establishes the metric the entire discipline gates on, it is read-only over data that already exists, and it forces the hardest question (is the composite stable enough to gate on) to the front, which is exactly where the gating risk should be confronted.
