# Ensemble (the harness capability layer, formerly Orchestrate) to Rust

Date opened: 2026-06-05
Status: PLAN. Names the rename, places Ensemble in the layering, and sequences the remaining Rust work against the two plans it sits between: `theorems-harness-plugin-switchover` (runtime + SDK, substantially landed) and `skill-encoder-theorem-port` (skill serving, the open core).
Coordination: room `repo:theorem:branch:main`, tenant `default`. This file is an API commit from claude.ai, so `git fetch origin` to see it. Path-scoped commits only; claim a file via `coordination_intent` before overlapping edits.

## The rename

Orchestrate becomes Ensemble; its Rust home is `ensemble.rs`. An orchestra is conductor-led and top-down; a jazz ensemble is peers improvising inside a shared structure, listening and responding, with no one conducting. That is the architecture: capability priors not fixed roles, mediate not separate, models coordinating as equals over a shared substrate. The name now matches the principle.

Ensemble is not "just coordination" and it does not dissolve into the harness. It is the capability layer: the pack registry, the routing and selection (the budgeted surface where Ensemble is the only plugin the agent sees and everything else is a hidden capability pack), the trust ladder, and the learning loop. It is the thing that makes Theorem function as a plugin.

## The layering (so we build the gap, not the whole)

Harness substrate: `theorem-harness-core`, `theorem-harness-runtime`, `rustyred-thg-core`. Memory, rooms and coordination, the graph, Prolly versioning, `event_log`, sessions-as-scopes, receipts. The durable mind.

SDK v2 / plugin-switchover: the `theorem-harness` crate plus `apps/theorem-harness-node` (NAPI-RS) and `apps/theorem-harness-swift` (UniFFI), plus the plugin `sdk/route-policy.mjs`. The runtime surface and generated bindings that let the plugin, Claude Code, and the app talk to the harness natively. Substantially landed: the Rust SDK surface (`RunHandle`, `RunStream`, `Session`, idempotency tokens, trace export), the Node and Swift bindings, idempotency persistence, and route-policy wiring for replay, remember, recall, and self_note. The substrate is around 70 percent of the SDK v2 destination.

Ensemble: the capability layer that plays capabilities through the harness. This is the remaining port.

## Already native (do not rebuild)

From the switchover: the SDK surface and the Node and Swift bindings, native memory recall, the run lifecycle with cancel, resume, and receipt-on-event, idempotency, and the route policy. From the skill plan: the full Rust corpus definition and the `code_repo` ingest worker on the Index-API side, and the native gRPC `CodeCrawlerService` in `apps/theorem-grpc` (Search, Recognize, Explore, Context, Explain, RecordUseReceipt over RedCore, with Rust AST call and dependency edges, trust tiers, community ids). The agent's code-query path is already largely native.

## Remaining work

### Ensemble core in `ensemble.rs`: registry, selection, trust

The pack registry stores each `CapabilityPackSpec` as a content-addressed node in the versioned graph (reuse `rustyred-thg-core/src/versioned_graph.rs`, the same store skill packs use), keyed by content hash, with edges to its artifacts. Pack kinds carry the existing taxonomy (skill, agent, tool, validator, renderer, compute, policy, domain, context), an exposure flag (`visibleToAgent: false`, `exposedThrough: ensemble`) so packs stay hidden behind the one surface, and a trust field on the `unverified -> first_party` ladder with a passport id.

The selector is the heart. Ensemble presents as the single tool the agent sees, and per task it picks the packs, agents, and tools to bring in under a budget. The selection is emitted as a replayable decision artifact (the former OrchestrateDecision, now the Ensemble decision): the selected packs, agents, and tools, the rejected candidates with reasons, the risk summary, and the priors it used. Replayability is both the audit and the training signal.

Ground this before building: confirm what the switchover's manifest and connector/skill lanes already make native, so `ensemble.rs` adds only the registry-and-selection gap rather than duplicating routing the runtime already does. The runtime and MCP seams overlap with Codex's active edits, so claim the file with `coordination_intent` first.

### Skill serving (the open core; skill plan Lane S, close to switchover connectors+skills)

This is the capability that proves the registry and selector end to end, and the piece flagged as the priority. It needs the `skill_pack` node contract in the versioned graph (store the `CapabilityPackSpec` by `pack_content_hash`, with edges to source and artifact hashes), the native MCP verbs `skill_list`, `skill_get`, `skill_apply`, and `skill_publish` (reads in read mode, apply and publish behind write mode), and running each pack's Rust validators in-process. Running them in-process is the payoff of a Rust corpus: the validators are Rust, so they execute natively rather than as advisory strings. Bring the gRPC code lane to full CodeCrawler parity by wiring the deployed MCP product backend to the live `theorem-grpc` app-affordance transport, and add parser-grade dependency and call-graph extraction for the non-Rust languages the contract needs. This is Codex's lane on the hot `rustyred-thg-mcp/src/lib.rs`; claim the file and a new skill module first.

The compile and encode pipeline does not move. It stays Python (corpus ingest, tree-sitter lower, the Pairformer GNN cluster, codegen, validation by running Claude Code), runs offline like training, and publishes finished packs to Ensemble by content hash, out of band. The only cross-boundary link is `skill_publish` accepting a finished pack by hash. Ingest is a separate process, not a harness runtime dependency.

### The three participants and patterns

Hermes joins the ensemble as a participant, not a fixed role: an open-weight, keyless-tier reasoning peer alongside DeepSeek V4, expressed as a capability prior in the selector so it is chosen when its strengths fit. The adapter is the native equivalent of the former `plugin_adapters/hermes.py`. The second half is resident-goes-live: drop a Hermes function-calling model into the model-free `ResidentModel` interface so the ambient perceive, propose, critique, render loop runs for real under the energy critic, rather than sitting as an empty interface.

OpenClaw is the ambient, always-on messaging-agent pattern: the resident as a persistent member that watches the room and acts, the `resident/` and Commonplace shape. This is the same primitive as the banked idea of seating a model as a live room member (see `docs/plans/composed-agent`), so build it once and let both the Theorem agent and the OpenClaw resident share it.

Perplexity is the rival, not a dependency. Ensemble answers the same class of open question a stateless answer engine does, but over a stateful, owned substrate, by exposing the compounding multi-source search Codex just shipped (the SearchProvider fan-out plus the crawl-feed loop) as a capability pack the selector can reach. The difference that matters: every answer feeds the graph, so the system that answers a question is better at the next one.

### Stays Python (named so no one rewrites it)

The skill compile pipeline above, and the offline evolution and learning workbench (the MAP-Elites, PBT, CMA-ES, bandit, and learned-scorer machinery). These are batch, like training. The runtime selection is native in `ensemble.rs`; the offline evolution stays Python and writes the priors the native selector reads. The seam is again a content-addressed publish, not a live call.

## Order

Stand up the Ensemble core registry and selector in `ensemble.rs` on the landed SDK and runtime. Land skill serving (Lane S) as the first real capability through it, which exercises the registry, the selector, in-process validators, and the use-receipt loop on one path. Wire Hermes as a participant and bring the resident loop live. Build the OpenClaw ambient resident on the shared room-member primitive. Expose the multi-source search as the Perplexity-class answering pack. Throughout, use receipts drive fitness and promotion so capabilities improve with use, the same loop the skill packs already define.

## Ownership

Claude Code: destination architecture and the `ensemble.rs` core. Codex: skill Lane S on the hot `rustyred-thg-mcp/src/lib.rs`, and the switchover migration mechanics. Coordinate in room `repo:theorem:branch:main`, tenant `default`, with path-scoped commits and a `coordination_intent` claim before overlapping edits.

## Confirm against code before building

What the switchover manifest and connector/skill lanes already make native, so the registry is not duplicated. Where the budgeted selector and the Ensemble decision artifact should live, in `ensemble.rs` or folded into the runtime. The current state of the `ResidentModel` interface and the energy critic, to know how much of resident-goes-live is wiring versus building.
