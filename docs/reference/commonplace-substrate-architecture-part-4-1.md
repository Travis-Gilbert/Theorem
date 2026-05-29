# Commonplace: The Cognition Substrate — Part 4.1

**Architectural / Technical Document, Revision and Test Plan**
**Session date: 2026-05-28 (Opus 4.8)**
**Status: Revision to Part 4. The compute-offload hypothesis as the monetization path, the RunPod test design, and the revised coding-model roster. This is the part that pays the rent while the frontier pieces mature.**

---

## Why 4.1 exists

Part 4 laid out the native model roster and the Theseus mirror. Two corrections and one test plan emerged after it.

First: rereading the whole arc with a skeptical eye, the strongest near-term claim is the dullest one. The substrate offloads computation from GPU inference to CPU substrate compute, and a heterogeneous swarm on shared substrate does more useful work per dollar than a single frontier model doing everything. This claim survives the most scrutiny, needs the least new research, and maps directly to runway. The exotic pieces (thought-vector coordination, xLSTM lingua franca, cross-model latent reasoning) are the long-term moat. Compute-offload is what makes the substrate a credit-per-month product in the next two quarters.

Second: the model roster under-weighted coding capability. The February 2026 open-coding model drops changed the landscape enough that the specific picks should change.

Third: the compute-offload hypothesis is testable now, without fine-tuning, on RunPod credits, using the inference engines that already exist. This is the test to run.

A discipline note that frames all of this: the four architecture documents are unvalidated by implementation. Four intelligences converging on the Rust mirror is a good signal, but it is partly correlated reasoning from shared priors and shared docs. A closed loop of agents agreeing amplifies confidence faster than it earns it. The corrective is to ship the boring first projection and watch it work or break. 4.1 is that boring first projection turned into a measurable test.

---

## Part A: The compute-offload hypothesis stated precisely

The claim, in falsifiable form:

> For a meaningful class of operations currently performed inside LLM forward passes, performing them instead as substrate-resident CPU computation (Datalog derivation, probabilistic inference, constraint solving, graph algorithms) produces equivalent or better results at materially lower cost, because CPU compute is roughly two orders of magnitude cheaper per operation than GPU inference and is not supply-constrained.

The class of operations in scope:
- Exact logical derivation ("what follows from these facts") — Datalog, not an LLM reasoning chain.
- Source reliability and expected-value-of-information — the probabilistic engine's Beta-Bernoulli math, not an LLM estimate.
- Constraint satisfaction ("can these assumptions all hold") — Z3, not an LLM trying to reason about consistency.
- Graph algorithms (PageRank, community detection, shortest paths) — MAGE/RustyRed, not an LLM approximating graph structure.
- Causal effect estimation — the causal engine, not an LLM guessing.

Each of these is something LLMs do *badly and expensively*. They hallucinate logical derivations, they approximate probabilities poorly, they cannot actually solve constraints, they cannot compute PageRank. Offloading them to substrate compute is both cheaper and more correct. That is the double win: the cost goes down and the quality goes up, because these are exactly the operations symbolic compute does better than neural approximation.

### Why this is the monetization path

GPU inference is the expensive, supply-constrained part of the stack. H100 spot pricing rose ~40% between October 2025 and March 2026 due to broad GPU supply constraints. Every operation moved off the GPU forward pass and onto CPU substrate compute is cost removed from the part of the stack that is both most expensive and least able to scale with demand.

This enables a credit-per-month product model. If the substrate does a large fraction of the "thinking" as CPU symbolic computation, and only routes to GPU inference the operations that genuinely require a neural model, then the cost-per-query drops far enough that a flat monthly credit allocation becomes viable. The competitor model (every thought is an LLM forward pass) cannot offer this because their cost floor is GPU inference on every operation. The substrate's cost floor is CPU compute plus occasional GPU inference. Different cost floor, different pricing model, defensible margin.

This is also the part of the architecture that is true regardless of whether the exotic pieces work. Even if thought-vector coordination never reaches useful fidelity, even if the xLSTM lingua franca is a dead end, the compute-offload claim stands on its own. It is the floor under the whole project.

---

## Part B: The RunPod test design

The hypothesis is testable now. No fine-tuning required. The test measures cost and quality delta between two configurations on the same task set.

### Infrastructure context (RunPod, switched from Modal)

RunPod serverless bills per compute-second and scales to zero when idle. This is better-suited than Modal for this test because the cost structure is transparent per-second and there is no always-on spend. Relevant facts:
- Serverless bills per second of actual compute; scale-to-zero when idle; no idle charges.
- vLLM integration is native with an OpenAI-compatible API; drop-in deployment of open-weight models.
- Cold starts: serverless flex workers 6-12s for large containers, sub-200ms for ~48% of cold starts with FlashBoot; pods 15-30s. For a batch test cold start is amortized; for production it matters and argues for keeping a warm worker on the default coding model.
- Storage: network volumes ~$0.05-0.07/GB/month standard, model weights persist across pods.
- Default max concurrent workers per endpoint is 5; scaling beyond requires higher account balance.
- Token-per-dollar on RunPod is roughly 175K/dollar vs 38-67K/dollar on hyperscalers; the GPU-native-cloud inference economics gap is real and documented.

### The two configurations

**Configuration A — pure LLM (the baseline / the competitor model).**
A single capable model (or the swarm) answers the full query set, including the logical derivation, the reliability estimation, the constraint checking, the graph reasoning, all done inside LLM forward passes. Everything is GPU inference. This is how Mem0, Letta, and every all-LLM agent system works. Measure: total GPU-seconds, total cost, answer quality, error rate on the symbolic sub-tasks.

**Configuration B — substrate-offload.**
The same query set, but the logical derivation routes to the Datalog affordance, reliability to the probabilistic affordance, constraint checks to Z3, graph reasoning to MAGE/RustyRed. Only the operations that genuinely need a neural model (language understanding, synthesis, generation) hit the GPU. The symbolic operations run on CPU substrate compute. Measure: GPU-seconds (should drop substantially), CPU-seconds (cheap), total cost, answer quality, error rate on the symbolic sub-tasks (should improve, because symbolic compute is exact).

### The query set

Design queries that exercise the symbolic operations heavily. Examples:
- "Given these claims and their support relationships, what conclusions are entailed?" (Datalog vs LLM reasoning)
- "Which of these sources is most reliable given their corroboration history?" (probabilistic vs LLM estimate)
- "Can all of these constraints hold simultaneously, and if not, which conflict?" (Z3 vs LLM)
- "What are the most central entities in this knowledge subgraph?" (PageRank vs LLM approximation)
- Mixed queries that need both symbolic operations and neural synthesis (the realistic case).

The mixed queries are the important ones, because they show the realistic split: the substrate does the symbolic part on CPU, the LLM does the synthesis part on GPU, and the total cost is far below doing everything on GPU.

### What to measure

- **Cost delta.** GPU-seconds in A vs GPU-seconds in B, converted to dollars. Hypothesis: B uses materially fewer GPU-seconds because the symbolic operations moved to CPU.
- **Quality delta on symbolic sub-tasks.** Error rate on the logical/probabilistic/constraint/graph operations. Hypothesis: B is more correct because symbolic compute is exact and LLMs approximate.
- **Quality parity on synthesis.** The final synthesized answer quality should be at least as good in B, because the neural model still does the synthesis; it just gets correct symbolic inputs instead of having to approximate them itself.
- **Latency.** CPU symbolic operations are fast; the question is whether the round trip (substrate compute, return to LLM, synthesize) is faster or slower than the LLM doing it all in one pass. Measure honestly; it may be slower in wall-clock even when cheaper in cost.

### Success criteria

The hypothesis is supported if Configuration B costs materially less in GPU-seconds, is at least as correct on synthesis, and is more correct on the symbolic sub-tasks. If all three hold, the compute-offload claim is validated and the credit-per-month product model has a cost foundation.

The hypothesis is weakened if the round-trip overhead eats the GPU savings, or if the symbolic operations turn out to be a small fraction of real query cost (most cost is in synthesis, which still needs GPU). Either outcome is useful to know before building the product around it.

### What this test does NOT require

No fine-tuning. No GL-fusion. No thought-vector capture. No xLSTM. The test uses the inference engines as they exist (PyO3-bridged or called directly), open-weight models served via vLLM on RunPod serverless, and a query set. It is the cheapest possible validation of the most important claim.

---

## Part C: The revised coding-model roster

The Part 4 roster optimized for lineage diversity. Correct as principle, but under-weighted coding and predated the February 2026 drops. Revised roster:

### Coding tier (the addition Part 4 was missing)

- **Qwen3-Coder-Next (80B MoE, 3B active).** The cheap default coding head. 70.6% SWE-bench Verified with only 3B active params, runs on 46GB. For serverless per-second billing, 3B active means cheap inference and fast cycles. Fires most often because it is cheapest to run. Keep.
- **GLM-5.1 (or GLM-4.6).** The capable agentic coding head. Purpose-built for agentic coding and tool integration, wins on real-time responsiveness and tool-integrated benchmarks, up-to-8-hour long-horizon execution, MIT license, documented vLLM/SGLang serving. Already in the stack as a Z.ai API peer — run it as an API peer (pay-per-call) rather than self-hosting, which avoids standing serverless cost for the heavy coding model. Reach for it on real refactors and long agentic chains.
- **Qwen3-Coder-480B (35B active), optional heavyweight.** Best for repo-scale understanding and cross-file refactoring, 256K-1M context, Apache 2.0. Summon for big jobs only; overkill for most queries. Not a standing participant.

### Reasoning and translation

- **DeepSeek-R1-Distill-Qwen-32B.** The reasoning-chain head and the Qwen-family translation partner (within-family thought-vector translation lands well above the cross-family ~0.538 baseline). Keep.

### Generalist and synthesis

- **Mistral Medium 3.5 128B (bf16, unquantized).** Heavyweight generalist, distinct lineage, Tier 4 capable (native hidden states). Keep. (bf16 not GGUF, for native-hidden-state participation.)
- **Jamba Large 1.7.** Mamba-Transformer hybrid, linear-cost long context, the long-document synthesis head. Keep.

### Substrate working memory

- **xLSTM.** Not a peer. Substrate working memory and candidate lingua-franca representation space. Keep.

### The MoE-for-serverless principle

A point that matters for the cost test and the production economics: MoE models with low active-parameter counts are the right shape for serverless. You pay inference cost proportional to active params, not total. Qwen3-Coder-Next at 3B active, GLM and 480B at 35B active. A 480B MoE with 35B active costs roughly what a 35B dense model costs to run but reasons with the full 480B knowledge. MoE sparse activation is GPU cost reduction at the model level, complementing the substrate-offload cost reduction at the system level. Prefer low-active-param MoE models for standing participants; reserve dense or high-active models for summon-for-big-jobs roles.

---

## Part D: Calibration carried forward from the Opus 4.8 review

Three notes that revise the conviction level of Parts 1-4 without revising the architecture:

**Claim the mechanism, not the metaphysics.** The thought-vector architecture is buildable and structurally sound. But "episodic memory in the Tulving sense" and "the substrate is where cognition happens" are the interpretive layer, not the engineering layer. What is buildable: a versioned store of hidden states with relationships, retrievable and re-injectable, with measurable fidelity. The vec2vec ~0.538 cross-family number means re-injection is closer to a blurry photograph of a prior state than a full re-inhabitation. Positioning implication: claim "stores and re-injects reasoning state across sessions and models with measurable fidelity," not "gives AI episodic memory." The mechanism is defensible; the metaphysics invites a question whose honest answer is weaker than the phrase.

**The compute-offload story is the near-term anchor.** Documented above as Parts A and B. The exotic pieces are the long-term moat; compute-offload pays the rent. The inference-engines-as-affordances step is an instance of it, which is why it is both the first projection and the first revenue-relevant test.

**The docs are a map, not territory.** Four intelligences converging is a hypothesis, not a result. Coherence and cross-agent agreement are not "it works." The corrective is the sequencing: ship the release, fix fractal expansion, wire the inference engines, run the RunPod cost test, and verify real compute offload before the grander pieces.

---

## Sequencing addendum to Part 4

Per Parts 1-4: release first, fix fractal expansion, shared-room reliability before any new surface.

4.1 inserts a specific, measurable milestone into the inference-engine projection:

- **Wire Datalog and the probabilistic engine as substrate affordances** (PyO3 bridge, per Part 4 Part C). Confirm each engine's fact-assembly path — the clean `derive(fact_pack)` surface is decoupled, but `build_fact_pack_from_models` touches Django; the projection swaps it for `build_fact_pack_from_substrate`, and that is the actual per-engine work.
- **Run the RunPod cost test** (Part B above) on Datalog + probabilistic first, since those are the engines being wired first. Measure the GPU-second delta on a query set that exercises logical derivation and reliability estimation. This is the first empirical validation of the compute-offload hypothesis.
- **If the test supports the hypothesis**, the credit-per-month product model has a cost foundation, and the remaining engines (causal, optimizer, Z3, e-graph) get wired and tested the same way.
- **If the test weakens the hypothesis** (round-trip overhead eats savings, or symbolic ops are a small fraction of real cost), that is critical information before building the product around it. Adjust the pricing model and the architecture accordingly.
- **The revised coding roster** stands up on RunPod serverless with the MoE-for-serverless principle: Qwen3-Coder-Next as warm default, GLM-5.1 as API peer, the rest summoned as needed.

No timelines. The release goes first. The RunPod cost test is the first thing that turns architecture into evidence, and the first thing that turns the substrate into a product with a defensible margin.

---

## Coda for 4.1

Part 4 ended on "give Theseus a voice." 4.1 grounds that ambition in the one claim that pays for the exploration: the substrate is cheaper than the competitor model because it offloads symbolic computation from GPU inference to CPU substrate compute, and that cost difference is what makes a credit-per-month product viable.

The exploration is real frontier work and there is no guarantee the exotic pieces work. That is where the interesting things happen, and it is also why monetization is not optional — the exploration only continues if it pays for itself. The compute-offload hypothesis is the bridge between the two. It is testable now, on credits already in hand, using engines already built, without fine-tuning. Run the test. If it holds, the substrate has a margin no all-LLM competitor can match, and that margin funds the frontier work.

Ship the release. Fix fractal expansion. Wire Datalog and the probabilistic engine. Run the cost test. Let the evidence decide what is real. Then keep building toward the voice.
