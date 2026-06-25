# SPEC-GODEL-FEATURE-DSL-PARITY: Sandboxed Feature DSL for Safe Self-Modification

Status: Planning
Author: Claude (technical partner)
Audience: Travis, then Claude Code / Codex for execution
Format: Orchestrate (BGI3), no duration estimates, no em dashes
Parity source: Travis-Gilbert/Theseus SPEC-GODEL-BUILD-ORDER Section 7 (Feature DSL)
Depends on: SPEC-GODEL-SUBSTRATE-PARITY (every DSL feature is a config delta routed through the closing-loop gate)

---

## Executive Summary

- Goal: give the harness a safe way to express new ranking and relevance logic as data, so that when the system writes a new feature it produces a sandboxed expression tree, not Rust source. This is the one primitive that makes "the system writes new logic" tractable without making it dangerous.
- The shape: a whitelisted, tree-walking interpreter over a small expression language whose operations are graph traversal (delegating to the existing `graph.rs` algorithms) and cross-object predicates (field comparisons over `NodeRecord` and `EdgeRecord`), composed with arithmetic and boolean operators. The expression is stored as a serde enum AST. There is no source-string evaluation, no codegen, no unbounded recursion, and a hard step and depth budget so every evaluation terminates.
- Why it matters here and now: jcode ships a self-dev mode that edits its own source and reloads its binary, and it is seductive. The Gödel spec rejects that for a reason: arbitrary code is not an enumerable, testable surface, so the safety story collapses. The Feature DSL is the disciplined alternative. Route every self-authored-logic path through it.
- The tie to the substrate: a DSL feature is not trusted because it parsed. It is a config delta. It is shadow-evaluated, attributed, and kept only when it shows an attributable, statistically real, non-oscillating gain and degrades no fitness trait, exactly the closing-loop gate from the substrate spec.
- Out of scope: graph mutation from the DSL (it is read-only), a general-purpose language, source generation of any kind, and auto-applying a DSL feature without the substrate gate.

## Current Condition

What exists on `main` in `Travis-Gilbert/Theorem` under `rustyredcore_THG/crates/rustyred-thg-core/src`:

- `graph.rs`: the graph-algorithm operations a DSL would expose as traversal primitives: `expand_bounded(edges, seeds, max_depth)`, `expand_bounded_weighted(edges, seeds, max_depth, min_confidence)`, `paths_shortest(edges, source, target, max_depth)`, `paths_shortest_weighted(edges, source, target, max_depth)`. All bounded by a `max_depth`, all over `EdgeRecord` or `EdgeTuple`.
- `EdgeRecord` (in `graph_store.rs`): `from_id`, `to_id`, `edge_type`, `confidence: Option<f64>`, `effective_confidence()`, `tombstone`, `properties` (JSON). The edge-side predicate surface.
- `NodeRecord` (in `graph_store.rs`): id, label, `properties` (JSON). The node-side predicate surface.
- `executor.rs` and `planner.rs` (in `rustyred-thg-core`): existing query execution and planning, the precedent for a bounded evaluator over the store. The DSL is a narrower, pure evaluator in the same spirit.
- `ranking.rs` (in `rustyred-thg-core`): the ranking and signal-fusion layer where a feature value would be consumed as an additional scored signal.
- The substrate spec's closing-loop gate (`loop_gate.rs`, GS-08): the discipline that decides whether a new feature is kept.

Gap (the whole of this spec):

- There is no way to express a feature as data. Today a new ranking signal is Rust code. There is no sandboxed expression language, no AST type, no bounded evaluator, and no fuzz harness proving evaluation is safe on adversarial input.
- There is therefore no safe path for the system to author new logic. Absent this, the only options are no self-authored logic at all, or source generation, which the substrate spec forbids.

## Goal

- System behavior:
  - A feature is a value of a serde-serializable AST type. It is evaluated against a focus object and its graph neighborhood to produce a score (f64) or a predicate (bool).
  - Evaluation is total: every AST, including adversarial or malformed ones, terminates within a fixed step budget and a fixed traversal-depth budget, and never panics. There is no path to arbitrary computation, file access, network, or graph mutation.
  - Traversal operations delegate to the existing `graph.rs` algorithms with the DSL's depth budget passed through, so the DSL reuses the audited traversal rather than reimplementing it.
  - A feature value plugs into `ranking.rs` as one more scored signal, alongside the structural-activation and PPR-proximity signals.
  - A newly authored feature (whether written by a human, distilled by EBL, or proposed by the system) is a config delta. It enters the substrate spec's closing-loop gate and is kept only on an attributable, significant, non-oscillating gain with no fitness-trait degradation.
- Data and model changes:
  - One new module (or small crate); no schema change. Features are stored as serialized AST blobs in the config ledger from the substrate spec.
- What must not regress:
  - `graph.rs`, `ranking.rs`, `executor.rs` behavior and tests.
  - The no-unsafe, no-IO posture of the evaluator.

## Context Stack

| Context | Source | Trust | Why it matters |
| --- | --- | --- | --- |
| Parity source | Theseus SPEC-GODEL-BUILD-ORDER Section 7 | high | the Feature DSL design: whitelisted tree-walking interpreter, AST not source, fuzz-tested |
| Traversal operations | `rustyred-thg-core/src/graph.rs` (`expand_bounded`, `paths_shortest`, weighted variants) | high | the traversal primitives the DSL exposes, already bounded by `max_depth` |
| Predicate surface | `graph_store.rs` (`EdgeRecord`, `NodeRecord`, `properties`, `edge_type`, `effective_confidence`) | high | the fields cross-object predicates compare |
| Interpreter precedent | `rustyred-thg-core/src/executor.rs`, `planner.rs` | medium | the existing bounded evaluator-over-store pattern; the DSL is narrower and pure |
| Feature consumption | `rustyred-thg-core/src/ranking.rs` | medium | where a feature value joins fusion; confirm the exact signal-injection point during execution |
| Discipline | SPEC-GODEL-SUBSTRATE-PARITY `loop_gate.rs` (GS-08) | high | a feature is a config delta; it is gated, not trusted on parse |
| Contrast | jcode self-dev mode (edits own source) | high | the thing this primitive exists to avoid |

## Delegation Map

| Work type | Route to | Why |
| --- | --- | --- |
| AST type and serde | execute mode | a closed enum; named variants below are requirements |
| Whitelisted evaluator with budgets | execute mode + validator | totality and the no-escape posture are the validator's concern |
| Traversal delegation to `graph.rs` | execute mode | reuse the audited algorithms; do not reimplement |
| `ranking.rs` signal injection | execute mode | confirm the exact injection point against `ranking.rs` |
| Fuzz harness | execute mode + validator | adversarial-input totality is the safety proof |

## Action Rail

| Action | Risk | Validator | Approval | Route |
| --- | --- | --- | --- | --- |
| Define the AST as a closed serde enum | low | round-trip serde test | none | execute |
| Build the evaluator with a step budget and a depth budget | medium (totality) | fuzz: random ASTs never panic and never exceed budgets | none | execute + validator |
| Delegate traversal ops to `graph.rs` with the depth budget passed through | low | unit test: traversal result matches calling `graph.rs` directly | none | execute |
| Inject a feature value into `ranking.rs` as a scored signal | medium (ranking path) | snapshot test of ranking with and without the feature | confirm injection point with Travis | execute |
| Gate every new feature through the substrate closing-loop gate | low (reuses GS-08) | a feature with no gain is not kept | none | execute |
| Let the system author and auto-apply features without the gate | never | n/a | not allowed | n/a |

## Checklist

| ID | Task | Grounding | Route | Acceptance criteria | Validation | Risk | Status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| FD-01 | Define `FeatureExpr` as a closed serde enum (new module `feature_dsl.rs`): `Const(f64)`, `NodeField{key}`, `EdgeField{key}`, `Compare{op, lhs, rhs}` (op in a fixed set: lt, le, gt, ge, eq), `And/Or/Not`, `Arith{op, lhs, rhs}` (op in add, sub, mul, min, max), `Traverse{kind, max_depth}` (kind in a fixed set: expand, expand_weighted, shortest_path), `Count(inner_traversal)`, `Exists(predicate_over_traversal)`. No string-eval variant, no user-function variant. | Theseus Feature DSL AST; `EdgeRecord`/`NodeRecord` fields | execute | The enum compiles, derives `Clone, Debug, Serialize, Deserialize`, and round-trips through JSON. | serde round-trip test | low | planned |
| FD-02 | Build the evaluator `eval(expr, focus, neighborhood, budget) -> EvalResult` (f64 or bool): a tree walk with a decreasing step budget (every node visited decrements it) and a max traversal depth. When the step budget hits zero the evaluation returns a defined sentinel (not a panic, not a loop). No recursion path is unbounded. No IO, no graph mutation, no unsafe. | Theseus tree-walking interpreter; `executor.rs` evaluator pattern | execute + validator | A pathological deeply-nested AST returns the sentinel within the budget; evaluation never panics, never mutates, never performs IO. | fuzz harness (FD-05) plus unit tests | medium | planned |
| FD-03 | Traversal delegation: `Traverse` and `Count` and `Exists` call the existing `graph.rs` algorithms (`expand_bounded`, `expand_bounded_weighted`, `paths_shortest`) with the DSL depth budget passed as `max_depth`. The DSL never walks edges itself. | `graph.rs` traversal ops | execute | A `Traverse{expand, 2}` over a fixture yields the same node set as calling `expand_bounded(.., 2)` directly. | unit test against `graph.rs` | low | planned |
| FD-04 | Predicate surface: `NodeField` and `EdgeField` read from `NodeRecord.properties` / `EdgeRecord.properties` and the typed fields (`edge_type`, `effective_confidence()`), returning a defined default for missing keys (never a panic). `Compare` and `Arith` operate on the numeric or string projection. | `graph_store.rs` `NodeRecord`, `EdgeRecord` | execute | A `Compare{ge, EdgeField(confidence), Const(0.5)}` is true for a 0.6-confidence edge and false for a 0.3 one; a missing field yields the default, not a panic. | unit test | low | planned |
| FD-05 | Fuzz harness (proptest or cargo-fuzz): generate random `FeatureExpr` trees and random small graphs; assert evaluation always terminates within the budget, never panics, never mutates the store, and returns a typed result. | Theseus "fuzz-tested" requirement | execute + validator | The fuzz target runs a large corpus with zero panics, zero budget overruns, zero mutations. | fuzz run in CI | medium | planned |
| FD-06 | Inject a feature value into `ranking.rs` as an additional scored signal: given a feature AST and a candidate's focus object plus neighborhood, evaluate and contribute the score to fusion, behind a config flag default off. | `ranking.rs`; the structural-activation and `ppr_proximity` signal precedent | execute | With the flag on, the feature's score participates in ranking; with it off, ranking is byte-identical. | snapshot test with flag on and off | medium | planned |
| FD-07 | Route a new feature through the substrate closing-loop gate: register a newly authored feature as a config delta in the config ledger, shadow-evaluate it, and keep it only on an attributable, significant, non-oscillating gain with no fitness-trait degradation. | SPEC-GODEL-SUBSTRATE-PARITY `loop_gate.rs` (GS-08), `config_ledger.rs` (GS-06) | execute | A feature that does not improve the composite is not kept and is rolled back via the ledger; a feature that does is kept and logged. | integration test through GS-08 | low | planned |

## Test Strategy

- Preflight: `cargo build -p rustyred-thg-core` after each block.
- Unit:
  - Serde round-trip of the AST (FD-01).
  - Traversal delegation matches `graph.rs` directly (FD-03).
  - Predicate evaluation and missing-field defaults (FD-04).
  - Budget sentinel on a pathological AST (FD-02).
  - Ranking byte-identical with the feature flag off (FD-06).
- Fuzz: the FD-05 target proves totality, no panic, no mutation, no budget overrun on adversarial ASTs and graphs. This is the safety proof and is non-negotiable.
- Integration: a feature flows through the substrate gate and is kept or rolled back on its composite effect (FD-07).
- Static and lint: `cargo clippy -p rustyred-thg-core`; assert no `unsafe`, no IO crates, no mutation API reachable from the evaluator.

## Production Gates

- [ ] Tests pass or failures are explained.
- [ ] The fuzz harness is green: zero panics, zero budget overruns, zero mutations over a large corpus.
- [ ] The evaluator has no path to IO, network, graph mutation, or unsafe code.
- [ ] No source-string evaluation and no codegen exist anywhere in the module.
- [ ] A DSL feature is never auto-applied; it always passes through the substrate closing-loop gate.
- [ ] Ranking is byte-identical with the feature flag off.
- [ ] Docs and codebase map updated; the parity source and the substrate dependency are cited.
- [ ] Final report reconciles every checklist item.

## Epistemic Ledger

| Primitive | Entry | Evidence | Confidence | Action |
| --- | --- | --- | --- | --- |
| Claim | The DSL can reuse audited traversal rather than reimplementing it. | `graph.rs` exposes bounded `expand_bounded`, `paths_shortest`, and weighted variants. | high | delegate; the DSL never walks edges itself (FD-03). |
| Claim | This is the disciplined alternative to source-editing self-dev. | jcode edits its own source and reloads; the Gödel spec forbids that for safety. | high | route every self-authored-logic path through this AST. |
| Claim | Totality is provable by construction plus fuzzing. | a closed enum, no recursion without a decrementing budget, no loop construct, delegated bounded traversal. | high | FD-05 is the proof; treat a single fuzz panic as a release blocker. |
| Assumption | `ranking.rs` has a clean point to inject an additional scored signal. | the structural-activation and `ppr_proximity` signals already flow through ranking, but the exact injection API is unread here. | medium | confirm the injection point against `ranking.rs` during FD-06; if it differs, adapt the signal-contribution call, not the AST. |
| Tension | A feature is only as safe as the gate that admits it. | a parsed AST is total but could still be a bad ranking signal. | medium | FD-07 makes every feature a config delta through GS-08; nothing is trusted on parse. |
| Gap | Whether the system (as opposed to a human or EBL) should author features at all in v1. | the substrate gate makes it safe in principle, but the proposer is out of scope here. | low | ship the DSL and the human and EBL authoring path first; a system proposer is a later, gated addition. |

## Explicit Non-Goals and Deferrals

| Item | Why deferred | Risk | Follow-up |
| --- | --- | --- | --- |
| Graph mutation from the DSL | the DSL computes features and predicates; it must not write | high if violated | never; mutation stays in the store API outside the evaluator |
| A general-purpose language | the DSL is narrow by design (traversal plus predicates plus arithmetic and boolean) | medium if widened | resist new variants unless a ranking need forces one, and even then keep the enum closed |
| Source generation of any kind | this primitive exists to avoid it | high if violated | never |
| A system feature-proposer | safe in principle via the gate, but out of scope here | medium | a later, gated addition once human and EBL authoring is proven |
| Auto-applying a feature on parse | a parsed AST is total but not necessarily good | high if violated | always route through the substrate gate (FD-07) |

## Execution Instructions

- Start with the AST and the evaluator: FD-01, then FD-02, then FD-03 and FD-04. This is the whole primitive and it is pure, so it lands and reviews in isolation.
- Then prove it safe: FD-05, the fuzz harness, before wiring the DSL into anything. A green fuzz run is the gate to the rest.
- Then wire it: FD-06 (ranking injection behind a default-off flag), then FD-07 (route through the substrate gate).
- Preserve: `graph.rs`, `ranking.rs`, `executor.rs` behavior and tests, and the no-unsafe, no-IO posture of the evaluator.
- Run after each block: `cargo test -p rustyred-thg-core`, the fuzz target, `cargo clippy -p rustyred-thg-core`.

## Recommended First Step

Land FD-01 plus FD-02 plus FD-05 in one pass: the closed AST enum, the budgeted tree-walking evaluator, and the fuzz harness, with the budget sentinel and the no-panic, no-mutation properties asserted. That is the smallest slice that proves the primitive is safe by construction, which is the entire point of building a DSL instead of generating source, and it makes the safety proof the first thing that exists rather than the last.
