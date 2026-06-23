# Handoff: Servo Browser-Use Agent in Theorem

A port of the Theseus perceive / govern / afford stack into Theorem (Rust), with an embedded Servo engine as the web execution route. Sibling to `docs/plans/live-web-reach/HANDOFF.md` (the engine plan) and `docs/plans/skill-encoder/v0.3-encoder-spec.md` (the skill packs the playbooks feed).

## North star

The browser-use agent is not a flat click loop. It is the epistemic perceive -> govern -> afford stack that already exists in Theseus (`apps/notebook`), re-homed into Theorem next to the graph, with an embedded Servo engine driving the page. Because the loop runs in-process with RustyRed, browsing becomes immediate graph ingestion: needing more info calls rustyred and the graph grows in the same step. Two surfaces ride the stack: browse_with_me (a shared live Servo session, the human and the agent in one tab) and browse_for_me (the same stack on autopilot for a given task). Per-intent playbooks are SKILL.md packs that go through the existing skill encoder, Ensemble selection, and the held-out gate.

## What exists today (the reference implementation: Theseus, Python, Playwright-backed)

Pinned at `apps/notebook` @ c37541a82ed19119f4a7cb17514a6275c9dff2ad. Four modules, each expressed as frozen dataclass contracts, which port cleanly.

`context_command/` is the governance front door. It resolves a raw client request into a `ContextCommandState` carrying explicit policy:
- PermissionPolicy: allow_read, allow_write_hot_graph, allow_write_canonical, allow_remember, allow_external_web, allow_disclosure, allow_agent_execution, require_confirmation_for_write, require_receipt.
- RetrievalPolicy: mode in {local_only, local_first, web_allowed, web_required}, freshness_required, include_counterevidence, include_tensions, include_user_priors.
- RiskMode: read_only | confirm_before_write | supervised_action | private.
- OutputTarget, GraphLayer scope, ToolName scope (ask, browser, capture, web_research, code, files, calendar, email, agents, theorem), TracePolicy.
This is the gating and scoping layer.

`perception/` is the epistemic perceive step. It produces a `PerceptionBundle`:
- PerceptionCandidate[] with kinds (object, claim, webdoc, url, tab, file, action, tool, memory, counterevidence) and status (known, local, external_unfetched, fetched_unadmitted, admitted, rejected).
- CoverageDiagnosis (has_known_context, has_browser_context, needs_web, needs_counterevidence, needs_freshness, confidence).
- ActionCandidate[].
Modes: ASK, BROWSE, CAPTURE, COMPARE, VERIFY, MONITOR, ACT.

`action_rail/` is affordance generation. It produces an `ActionRailBundle`: ranked, grouped ActionCandidate[] with action_type from a fixed catalog (summarize_page, summarize_selection, explain_selection, ask_with_context, capture_page, extract_claims, compare_to_graph, find_counterevidence, verify_claim, inspect_source_quality, show_related_objects, open_related_sources, create_report, draft_memo, monitor_page, remember_to_project, mark_source_trusted, mark_source_noisy, exclude_source, inspect_permissions, switch_private_mode), a category, a risk, a status, and an execution_route (ask_pipeline, capture_api, web_api, context_command_api, monitor_api, writeback_api, frontend_only, not_implemented).

`browser_playbooks/` is per-intent SKILL.md playbooks: deep_research, docs_search, paper_search, pricing_tracker, product_research, source_verification. Each is concise prose guiding a native search session (prefer WebDoc ingestion, preserve source URLs privately, emit structural trace events before any training export).

The low-level page driving (click, type, navigate) today is Playwright (root `.playwright-cli/` + `playwright.config.ts`). That is the one piece re-targeted onto embedded Servo. The epistemic stack above it keeps its shape.

## Target placement (settled)

rustyred-web is the unified web access point: RustyWeb fetch, crawl, scrape, plus the drivable Servo engine (the three fetch tiers, Tier 3 = Servo render). All outbound web I/O lives here.

The perceive -> govern -> afford stack (the four modules, ported to Rust) lives in the RustyRed core/runtime, in-process with the graph. ActionCandidates whose execution_route is web_api or whose tool is browser call the Servo engine in rustyred-web.

Payoff: admitting a WebDoc is a local write, not a round-trip. Needing more info calls rustyred and the graph grows in the same step.

## The port, module by module (Python contracts -> Rust)

Each frozen dataclass becomes a Rust struct, each Literal a Rust enum. Keep the contract names. The resolver/generator logic ports alongside.

context_command -> a Rust ContextCommand resolver producing ContextCommandState. PermissionPolicy and RiskMode are the native home for the browse_with_me vs browse_for_me distinction and for robots/confirmation gating (below). Collapse GraphLayer: drop falkor_hot, keep memgraph_canonical, rustyred_hot, redis_hot, local_webdocs, since FalkorDB was retired after RustyRed-THG parity. Fold the hot-graph reads onto rustyred_thg.

perception -> a Rust Perception kernel producing PerceptionBundle. Candidates resolve against the in-process graph (objects, claims, webdocs) plus the open tabs plus the live Servo page. CoverageDiagnosis.needs_web drives escalation to the fetch cascade and Servo.

action_rail -> a Rust ActionRail producing ActionRailBundle. The catalog ports as is. execution_route becomes the dispatch table: web_api and browser route to the Servo engine, capture_api and writeback_api to graph writes, ask_pipeline to the model. Every ActionCandidate that writes is gated by PermissionPolicy and RiskMode.

browser_playbooks -> the SKILL.md playbooks are ingested by the existing skill encoder into content-addressed skill/domain packs, surfaced by intent (and later by domain) via Ensemble, promoted or retired by the held-out gate and UseReceipts. Not a parallel store. New playbooks (agent-authored, or learned from a co-browse demonstration) re-enter the same encoder.

## The drivable Servo engine (the web route, in rustyred-web)

The low-level primitive under the rail. Async navigate(url), observe() -> PageState, act(Action), extract(schema) -> json, render() -> screenshot. PageState is url, title, distilled text, and interactive elements [{element_id (stable index), role, name, value, bbox, visible}] built from Servo's accessibility tree in-process, with no CDP round-trip. Action enum: Click(element_id), Type(element_id, text), Select(element_id, value), Scroll(delta), Back, Forward, WaitFor(condition), Submit. Pooled BrowserPool of N Servo instances, on-demand, RunPod-offloaded. This is the same engine FetchTier::Rendered uses for crawl (live-web-reach Part B), built drivable from the start rather than as a one-shot renderer.

## Two surfaces

browse_with_me (the priority): the human and the agent share one live Servo session. RiskMode supervised_action, allow_agent_execution gated by a per-action preview the human can veto; the human can take the wheel and hand it back; the human's manual actions are captured as demonstration and become playbook/skill candidates. This is the Cowork-surfs-with-me surface and the biggest leverage for knowledge work and computer tasks.

browse_for_me (autopilot): the same stack runs the perceive -> afford -> act loop to completion for a given task, bounded by an Ensemble budget, emitting a BrowsingRun and a playbook. RiskMode confirm_before_write for state-changing actions, otherwise autonomous.

Both ride the same engine and ingest to the graph. A read/extract primitive (web_consume) serves both.

## MCP surface

Register in `rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs`, mirroring the fractal_expansion handler from live-web-reach. browse_with_me (co-browse session control: control modes human_drive, agent_drive, pair, pre-action preview, demonstration capture), browse_for_me (autopilot over a task), web_consume (navigate, observe, extract, ingest to the quarantined graph). Each returns a receipt: BrowsingRun id, pages reached, actions applied, data extracted, playbooks or skills used or created.

## Pluggable driving model

The loop is model-agnostic. The model that selects the next ActionCandidate is a parameter: a local agent, Claude, Codex, Mistral, or any API agent. Peers over the substrate.

## Gating and safety (native, via Context Command and robots)

PermissionPolicy and RiskMode govern read vs write-hot vs write-canonical vs remember vs external-web vs agent-execution, and the confirmation and receipt requirements. State-changing actions (Submit, purchase, post) require confirm_before_write or a higher trust tier, especially in co-browse. Tier 2/3 promotion (impersonate, render) passes robots.rs first; the currently-ungated promote() seam from live-web-reach must land before this turns on. Extraction stays quarantined (open_web_unverified, confidence_ceiling 0.35) until promoted.

## Receipts and trace

ContextCommand already requires receipts (require_receipt) and a TracePolicy (graph_trace, receipts, context_preview). Map this onto the harness run/event-ledger: a session is a content-addressed BrowsingRun, the ordered ledger of perceive/afford/act/observe events, replayable and forkable like an EnsembleDecision. This is also the billing telemetry: meter receipted compute (tokens plus GPU-seconds plus renders), with sessions or plans as the package on top, and a bring-your-own-model tier that charges for the substrate rather than tokens.

## Observable acceptance

- The four contracts exist as Rust types and round-trip the same JSON the Python emits.
- ContextCommand resolves a request into a ContextCommandState with the policies set; a read_only command cannot trigger a write.
- The Servo engine navigates, observes, and acts on a live page in-process with no external browser.
- A PerceptionBundle resolves candidates against the in-process graph, and a CoverageDiagnosis with needs_web set triggers the fetch cascade.
- An ActionRailBundle dispatches a web_api action to the Servo engine and a capture action to a graph write.
- browse_for_me completes a multi-step flow on a permitted site and emits a BrowsingRun and a playbook; browse_with_me hands control back and forth with a pre-action preview.
- The robots gate blocks action on a terms-forbidding domain.

## Where compute-code helps

On the Theorem side during the rebuild, code_search and harness_kg_impact tell Codex and Claude Code what a multi-crate change touches across rustyred-web, rustyred-thg-core, and rustyred-thg-mcp, and find the attachment points for the ported stack. On the Theseus side, it can map the perception -> context_command -> action_rail -> playbooks dependency graph if `apps/notebook` is ingested into the code graph. It is structural code intelligence, not a porter: it shows the shape and the blast radius, the Rust is still authored. The four module contracts above are the ground truth for the types.

## Open

- The loop lives in core (settled); the engine is in rustyred-web (settled). Confirm whether the ported stack is its own crate inside core or folded into an existing one.
- The Servo V2 embedding API and the rquest pin (flagged in live-web-reach) confirmed at build time.
- Whether browser_playbooks stays per-intent (today) or also grows per-domain (browser-harness style) once co-browse demonstration capture lands.
