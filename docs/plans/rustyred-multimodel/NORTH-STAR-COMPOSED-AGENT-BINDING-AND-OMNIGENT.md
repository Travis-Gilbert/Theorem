# Composed Agent North Star: A Better Binding, and the Omnigent Posture

High-level direction, not execution handoffs. Two work items in one frame: deepen the binding (the moat, below the application), and set the Omnigent posture (the operator layer, above the application). The composed agent is the product wedge; a credible SWE-bench number sells it; the binding is what competitors cannot copy because it rides the substrate.

Grounded in the current binding contract (`theorem-harness-core/src/agent_binding.rs`, `binding_store.rs`) and Omnigent's current code (`omnigent-ai/omnigent`).

## The frame

Two layers, and they are not the same product.

- Below the application: the binding. One agent, many heads, sharing uncommitted working memory over the substrate. This is the moat and the SWE-bench lift mechanism.
- Above the application: orchestration. Spawning and governing whole agent processes, routing tasks between them, the device and collaboration surface. Omnigent owns a good version of this. It is not the binding.

The binding deepens. The orchestration layer gets borrowed from and distributed onto, not cloned.

## Part 1: A better binding

### What exists, and stays

The current binding is a strong structural, lifecycle, and governance object. It is not scaffolding:

- Identity with a `composition_hash` over roster plus active head set (swap heads, new hash).
- Heads carrying `provider`/`model`/`transport`, a `cost_profile`, a `reliability_profile` (success rate, latency), `capabilities`, and `allowed_tools`.
- A four-zone memory hierarchy: `head_local` to `binding_private` to `agent_published` to `commons`.
- A parent-chained scratchpad (`ScratchpadDocument`/`ScratchpadRevision`), each revision attributed to an `actor_head_id`.
- A hard budget governor with `max_parallel_heads`.
- A compiled charter (stance plus capability enumeration), a capability scope (visible/callable tools).
- Real alignment guards already proven in tests: `binding_budget_overspent`, `binding_policy_denied`, `consensus_below_threshold`, `grounding_missing`, `tier_requires_human_authorization`.
- The lifecycle: resolve, probe heads, mount memory, compile charter, select capabilities, allocate budget, run, open private work, heads contribute, synthesize drafts, propose publication, policy check, publish.

Keep all of it. The upgrades sit on top.

### What is open, and where lift lives

The README names the intra-agent loop (primary proposal, critic contributions, synthesis, policy check, publication receipt) and routing as not built. That open half is exactly where a composition beats the best single head. The six upgrades below are that half, and each is something only the substrate can do.

### Upgrade 1: scratchpad from linear log to collaborative epistemic DAG

Today revisions are a single parent-chained sequence (`seq` 1, 2, 3), each by one head. That is a log, not a shared workspace, so heads append in turn rather than build on each other.

Direction: revisions form a DAG. A head can fork, annotate, and supersede another head's revision, and disagreements between heads are first-class through the substrate's epistemic edges (`CONTRADICTS`/`SUPPORTS`/`UNDERCUTS`) applied to scratchpad revisions. The contradiction detection that already exists (the TRR-007 same-subject-different-object pass, the shadow epistemic graph) runs on the scratchpad, so when two heads disagree it is a detected contradiction synthesis must resolve, not two lines in a log. The four memory zones already exist; this changes the revision structure inside `binding_private`.

This is the difference between deep composition (heads improving each other's partial reasoning) and juxtaposition (two finished answers placed side by side). Juxtaposition is what the orchestration layer does. The DAG is what the substrate makes possible.

### Upgrade 2: routing from static active set to adaptive per-subtask by learned reliability

Today `active_head_set` is fixed, all active heads contribute, and `reliability_profile.success_rate` is one scalar.

Direction: decompose the task and route each subtask to the head with the best learned reliability for that capability and domain. Per-head, per-capability reliability is the substrate's probabilistic source reliability (Beta-Bernoulli) and the affordances PPR-over-outcome-edges fitness, pointed at heads instead of external sources. The Rust subtask goes to the head that has been right on Rust; the planning subtask to the head best at planning. This is the router the README leaves open, made learned and substrate-grounded rather than a fixed committee.

### Upgrade 3: synthesis from consensus event to substrate-grounded verification

Today synthesis is a `DRAFTS.SYNTHESIZED` event gated by a consensus threshold. Consensus is agreement, which is weaker than correctness.

Direction: ground synthesis in verification. Best-of-N with a real verifier beats voting, and you can run a real verifier where the orchestration layer cannot. For coding, candidate patches are executed against tests and selected by outcome. For claims, the epistemic engine checks grounding. Make a verifier a first-class role (a `HeadKind` or a verification plane), and wire in the adversarial verify-nodes and falsification receipts the multihead work graph already has (`multihead_spawn_verify`, `multihead_submit_verify`, defect/falsification receipts) so verification is part of the binding loop, not a separate tool.

This is the single biggest lever for the SWE-bench number, and the one thing Omnigent's "another model reads the diff" cannot match, because it has no substrate to execute or check against.

### Upgrade 4: budget from hard cap to value-aware governor

Today budget is a hard ceiling plus `max_parallel_heads`. It does not know where spending helps.

Direction: allocate budget across heads and rounds by marginal expected value. Spend more where it lifts, abstain where one head already nailed it. The substrate's expected-value machinery (the "is it worth validating" expected-value-of-checking) applies to "is it worth another head's contribution." A cheap task short-circuits to one head; a hard, contested one escalates. This directly answers the cost and latency objection to composing five frontier models: you pay for five only when the expected value justifies it.

### Upgrade 5: loop from single pass to iterative rounds with convergence stopping

Today the path is contribute, synthesize, publish, once.

Direction: rounds of propose, cross-annotate and critique in the collaborative scratchpad, verify, re-synthesize, with the value-aware governor deciding when to stop (converged, verified, or budget exhausted). This is the intra-agent loop the README marks open, designed as the lift engine rather than a single fan-in.

### Upgrade 6: the binding compounds

Today `reliability_profile.last_outcome_hash` holds one last outcome.

Direction: every run's outcomes feed per-head, per-domain reliability and the routing policy, so the binding gets better at composing as it does more work. This is the data-appreciates thesis applied to the binding itself, grounded in the substrate's fitness and training machinery (invocation-outcome fitness, pairformer writeback). A competitor reading the binding code gets a cold binding. Yours has accrued per-head reliability across thousands of tasks. That is the moat that cannot be copied from source.

### The through-line

Omnigent composes black-box harnesses by exchanging artifacts above the application. A better binding composes models by shared epistemic working memory, learned routing, substrate-grounded verification, value-aware budget, and outcome-driven compounding, below the application, all requiring the substrate. That is simultaneously the SWE-bench lift and the moat.

## Part 2: the Omnigent posture: borrow and distribute, not parity

### What Omnigent actually is (grounded in their code)

A meta-harness and agent framework that gives one orchestration layer over Claude Code, Codex, Cursor, Kimi, Pi, and custom YAML agents. It spawns and supervises whole agent processes in sandboxes (Modal, Daytona, E2B, Kubernetes, and others) or local tmux/PTY wrappers, routes tasks between them, governs them with stacked policies, and provides a device and collaboration surface (session follows you across devices, share, co-drive, fork, multi-user, invites, OIDC).

Its composition is shallow and external. Polly runs separate coding sub-agents in separate git worktrees and routes each diff to a reviewer from a different vendor. Debby asks two independent heads and lays the answers side by side, with a few rounds of mutual critique on `/debate`. There is no shared sub-agent working memory, no substrate, no epistemic structure below the orchestrator. Coordination happens above the application by passing diffs and messages.

### Why parity is the wrong move

A feature-parity doc clones the operator and orchestration layer, which is the crowded, giant-adjacent layer we already agreed to stay out of. This is not the Plane situation: Plane is a product in CommonPlace's category, so matching its surface is the game. Omnigent is a different layer. Cloning it builds a competing meta-harness and abandons the moat.

### What to borrow (the operator surface; your weak spot, their strength)

Study Omnigent's operator surface the way the parity map studies Plane's PM surface, and build your own:

- The session model that follows the user across terminal, browser, and phone, with sub-agents, terminals, and files in sync.
- Share, co-drive (a teammate's messages execute on your machine), and fork from a point.
- Multi-user accounts, single-use invite links, OIDC.
- Stacked policies across server, agent, and session, with spend caps, tool limits, and pause-for-approval as builtins.
- The per-session disposable cloud sandbox abstraction.
- The YAML agent definition where agents build agents.

This is the harness-as-service surface. They have a clean reference; take the patterns, not a dependency.

### What to do with Omnigent directly (the one narrow beneficial use)

Make the composed agent runnable as an Omnigent agent. Omnigent supports custom YAML agents and any compatible gateway, so the composed SWE-bench agent can run inside their framework and land in front of their audience and momentum. That is a distribution channel, not a foundation. Be compatible, do not build on it, do not let it own the surface or the story.

### What not to outsource (the moat)

The binding (deep composition over the substrate), RustyRed (the any-data-type knowledge substrate and the write-back loop), and substrate-grounded verification. Omnigent has none of these, because they live below its layer.

### The posture in one line

Omnigent is the operator shell above the application and a distribution surface. The binding is the moat below it. Borrow its surface, ship onto its audience, keep the substrate.

## Grounding sources

- Current binding contract: `rustyredcore_THG/crates/theorem-harness-core/src/agent_binding.rs` (BindingIdentity, BindingComposition, AgentHead, HeadKind, BindingMemoryScope, four MemoryZone kinds, ScratchpadDocument/Revision, PublishedScope, BindingCapabilityScope, budget and alignment guards, the lifecycle proven in tests) and `binding_store.rs` (persistence through `theorem-harness-runtime`).
- Open work: `docs/plans/composed-agent/README.md` (Lane A complete; the intra-agent loop and routing open).
- Substrate primitives reused by the upgrades: epistemic edges and contradiction detection (the shadow epistemic graph, TRR-007), probabilistic source reliability (Beta-Bernoulli) and expected-value-of-checking (the symbolic engine), affordances PPR-over-outcome fitness, the multihead adversarial verify-nodes and falsification receipts, invocation-outcome fitness and pairformer writeback.
- Omnigent: `omnigent-ai/omnigent` README and examples (Polly, Debby, Scribe), the policy and sandbox model, the YAML agent spec.
