# RT-5 Analysis: the e-graph and the former seven cold-list engines, native or not

**Status:** Decision-support analysis plus RT-5.2a update. Grounded in the live engine code 2026-05-29.
**Question:** What would it take to make RT-5 (e-graph generalization + the 7 cold engines) native Rust, what does each add, and what do we lose by leaving it Python?

---

## The framing that decides everything

Native Rust (Path 2) is never a correctness requirement. The Python engines ARE the reference; they are already correct, and the PyO3 bridge (Path 1) already lets the substrate call them. Native changes two things and only two:

1. **Cost/latency** for engines that run on a hot path (CPU-native beats the Python interpreter, and far beats doing the same work inside an LLM forward pass). This is the entire compute-offload thesis (Part 4.1).
2. **Transport** for substrate-native deploy and MCP reach to visiting agents without a Python process in the loop.

So a port is worth it only when an engine is BOTH (a) on a hot path and (b) doing real symbolic compute. An engine that is cold, or that is a lightweight fallback stub, gains nothing from a native port except the work of writing and byte-parity-verifying it.

A second fact that gates most of this: the symbolic affordances are not yet wired into the ask/query hot path (no caller of `run_*_affordance` in `services/` or `encode/`). Evolution was the exception because MAP-Elites is hot in discovery/orchestrate loops outside ask/query. Porting more engines before they have a real query or orchestration path is optimizing a path that is not yet load-bearing.

---

## Per-engine reality

| Engine | What it computes | Real compute or fallback stub? | Hot or cold (real callers) | Native already? |
|---|---|---|---|---|
| **datalog** | 14 derivation rules over fact packs | real | kernel + evidence_assembly | YES (verified, RT-1) |
| **probabilistic** | Beta-Bernoulli source reliability + EVI | real (but cheap math) | kernel | YES (verified, RT-2) |
| **egraph** | equivalence-preserving extraction; `context_pack` dedup/drop | real (egg-backed) | kernel | PARTIAL: `context_pack` domain native; other domains not |
| **causal** | `intervention_effect` = treated-mean minus control-mean; `recommend_experiment` | FALLBACK stub (mean diff, not DoWhy/EconML) | COLD (no external caller) | no |
| **optimizer** | `optimize` / `schedule_validators` (greedy/knapsack-ish selection under budget) | FALLBACK stub (greedy) | COLD | no |
| **proof** | `create_obligation` / `mark_unknown` (obligation bookkeeping) | FALLBACK stub (no prover) | kernel dispatch only | no |
| **solver** | `counterworld_from_problem` / `unsat_core_ref` (bounded counterworld) | FALLBACK stub (NOT real Z3 yet) | COLD | no |
| **simulation** | `dry_run` / `skipped` (records inputs + expected as a receipt) | FALLBACK stub (recorder) | kernel dispatch only | no |
| **evolution** | MAP-Elites quality-diversity `archive` | real | HOT: policy_evolution, map_elites_tick/bridge, archives, orchestrate tasks | YES (RT-5.2a) |
| **expression** | `compile_scene`: renders receipts into Scene OS packages | NOT symbolic; LLM/scene-backed | rendering layer | N/A (do not port) |

---

## What each adds native, and what we lose by leaving it Python

- **expression: do not port. Category error.** It is the LLM/Scene-OS rendering layer, not symbolic compute. It orchestrates generation and scene packaging. Native Rust adds nothing and removes the thing it is for. Leaving it Python loses nothing. Drop it from the RT-5 list.

- **causal, optimizer, proof, solver, simulation: defer; porting now is premature.** All five are explicit fallback stubs, cold or kernel-only, doing trivial deterministic work. Native adds: a faster path that nothing currently calls, for a computation that is a placeholder anyway. We lose, by leaving them Python: effectively nothing today. The higher-leverage move for these is to make the Python implementation REAL before any port: a real causal estimator, a real Z3-backed solver, a real proof checker. Porting a stub to Rust just produces a faster stub, and then a second rewrite when the real implementation lands. Solver is the worst port-now candidate: a faithful native version needs real SMT (z3.rs bindings) AND byte-parity with the Python receipt, which is far harder than the datalog rules were.

- **evolution: promoted now as RT-5.2a.** It was the only cold-list engine that was both real symbolic compute and already hot in discovery/orchestrate loops. Native adds throughput on archive operations at scale. The implementation keeps Python as the reference contract and routes `archive_candidates`, policy evolution, the kernel candidate archive, and `map_elites_tick` through `NativeEvolutionEngine`, which falls back to Python when the wheel/export is absent or native dispatch is disabled.

- **egraph generalization (RT-5.1): only when a second consumer exists.** Native `context_pack` is done. Generalizing the native e-graph to other domains (query/plan rewriting, claim canonicalization) adds substrate-native equivalence rewriting for those domains. We lose, by leaving it at `context_pack` only: native rewriting for domains that have no consumer yet. There is no second consumer today, so there is nothing to lose yet. Generalize when a real second rewrite domain appears.

---

## What would be NEEDED to port one (the per-engine recipe)

Same shape that worked for datalog/probabilistic, per engine:

1. Port the compute into `rustyredcore_THG/crates/rustyred-thg-core/src/symbolic.rs`, matching the Python receipt byte-for-byte (the hard part: float formatting, and any list ordering that feeds a content hash).
2. Add a `bgi_<engine>_*_json` PyO3 export in `bgi.rs` + register it in `src/lib.rs`.
3. Add the engine/method to the native verified export (`bgi_datalog_verified_rule_ids_json` equivalent) so the gate admits it.
4. Extend the byte-parity gate (`apps/notebook/benchmarks/datalog_derivation_parity.py`) with that engine's differential.
5. Wire the native-aware factory in the Python bridge (`<engine>/native.py`) so callers transparently pick native when verified.

Cost scales with the engine's compute complexity: a stub is a day; a real-Z3 solver with receipt parity is a different order of effort.

---

## Recommendation

RT-5 stays demand-driven, as the implementation plan already says, but with the list pruned and a concrete trigger:

- **Remove `expression`** from RT-5 entirely (not symbolic).
- **Hold `causal`, `optimizer`, `proof`, `solver`, `simulation`** until each has (a) a real (non-stub) Python implementation AND (b) a hot-path caller AND (c) a profiled bottleneck. Today none meet even one of the three.
- **`evolution`** is now the first RT-5 demand-driven port. Keep it covered by byte-parity tests before relying on it for throughput.
- **`egraph`** generalizes when a second rewrite-domain consumer exists.

The honest bottom line for the remaining cold engines: by NOT porting them we lose no correctness and no capability (Python + the PyO3 bridge already provide them substrate-side). We lose only a cost/latency optimization, and that optimization has no hot path to apply to until the symbolic affordances are wired into the real query path or the engines become real orchestration bottlenecks. The next leverage is on the wiring (RT-3) and on making the stub engines real, not on porting placeholders to Rust.
