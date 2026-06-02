# The Connector Layer as a Substrate Learning Registry

Status: design capture, 2026-06-02. Grounded in RustyRed 0.6.0 (the MCP adapter, the
graph store, the affordance/receipt pattern) and the agent-as-composition / AgentBinding
decisions from late May 2026. This is the spec for an idea that surfaced in conversation
and should not be lost: do not build a dumb MCP-of-MCPs passthrough. Make the substrate
the connector layer, and make the connector layer something the system learns to use.

## The idea in one paragraph

Most "connect to everything" integrations are passthrough aggregators: a router that
forwards a tool call to whichever downstream MCP server owns the tool, and forwards the
result back. That design adds reach but no intelligence, and it does not compound. The
move here is different. Every connector (every MCP server, every tool) is ingested into
the substrate as a first-class affordance node in the graph. The Pairformer learns, from
accumulated outcomes, which affordances to reach for in which situations. The charter
enumerates the relevant subset for a given agent so the model is primed with the tools
that matter rather than drowning in a thousand. This turns "connect to everything" from
an access problem (can I reach this tool) into a proactive-selection problem (which tool
should I reach for, and when), and proactive selection over accumulated experience is a
moat a passthrough aggregator cannot copy. It is the capability-scope plane of the
AgentBinding.

## Why a passthrough aggregator is the wrong default

A passthrough MCP-of-MCPs has three problems that get worse with scale, not better.

The first is the tool-overload problem. When an agent is handed hundreds or thousands of
tools across dozens of connected servers, the model's ability to pick the right one
degrades. Tool selection becomes the bottleneck. A passthrough layer makes this worse by
design: it connects more, it surfaces more, and it leaves selection entirely to the model
at call time with no learned prior. The system gets more capable on paper and less usable
in practice.

The second is that it does not compound. A passthrough call to a downstream tool produces
a result and forgets it. The hundredth time the agent faces a situation where a particular
tool is the right move, it is no better at recognizing that than the first time, because
nothing recorded that the tool worked there. The reach is static. The whole thesis of this
system is that intelligence accumulates in the substrate; a passthrough connector layer is
the one place that accumulation does not happen, which makes it the weak seam.

The third is that it is trivially copyable. Anyone can build a router that forwards MCP
calls. If the connector layer is just reach, it is not a differentiator, because reach is
a commodity (the protocol is open, the servers are public). The differentiated thing is
not the reach; it is knowing what to do with it.

## What the substrate-as-connector-layer does instead

The connector layer becomes part of the graph, and the graph's existing machinery
(structural learning, the Pairformer, the charter, the receipt pattern) operates on it.

### Affordances are nodes

When a connector is registered, each of its tools becomes an Affordance node in RustyRed,
with the tool's schema, its owning server, its permission requirements, its cost shape, and
a semantic description embedded as a vector (so affordances are retrievable by similarity,
matching the deliberate-vector-class decision: an affordance is a class of information that
needs semantic retrieval). Edges connect affordances to the task types they have served, to
the outcomes they produced, and to the other affordances they are commonly sequenced with.
The connector layer is now a subgraph, not a routing table.

This reuses what RustyRed already is. The graph store holds nodes with labels, properties,
vector designations, and HNSW search. An affordance is just a node with a label, a schema
property, and an embedded description. No new storage primitive is required; the affordance
registry is a labeled region of the existing tenant graph.

### The Pairformer learns to select affordances

This is the core. The Pairformer (the learned routing/orchestration layer) already learns
to route work across the composition. Extend its responsibility: it also learns, from
accumulated run outcomes, which affordances to propose for a given task. Every time an
affordance is used and the run succeeds or fails, that outcome is an edge in the graph and
a training signal for the Pairformer. Over time the Pairformer develops a learned prior:
"for this shape of task, in this context, these affordances have worked." Selection stops
being a cold guess by the model at call time and becomes a warm, learned recommendation
grounded in what has actually worked before.

This is the same compounding loop as everywhere else in the system, applied to tools: act,
record the outcome, retrain on the accumulated record, select better next time. The
connector layer compounds because it is inside the substrate that compounds. It is also
the natural consumer of the lab-graph / reasoning-trace corpus: the traces record which
tools were chosen and whether the run succeeded, which is exactly the affordance-selection
training signal.

### The charter enumerates the relevant subset

The tool-overload problem is solved at the charter layer. An agent is not handed every
registered affordance. The charter (the agent's binding-level configuration) enumerates the
subset of affordances relevant to that agent's purpose, and the Pairformer's learned prior
ranks within that subset. The model sees a curated, ranked set of tools it is primed to use
well, not a thousand it has to triage. This is the capability-scope plane of the
AgentBinding made concrete: the binding does not just hold execution heads and a memory
scope, it holds a capability scope (which affordances this agent can reach and is primed
for), and that scope is enforced and learned, not merely declared.

### Selection is proactive, not reactive

A passthrough layer is reactive: the model decides it wants a tool, asks, and the layer
forwards. The substrate connector layer is proactive: because the Pairformer has a learned
prior over affordances and the charter has scoped the relevant set, the system can surface
"the tool you probably want here" before the model gropes for it, the way a good colleague
hands you the right instrument because they have seen this kind of work before. Proactive
selection over accumulated experience is the felt difference between a tool that has reach
and an agent that knows its own toolkit.

## How this rides existing RustyRed machinery

This spec deliberately adds no new infrastructure; it composes what 0.6.0 already has.

The MCP adapter (the `rustyred-thg-mcp` crate) already turns the core into a JSON-RPC
tool/resource server with read-only and admin gating. The connector registry is the inverse
direction: incoming connectors register their tools as affordance nodes through the same
core executor the adapter already wraps. The read/admin gating model already exists for
deciding what a caller may do; affordance permission requirements reuse it.

The affordance nodes use the existing node/label/property/vector model in the graph store.
The "which affordances were sequenced together" and "which affordance served which task"
relationships use the existing edge model, including the epistemic edge types where an
affordance outcome supports or contradicts a selection heuristic. The receipt pattern (every
hot inference records the graph version and snapshot it used) extends to affordance calls:
every affordance invocation records which affordances were considered, which was selected,
and what the outcome was, which is both the audit trail and the Pairformer's training data.

The graph-aware cache already invalidates results when the graph mutates; affordance
selection results cache and invalidate the same way when the affordance subgraph changes
(a connector added, a tool's success record updated).

## The build, in dependency order

This is descriptive, not a schedule. Each piece depends on the one before it.

Define the Affordance node shape in the tenant graph: schema, owning server, permission
requirements, cost shape, embedded semantic description as a designated vector property.
Define the edge types: SERVED_TASK (affordance to task type), PRODUCED_OUTCOME (affordance
to outcome), SEQUENCED_WITH (affordance to affordance). This is a schema addition to the
existing store, not a new store.

Build the connector registration path: when an MCP server is connected, walk its tool
catalog and upsert an Affordance node per tool, embedding the description through the
existing embedder machinery. Idempotent on re-registration (same connector, same tools,
updated metadata). This is a write path through the core executor, parallel to how the
MCP adapter already exposes the core.

Instrument affordance invocation with receipts: every call records the candidate set, the
selection, the graph version, and the outcome as an edge and a receipt. This is the same
receipt pattern already used for hot inference, pointed at tool calls. It produces the audit
trail immediately and the Pairformer's training corpus over time.

Extend the Pairformer's training to consume affordance-outcome edges: affordance selection
becomes a learned head alongside the existing routing. Train on the accumulated invocation
receipts (and the imported reasoning-trace corpus, which already records tool choices and
outcomes). Validate on held-out task/affordance pairs, not on the training distribution
(the same selected-data caution that applies to all the lab-graph training: successful runs
are over-represented, so verify the learned selector improves held-out outcomes rather than
assuming more invocations make it better).

Add the charter capability-scope field: the binding enumerates the affordance subset for
the agent, and the Pairformer ranks within it. The model is primed with the scoped, ranked
set. This closes the loop from "connected to everything" to "primed for the right things."

## What this is not

It is not a passthrough aggregator. Forwarding is the fallback for an affordance that has
no learned prior yet (a freshly connected tool), not the default behavior.

It is not a place to put a thousand tools in front of the model. The charter scope and the
learned ranking exist precisely so the model never sees the full registry.

It is not new infrastructure. It is a labeled subgraph plus a Pairformer head plus a charter
field, all riding the graph store, the embedder machinery, the receipt pattern, and the MCP
adapter that already exist in 0.6.0.

## Why it matters for the runway

The differentiated-action throughline from earlier in this work asks of any capability:
does this improve with substrate accumulation, or would a stateless tool do it as well? A
passthrough connector layer is the stateless-tool answer: anyone can forward MCP calls.
The substrate-as-connector-layer is the compounding answer: the system gets better at using
its tools every time it uses them, scoped per agent, grounded in provenance. That is action
across the user's whole connected world that improves with use, which is exactly the kind of
action the runway argument says is the wedge rather than the table stakes. The connector
layer is where reach (the commodity) becomes knowing-what-to-do-with-reach (the moat).
