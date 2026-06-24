# SPEC: CommonPlace Harness — Proactive Tool Governance, Hyperlink Memory, Reasoning Bank, Modular Substrate

Execution handoff. Four components that share one premise: the harness should make an
API agent smarter by doing work the agent does not know to ask for, and by letting that
work compound. Component 1 is the dispatch layer that pushes substrate tools at the right
moment. Components 2 and 3 are the memory upgrades that the dispatch layer pulls from and
writes to. Component 4 is the substrate all of it rides on, written out in the most detail
because it is the least settled.

Grounded in the current backend (Rust Axum, RustyRed / `rustyredcore_THG`, Valkey, an
idle Postgres, a reranker), the Theorems-Harness V2 tool surface, and the committed
CommonPlace frontend trees. No Django, no Redis brand, no Memgraph in this path.

---

## The control fact that shapes everything

There are two agent surfaces, and they have different control over the loop.

The harness UI runtime, where API agents work inside CommonPlace, is a loop you own. You
sit between the user turn and the provider API call. You can run substrate tools yourself
and inject their results into the model context before it answers. This is real proactive
dispatch because nothing in the protocol is in your way.

External MCP clients (Claude Code, Codex, Cursor) are loops you do not own. The only MCP
primitive for server-initiated model calls is sampling (`sampling/createMessage`), and it
is now a dead end for this purpose: deprecated in the 2026-07-28 spec under SEP-2577 with
the guidance to integrate directly with provider APIs instead, human-in-the-loop by design
(every request user-approved, no silent completions), and unevenly implemented across hosts
(absent in OpenAI Apps, error-prone on several gateways). Its stateless replacement (Multi
Round-Trip Requests, SEP-2322) returns an `InputRequiredResult` from a `tools/call` and
makes the client re-issue, which is heavier than it is worth for automatic governance.

Conclusion, and it is the spec's spine: the governor lives in the harness UI runtime as
control-plane middleware, calling the provider API directly. It does not live in MCP, and
it does not live inside RustyRed. For external clients you do not own, the governor's output
rides back as enrichment appended to whatever tool the agent already pulled.

---

## Component 1: The Governor (proactive tool dispatch)

### Placement

A middleware stage in the harness UI agent runtime, between the inbound user turn and the
outbound provider API call. It reads RustyRed graph state and the current working set. It is
control plane, kept separate from the storage engine. RustyRed stays a fast RAM-resident
store; embedding a model inside it couples two things that need to scale independently and
makes the hot path heavier. The "small model in the database" instinct is right about
co-location of cheap judgment with graph state and wrong about where it sits. It sits in the
runtime, reads from the store.

### Three tiers, escalating cost

The reason a hook cannot do neuro-symbolic offload is that a hook decides on a fixed event,
not on whether firing is worth it now. The fix is not to replace the hook but to stage it.

Tier 1, the eligibility detector (hook-cheap, deterministic). A structural check over
recent graph events and working-set node types that marks which substrate tools are even
eligible. A hook is excellent at this; it shrinks the candidate set before anything
expensive runs. Concrete eligibility rules:
- `source_reliability` is eligible when a node of type `source` or `claim` enters the
  working set, or when the agent is assembling cited evidence for an assertion.
- `datalog_derive` is eligible when new typed facts enter the graph that match a rule body
  (a rule antecedent references the new fact).
- `expected_value` (EVOI) is eligible immediately before any costly check (adversarial
  verify, deep retrieval, large recall). It is the meta-gate that decides whether that
  check runs at all.

Tier 2, the reranker as relevance scorer (cheap, fixed latency, no generation). The reranker
you already run scores each eligible tool's description against the current state vector
(working-set types, the agent's pending action, the recent event window). Tools above a
threshold become live candidates. This answers "is any substrate tool worth running right
now," using a model you already operate, with bounded per-turn latency and zero generation
cost.

Tier 3, the small generative governor (capable, fired rarely). Only when Tier 2 produces
live candidates does a small general model (a 1 to 4B class instruct model running on the
existing GPU burst path) read the candidates plus state and emit a structured dispatch
decision: tool name, arguments, and a one-line rationale. It constructs the actual call and
orders multiple calls when needed. Because Tiers 1 and 2 gate it, it runs on a small
fraction of turns, so its cost stays low. This is the EVOI principle applied to the
orchestration layer itself: cheap gate first, expensive reasoning only when it pays.

### Result injection

In the harness UI loop, the governor runs the chosen tool server-side and injects the result
into the model context as a synthetic tool-result message before the model generates. From
the agent's view the knowledge was simply present. For an external MCP client, the same
governor output is appended to the return value of the next tool the agent pulls, so the
signal still arrives without a server-initiated call.

### Self-improvement

Every governor decision and its downstream outcome is encoded to the harness (`encode`,
`kind` of `feedback` for routine signal or `postmortem` for a decision that hurt, with
`outcome` set, `tenant_slug` `Travis-Gilbert`, tagged `governor-decision`). Before deciding,
the governor recalls against those records (this is the reasoning bank, Component 3), so the
dispatch policy learns which tools in which graph states paid off. The orchestration layer
self-evolves on the same memory machinery the agents use.

### Tool surface referenced

`rustyred_thg_symbolic_datalog_derive`, `rustyred_thg_symbolic_probabilistic_source_reliability`,
`rustyred_thg_symbolic_probabilistic_expected_value`, `compute_code`, `harness_kg_impact`,
`encode`, `recall`.

### Acceptance

An agent assembling cited evidence in the harness UI receives a source-reliability score it
never requested, injected before its answer. An agent about to run an expensive verify has
EVOI gate that verify and sometimes cancel it. The Tier 2 reranker pass adds a bounded,
measurable latency per turn; the Tier 3 model fires only on turns where Tier 2 flagged a
candidate, and that fraction is observable in logs. Turning the governor off returns the
agent to plain pull-with-enrichment with no error. Governor decisions appear as encoded
records that a later recall retrieves.

---

## Component 2: Hyperlink Memory (slim-first recall)

The primitive already exists on `recall` (`hydrate`, `hydrate_top_k`, `include_content`).
This makes slim-first the default and wires both consumption surfaces.

### Behavior

A default `recall` returns references only: `title`, `id`, `score`, `tags`, and a short
snippet. No full content, and no duplicate content field. Full content arrives on a second
call that hydrates chosen ids (`hydrate` for all returned, `hydrate_top_k` for the top N,
`tool_result_fetch` for byte-slicing an oversized hydrate).

Two surfaces consume the reference list:
- Chat UI. The references render as clickable titles. A click hydrates that id and expands
  it in place. This is the literal hyperlink.
- Agent loop. The model receives the slim reference list, reasons over the titles, and calls
  hydrate only on the ids it judges relevant. Triage then read, rather than drowning in full
  documents. In the agent case this is a reasoning-quality gain, not only a token saving.

This subsumes the in-progress recall payload fix (drop the duplicate content field, default
to summary plus id plus score plus tags, hydrate on demand). The reframe is that this is a
cognitive model, not a compression trick.

### Tool surface referenced

`recall`, `tool_result_fetch`.

### Acceptance

A default recall returns references and a small payload with no full bodies. Hydrating one id
returns that record's full content. The chat surface renders titles that expand on click. An
agent run shows the model returning a slim list and then hydrating a strict subset of the ids
it was given.

---

## Component 3: Reasoning Bank

Direct references, as requested.
- Repository: `github.com/google-research/reasoning-bank` (Apache 2.0, Python). Its own
  disclaimer marks it demonstration-only and not for production, so the code is a reference,
  not a dependency.
- Paper: Ouyang et al., "ReasoningBank: Scaling Agent Self-Evolving with Reasoning Memory,"
  ICLR 2026 (The Fourteenth ICLR). OpenReview forum id `jL7fwchScm`.
- Evaluated on SWE-Bench (software engineering) and WebArena (web browsing).
- Builds on Agent-Workflow-Memory, `github.com/zorazrw/agent-workflow-memory`.
- The mechanism to borrow: memory items are distilled reasoning strategies extracted from
  both successful and failed trajectories, retrieved at test time, with "memory-aware
  test-time scaling" treating accumulated experience as a scaling dimension alongside model
  size and sampling.

### What already exists here

`encode` with `kind` of `postmortem` and `outcome` of `negative` is the failure-as-signal
primitive. `recall` is the retrieval primitive. Two halves are missing around them.

### Distillation (the write path)

After a completed agent trajectory, success or failure, a distillation pass turns the
trajectory into a reusable strategy memory rather than a raw transcript. It records the
situation, the approach taken, what worked or failed, and the generalizable lesson. The
distillation pass is the small generative governor model from Component 1 reused, run once
at trajectory end. The result is encoded via `encode`: `kind` of `solution` for a win,
`kind` of `postmortem` for a failure, tagged `reasoning-strategy`, with `outcome` set. Raw
traces are not stored as strategy memory; only the distilled lesson is.

### Retrieval at test time (the read path)

Before an agent acts on a task, a slim-first `recall` (Component 2) against the
`reasoning-strategy` memories pulls the top relevant strategies and injects them into context.
In the harness UI this is part of the governor's pre-turn dispatch, so the agent starts a task
already holding the lessons of prior similar tasks, including the failures.

### Acceptance

Completing a coding task writes one distilled strategy memory, not a transcript, retrievable
by `recall` with the `reasoning-strategy` tag. Starting a similar task injects that strategy
before the agent acts. A failed trajectory produces a postmortem strategy that is retrieved on
a later similar setup. The benchmark hook is a measured SWE-bench pass-rate delta with the bank
populated versus empty, which is the quotable number behind the "agents that get smarter" claim.

---

## Component 4: The Modular Substrate

This is the part understood least, so it is written out fully.

### What "modular" means here, precisely

Not a plugin framework. Not micro-frontends. It means every surface (Index, coding, project
management, compose, and surfaces not yet built) is a view over one typed-object graph in
RustyRed, and every agent action is a transform on that same graph. The surfaces are
interchangeable lenses. The substrate is the single source of truth. Modularity is a property
of having one object model that many lenses read, not a property of having many independent
apps bolted together.

The core primitive already exists: `objectRenderables.ts` funnels every data shape into a
`RenderableObject` keyed by `object_type_slug`, and `ObjectRenderer.tsx` dispatches to the
per-type renderers. That dispatch is the modularity engine. This spec makes it the organizing
principle of the product rather than an implementation detail of one view.

### The three layers, named

Layer 1, the object substrate (RustyRed). One graph. Everything is a typed node: `source`,
`claim`, `quote`, `person`, `note`, `task`, `issue`, `cycle`, `code-symbol`, `draft`,
`artifact`, and whatever a new surface introduces. Edges carry provenance and a relation type.
This is the reservoir. Every surface reads and writes here. Nothing owns a private store. A
task is not a project-management row; it is a node that the coding surface and the graph view
also see, linked to the claims and sources that motivated it and to the code that implements it.

Layer 2, the object contract (`RenderableObject`). The single shape every surface speaks.
`object_type_slug` selects the renderer, so the same underlying object renders as a card in the
Index, a row in project management, a node on the React Flow board, and a citation in compose,
with no per-surface copy of the data. Adding a new object type is adding a slug and a renderer.
Adding a new surface is adding a query and a layout over objects that already exist. Neither is
a new backend.

Layer 3, the surfaces (lenses). The Index is the triage lens. Project management is the work
lens. The React Flow board is the spatial lens (the editable investigation-board view from the
flowsint borrow, distinct from the cosmos.gl whole-graph view). Compose is the authoring lens.
Each lens is a query over the substrate, a layout, and the set of actions it exposes. The
actions are the `organizeAction` verbs (file, delegate, draft, develop). They are graph
transforms, so they are available from any surface, because they operate on objects rather than
on a surface.

### Why this is the moat and not just tidy architecture

A competitor who rebuilds any single surface rebuilds a lens over an empty graph. The value is
the cross-surface compounding: a task linked to its sources, linked to the code that implements
it, linked to the draft that documents it, which exists only because all of it is one graph. An
agent operating on any one surface has the whole graph as context, which a single-purpose tool
structurally cannot match. This is the standing defensibility frame made concrete: the surfaces
are faucets, the object graph is the reservoir, and the reservoir is what does not get rebuilt.

### The go-to-market discipline

Each surface stands alone as a product. The Index is useful with no other surface. Project
management is useful alone. Coding is useful alone. They compound when combined. Ship products
that share a substrate, not a framework, because a framework asks the user to adopt a worldview
before getting value and a product does not. Coding is the wedge surface, because that is where
the benchmark makes the substrate advantage quotable; the other surfaces arrive as "and it is
all connected." Standalone usefulness is the adoption path. The integration is the moat.

### Structure referenced

`src/lib` (`objectRenderables.ts`: `RenderableObject`, `object_type_slug`),
`src/components/commonplace/objects/ObjectRenderer.tsx` (dispatch),
`src/lib/commonplace` (`OBJECT_TYPES`, `getObjectTypeIdentity`, `CapturedObject`,
`CaptureMethod`), the `organizeAction` verbs, `LibraryView.tsx` as the Index lens, the React
Flow board as the spatial lens, and the capture contract where `CapturedObject` maps onto the
substrate `IngestInput`.

### Acceptance

An object created in one surface appears, correctly rendered, in every other surface that
queries its type, with no per-surface storage. A new object type is added by registering a slug
and a renderer with no backend change. An `organizeAction` verb invoked from one surface
performs the same graph transform as from any other. A single agent query returns objects
spanning multiple surfaces in one result.

---

## Open forks

1. Tier 3 governor model: a hosted small instruct model on the GPU burst path, versus a local
   quantized model co-resident with the runtime. The reranker (Tier 2) is fixed; this fork is
   only about the generative tier.
2. Reranker scoring input: tool descriptions scored against a state vector built from
   working-set node types only, versus a richer state vector that also encodes the recent event
   window and the agent's stated intent. The richer vector costs more to assemble per turn.
3. Distillation timing: distill at trajectory end only, versus also distilling at long
   intermediate checkpoints for very long-horizon tasks.
4. External-client enrichment: append governor output to every pulled tool result, versus only
   to results of tools that are plausibly related to the governor's finding.
