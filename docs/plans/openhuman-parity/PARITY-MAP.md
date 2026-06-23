# OpenHuman to Theorem's Harness: one-to-one feature parity

Every OpenHuman feature, mapped to Theorem's status and the Theorem-specific way to build it. Treat OpenHuman as the baseline; the right column is the build target, expressed so it lands as a Theorem feature, not a clone.

Sources: OpenHuman GitBook + repo (GPLv3), read 2026-06-07. Theorem from the live MCP surface and code/commits through 2026-06-08 01:47 (the Servo browser-use job lineup, the shipped dispatch queue + theorem-receiver, the multi-head verbs, the desktop baseline), not the stale README/CLAUDE.md.

License fence for this whole lane: OpenHuman and boltbrowser are GPLv3. Read for architecture, behavior, and feature surface; reimplement in Theorem's own shapes. Never port code, structures, or prompts verbatim into the substrate. Patterns and ideas are the baseline; code is contamination.

Status legend: HAVE = shipped and code-grounded. BUILDING = active commits or a current handoff. PARTIAL = adjacent capability exists, not full parity. GAP = no answer yet, a real build target.

## 1. Memory

| OpenHuman feature | What it does | Theorem status | Theorem equivalent and the twist |
|---|---|---|---|
| Memory Tree | Markdown -> <=3k chunks -> scored -> per-source/topic/day summary trees in SQLite | PARTIAL | RustyRed/THG graph store with fitness scoring + communities; build the summary-tree rollup as graph community summaries, not a separate tree. Twist: the tree is a projection of the graph, not a parallel store |
| Obsidian vault | Same chunks as editable `.md` in a vault | HAVE | `apps/obsidian-sync` (pull + write-back, `MEMORY_RELATES` edges, `upsert_note`). Twist: the vault edits write back into the graph as edges, bidirectional |
| Memory tools | store / recall / forget / search | HAVE | `remember` / `recall` / `encode` / `forget` / `self_note` / `self_revise` / `self_archive`. Twist: encode carries outcome + fitness + training metadata; recall is graph-native (PPR + vector + BM25 + symbolic) |
| Tool-scoped memory | Stores guidance on how to use specific tools | HAVE (deeper) | The affordance layer: per-connector invocation receipts + per-affordance fitness. Twist: Theorem learns tool use from outcomes instead of storing written tips |
| Memory graph viz | Force-directed entity graph | BUILDING | scene-os-core/scene-os-web projections (force_graph, radial_rings, tree_layout, fractal_expansion) + iOS renderer. Twist: it is the real substrate graph, not a derived entity index |
| agentmemory backend | Pluggable Memory trait -> shared cross-agent store | BUILDING | Interoperate decision: expose/consume the agentmemory REST contract (store/recall/forget/smart-search) so Theorem is the substrate other agents mount, not a client |

## 2. Model selection and cost

| OpenHuman feature | What it does | Theorem status | Theorem equivalent and the twist |
|---|---|---|---|
| Automatic model routing | hint:reasoning/fast/vision/code -> (provider, model); the task emits the hint | HAVE (deeper) | Ensemble: capability-pack registry + budgeted selector + trust gating; node_type binds a pack. Twist: routing is learned from receipts and graph-scoped, not a static hint table |
| One subscription, many providers | Backend brokers Anthropic/OpenAI/Google/Groq | GAP (by design) | Theorem heads use their own logins (Claude Code, Codex); no LLM resale. Different model on purpose; note it, do not build it |
| Local AI (Ollama/LM Studio) | On-device embeddings, summaries, background eval | BUILDING | Ollama local lane wired in the desktop baseline (1c65be2a, pending verification). Remaining: on-device embed/eval for the background loop |
| Smart token compression (TokenJuice) | Rule overlay compacts tool output pre-context | PARTIAL | context-web packing + summarizer detour + the token plan in the context brief. Build a graph-aware compactor: dedupe tool output against nodes the graph already holds, send the delta. Strictly better than rule overlays |
| Per-turn USD cost accounting | Sums charged amount across provider calls | BUILDING | Per-turn cost accounting delivered in the desktop baseline (1c65be2a, pending verification; real cost lines gated on the model_chat blocker) |

## 3. The toolbelt (native tools)

| OpenHuman tool family | What it covers | Theorem status | Theorem equivalent and the twist |
|---|---|---|---|
| Web Search | Live web via managed proxy or SearXNG | HAVE | rustyred-web multi-source search broker (provider fan-out + 4B embed + eleven-stage epistemic filter + Perplexity provider). Twist: results are graph-yielding and epistemically filtered, not a raw SERP |
| Web Scraper | Clean text from any URL | HAVE | rustyred-web `web_consume_to_graph` + the V2 crawler. Twist: scraped pages land in the `open_web_unverified` quarantine tier with provenance |
| Browser and Computer Control | Open, screenshot, click, type, mouse | BUILDING | Servo browser-use rebuild in progress (jobs 007-009: parity on Servo, then engine-only and substrate-only abilities) + theorem-browser-agent (perceive/govern/afford) + computer-use. Twist: actuation feeds the substrate and is governed by an action-risk rail |
| Coder | read/write/edit/patch/glob/grep/git/lint/test | HAVE | The heads are Claude Code/Codex (full coder tools) + compute_code/code_search/code_crawl + instant-KG code encoder. Twist: code edits become a served instant KG with receipts |
| Cron and Scheduling | Recurring jobs, reminders, scheduled runs | HAVE | Scheduled tasks + the dispatch-v2 Job (single-shot plus not_before today). Build the recurring lane as job templates on the queue |
| Voice | STT in, TTS out, live Meet agent | GAP | No voice surface. A genuine build target if the desktop goes consumer-adjacent; otherwise a deliberate non-goal |
| Memory Tools | (see Memory section) | HAVE | as above |
| Third-party Integrations | Agent's view of 118+ services | PARTIAL | The connector/affordance layer (learn any MCP tool as an Affordance node). Twist: depth over breadth; learns which connector to reach for. Breadth catalog is the build target |
| Agent Coordination | Spawn subagents, delegate, plan, ask | HAVE (superset) | Cross-frontier-agent coordination: rooms, intents, mentions, presence, ambient verify. Twist: coordinates independent agents you do not own, not in-process subagents |
| System and Utilities | shell, node, SQL, time, push, LSP | PARTIAL | Heads have shell/code; RustyRed speaks RESP (SQL-adjacent graph query). Add push + LSP surfaces as needed |
| Image Tools | Image gen + local image inspection | GAP | No image tool surface. Build target if desktop needs it |

## 4. The agent runtime (harness)

| OpenHuman feature | What it does | Theorem status | Theorem equivalent and the twist |
|---|---|---|---|
| Orchestrator + archetypes | One senior agent spawns ~14 specialist archetypes (planner, researcher, critic, archivist, tool_maker...) | HAVE (different) | node_type to skill-pack binding via Ensemble; the work graph hands a head the right pack per node. Twist: specialists are skill packs on nodes, and the heads are separate frontier agents |
| Tool-call loop | One engine, three entry points, dialects, compaction | HAVE | theorem-harness-core run kernel + transitions; heads run their own loops, the substrate owns state |
| Sub-agent spawn hierarchy | 3-tier chat/reasoning/worker, depth<=3 | HAVE (different) | Multi-head work graph + scheduler with head fitness + explore-aware routing. Twist: heads are not interchangeable workers; node-type-to-head preference is learned |
| Critic / consensus | Critique archetype + publication consensus gate | HAVE (superset) | The ambient verify node: completion auto-spawns an adversarial verify assigned to the other head; receipt must be a falsification attempt; consensus gate (`MIN_CONSENSUS_HEADS`). Twist: review is continuous and grounded, with teeth |
| Self-healing | ToolMaker writes polyfills for missing commands | GAP | Adopt: a node_type that synthesizes a missing tool, registered as an Ensemble pack at draft trust tier, promoted by use-receipts. Rides the corpus2skill lifecycle |
| Trigger triage | Local LLM classifies webhook/cron/event -> drop/notify/spawn | PARTIAL | Dispatch v2 + receiver launch jobs; add a triage classifier in front of job_submit. Build target |
| Stop / post-turn hooks | Budget caps; archivist/learning/cost/episodic indexing | PARTIAL | Guard table + encode/fitness loop + receipts. Wire an explicit post-turn hook surface |
| Interrupts | Cancellation fence | HAVE | `RUN.CANCELLED` + cancel handle in the SDK surface |
| Replay / fork / KV-cache | Replayable turns, fork context | HAVE (superset) | Content-addressed state hashing + replay + fork over the event ledger; idempotency keys. Twist: the whole multi-head run replays to the same state hash |

## 5. Background autonomy

| OpenHuman feature | What it does | Theorem status | Theorem equivalent and the twist |
|---|---|---|---|
| Subconscious loop | Heartbeat ticks evaluate user/system tasks vs workspace; skip/act/escalate; local-then-cloud | PARTIAL | Dispatch v2 IS the chassis: a scheduled self-dispatching job reads the graph situation and job_submits; receipts are the decision log; not_before is the heartbeat. Their engine/executor/store/decision_log subsystem collapses to a queue usage pattern |
| System + user tasks (HEARTBEAT.md) | Seeded watchers + user-added tasks | GAP | A committed task list whose entries are recurring job templates with an enable toggle |
| Approval gate (unsolicited writes) | Cloud agent runs analysis-only; unsolicited write needs an approval card | PARTIAL | The action-tier policy + read-only/write-mode gating already separate read from write. Surface the approval card on the desktop; job receipts are the audit trail. Adopt this pattern; it is good safety design |
| Auto-fetch (every 20 min) | Folds new integration data into memory | GAP | A per-affordance scheduled job that ingests through the epistemic filter into quarantine. The morning twist: a consolidation job that PPRs fresh quarantine against the user's active subgraphs and posts a room digest. Not just fetched: positioned against what you know |

## 6. Embodiment and voice

| OpenHuman feature | What it does | Theorem status | Theorem equivalent and the twist |
|---|---|---|---|
| Desktop mascot | A character with a face, reactions, lip-sync | GAP | No analog. Deliberate decision: consumer delight vs developer focus. If yes, it is net-new |
| Live Google Meet agent | Joins a call, transcribes to memory, speaks | GAP | Net-new. High-effort, high-delight, off the current wedge |

## 7. Platform and deployment

| OpenHuman feature | What it does | Theorem status | Theorem equivalent and the twist |
|---|---|---|---|
| Desktop app | React + Tauri v2, Rust core, JSON-RPC | BUILDING | theorem-desktop baseline committed (Jun 8, local-first, Dia rebuild as phase one); phases for local node + receiver, version-pack sync, agent-space surfaces, Servo agent surfaces |
| Headless core + thin client | openhuman-core JSON-RPC on a server | HAVE | theorem-harness-server (Axum HTTP) + theorem-grpc + plugin/SDK clients. Twist: also a Claude Code/Cowork plugin and an MCP, not only a desktop sidecar |
| Cloud deploy | DigitalOcean / Fly / Docker Compose | HAVE | Railway services + Dockerfiles (harness-server, grpc, product MCP). Add a one-command self-host recipe |
| Local-first storage | SQLite + vault on device | HAVE | RedCore (AOF + snapshots) local-first store; the desktop is local-first by design |
| Version sync | (none stated) | HAVE (superset) | Prolly version-pack sync (theorem-desktop phase three): content-addressed, mergeable version packs. OpenHuman has no equivalent |

## 8. Extensibility, privacy, license

| OpenHuman feature | What it does | Theorem status | Theorem equivalent and the twist |
|---|---|---|---|
| MCP registry | External MCP servers on top of native tools | HAVE | rustyred-thg-connectors (live MCP transport) + affordances (learnable). Twist: registered tools are learned, not just listed |
| Skills with manifests | TOML archetypes, model pins, tool scope | HAVE (superset) | The skill encoder (corpus2skill): operators with a draft->canonical lifecycle, use-receipts, outcome-driven promotion. Twist: skills are grounded and promoted by real outcomes |
| Privacy boundary | Local data stays; backend brokers LLM/search/OAuth | HAVE | Local-first RedCore + tenant scoping + bearer auth on the product surface |
| OS keyring secret storage | Secrets in the platform keyring | BUILDING | Keychain backend delivered in the desktop baseline (1c65be2a); blocked on the Settings UI (review blocker) before keys can actually be entered |
| License | GPLv3, fully open | DIFFERENT | OSS plugin + SDK over a closed substrate. Your choice; cleaner commercial wedge than copyleft |

## What OpenHuman is genuinely ahead on (the baseline build list)

Ranked by build value for the stated market (knowledge workers, researchers, students, devs), not by how hard:

1. The desktop experience and onboarding. They ship a polished, UI-first, install-to-working-agent flow. The baseline is committed; the gap is polish and the agent-space surfaces, not architecture.
2. Auto-fetch + the subconscious loop. The "it already has tomorrow's context this morning" feel is real product magic and the pieces exist (connectors + dispatch v2 + the graph). Build the tick loop and the per-integration fetch, ingested through the epistemic filter.
3. Breadth of one-click integrations. They have 118 via Composio. Theorem has a deeper mechanism (learned affordances) but less breadth. Wrap a catalog with one-click OAuth on top of the connector layer.
4. The approval gate for unsolicited writes. Adopt it verbatim as a pattern; it is the right safety ergonomics for autonomy.
5. Self-healing ToolMaker. A small, cheap, delightful idea: a node_type that synthesizes a missing tool and retries.
6. Local AI lane. An on-device embed/eval model for the desktop keeps the background loop free and private.
7. (Optional, off-wedge) Voice, mascot, image tools. Net-new consumer surfaces; only if the desktop leans consumer.

## What Theorem has that OpenHuman has no answer to (do not bury these)

Cross-frontier-agent coordination (two rival-lab agents on one grounded run, ambient falsification verify, consensus gate). The graph substrate (PPR, symbolic, vector, GraphRAG, fractal expansion). Epistemic grounding (eleven-stage filter, `open_web_unverified` quarantine, provenance, receipts, replay/fork, byte-parity). Affordance learning from outcomes. The skill encoder (corpus2skill). Dispatch v2 + theorem-receiver (run claude/codex locally on existing logins, permissionless). Prolly version-pack sync. iOS + gRPC. These are the moat; the parity build is the cost of entry, these are the reason to switch.

## boltbrowser: the idea worth stealing (pattern, not code; it is GPLv3 Go over BoltDB)

boltbrowser is a terminal UI that opens a BoltDB file and lets you navigate its nested buckets and key/value pairs by keyboard, expand/collapse, view and edit values, and search. The borrowable pattern:

- A `theorem browse` TUI over the substrate. Navigate the graph the way boltbrowser navigates buckets: nodes and edges as an expand/collapse tree, memory zones and runs and jobs as top-level buckets, drill into a node to see its fields, receipts, and neighbors, edit a value, search by key or vector. This is the developer-ergonomics surface that fits the "agent space cooperation + ergonomics" focus, and it doubles as the debug view for a multi-head run (watch intents, verify nodes, and receipts live).
- The same pattern is the desktop's graph-inspector panel: boltbrowser proves a nested store is legible from a keyboard alone, which is exactly the "you can't trust a memory you can't read" bar OpenHuman set, met at the substrate level instead of via an Obsidian export.
- For the Prolly version-packs: a boltbrowser-style read-only viewer over a version pack (open a pack, walk its tree, diff two packs) makes the version sync inspectable, which is the thing that makes content-addressed sync trustworthy.

Small, high-leverage, on-brand. A weekend TUI that becomes the live debugger for the hero demo.

## The principle for the whole build

Do not clone OpenHuman feature by feature into a consumer assistant. Build each baseline feature as the grounded, coordinated, replayable version of itself: every memory has provenance, every tool call has a receipt, every background action is replayable, every autonomous write passes a gate. The baseline list is the cost of entry; the twist column is why someone leaves OpenHuman for Theorem.

## Code-reference map (for the heads, read via GitHub MCP; CodeCrawler ingest once theorem-grpc is back up)

- OpenHuman is Rust core + Tauri + TS workspace (Cargo.toml at root, app/, packages/, src/). Read their architecture decisions freely; the stacks rhyme.
- Subconscious subsystem: `src/openhuman/subconscious/` (engine.rs, executor.rs, store.rs, types.rs, schemas.rs, decision_log.rs, global.rs, heartbeat/rpc.rs, agent/prompt.md, agent/agent.toml, source_chunk.rs) plus `src/core/subconscious_cli.rs`. This is the reference surface for the autonomy lane; in Theorem it collapses onto dispatch v2.
- boltbrowser: main.go + internal/ + pkg/, tiny. Reference for the TUI's interaction model only.
