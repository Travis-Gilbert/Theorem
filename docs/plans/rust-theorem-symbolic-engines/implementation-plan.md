# Rust Theorem: Native Symbolic Engines, Implementation Plan

**Status:** RT-0 through RT-4 implemented and validated on 2026-05-29. RT-5.2a evolution archive native path added on 2026-05-29; remaining RT-5 work stays profile/use-case driven.
**Date:** 2026-05-29
**Spec backrefs:** Part 4 Part C (Path 2 native Rust; ordering Datalog, then probabilistic, then causal, then optimizer, then the rest), Part 4.1 sequencing addendum ("Native Rust promotion (Path 2) for hot engines. Datalog and probabilistic first, when profiling justifies it"), Part 3 Section 1.4 (Datalog before Z3 before e-graphs).

Every checklist item carries a spec backref. No item is a silent scope cut. Deferrals are surfaced individually in Section 8.

**Implementation closeout:** The baseline in Section 1 records the pre-implementation source reality used to scope the work. The completed slice now has shared Rust receipt logic in `rustyredcore_THG/crates/rustyred-thg-core/src/symbolic.rs`, PyO3 exports in `rustyredcore_THG/src/bgi.rs`, native capability handshakes in `apps/notebook/inference_engines/datalog/native.py`, `apps/notebook/inference_engines/probabilistic/native.py`, and `apps/notebook/inference_engines/evolution/native.py`, parity-aware kernel/affordance routing, and RustyRed THG MCP symbolic tools. The native Datalog implementation tracks the current Python `DEFAULT_RULES` dynamically; at closeout that means 14 rules, including the civic `demolition_window`, `conflict_set`, `vacancy_duration`, and `ownership_chain` rules that landed after this plan's original ten-rule wording. RT-5.2a adds the first demand-driven cold-list engine promotion: the MAP-Elites/evolution archive path now has a Rust receipt implementation, PyO3 export, native-aware Python adapter, and parity coverage while preserving the Python engine as the reference contract.

**Closeout validation:** `maturin develop --manifest-path rustyredcore_THG/Cargo.toml` rebuilt `theseus_native` into `.venv`; `rustyredcore_THG/tests/test_datalog_derivation_parity.py` passed 3/3 byte-parity tests; focused Django native/affordance/kernel/civic tests passed 23/23; Rust core symbolic tests passed 2/2; RustyRed THG MCP symbolic tests passed 2/2; `rustyredcore_thg` PyO3 crate tests passed 4/4.

---

## 1. Verified code reality (2026-05-29)

Original baseline, read from the live tree before RT-0 implementation (file:line anchors):

- **Python reference engines (the parity target):**
  - `apps/notebook/inference_engines/datalog/contracts.py`: `DatalogFact`, `DatalogFactPack` (deterministic `pack_hash`), `DerivedFact`, `DatalogReceipt`. Hashing via `_stable_hash` = `sha256(json.dumps(value, sort_keys=True, separators=(',',':'), default=str))`.
  - `apps/notebook/inference_engines/datalog/rules.py`: `DEFAULT_RULES`, ten faithful rules (`unsupported_claim`, `dependent_claim`, `source_reused_support`, `likely_duplicate_entity`, `evidence_path_too_long`, `claim_has_no_independent_support`, `object_in_unresolved_tension_neighborhood`, `code_symbol_touched_by_failing_postmortem_pattern`, `context_atom_tainted_by_generated_artifact`, `private_source_reaches_export_candidate`).
  - `apps/notebook/inference_engines/datalog/engine.py`: `DatalogEngine.derive(fact_pack, rule_ids=None)`. Engine id `python-reference-datalog`. Output sorted by `(rule_id, subject_id, fact_id)`, deduped by `fact_id`.
  - `apps/notebook/inference_engines/datalog/native.py`: `NativeDatalogEngine(DatalogEngine)` already bridges to `theseus_native.bgi_datalog_derive_core_json`, gated by `bgi_native_symbolic_enabled()`, restricted to `native_rule_ids = {'unsupported_claim', 'likely_duplicate_entity'}`, falls back to Python otherwise.
  - `apps/notebook/inference_engines/probabilistic/engine.py`: `ProbProgEngine` Beta-Bernoulli `source_reliability` and `expected_value_of_information`. Engine id `beta-binomial-python-fallback`.
  - `apps/notebook/inference_engines/probabilistic/contracts.py`: `PosteriorReceipt` with content-addressed `receipt_hash` via `stable_hash`.
  - `apps/notebook/inference_engines/egraph/native.py`: `NativeEGraphTheorem(EGraphTheorem)` bridges to `theseus_native.bgi_egraph_extract_context_pack_json` for the `context_pack` domain only.

- **Rust native scaffold (`rustyredcore_THG/src/bgi.rs`, 458 LOC):**
  - Hashing: `bgi_stable_hash_json`, `bgi_fact_pack_hash_rows_json` (parity helpers for `stable_hash` / `fact_pack_hash`).
  - Receipt summaries: `bgi_datalog_receipt_summary_json`, `bgi_egraph_receipt_summary_json`, `bgi_compact_receipts_json`.
  - Datalog: `bgi_datalog_derive_core_json`. Implements only two rules, and they diverge from the Python rule semantics (the Rust `unsupported_claim` checks `claim.status in {accepted, supported}`; the Python rule checks for absence of supporting `evidence_link` of relation_type in {supports, derived_from, cites, references} and absence of `claim_dependency`). `let _iteration = datafrog::Iteration::new();` is created and never used: the loops are hand-rolled, not datafrog joins.
  - Egraph: `bgi_egraph_extract_context_pack_json` (one domain, two rewrites: `drop_empty_optional`, `dedupe_same_obligation`; `egg` used for a smoke probe only).
  - Probabilistic: none in Rust.
  - Causal, optimizer, proof, solver, simulation, evolution, expression: none in Rust.

- **PyO3 module wiring:** `rustyredcore_THG/src/lib.rs:57-74` registers the `bgi_*` functions into the `theseus_native` pymodule. Module name override `#[pyo3(name = "theseus_native")]` is load-bearing (PT-004). Build via `pyproject.toml` + maturin. Workspace `Cargo.toml` already declares `datafrog = "2.0"` and `egg = "0.10"`.

- **Kernel routing:** `apps/notebook/inference_kernel/` holds `native_strategy.py`, `router.py`, `registry.py`, `execution.py`, `persistence.py`, `contracts.py`, `settings.py`. Flag `bgi_native_symbolic_enabled()` reads `THESEUS_BGI_NATIVE_SYMBOLIC_ENABLED` (default True).
  - **The reachable native call site is `execution.py:108-117`.** For `kernel_id == 'bgi_datalog_deriver'` it does `engine = NativeDatalogEngine() if payload.get('native', True) else DatalogEngine()`, so native is the default. For the egraph kernel it calls `NativeEGraphTheorem().context_pack(...)` unconditionally.
  - **`native_strategy.py` is a guardrail layer (`NativeFeatureSpec`, `native_enabled`, `can_write_canon` gate, `THESEUS_DISABLE_NATIVE`) that `execution.py` does NOT consult for the datalog/egraph choice.** The governance exists but is not wired into the decision; `execution.py:110` makes a raw `payload.get('native', True)` choice instead.

- **Parity harness that already exists:** `apps/notebook/benchmarks/bgi_native_parity.py` + `apps/notebook/management/commands/bgi_native_parity.py` + `rustyredcore_THG/tests/test_bgi_parity.py`. The native parity surface is real; this plan extends it rule-by-rule rather than inventing a new one.

- **Boot probe precedent:** `apps/notebook/apps.py` logs an INFO/WARNING when `theseus_native` is absent at boot (the push_ppr fallback observability from PT-004). Extend the same probe to cover the symbolic functions.

## 2. The load-bearing invariant and the two risks that flow from it

**Invariant (parity):** for the same fact pack and rule set, the native receipt must be byte-identical to the Python reference receipt on the fields the differential gate compares (`fact_pack_hash`, the sorted `derived_facts`, `rule_ids`). The compute-offload Gate 0 (`apps/notebook/inference_engines/benchmark/differential.py`, owned by the other lane) is the external judge of this invariant. The Rust theorem is "done" for an engine only when Gate 0 passes with the native path enabled for that engine.

- **R1 (serialization byte-parity).** Python hashes with `json.dumps(sort_keys=True, separators=(',',':'), default=str)`. The Rust `canonical_json` uses `serde_json::to_string`. serde_json's `Map` is a `BTreeMap` (sorted keys) only when the `preserve_order` feature is OFF; confirm it is off in `Cargo.lock`. Float formatting must also match (Python repr vs serde_json `f64`): integer-valued floats (`1.0`), long mantissas (`0.6666666666666666`), and `confidence` values are the danger cases. R1 is the foundation; if it is wrong, every receipt hash diverges and nothing else matters. RT-0 nails it with golden vectors before any rule work.

- **R2 (reachable correctness bug, fix first).** This is not dormant. `execution.py:110` defaults to `NativeDatalogEngine()` (`payload.get('native', True)`), the flag `THESEUS_BGI_NATIVE_SYMBOLIC_ENABLED` defaults True, and the native engine diverges from Python two ways:
  1. The two implemented rules use different predicate logic than the Python rules (Rust `unsupported_claim` keys on `claim.status in {accepted, supported}`; Python keys on the absence of supporting `evidence_link`/`claim_dependency`).
  2. With `rule_ids=None` (the "run all rules" call), `native.py`'s guard `requested and not set(requested).issubset(native_rule_ids)` short-circuits on the empty set (empty set is a subset of everything), so the native path runs and returns only its 2 hardcoded rules, while the Python path would return all 10. Requesting all rules silently drops 8.
  So any caller of the `bgi_datalog_deriver` kernel today gets a receipt that would fail Gate 0. Until RT-1 lands faithful rules, RT-0 must (a) shrink `native_rule_ids` so the native path is never silently wrong AND fix the empty-`rule_ids` short-circuit so "all rules" falls back to Python until all ten are native, then (b) make the rules faithful in RT-1. Pick (a) as the immediate safety patch.

## 3. Phasing and checklist

### RT-0: Parity foundation and safety patch

- **RT-0.1** (Part 4 Part C "receipt surface stays identical"): Add Rust-to-Python golden-vector parity tests for the serialization primitives. Extend `rustyredcore_THG/tests/test_bgi_parity.py` and `apps/notebook/benchmarks/bgi_native_parity.py` with cases that assert `bgi_stable_hash_json(x) == _stable_hash(x)` for: nested dicts (key-order), integer-valued floats, long-mantissa floats, unicode, empty containers. Confirm `serde_json` `preserve_order` is off in `Cargo.lock`.
- **RT-0.2** (Part 4.1 "the receipt contract is the invariant"): Safety patch R2. In `apps/notebook/inference_engines/datalog/native.py`: (1) shrink `native_rule_ids` to only rules proven faithful (empty until RT-1), and (2) fix the empty-`rule_ids` short-circuit so a `rule_ids=None` ("all rules") call falls back to Python until all ten rules are native (currently the empty-set-subset test lets it run the 2-rule native path and silently drop 8). Add a regression test asserting native and Python receipts match for the rules still in `native_rule_ids`, and that `rule_ids=None` returns the full 10-rule Python result.
- **Acceptance:** golden vectors pass; no rule is marked native-safe unless its native receipt byte-matches Python on a known fact pack.

### RT-1: Datalog native parity, all ten rules

(Part 4 Part C: Datalog first. Part 3 Section 1.4: Datalog before Z3 before e-graphs.)

- **RT-1.1** Port the six relational rules with real `datafrog` relations and joins (replace the unused `Iteration`): `unsupported_claim`, `dependent_claim`, `source_reused_support`, `likely_duplicate_entity`, `claim_has_no_independent_support`, `object_in_unresolved_tension_neighborhood`. Match the Python predicate logic exactly (relation_type sets, status exclusions, threshold counts).
- **RT-1.2** Port the four scan/predicate rules: `evidence_path_too_long` (default `max_length=3`), `code_symbol_touched_by_failing_postmortem_pattern`, `context_atom_tainted_by_generated_artifact`, `private_source_reaches_export_candidate`.
- **RT-1.3** Match `DerivedFact` byte-for-byte: `fact_id` = `_stable_hash({rule_id, relation, subject_id, attributes, dependency_fact_ids})`; identical `reason` strings (the reason text is in the receipt and is part of the hash surface); `confidence`; `writeback_policy` (note `likely_duplicate_entity` and `private_source_reaches_export_candidate` are `proposal-only`, not `read-only`); `attributes` shape and key order.
- **RT-1.4** Match receipt assembly: dedupe by `fact_id`, sort by `(rule_id, subject_id, fact_id)`, `fact_pack_hash` from the pack, `rule_ids` reflecting the selected set.
- **RT-1.5** Resolve the `engine` field policy. Python emits `engine='python-reference-datalog'`; native emits `engine='rust-datafrog-core'`. Read `benchmark/differential.py` (other lane) to confirm whether `engine` is in the compared surface. If it is, coordinate one of: exclude `engine` from the diff, or have the native receipt carry the reference engine id with a `native_backend` attribute. Do not change `differential.py` unilaterally; raise it with the offload lane.
- **RT-1.6** Restore the faithful rules to `native_rule_ids` in `datalog/native.py` one at a time, each gated by a passing parity test.
- **Acceptance:** for each of the ten rules, native receipt == Python receipt on `fact_pack_hash` + sorted `derived_facts` + `rule_ids` for a representative fact pack; Gate 0 (other lane) passes with native Datalog enabled.

### RT-2: Probabilistic native parity

(Part 4 Part C: probabilistic second. "Native probabilistic for the Beta-Bernoulli source-reliability math.")

- **RT-2.1** Add `bgi_probabilistic_source_reliability_json` and `bgi_probabilistic_expected_value_json` to `src/bgi.rs`. Match the math exactly: `alpha = prior_alpha + max(0, corroborated)`, `beta = prior_beta + max(0, contradicted)`, `mean = alpha/total`, `variance = (alpha*beta)/((total^2)*(total+1))`; EVI = `max(0, current_uncertainty - expected_uncertainty_after) * decision_value - validator_cost`.
- **RT-2.2** Emit a `PosteriorReceipt`-shaped dict byte-identical to Python including `receipt_hash` (content-addressed over engine, model_id, prior, observations, posterior, metadata). Float formatting is the hazard: add golden vectors covering `0.5`, `1.0`, and long-mantissa posteriors.
- **RT-2.3** Add `NativeProbProgEngine` Python bridge in a new `apps/notebook/inference_engines/probabilistic/native.py`, mirroring `datalog/native.py` (flag-gated, fall back to Python, only expose methods proven at parity).
- **RT-2.4** Register the two functions in `src/lib.rs` pymodule.
- **Acceptance:** native probabilistic receipt == Python on `receipt_hash` and `posterior` for source_reliability and EVI golden vectors; Gate 0 probabilistic parity (other lane) passes with native enabled.

### RT-3: Wire native into the kernel routing and the affordance seam

(Part 4 Part C: "callers do not change." Part 4.1: native is a transport choice behind the affordance contract.)

- **RT-3.1** Route the native-vs-Python choice through one parity-aware decision point. Today `execution.py:110` makes a raw `payload.get('native', True)` choice and bypasses the `native_strategy.py` guardrail entirely. Wire `execution.py` (the `bgi_datalog_deriver` and egraph branches at `execution.py:108-117`) through `native_strategy.native_enabled(...)` plus a parity-verified-rules check, so native is selected only when `bgi_native_symbolic_enabled()` AND the requested rules/methods are in the parity-verified set, else Python. Single decision point; do not scatter the choice.
- **RT-3.2** The affordance wrappers `run_datalog_affordance` and `run_probabilistic_*` in `affordances.py` instantiate `DatalogEngine()` / `ProbProgEngine()` directly. Introduce a native-aware engine factory so they pick the native engine when enabled, WITHOUT changing the `AffordanceReceipt` contract or the `affordance_id` strings. `affordances.py` is co-owned with Codex (offload lane): coordinate this one-line-per-callsite swap, do not rewrite the module.
- **RT-3.3** Extend the boot probe in `apps/notebook/apps.py` to log whether the native symbolic functions (`bgi_datalog_derive_core_json`, `bgi_probabilistic_*`) are present, mirroring the existing push_ppr probe. A silent fallback to Python on Railway must be observable.
- **Acceptance:** with the flag on and the wheel present, an affordance call returns a native-produced receipt that still satisfies the Gate 0 differential; with the wheel absent, it falls back to Python and logs a WARNING; the `AffordanceReceipt` shape is unchanged in both cases.

### RT-4: MCP exposure of the symbolic affordances (parity-gated)

(Part 4 Part C affordance surface: "callable by any participant, native models directly, visiting agents via the MCP shim." Mirrors the already-shipped `rustyred_thg_algorithm_*_inline` tools from `docs/plans/rustyred-inline-compute-mcp/`.)

- **RT-4.1** Add native MCP tools `rustyred_thg.symbolic.datalog_derive` and `rustyred_thg.symbolic.probabilistic_source_reliability` (and `..._expected_value`) in `rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs`, following the inline-algorithm registration pattern (dispatch at the tool match, payload handler, registry list). Input: inline fact pack / evidence records (JSON), mirroring the affordance input shapes. Output: the same receipt JSON.
- **RT-4.2** Bound the inline payload (reuse the inline-algorithm `payload_too_large` precedent; recommend a fact-count cap with an explicit error above it).
- **RT-4.3** Tests in the MCP crate: known fact pack in, deterministic receipt out, matching the Python reference.
- **Acceptance:** a visiting agent can call the Datalog and probabilistic affordances over MCP and get a receipt that matches the Python reference; tools appear in the MCP registry.

### RT-5: E-graph generalization and remaining engines (profile/use-case gated)

(Part 4 Part C ordering: "Proof, solver, e-graph, simulation, evolution, expression promoted as use cases demand." Path 2 promotes hot engines only.)

- **RT-5.1** E-graph: generalize `bgi_egraph_extract_context_pack_json` beyond the single `context_pack` domain only when a second real consumer exists; keep parity with the Python `EGraphTheorem` receipt.
- **RT-5.2a** Evolution archive: implemented. `bgi_evolution_archive_json` mirrors `EvolutionEngine.archive()` byte-for-byte for `archive_hash`, `elites_by_niche`, `rejected_count`, and `writeback_policy`. Hot callers (`archive_candidates`, policy evolution, kernel candidate archive, and MAP-Elites tick) now use `NativeEvolutionEngine`, which falls back to Python when the export is absent or native dispatch is disabled.
- **RT-5.2b** Causal, optimizer, proof, solver, simulation: still deferred. Promote to Rust one at a time only when each has a real non-stub Python implementation, a hot-path caller, and measured throughput pressure. Until then they stay Python (Path 1 already covers them via the engine cores).
- **RT-5.2c** Expression: do not port as a symbolic engine; it is the rendering/Scene OS layer, not a Rust symbolic-compute candidate.
- **Acceptance:** no engine is promoted to Rust without (a) a profiling justification and (b) a passing parity test against its Python receipt. This phase is explicitly demand-driven, not a checklist to clear.

## 4. The parity contract, stated once

For any engine E and input I:

```
native_receipt(E, I).compared_surface == python_receipt(E, I).compared_surface
```

where `compared_surface` is exactly what `benchmark/differential.py` asserts (do not invent a second definition). For Datalog: `fact_pack_hash`, sorted `derived_facts` (by `fact_id`), `rule_ids`. For probabilistic: `receipt_hash`, `posterior`. The native engine id and `native_backend` attribute are allowed to differ from the Python engine id only if RT-1.5 confirms `engine` is outside the compared surface.

## 5. Tests

- Rust unit/integration tests: `rustyredcore_THG/tests/test_bgi_parity.py` (extend per rule and per probabilistic method).
- Python-side parity harness: `apps/notebook/benchmarks/bgi_native_parity.py` and the `bgi_native_parity` management command (extend with the ten Datalog rules and the two probabilistic methods).
- Differential gate (other lane, do not modify): `apps/notebook/inference_engines/benchmark/differential.py` / `gate0.py`. Run it with the native flag on to validate end-to-end; report results to the offload lane.
- Build: maturin wheel from `pyproject.toml`; verify the symbol/module-name override still produces `theseus_native` (PT-004 regression guard).

## 6. Rollout

- `THESEUS_BGI_NATIVE_SYMBOLIC_ENABLED` currently defaults True. Until RT-1/RT-2 parity is proven, the native path must be empty-by-rule (RT-0.2), not flag-off, so the rest of the system keeps using Python transparently. As each rule/method reaches parity, it joins the native-safe set.
- Docker: the maturin wheel must be installed in both `Dockerfile.web` and `Dockerfile.worker` runtime images (the push_ppr wheel-install pattern). The RT-3.3 boot probe surfaces a missing wheel as a WARNING rather than a silent slow path.

## 7. Risks (carried from Section 2 plus build-time)

- **R1 serialization byte-parity.** Mitigation: RT-0.1 golden vectors; confirm serde_json `preserve_order` off.
- **R2 live divergent native rules.** Mitigation: RT-0.2 safety patch before anything else.
- **R3 lane collision.** `benchmark/` and `affordances.py` are the offload lane. Mitigation: Section 9 boundary; the only shared edit is the RT-3.2 engine-factory seam, coordinated, not unilateral.
- **R4 silent wheel fallback on Railway.** Mitigation: RT-3.3 boot probe + Dockerfile wheel install.
- **R5 datafrog learning curve.** The current scaffold avoided real datafrog (hand-rolled loops). RT-1.1 requires actual relations/joins. Mitigation: start with the two simplest relational rules, prove the datafrog pattern, then replicate.

## 8. Deferrals, surfaced individually (require Travis consent to defer further)

- RT-5 (e-graph generalization + the seven cold engines) is demand-driven by Part 4's own ordering, not a silent cut. Datalog and probabilistic are the named hot engines; the rest are explicitly "promoted as use cases demand" in Part 4 Part C. This is a spec-sanctioned deferral, recorded here so it is not assumed done.
- The native model roster (Part 4 Part A) and thought-vector capture (Part 4 Layer 3) are deferred by the compute-offload plan's sequencing item 4 and are out of scope for this plan. Not cut, sequenced.

## 9. Lane boundary (do not collide with the offload-cost-test team)

Per the project decision "claim-before-build on a shared multi-agent tree" and Travis's note that another Claude Code session plus a Codex agent own the offload cost test:

- **Other lane owns (no-touch from this plan):** `apps/notebook/inference_engines/benchmark/` (all of it: `gate0.py`, `differential.py`, `ledger.py`, `arms.py`, `preregistration.py`, `report.py`, `records.py`, `receipts.py`), `docs/plans/compute-offload/implementation-plan.md`.
- **Co-owned (coordinate before editing):** `apps/notebook/inference_engines/affordances.py` (Codex). This plan's only edit there is the RT-3.2 engine-factory seam.
- **This plan owns:** `rustyredcore_THG/src/bgi.rs`, `rustyredcore_THG/src/lib.rs` (registration lines), `rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs` (RT-4), `apps/notebook/inference_engines/datalog/native.py`, `apps/notebook/inference_engines/egraph/native.py`, the new `apps/notebook/inference_engines/probabilistic/native.py`, `apps/notebook/inference_kernel/native_strategy.py` and `apps/notebook/inference_kernel/execution.py` (RT-3.1 routing), `apps/notebook/benchmarks/bgi_native_parity.py`, `rustyredcore_THG/tests/test_bgi_parity.py`, `apps/notebook/apps.py` (probe extension).

Commit with explicit pathspecs only (never a bare `git commit`) because the working tree is shared.

## 10. Build order summary

RT-0 (safety + parity foundation) -> RT-1 (Datalog ten rules) -> RT-2 (probabilistic) -> RT-3 (kernel + affordance seam + boot probe) -> RT-4 (MCP exposure) -> RT-5 (demand-driven rest). RT-0 is the gate for everything; RT-1 and RT-2 are the value; RT-3 makes it reachable; RT-4 makes it reachable by visiting agents; RT-5 is profile-driven.
