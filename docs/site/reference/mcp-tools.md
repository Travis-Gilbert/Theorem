# MCP tool catalog

The primary way to consume Theorem's Harness is as an [MCP](https://modelcontextprotocol.io) server. An MCP-capable client (Claude Code, Codex, the claude.ai connector) connects and gets the tools below. They are served by `rustyred-thg-mcp` and exposed over stdio or over HTTP at `POST /mcp`.

This catalog groups the tools by what they do. The first four groups are the headline product capabilities; the rest are graph and engine power tools you reach for as needed.

> Terminology note: some tool names carry internal vocabulary (`epistemic`, `designate`, `ppr`). The [Glossary](glossary.md) translates each. Descriptions below use plain terms where possible.

## Memory

Durable, typed memory that survives across sessions.

| Tool | What it does |
|---|---|
| `remember` | Write a memory document or typed memory node. |
| `recall` | Retrieve memory by query, weighted by relevance and recency. |
| `relate` | Find graph neighbors connected to a saved memory. |
| `observe` | Read harness context without writing or consuming state. |
| `self_note` | Write a typed self-memory document. |
| `self_revise` | Create a revision-tracked replacement for a memory. |
| `self_archive` | Move a memory to the on-disk (cold) tier. |
| `self_recall_archive` | Recall archived memory atoms. |
| `encode` | Record feedback, a solution, or a postmortem with an outcome signal. |
| `upsert_note` | Create/update a note by stable id and reconcile its `[[wikilinks]]` into edges. |
| `forget` | Soft-delete a memory with an audit reason. |
| `handoff` | Create a cross-agent handoff memory. |

## Coordination

The shared room where several heads see each other's work.

| Tool | What it does |
|---|---|
| `coordination_room` | Join or inspect a coordination room. |
| `coordination_intent` | Announce what you are doing now and which files your hands are on. |
| `coordinate` | Post a direct message, optionally with @mentions and a wake. |
| `coordination_record` | Write a durable record (event, decision, tension, reflection). |
| `coordination_contribution` | Capture a contribution as an event record. |
| `coordination_context` | Read a bundled context packet for turn-start injection. |
| `presence` | Read, refresh, or end an actor's presence. |
| `mentions` | Read pending @mentions for an actor. |
| `read_intents_for_room` | Read intents for a room. |
| `read_messages_for_room` | Read direct messages for a room. |
| `read_records_for_room` | Read durable records for a room. |

## Jobs (dispatch)

Hand work from a planning surface to an executing head.

| Tool | What it does |
|---|---|
| `job_submit` | Create or upsert a pending job (idempotent). |
| `job_list` | List the job board by priority and time, filterable by repo and state. |
| `job_note` | Append a receipt to a job thread. |
| `job_archive` | Archive a job with a reason. |

## Harness lifecycle

Prepare context, run, and inspect the typed event log.

| Tool | What it does |
|---|---|
| `harness_prepare` | Compose a context brief from capability selection plus memory recall. |
| `harness_run` | Read a run and its ordered event log. |
| `harness_append_transition` | Append a transition to a run's event log. |
| `composed_agent_run` | Run one composed-agent turn through the scratchpad and alignment gate. |
| `ensemble_select` | Select capability packs under task, budget, and trust constraints. |
| `ensemble_register` | Register a content-addressed capability pack. |
| `skill_list` / `skill_get` | Browse and read skill packs. |
| `skill_publish` / `skill_apply` | Publish a skill pack; apply one and record a use receipt. |
| `spawn_session` | Spawn a room-visible session via the handoff workflow. |
| `tool_result_fetch` | Page through a large tool result that exceeded the MCP size boundary. |

## Multi-head work graph

Durable parallel work: claim, patch, and adversarially verify tasks.

| Tool | What it does |
|---|---|
| `multihead_run` | Start or inspect a multi-head work-graph run. |
| `multihead_next` | Route the next claimable task to a head. |
| `multihead_task` | Create a durable claimable task node. |
| `multihead_claim` | Acquire or release a leased claim on a task. |
| `multihead_refine` | Split a claimed task into children. |
| `multihead_patch` | Mark a claimed task as patch-proposed. |
| `multihead_proof` | Run a proof command and persist the receipt. |
| `multihead_review` | Open or complete an adversarial verify node. |
| `multihead_spawn_verify` / `multihead_submit_verify` | Spawn and submit a falsification check for a patch. |

## Code intelligence

Search and ingest code as a graph.

| Tool | What it does |
|---|---|
| `compute_code` | Native code discovery: search, context, explain, recognize, explore. |
| `code_ingest` | Ingest or reindex a local repository into the code graph. |
| `reconstruct_binary` | Load/analyze/lift binary artifacts through the reconstruction harness and return evidence-backed reconstruction receipts. |
| `harness_kg_status` | Status of the merged code-graph view (base + session delta). |
| `harness_kg_search` | Lexical code-object search. |
| `harness_kg_ppr` | Rank code objects by relevance to seeds. |
| `harness_kg_impact` | Blast radius of changing a code object. |
| `harness_kg_related_objects` | Find related code objects. |
| `harness_kg_explain_edge` | Explain why two code objects are connected. |

## Graph queries and algorithms

Read and compute over the graph store.

| Tool | What it does |
|---|---|
| `rustyred_thg_graph_neighbors` | Read adjacency neighbors. |
| `rustyred_thg_graph_query` | Bounded neighbors / exact-match query. |
| `rustyred_thg_relational_query` | Run a native relational planner query or GraphQL-style selection across graph-backed relations. |
| `rustyred_thg_graph_explain` | Explain a bounded query plan. |
| `rustyred_thg_graph_schema` | Labels, edge types, stats, capabilities. |
| `rustyred_thg_graph_index_status` | Index health and drift. |
| `rustyred_thg_index_spine` | Inspect adaptive-index manifests, query receipts, advisor proposals, context views, map artifacts, training runs, and export redaction validation. |
| `rustyred_thg_epistemic_neighbors` | Traverse confidence-weighted edges (supports/contradicts/refines/cites). |
| `rustyred_thg_algorithm_ppr` / `_pagerank` | Personalized and global PageRank. |
| `rustyred_thg_algorithm_components` | Connected components. |
| `rustyred_thg_algorithm_communities` | Community detection. |
| `..._ppr_inline` / `_pagerank_inline` / `_components_inline` / `_communities_inline` | The same algorithms over an inline adjacency you pass in (stateless). |

## Graph versioning

Git-like history for the graph.

| Tool | What it does |
|---|---|
| `rustyred_thg_graph_version_compile` | Commit the current graph to a content-addressed pack. |
| `rustyred_thg_graph_version_diff` | Diff two snapshots. |
| `rustyred_thg_graph_version_ref` | Update a branch ref. |
| `rustyred_thg_graph_version_log` | Walk commit history. |
| `rustyred_thg_graph_version_checkout` | Reconstruct a snapshot. |
| `rustyred_thg_graph_version_merge` | Three-way merge with conflict detection. |

## Search and indexing

Register and query vector, spatial, and full-text indexes. ("Designate" = register a property for an index.)

| Tool | What it does |
|---|---|
| `rustyred_thg_vector_designate` / `_vector_search` / `_vector_hybrid` | Register vectors; nearest-neighbor and graph-blended search. |
| `rustyred_thg_fulltext_designate` / `_fulltext_search` | Register and search full-text. |
| `rustyred_thg_spatial_designate` / `_spatial_radius` / `_spatial_bbox` | Register coordinates; radius and bounding-box search. |
| `rustyred_thg_bulk_nodes` / `_bulk_edges` | Bulk upsert nodes and edges. |

## Symbolic reasoning

Classical reasoning offloaded from the model.

| Tool | What it does |
|---|---|
| `rustyred_thg_symbolic_datalog_derive` | Derive facts from rules (forward chaining). |
| `rustyred_thg_symbolic_probabilistic_source_reliability` | Source-reliability estimate from corroboration/contradiction counts. |
| `rustyred_thg_symbolic_probabilistic_expected_value` | Expected value of running a check. |

## Web and browser

Fetch, search, and drive the web into the graph.

| Tool | What it does |
|---|---|
| `web_consume` | Fetch, observe, and optionally ingest one page. |
| `browse_with_me` | Supervised co-browse with pre-action preview. |
| `browse_for_me` | Autonomous browse bounded by policy. |
| `web_search_graph` | Search that returns graph-shaped results. |
| `rustyweb_search_acquisition` | Queue a multi-provider search fan-out (pollable). |
| `hippo_retrieve` | HippoRAG-style retrieval over the corpus. |
| `fractal_expansion` | Progressive-refinement search that grows the corpus. |
