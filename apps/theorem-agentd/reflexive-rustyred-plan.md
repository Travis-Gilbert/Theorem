# Reflexive RustyRed: three learned organs, grounded and reshaped

The three "reflexive database" ideas are each a real, established research area, not uncharted territory. This document names the reference points you were missing, then reshapes each concept so it is safe to build (bounded blast radius, modest data needs, a fallback floor) instead of free-running.

The single rule that ties all three together is at the bottom and it is the most important part: in every one of these, a learned model ranks or steers within a bounded space. It never authors free-form output. That is the same rule as the agent action catalog and the context scorer, and it is what keeps a "self-modifying database" from corrupting itself.

---

## 1. Reflexive composition / densification = link prediction, done on bounded subgraphs

What it actually is: knowledge-graph completion. The Pairformer inferring `i -> j` from `i -> k -> j` is a learned path-composition operator. This is a deep, well-studied task, so you have plenty to compare against:

- KGE scorers for missing edges: TransE, RotatE, ComplEx. Cheap, global, no message passing.
- GNN link prediction: SEAL (subgraph classification), GraIL (inductive, relation-aware).
- Path-based reasoning: NBFNet and A*Net, a learned generalised Bellman-Ford that literally scores `i -> j` by composing the representations along `i ... j` paths. This is the closest match to your triangle-update intuition: the AF triangle multiplicative update is the dense, all-pairs version of the same "compose two edges to infer the third" bias.

Reshape for RustyRed:

- The dense `TriangularBlock` runs only on a bounded extracted subgraph, never the whole graph. Sample a PPR or k-hop neighbourhood around the seed entities and cap N (tens to low hundreds). The cost note in the corrected code is the reason: the triangle multiply is O(N^3) per channel. You already expose PPR (`rustyred_thg_algorithm_ppr`), so the sampler is in hand.
- For global completion across the whole graph, use the sparse `EdgeMPNNLayer` in an NBFNet-style scorer, not the dense block.
- Do not auto-insert inferred edges into `instant_kg.rs`. Gate insertion behind a confidence threshold and mark every inferred edge as such, using the `admission_tier` / `confidence_ceiling` properties that already exist in the schema. This is the same quarantine discipline you apply to external crawled content: a densification pass that silently writes high-confidence-looking edges will degrade the graph it is supposed to enrich. Inferred edges are advisory until corroborated.

Net: keep the idea, run the dense reasoner on bounded neighbourhoods, score global completion with the sparse model, and quarantine what the model invents.

---

## 2. Graph adapters / in-database LoRA = GraphLoRA, with weights in a sidecar not in the node struct

What it actually is: parameter-efficient adaptation of a GNN. Your citation is right. The reference is GraphLoRA (Yang et al., KDD 2025, arXiv 2409.16670): inject a small trainable low-rank GNN alongside a frozen pre-trained one for cross-graph transfer, tuning roughly 7 to 20 percent of parameters, using PPR diffusion for structure awareness. There is also newer work (arXiv 2606.07526) that embeds a message-passing module inside the LoRA bottleneck as a "learnable reasoning pathway," which is almost exactly your in-database-inference framing, and a body of federated-graph-plus-LoRA work for sharing adaptation across nodes without moving data (which is your federation tier).

So the concept is sound and current. The problem is in the code design, not the idea:

- Do not make the core graph types generic over a Burn backend and store `Option<Tensor<B,1>>` inline on `Node` / `Edge`. That couples your storage layer to a tensor backend, forces the whole graph store to be backend-parametric, and creates a serialization and memory mess (you cannot persist or load the graph without instantiating a tensor backend, and every node now carries a live tensor).
- Instead, keep the graph as plain topology and put representations and adapters in a separate aligned representation store keyed by node and edge id (a typed sidecar, a parallel column). The executor joins topology with the sidecar at query time. This is how production systems separate structure from learned features, and it lets the embedding store re-embed on a Modal background pass without touching the graph store.
- Keep the genuinely novel part, which is the in-database inference: when the executor walks a `MATCH (a)-[*2]->(c)`, it fetches the local neighbourhood's pair representations from the sidecar and runs the Pairformer rather than a pure pointer chase. That is the interesting capability. Just feed it from the sidecar.

Net: keep in-DB inference, adopt GraphLoRA's low-rank adapter pattern, and move the weights out of the node struct into a representation sidecar.

---

## 3. Learned databases / query optimization = steer the planner (Bao), do not generate plans (Neo/Balsa)

What it actually is: learned query optimization, a mature field with two distinct families, and the distinction is the whole fix:

- De novo: Neo and Balsa generate plans from scratch with reinforcement learning and a tree-convolution value network, searching the plan space by best-first or beam search. Powerful, but data-hungry, slow to train, and capable of catastrophic tail plans.
- Steered: Bao (the bandit optimizer) leaves the native rule-based optimizer in place, has it emit a small set of candidate plans via hints, and uses a learned value function plus Thompson sampling to pick among them. An order of magnitude faster to train, adapts online, and the worst case is bounded by the native plan.

Your draft (`build_plan_from_attention`) is de novo: the Pairformer authors the plan. That is the risky, data-hungry path, and the query planner is not your bottleneck yet.

Reshape: go Bao-style. The rule-based planner in `planner.rs` proposes a small candidate set; the Pairformer (or any value model) ranks or steers among them using the historical execution metrics in `metrics.rs`. You learn a ranker over a bounded action set, not a generator over the whole plan space. This needs far less data, has a safety floor (fall back to the native plan), and adapts with Thompson sampling. Tree-convolution is the standard encoder here; your triangle-attention over the query grid is a fine alternative encoder, but the learning formulation must be "rank the candidates," not "emit a plan."

Cold-start caveat (shared with the memory scorer): until `metrics.rs` has enough execution traces, the value model abstains and the native planner runs. Same pattern as the memory GNN waiting on use-receipts.

Net: keep the Pairformer-as-encoder idea, switch the formulation from generating plans to steering among the native planner's candidates.

---

## The rule that unifies all three (and the rest of the system)

Densification gates and quarantines inferred edges. The optimizer steers among candidate plans. And from the prior sessions: the browser-use agent picks from a fixed action catalog, the context scorer ranks atoms under a budget, and the agent surface is a compiled toolkit not a free-form GraphQL endpoint.

These are one principle wearing five costumes:

> A learned model ranks or steers within a bounded, enumerated space. It does not author free-form output. The bounded space gives you a safety floor, far smaller data needs, and a blast radius you can reason about.

This is the invariant that makes a self-modifying, learned, reflexive database safe rather than a system that slowly poisons itself. Hold it everywhere.

---

## Theorem is ~10 percent of Theseus, and that is the right 10 percent

Theorem does not need Theseus's full epistemic apparatus. It needs the substrate plus the few learned organs that make the database itself smarter, which is exactly what these three are: infrastructure intelligence, not domain epistemics. The right next increment, in order of leverage and readiness:

1. The context scorer / memory GNN (prior session): closes the hard part the context-management spec hand-waves, reuses the sparse `EdgeMPNNLayer`, and its label source (use-receipts) is already being recorded. Highest leverage, soonest ready.
2. Densification / link prediction: reuses the Pairformer you already wrote, and the prolly-tree diff engine feeds it cleanly (densify only the changed subgraph on each commit, not the whole graph). The dense block runs on bounded neighbourhoods; the sparse model scores global completion.
3. In-DB LoRA via a representation sidecar: the storage refactor (weights out of the node struct) is the prerequisite; do that first, then the adapters.
4. The steered query optimizer: last, because it is the most data-hungry and the query planner is not yet the bottleneck. Build it when `metrics.rs` has accumulated real workload.

---

## Reference material to compare against

- Message passing in Rust: SAGA paradigm (Scatter-ApplyEdge-Gather-ApplyVertex) over sparse COO; Burn `scatter` with `IndexingUpdateOp::Add` and `select`. No PyG/DGL-equivalent library exists in Rust, so layers are hand-rolled.
- AlphaFold3 Pairformer block: triangle multiplicative update (outgoing, incoming), triangle attention (starting node row, ending node column) with logits biased by the third edge, SwiGLU transition; single representation updated separately by attention-with-pair-bias; pair does not flow back from single; 48 blocks, no weight sharing.
- Link prediction / KG completion: TransE, RotatE, ComplEx (KGE); SEAL, GraIL (GNN); NBFNet, A*Net (path-based, the closest to the triangle-composition bias).
- Graph adapters: GraphLoRA (Yang et al., KDD 2025, arXiv 2409.16670); structure-aware low-rank propagation (arXiv 2606.07526); federated-graph-plus-LoRA literature.
- Learned query optimization: Neo (Marcus et al.), Balsa (Yang et al.) for de novo; Bao (Marcus et al., arXiv 2004.03814) for steered with a bandit; Lero for the pairwise-ranking variant.
