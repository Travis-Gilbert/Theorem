# SPEC-7: The Standing-Pass Organizer Engine and the Residual Gap Map

Register: mixed. Section B is an execution handoff. Sections A and C are a prioritized roadmap with grounded state, enough to spec each item next.

## Purpose

A code-grounded map of what is not yet built or specced across the ecosystem, compiled from a direct pass over the monorepo, organized around one architectural spine: the standing-pass organizer engine. The spine matters for two reasons. It resolves the latency problem that has blocked running the epistemic engines (the reason unified retrieval has never performed quickly is that it ran at query time; the fix is to run the same reasoning in the background and read the precomputed result on the hot path). And it is the slot the learned temporal GNN in SPEC-8 plugs into.

## Corrected baseline (findings from this pass)

These correct earlier assumptions, including the preamble of the OpenSandbox spec.

The real provider invoker is built. `rustyredcore_THG/crates/theorem-harness-runtime/src/head_invoker/` contains `RealHeadInvoker` (aliased `ProviderHeadInvoker`) making real `reqwest::blocking` calls and routing by transport: `Api` to `api::invoke_api_head`, `Mcp` to `mcp::invoke_mcp_head`, and `Local` plus `Hosted` to an OpenAI-compatible chat-completions path. It carries a `CredentialResolver` (env, secret, secret-store refs) and an env-driven `EndpointMap` covering Anthropic, DeepSeek, MiniMax, Gemma, a local server, and a hosted endpoint, including the LiteLLM seam via `THEOREM_LITELLM_BASE_URL`. FakeHeadInvoker is now the test double. The composed open-model agent therefore has its live machinery, and OpenSandbox deliverable 5 plus part of deliverable 4 are already done. One open item: whether the composed-agent run path constructs `RealHeadInvoker` by default and has been exercised end to end, which is one read of `composed_agent.rs`.

The reflexive layer already runs a bounded learned scorer. `rustyredcore_THG/crates/rustyred-thg-adapters/src/pairformer.rs` (plus `burn_pairformer.rs` and `pairformer_cubecl.rs`) implements an AlphaFold-style Pairformer that takes a bounded node and edge set and emits advisory link scores with two-hop support paths, keeping graph mutation outside the model. The organizer engine in Section B is a generalization of this existing pattern, not a greenfield build. The design context is `docs/plans/reflexive-rustyred/` and `apps/theorem-agentd/reflexive-rustyred-plan.md`.

The hooks seam is built. `rustyred-thg-core/src/hooks` is a complete post-commit, coalesced, fail-open, loop-guarded `MutationEvent` dispatcher (`HookHandler = Arc<dyn Fn + Send + Sync>`, installed via `attach_hook_emitter`). This is the trigger edge the standing pass runs off.

Still missing: the sandbox backend (`sandbox_exec.rs` is absent from `theorem-receiver/src`). EpistemicRAG and the shadow graph were not re-verified this pass; treat as partial.

## Section A: Specced but unbuilt (the build queue)

These have designs. They need building, not designing. Listed so the map is complete.

1. SPEC-1 reflexive geo and time property. Rule-based generators for the standing pass. Feeds Section B as the first generator class.
2. SPEC-2 Item projection and changefeed. The Item domain on the graph plus the live changefeed. Prerequisite for the UI transport and the consumer surface.
3. SPEC-3 space-type registry. The typed-object registry. Prerequisite for the UI plugin loader in Section C.
4. SPEC-4 scheduled agent tasks and harness hooks. Includes the SessionStart engage hook and the Stop write-back hook, the recurring Valkey enqueuer, and the receiver on the subscription token. The write-back hook is the half that surfaces a run in CommonPlace.
5. SPEC-5 WASM database-plugin host. Runtime-loadable DB extensions as ordinary `RustyRedPlugin` entries. An alternative home for learned organizers that should be hot-swappable rather than compiled in.
6. SPEC-6 UI GraphQL transport. The HTTP `/graphql` endpoint over the existing schema. Prerequisite for the consumer UI and the mobile app.
7. OpenSandbox substrate backend, on a branch and partial. Add the `Subscription` model-backend kind so coding CLIs default to the native-subscription path and the LiteLLM seam stays scoped to the composed agent and the deliberate alternate-model case. Build `sandbox_exec.rs`. Carry the SPEC-4 hooks into a sandboxed run. Add a retention policy to the persistent volume.

Open verification for the queue: whether the A6 cluster resolvers (`harness_run`, `skill`, `ensemble`, `job`) are merged into the GraphQL schema or only present as invoker methods. SPEC-6 depends on this being complete.

## Section B: The standing-pass organizer engine (GAP 1, the spine)

Generalize the reflexive Pairformer pattern into a multi-generator engine so that all background structure-making shares one contract, one admission path, and one trigger.

### The generator contract

Mirror the existing Pairformer input and output so generators are interchangeable to the engine. A generator receives a bounded node and edge set and returns advisory candidates with support, and never mutates the graph itself.

```rust
pub struct GeneratorInput {
    pub nodes: Vec<GraphNode>,          // bounded, like PairformerInput.nodes
    pub edges: Vec<GraphEdge>,          // bounded; carries optional timestamp for temporal generators
    pub query: GeneratorQuery,          // candidate pairs, a seed node, or a region
}

pub struct AdvisoryCandidate {
    pub kind: CandidateKind,            // ProposedEdge, EquivalenceMerge, DerivedFact, ReliabilityAnnotation, SalienceUpdate
    pub subject: CandidateRef,
    pub score: f32,
    pub support: Option<SupportPath>,   // the same shape as PairformerSupportPath
    pub generator_id: String,
}

pub trait StandingGenerator {
    fn id(&self) -> &str;
    fn generate(&self, input: &GeneratorInput) -> ThgResult<Vec<AdvisoryCandidate>>;
}
```

`SupportPath` reuses `PairformerSupportPath` (edge ids, node ids, relation hint, confidence) so every candidate carries its justification. The advisory-only rule is the existing Pairformer invariant: the model proposes, admission disposes.

### The generator set

- Rule based: the SPEC-1 geo and time generators.
- Structural learned: the Pairformer, already built, an atemporal triangle reasoner over a bounded snapshot.
- Temporal learned: HOT, SPEC-8, the higher-order temporal scorer over the history of updates. Complementary to the Pairformer because the Pairformer has no time encoding.
- Symbolic: datalog derivation, egglog equivalence and duplicate collapse, Beta-Bernoulli source reliability. These live in `rustyred-thg-core/src/symbolic` and the harness symbolic surface; this gap is wiring them as generators, not writing them.
- Structural retrieval: shadow-graph PPR over the epistemic overlay (folds in the EpistemicRAG completion from Section C).

### The standing pass

The trigger edge exists: `rustyred-thg-core/src/hooks` emits coalesced `MutationEvent`s post-commit. The pass subscribes there, and on a debounced batch:

1. Selects a bounded candidate region around the changed nodes (the same bounding discipline as the Pairformer `max_nodes`).
2. Runs the registered generators over that region.
3. Passes their `AdvisoryCandidate`s through the admission and confidence dial (the existing confidence-ceiling and admission-tier mechanism used by the plugin registry for `writes_graph`), which decides what is written back and at what confidence.
4. Writes admitted structure back through the normal mutation path, which re-triggers the hooks and lets the pass converge.

`commonplace/src/reflexive.rs` is the existing reflexive entry; `apps/theorem-agentd/reflexive-rustyred-plan.md` is the design. The North Star bounded candidate engine (`NORTH-STAR-organizing-engine.md`) is the design target for the bounding and admission discipline.

### Acceptance

A generator registered with the engine runs on the standing pass off the hot path, emits advisory candidates with support, has them admitted through the confidence dial, and never blocks a read. Two generators (Pairformer structural, HOT temporal) run over the same bounded region and the engine admits the union. The query path reads only precomputed structure. No query-time call into a reasoning engine.

## Section C: Residual unspecced gaps (prioritized)

Each item is a gap that needs its own spec. State is grounded; shape and dependencies are enough to start.

1. Local-to-hosted sync. State: RustyRed has CRDT merge, HLC, and yrs locally (`rustyred-thg-core/src/crdt`), but the local-engine to hosted sync protocol is unspecced. Shape: a sync protocol over the existing CRDT and HLC, per-item residency enforcement (Local, Synced, Hosted on the Item), an outbox for offline writes, and a mobile read cache. Why: it is the mobile backbone, the chargeable tier, and the no-lock-in moat at once. Dependencies: the Item domain (SPEC-2). This is the single highest-leverage unspecced item.

2. Performance-proof eval and capability demo. State: unbuilt and unspecced. Shape: an in-repo, reproducible number showing agent-plus-substrate beating agent-alone (the SWE-bench-style multi-session sequential-instance protocol with the persistent memory server, measuring resolve rate and tokens-to-resolution), paired with a capability demo of something flat memory cannot do (contradiction catching, belief revision that does not rot, graded answers under contradiction). Why: the entire epistemics-as-capability thesis and the launch positioning rest on this proof existing. Dependencies: the organizer engine (Section B) for the capability side.

3. One-command MCP install. State: unspecced. Shape: a single command that installs and authenticates the harness MCP and CommonPlace for a new user. Why: it is the acquisition wedge; exposure without an install-to-aha loop earns nothing. Dependencies: none hard.

4. CommonPlace consumer surface. State: the data layer is covered by SPEC-2, SPEC-3, and SPEC-6; the consumer UI itself is unspecced. Shape: the workspace view where everything a user and their agents do organizes itself, with the Auto-Organizer as the visible face of the engine. Component direction is the warm-paper, typed-object Capacities north star already established. Dependencies: SPEC-6 (transport), SPEC-2 (Items), Section B (the engine it visualizes).

5. Hosted CommonPlace. State: unspecced. Shape: the hosted instance that is the chargeable tier. Why: the revenue surface. Dependencies: sync (item 1).

6. Mobile app, React Native and Expo. State: unspecced. Shape: a thin GraphQL client over SPEC-6 plus the changefeed, sharing schema, types, client, and logic with web, living in Synced and Hosted residency with the offline cache from item 1. Why: the real mobile app, the priority over the interim PWA. Dependencies: SPEC-6, sync (item 1).

7. UI plugin loader and Tauri desktop packaging. State: SPEC-3 covers the space-type registry; the JS and TS plugin loader into the webview and the thin Tauri shell onto the local engine are unspecced. Shape: the engine serving the UI and the plugin registry over HTTP, the loader registering plugins into the space-type registry, a trust gate (unsigned allowed on desktop, curated on hosted), and the Tauri window onto the local engine process. Why: open user plugins are the desktop differentiator. Dependencies: SPEC-3, SPEC-6.

8. EpistemicRAG and shadow graph completion. State: partial. Shape: two halves. The write half folds into Section B as the shadow-PPR generator and the epistemic overlay it maintains. The read half is the dual-retrieval read path (structural PPR plus ANN over GNN embeddings, fused via RRF) that consumes the precomputed overlay rather than computing it at query time. Why: this is the concrete form of the speed resolution. Dependencies: Section B.

## Priority ordering

Section B first, because it resolves the latency problem, generalizes a pattern that already exists, and is the slot SPEC-8 needs. Then item 1 (sync) and item 2 (the proof), which unlock the tier and the positioning. Then item 3 (the wedge). Then the surfaces: items 4, 5, 6, 7. Item 8 rides along with Section B.

## Implementation Notes

- Implemented Section B in `rustyred-thg-adapters/src/standing_pass.rs`, not `rustyred-thg-core`, because the first concrete generator set depends on adapter-layer Pairformer and reflexive candidate functions while reusing core hooks and graph mutations.
- Added the shared `StandingGenerator` contract, bounded `GeneratorInput`, `AdvisoryCandidate` output, `StandingPassEngine`, and `standing_pass_hook`.
- Default registered generators are Pairformer structural, spatial proximity rule, and `hot-temporal/heuristic-v0`. The temporal slot is deliberately named heuristic-v0 because the trained SPEC-8 HOT scorer is not present on `main` yet.
- Admission reuses the existing quarantine candidate path for traceability, then auto-applies proposed edges through normal graph mutations only when the candidate reaches the confidence ceiling.
- Acceptance is covered by `standing_pass_test`: generator union, hook-triggered off-hot-path materialization, and read-time use of precomputed structure without re-entering a generator.
