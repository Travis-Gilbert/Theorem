# Compute-Offload — Reconciled Plan (Theorem-native, harness centerpiece, OSS-portable)

Date: 2026-06-27. Reconciles the scattered Theseus-era compute-offload notes
(`Index-API/Theseus/Compute Offload.md`, `Index-API/Theseus/CommonPlace/compute-offload-cost-test.md`,
architecture `part-4-1.md`) with what now exists natively in Theorem, and sets the build path to make
compute-offload a centerpiece of the harness and portable to a standalone open-source repo.

## 1. The thesis (one line)
**GPU inference is the scarce operator; the substrate is a query planner that routes each operation to its cheapest-sufficient executor and minimizes GPU operations.** Symbolic offload is the floor; the full win is five cost axes stacked — plus a sixth, verification offload, that targets quality.

The five axes (from the 2026 DB/cascade literature, reconciled in the notes):
1. **Symbolic offload** — Datalog/logic, probabilistic (Beta-Bernoulli), constraint (Z3), graph algorithms run on CPU, not in the LLM forward pass. Cheaper (~2 orders/op) AND more correct (exact vs approximate).
2. **Predicate pushdown** — run cheap CPU filters first so the expensive LLM op runs on pruned input (often bigger than the offload itself).
3. **Operator fusion** — batch N LLM ops into one call (matters most in the RustyWeb crawl/ingest path).
4. **Model cascade** — cheap model first, escalate on *calibrated* confidence (isotonic on token-level uncertainty — never intuition thresholds). Literature: 45-85% cost cut at ~95% quality (classification; expect less on synthesis).
5. **Computation reuse** — read already-computed results (reliability, derivations, embeddings) from the graph instead of recomputing. This is the **margin moat that widens** with the corpus — stateless competitors recompute every query.
6. **Verification offload** (2026 addition) — synthesis stays on GPU, but the substrate *checks* the model's output cheaply (Datalog re-derives, the graph confirms claims against the corpus, Z3 checks constraints), cutting the expensive LLM self-correction/re-query loop and raising trust. Targets **quality**, not just cost — the harder moat. Grounded in NSVIF / ConstraintLLM.

## 2. Reconciliation: old notes vs Theorem now
The notes were written for **Theseus** (Python + the PyO3 bridge, MAGE, Django receipts). The instruction now: build it **natively in Theorem (Rust substrate)**. Status:

| Axis / piece | Theseus-era note | Theorem now (built this session, PR #60) | Gap to close |
| --- | --- | --- | --- |
| Move 1 symbolic offload | planned, primary test | `rustyredcore_THG/crates/rustyred-thg-offload`: `OffloadClassifier` (4 symbolic kinds vs `NeuralSynthesis`), `OffloadEngine` (classify→route→ledger), `OffloadAffordance` seam, `OffloadLedger` (`gpu_seconds_saved`). ONE real affordance: exact PageRank via `rustyred_thg_core::pagerank`. Ambient `OffloadPass` routes change-derived centrality through it. | Wire the other affordances: Datalog (`datafrog`), probabilistic (Beta-Bernoulli), Z3/constraint, more graph algos. Route real query ops, not just the ambient centrality. |
| Move 2 pushdown | planned (DBPlanBench) | — | the planner: reorder cheap CPU filters before expensive LLM ops. |
| Move 3 fusion | planned | — | batch LLM ops; first target = RustyWeb crawl path. |
| Move 4 cascade | planned (45-85%) | — | cheap→escalate router + **calibrated** confidence (isotonic regression on token-level uncertainty). Maps to the model roster. |
| Move 5 reuse | planned (the moat) | the graph (commonplace + RedCore) IS a durable cache; nothing reads it as a *computation* cache yet | reuse-lookup keyed on op+inputs + a **staleness/invalidation** policy keyed on graph version (serving stale computed results is a correctness bug). |
| **The query planner** | "the architectural piece that doesn't exist" | — | `OperationPlanner`: decompose a query into ops, route each to cheapest-sufficient executor, minimize GPU. The Pairformer's richer role = *operation-level cost routing*, not just participant relevance. |
| Cost test / benchmark ledger | Gate 0 / Gate 1 (A/B0/B1/B2) + cascade arm + reuse arm; pre-registered | the `OffloadLedger` records `gpu_seconds_saved` per decision (a seed) | build the full benchmark ledger; **it doubles as the router's training data + the cascade's calibration data** (one run, three payoffs). |

Net: **Move 1's skeleton + the cost ledger exist natively.** The planner, cascade, reuse, and the benchmark are the build.

## 3. Native-Theorem build path (phased; each phase oracle-gated by `cargo test` + `clippy`)
- **Phase 1 — affordance bench (extend what's built).** Wire the existing substrate engines as `OffloadAffordance`s behind the seam: Datalog (`datafrog`), probabilistic (Beta-Bernoulli reliability), graph algos (PageRank done; add community/centrality/shortest-path), Z3 if/when present. Each affordance is "real CPU compute vs LLM approximation," with a differential-correctness check (Gate 0: substrate affordance vs a reference receipt — assert identical, no cost claims until it passes).
- **Phase 2 — the `OperationPlanner`.** Decompose an operation set; route each op to {CPU affordance | cached result | cheap model | expensive model}; implement pushdown (Move 2) + fusion (Move 3). Emits a replayable plan + cost estimate.
- **Phase 3 — the cascade (Move 4).** cheap→escalate with isotonic-calibrated token-level confidence. No guessed thresholds; calibration is a hard dependency fed by Phase 5.
- **Phase 4 — reuse (Move 5).** Read computed results from the graph keyed on (op, inputs); staleness/invalidation keyed on `stats().version` so a graph mutation invalidates dependent cached computations.
- **Phase 5 — the benchmark ledger / cost test.** Port the Theseus cost-test spec to Theorem: Gate 0 (differential correctness) → Gate 1 (A / B0 oracle-routed / B1 deterministic-planner / B2 LLM-tool-calling) → cascade arm → reuse arm, with the **pre-registered predictions** (carried forward below). Produces the calibration + router-training data.

## 4. Pre-registered ceiling (carried forward from claude-code's 2026-05-28 position — do not let the goalposts move)
| Axis | Predicted cost reduction | Quality |
| --- | --- | --- |
| Move 1 symbolic offload, mixed queries | 10-25% | symbolic exact; synthesis >= baseline |
| Move 4 cascade, full stream | 25-45% | 88-93% retention (NOT the lit's 95% — synthesis calibration is harder) |
| Move 5 reuse, realistic-locality stream | 15-35% (≈0 on a fully-novel stream) | must hold a staleness policy |
| **COMBINED, realistic mixed stream** | **40-60%** | >= 90% |

**GO** if >= 40% sustained combined reduction at >= 90% quality with a reuse hit-rate that grows with corpus size. **REPRICE** if < 30%. The honest story is a structural margin that **widens over time**, not orders of magnitude (synthesis tokens dominate and stay on GPU).

## 5. Harness centerpiece
With the `OperationPlanner` over the affordances, the harness stops being "coordinate agents" and becomes "route every operation to its cheapest executor." Compute-offload is then the harness's **economic engine**: a CPU-floor cost structure (CPU compute + occasional GPU) that makes a credit-per-month product viable where the all-LLM competitor's floor is GPU-per-thought. The cognitive-OS framing from the notes lands here: the planner is the OS scheduler for *cost*.

## 6. OSS-portable standalone repo
`rustyred-thg-offload` is already the seed: a focused crate whose only non-trivial dep is the affordance it wires, behind a trait. Carve the **portable core** = the operation taxonomy + classifier + cost model + `OperationPlanner` + cascade + calibration + ledger, with ALL substrate specifics (PageRank-via-rustyred-thg-core, Datalog-via-datafrog, the RedCore reuse cache) behind the `OffloadAffordance`/executor traits. The OSS repo's pitch: **"a query planner that treats LLM inference as the expensive operator to minimize"** — a general library with backend adapters. Theorem ships the substrate adapters; the open-source core has no proprietary deps. Decision point: carve now (small core, clean seam) or after the planner lands (richer core, more to extract).

## 7. Field-grounded refinements (2026 research) + the proxy/MCP reframe
Web-grounded review (FrugalGPT, RouteLLM, UCCI, LOTUS/Palimpzest, BigQuery/AlloyDB AI proxies, GPTCache, NSVIF, ConstraintLLM) sharpens the plan:

- **Build the OperationPlanner as a real cost-based query optimizer (LOTUS/Palimpzest pattern), not ad-hoc routing.** The biggest measured wins live here: LOTUS ~10^3x operator / ~3.6x program; learned-proxy offload (BigQuery/AlloyDB) ~329-991x latency and ~728-792x cost by running a cheap proxy and escalating to the LLM only on hard rows. Give the planner a logical/physical operator algebra + a per-operator cost model; predicate-pushdown and reordering are first-class rewrites. The planner is the headline lever, not a side-axis.
- **Verification offload is the sixth axis and the one that moves QUALITY.** NSVIF (per-constraint Z3 checkers) + ConstraintLLM (~2x on its benchmark): symbolic checking of LLM output cuts the self-correction/re-query loop AND raises trust. The substrate's structural edge — it has the graph to check against. This is the lever on the "will they want it" risk, not just "can we afford it".
- **The cascade's value is calibration; the heterogeneous roster is a routing asset.** FrugalGPT up to 98% cost cut; RouteLLM 2-3.66x; UCCI: isotonic calibration ECE 0.12->0.03, 31% cost at F1 0.91, O(n^-1/3) sample complexity — and the punchline: most cascade value comes from a *calibrated* routing signal, not threshold tuning (validates the plan's isotonic insistence). RouterEval found a "model-level scaling-up phenomenon": more candidate models => routing beats the best single model. So Claude/Codex/31B/local-MoEs make the router structurally better; the Pairformer = the learned cost-based router (train on preference data + the benchmark ledger).
- **Reuse: the substrate already solves the field's open problem, and gets two upgrades.** GPTCache 2-10x on hits, but the field's unsolved issues are "mismatch cost" + staleness/invalidation — both handled by the substrate's (op, inputs)-keyed, provenance-bearing, `stats().version`-invalidated cache. Upgrades: (a) **cache-as-a-router-arm** — a cached exact result is a zero-cost candidate the router tries before any model; (b) **code-synthesis surrogates** (Palimpzest) — compile a repeated LLM op into reusable CPU code once (fuses with the skill-encoder/Futamura direction).
- **Keep NL->formal translation on the LLM.** The field's named failure mode is translating natural language into Datalog/SMT/graph queries (error-prone) — the one step LLMs do well. Division of labor: LLM translates, substrate solves exactly. Spend the model budget on translation quality.

### The proxy/MCP reframe (changes the sequencing)
Compute-offload is NOT gated on a self-hosted GPU cluster. The five app-level axes (symbolic offload, pushdown, cascade, reuse) + verification work for **proxy/API models (Claude, Codex, Gemini) via MCP tool-calling**: the affordances (Datalog, Z3, graph algos, PageRank) are reachable as harness tools — kept behind the **affordance-router (`tool_search`/`invoke`)**, not advertised as N flat model tools — the API model invokes one, the CPU runs it, the exact result returns to context. The provider bills only **tokens**; the heavy compute is on your CPU. So wiring `rustyred-thg-offload`'s affordances into the affordance-router gives the harness's CURRENT proxy agents offload **now** (cost lever = saved tokens), with the version-invalidated reuse cache widening the margin. `compute_code` is already a live instance.

| Axis | Proxy/API (via MCP affordance-router) | Self-hosted | Cost lever |
| --- | --- | --- | --- |
| symbolic offload / pushdown / cascade / reuse / verification | yes (tool_search/invoke) | yes | proxy: **tokens** + fewer expensive calls · self-hosted: **GPU-seconds** |
| speculative decoding / constrained-grammar decoding / KV-cache reuse / custom batching | no (no control of the sampler) | yes only | GPU FLOPs/token |

The **serving-level** techniques (speculative/constrained decoding, KV reuse, batching) are a SECOND tier that only applies when the standing-participant MoE roster runs self-hosted — they stack on top of the MCP tier, not replace it. Near-term win: route proxy agents' ops to CPU affordances over MCP; the self-hosted cluster is an amplifier added later. This refines but does not change the pre-registered ceiling (40-60% combined cost at >=90% quality; verification additionally moves quality).

## 8. Open decisions (for Travis)
- **Cost test sequencing (decided):** NOT front-loaded and NOT deferred — it runs *just after* the build phases. Travis has the credits; its pricing-validation purpose has lessened (the pricing model evolved), but it still produces the calibration + router-training data Phases 3-4 depend on, so it follows the build directly rather than gating it or being pushed out indefinitely.
- **OSS split timing** — carve the portable core now vs after Phase 2.
- **Cascade roster** — the notes name Qwen3-Coder-Next (3B active) → GLM-5.1 → DeepSeek V4. Confirm the current roster.
- **Reuse staleness policy** — version-keyed invalidation is the safe default; confirm acceptable staleness windows per op type.

## Sources reconciled
- `Index-API/Theseus/Compute Offload.md` (the five-axis reframe, cognitive-OS framing, sequencing).
- `Index-API/Theseus/CommonPlace/compute-offload-cost-test.md` (the runnable A/B spec, buckets, success criteria).
- architecture `part-4-1.md` (offload as monetization anchor; Gate 0/1/2 + four-arm design; pre-registration).
- Built this session: `rustyred-thg-offload` + the ambient `OffloadPass` (PR #60 "Coedit + Local files system").
