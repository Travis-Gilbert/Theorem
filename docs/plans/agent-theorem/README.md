# Agent Theorem

The agent the user actually talks to. One identity, one voice, one continuous memory of *you* — composed of multiple LLM substrates that the user never has to think about. The substrate work is done; the **user-presentation layer** that turns "infrastructure for a composed agent" into "Theorem, who knows you" is what this plan owns.

Date: 2026-06-28. Author: claude-code (this session). Status: active implementation.

## 0. Why this doc exists

Across [`composed-agent/`](../composed-agent/) and [`compute-offload/`](../compute-offload/), Travis has built two of the three layers needed for a multi-model single-identity agent:

1. **Compute-offload** (`rustyred-thg-offload`, merged): the planner that routes operations to their cheapest CPU executor (six axes: symbolic / pushdown / fusion / reuse / verification / [cascade-dropped]).
2. **Composed-agent kernel** (`theorem-harness-core`, Lane A COMPLETE): the `AgentBinding` runtime — multiple models bound as one agent, heads share an uncommitted scratchpad, alignment lives in the binding (not the heads), identity persists when heads swap. **Genuinely novel as a coherent architecture** (see §1).

What's missing is the **third layer**: the user-presentation surface that turns the substrate into an agent the user *experiences* as one entity named Theorem who remembers them. That gap is the subject of this plan.

## 1. Novelty verification (research, 2024-2026)

A focused web-grounded survey (Perplexity deep research with high reasoning effort, citations [1]-[18]+) confirmed: **no published or shipped system fully implements** the composed-agent primitive. Quoting the survey directly:

> "Your proposed pattern — multi-model, single-identity agents with a binding-as-identity runtime and shared scratchpad — is a real conceptual gap, not a repetition of existing nomenclature. It appears to be novel as a coherent architecture."

The closest partial matches break into three families, **none of which hit the same primitive**:

| Family | Examples | Why it's not the same |
|---|---|---|
| Multi-model orchestration | Together AI Mixture-of-Agents (Wang et al. 2024: +7.6pp over GPT-4o), Corex, blackboard/Global Workspace architectures | Each model remains an autonomous agent in a collective. Outputs are aggregated, not unified into one identity. |
| Identity/governance work | Ephemeral agent credentialing, persona-backstory binding, collaborative memory frameworks | Locate identity in runtime objects separate from weights, but **do not bind multiple LLM substrates into one agent unit**. |
| Shipped frameworks | OpenAI Responses API, Microsoft Agent Framework, Google GKE Code Generation Agent, Suprmind, AutoGen v2/v3, MetaGPT, CAMEL, Letta/MemGPT | Provide agentic loops with multi-model fallbacks under a single *application* identity — but stop short of treating multiple models as interchangeable heads of one cognitively unified agent. |

Theoretical antecedents (not products): Minsky's **Society of Mind**, Baars' **Global Workspace Theory**. Within-model precedent: **Mixture-of-Experts** (MoE) routes inside a single architecture, not across substrates.

**Conclusion**: directionally novel. Implementation precedent does not exist; component primitives do. The risk of being scooped on the *primitive* is low; the risk of being scooped on the *application* (a "talk-to-Theorem" product) is the usual product-execution risk.

### Adjacent measured evidence relevant to feasibility
- **Tool-injection vs in-token CoT** is well-measured: PAL (+15-40pp on GSM8K/GSM-HARD), SplitReason (+24-28% AIME24 while offloading only 1.35-5% of tokens, 4-6× speedup), Voyager (3.3-15× gains). Mechanism is **token reallocation** (model stops burning tokens on arithmetic, spends them on synthesis), NOT "model awareness reduces effort" — that laziness framing has no empirical support and the design does not depend on it.
- **MoA collaborativeness phenomenon** (Wang et al.): LLMs produce better outputs when shown peer outputs, even from weaker models. Validates the intra-agent loop's propose/critique/synthesize round structure — but stops short of binding the heads as one agent.
- **Self-MoA caveat**: aggregating N samples from one strong model can *beat* mixing different LLMs (+6.6% on AlpacaEval 2.0). MoA-style ensembles are sensitive to the weakest member dragging the panel down. Implication for composed-agent: **head composition matters**; trust tiers and per-capability reliability routing (already built in `theorem-harness-core`) are the right defenses.

## 2. What's already built (the foundation)

Layer-by-layer status against the composed-agent spec ([`../composed-agent/implementation-plan.md`](../composed-agent/implementation-plan.md)):

| Layer | Status | Notes |
|---|---|---|
| Binding state machine (16 events) | ✅ COMPLETE | `apply_binding_transition` in `theorem-harness-core/src/agent_binding.rs` |
| `AgentBinding` + 7 planes + composition_hash | ✅ COMPLETE | Heads swappable; hash changes ⇒ new version, same named lineage |
| Scratchpad + 4 memory zones (HeadLocal/BindingPrivate/AgentPublished/Commons) | ✅ COMPLETE | DAG of revisions persisted to GraphStore |
| Budget governor as hard guard | ✅ COMPLETE | Marginal-value decisions + per-head caps + total cap |
| Alignment guards (grounding strict, consensus, action tiers) | ✅ COMPLETE | Ungrounded/below-threshold/un-authorized-tier-3 publications blocked at `POLICY.CHECKED` |
| Runtime persistence (binding + events + scratchpad to GraphStore) | ✅ COMPLETE | Survives `RedCoreGraphStore` reopen |
| `AgentHeadRegistry` (fake-endpoint resolution) | ✅ done | Lane B step 3 partial |
| `run_fake_intra_agent_loop` w/ North Star stages 1-6 | ✅ done (deterministic) | Per-capability reliability routing, synthesis verification receipts, marginal-value budget, bounded rounds, outcome-fed reliability compounding |
| Charter compiler (`compile_binding_charter`) | ✅ done | Zero-silent-capability enumeration into `CHARTER.COMPILED` |
| Builtin + Theseus app affordances as metadata | ✅ done | `register_builtin_affordances` + `register_theseus_app_affordances` |
| `composed_agent_run` callable from MCP / GraphQL / server | ✅ wired | `rustyred-thg-mcp`, `rustyred-thg-server/router.rs`, `theorem-harness-runtime/composed_agent.rs` |
| Compute-offload planner (operation-level routing) | ✅ COMPLETE (merged PR #61) | All 6 axes incl. verification; exposed as `compute_offload.route_operation` affordance behind the affordance-router |

This is a lot. The composed-agent kernel is correctness-critical, parity-tested, persisted, and the offload planner sits beside it for operation routing. **The infrastructure works.**

## 3. Holes in prior plans (be honest about what isn't built)

These are open work items the composed-agent spec acknowledges or that fell out of scope; each is a real prerequisite to "Agent Theorem" the user can actually use.

### A. Lane B model layer ✅ COMPLETE (was named open; S4 audit on 2026-06-28 found it already shipped)
From [`composed-agent/lane-b-next-plan.md`](../composed-agent/lane-b-next-plan.md):
- ~~**`AgentHeadRegistry` runtime/provider credential resolver**~~ **DONE.** `theorem-harness-runtime/src/head_invoker/credentials.rs::CredentialResolver` exchanges `env:`/`secret:`/`secret-store:` refs at call time with typed errors; never persists to GraphStore.
- ~~**Live model invocation receipts**~~ **DONE.** Production paths (`rustyred-thg-mcp::composed_agent_run_to_store`, `rustyred-thg-server::TenantGraphStore::composed_agent_run`, `apps/commonplace-api/src/schema.rs`) default to `ProviderHeadInvoker::from_env()`. `FakeHeadInvoker` only used in tests via `<I: HeadInvoker>` injection.
- ~~**Live `theorem_grpc.*` invocation adapter**~~ **DONE.** `TenantGraphStore::invoke_app_affordance` in `rustyred-thg-server/src/state.rs:2618-2810` — real tonic dispatch against `/theorem_grpc.AppAffordanceService/InvokeAffordance` with env-resolved endpoint. See [`../composed-agent/lane-b-next-plan.md`](../composed-agent/lane-b-next-plan.md) §"S4 audit verdict (2026-06-28)".
- **Charter-surface reintegration** — the charter compiler is built but its visibility surface (what each head *sees* it can call when contributing to the scratchpad) needs a final reintegration pass.
- **Pairformer A/B for intra-agent router moderation** — referenced in CA-B2.2; landed as per-capability reliability routing but the explicit pairformer wiring for proposal-vs-critique routing wasn't called out as done.

**Status**: ~70% of Lane B has landed; the live-model + live-adapter completion is the real bridge to "the binding actually runs Claude+Codex+etc."

### B. PG wire (`rustyred-thg-pg-server`) not wired to bindings
The Postgres wire-protocol server exposes native views (`nodes` etc.) over SQL. **Bindings, agent runs, and scratchpad revisions are not projected as SQL-queryable views.** Any pg-speaking tool (Postico, DBeaver, BI dashboards, Grafana, n8n, …) cannot today see "the agent's outputs as data." Easy fix in principle — add views to the relational planner — but it's net-new work.

### C. Computation reuse cache is not agent-scoped
Per [`compute-offload/RECONCILED-PLAN.md`](../compute-offload/RECONCILED-PLAN.md) and the offload crate: reuse is keyed on `(op, inputs, graph_version)`. **Not on `agent_id`.** Implication: when "Theorem" computes something, it's reusable by any agent over the same graph. Could be intended (the substrate is shared); could be a leak (one agent's cached belief becomes another's). Worth a *decision* either way, then encoded.

### D. The Axum app exposes operator surfaces, not user surfaces
`composed_agent_run` is callable from `rustyred-thg-mcp`, `rustyred-thg-server/router.rs`, and `apps/harness-console` (per grep into the harness console's chunks). **None of these is a "talk to Theorem" surface** for an end-user who isn't operating the harness. `apps/theorem-gateway` exposes a `sceneForInput` and an `askAgent` GraphQL — the latter is the natural seam for a public "ask Theorem" route, but it returns "answer + graph context", not a binding-scoped conversation thread.

### E. No system-prompt / persona / voice layer
The binding has `agent_id`, `agent_name`, `trust_tier`. **No `agent_voice` / `agent_persona` / `agent_constitution` field.** Without one, heads contribute in their own training-default tone. **This is the #1 reason a user would feel "I'm talking to a multi-head system" instead of "I'm talking to Theorem"** — the synthesis step can stitch the prose, but tone leaks through.

### F. No user model the binding consults
`theorem_grpc.user_model` is registered as affordance metadata. **The MOUNTED-at-run-start contract that says "every binding run reads the user model and shapes responses by it" is not wired.** A "user model" here = a structured representation of the user's preferences, recent work, stylistic norms, what frustrates them, what they're optimizing for. Beyond freeform Commons memory: the *agent's mental model of you*, that conditions every contribution.

### G. Memory continuity across runs needs verification
`MEMORY_PATCHES.PROPOSED` writes; `MEMORY_SCOPE.MOUNTED` should mount what was published in prior runs. **Need to verify in code** that mount-at-start actually retrieves the agent's accumulated memory (including the user model from F). If it doesn't, every conversation starts fresh — fatal for the "Theorem who knows me" feeling.

### H. Spec-deferred (not gaps; named for completeness)
- Step 10: compose-your-own-agent UI (spec says "ship the static binding first")
- Step 11: self-optimizing / dynamic compositions (spec defers)
- MCP-as-substrate-learning-layer (the connector moat — `docs/plans/mcp-learning-layer/`)
- OpenClaw action layer (deferred; tier-3 alignment guard is the hook it lands behind)
- Latent memory / xLSTM / native hidden-state capture (open research)

### I. Qwen and Mistral are callable, but must become resident participants
Qwen (the spoken "Quinn" shorthand) and Mistral are no longer just optional model-provider smoke targets. They should be first-class room participants in both the local RustyRed room and the general room.

The distinction matters:
- **Callable provider head**: the harness can invoke `qwen` or `mistral` when a run explicitly asks for that head.
- **Always-on participant**: a running room runner posts presence, consumes wake or mention events, invokes the configured head through `ProviderHeadInvoker`, and writes the contribution/reflection back into the coordination room.

The bridge is the configured room runner, not a chat transcript file. `THEOREM_AGENT_HEADS=qwen,mistral` plus the matching `QWEN_API_KEY` / `MISTRAL_API_KEY` makes those heads available to the binding; `THEOREM_AGENT_ROOM_RUNNER=1` makes the binding resident in the room. Acceptance is a room-level proof: a message mentioning `@theorem` causes the runner to publish a Qwen/Mistral-backed contribution record without manually calling a smoke script.

## 4. The four user-presentation completion items

To turn the substrate into the user experience of "Theorem", in priority order:

### #1 (foundation) Persona/voice layer in the `AgentBinding`
- New field on identity plane: `agent_voice: Persona` or `agent_constitution: String` (the "who is Theorem" definition).
- Surfaced into the intra-agent loop's `DRAFTS.SYNTHESIZED` step as the prompt the synthesis head conditions on.
- Smaller alternative: a system-prompt-style preamble loaded into every head's invocation prompt at `HEADS.CONTRIBUTE`.
- **Why first**: without this, the multi-head illusion breaks the first time the user notices Claude's vs Codex's tone. All the other items below assume Theorem already has a coherent voice to put words in.

### #2 User model consulted at `MEMORY_SCOPE.MOUNTED`
- A structured `UserModel` node in the graph (preferences, recent work, style notes, open frustrations, current focus).
- Mount contract: every binding run pulls the user model into the scratchpad as `EntryKind::Context` before `HEADS.CONTRIBUTE`.
- Write path: `MEMORY_PATCHES.PROPOSED` can update the user model; alignment guards check the update is sourced from the conversation, not invented.
- Wire the existing `theorem_grpc.user_model` affordance to actually populate/query this.

### #3 Memory continuity verification + binding lineage
- Verify `MEMORY_SCOPE.MOUNTED` retrieves the agent's `AgentPublished` memory + the user model from prior runs.
- A `binding_lineage(agent_id)` query: walk the chain of binding versions (same `agent_name`, different `composition_hash`) so memory follows the agent through head swaps.
- Spec already supports this (composition_hash → new version, same named lineage); just needs the mount path to consume it.

### #4 User-facing "Theorem" surface
- A `/v1/talk/theorem` (or GraphQL `chat(agent: "theorem", message: ...)`) endpoint in `apps/theorem-gateway` or `apps/commonplace-api` (or a new app) that:
  - Resolves to the active Theorem binding,
  - Calls `composed_agent_run` with the user's message,
  - Streams the binding's published output as one agent's reply,
  - Persists the conversation thread into the AgentPublished memory zone (closing the loop with #3),
  - Optionally surfaces a "Theorem is thinking…" presence indicator + the per-head contributions on demand (for transparency, not as default).
- The UI surface: either extend `apps/commonplace` (web/mobile) with a "Theorem" chat view, or a new `apps/theorem` app. The harness-console is for operators; this is for the user.

### Plus B/C from §3 (not user-facing but worth resolving)
- B PG wire: project `agent_binding`, `binding_event_log`, `scratchpad_revisions`, `agent_published_memory` as SQL views. One PR.
- C Cache scoping: decide on `agent_id` in the reuse cache key (decision-only; small edit to `OperationPlanner` if yes).

## 5. Suggested build sequence

Each slice is oracle-testable (cargo test + clippy):

1. **Slice S1 — persona/voice layer** (binding kernel)
   - Add `agent_voice: Option<AgentPersona>` to `BindingIdentity`.
   - Thread into `run_fake_intra_agent_loop`'s synthesis prompt.
   - Test: a binding with a defined voice produces a synthesis that includes the voice's signature traits; a binding without one falls back to head-default.
   - Cargo: `cargo test -p theorem-harness-core`.

2. **Slice S2 — UserModel node + MOUNTED contract** (commonplace + kernel)
   - `UserModel` graph node schema (in `commonplace` or a new `theorem-user-model` crate).
   - `MEMORY_SCOPE.MOUNTED` transition reads the active `UserModel` for the binding's owner and injects it as scratchpad `EntryKind::Context`.
   - Test: a run mounts the user model; the scratchpad shows it as Context before HEADS.CONTRIBUTE.

3. **Slice S3 — memory continuity audit**
   - Read the existing MOUNTED implementation; verify it pulls `AgentPublished` memory; add tests if not.
   - Add `binding_lineage(agent_id)` query in `theorem-harness-runtime`.

4. ~~**Slice S4 — Lane B model layer completion**~~ ✅ **COMPLETE** (audit on 2026-06-28: already shipped on `main`; no code written for this slice). Live credential resolver, `ProviderHeadInvoker` production default, and live tonic dispatch for `theorem_grpc.*` app affordances are all in place. See [`../composed-agent/lane-b-next-plan.md`](../composed-agent/lane-b-next-plan.md) §"S4 audit verdict (2026-06-28)".

5. **Slice S4.5 — resident provider participants**
   - Register Qwen as an OpenAI-compatible provider profile and keep Mistral on `mistral-small-latest`.
   - Make `agent_runner` use the configured composed-agent path so one configured provider head can answer a room wake in single-head mode, while multiple configured heads still use the composed loop.
   - Add a sourceable local helper for `qwen,mistral` room participation that loads the private env files and sets `THEOREM_AGENT_ROOM_RUNNER=1`.
   - Test: offline runner regression proves a configured Qwen head wakes from a room mention and publishes a contribution; live acceptance uses the local env files and provider keys.

6. **Slice S5 — user-facing Theorem surface**
   - `apps/theorem-gateway` route: `chat(agent: "theorem", ...)` resolving to `composed_agent_run`.
   - Optionally: extend `apps/commonplace` with a Theorem chat view (or new app).

7. **Slice S6 — PG wire views + cache scoping decision**
   - SQL views for binding/event/scratchpad/memory.
   - Encode the agent_id cache-scoping decision; implement if yes.

S1-S3 are user-presentation-layer (the gap this plan owns). ~~S4 is composed-agent Lane B's named-open work~~ — S4 was already shipped (audit on 2026-06-28). S4.5 makes provider heads resident in rooms. S5 is the new app surface. S6 is infrastructure cleanup.

## 6. Resolved decisions (2026-06-28)

| # | Decision | Implication |
|---|---|---|
| Persona | "Helpful in service of reality; disagrees without being disagreeable; loosely Uncle Iroh (wise, caring, easygoing, morally aligned)." Draft constitution at [`constitution.md`](constitution.md) (270-word canonical text). | S1 loads this text into `BindingIdentity::agent_constitution` and prefixes it on synthesis. |
| User model schema | `preferences` / `style_notes` / `recent_focus` / `open_frustrations` / `working_on`. | S2 graph schema follows this; pre-existing `theorem_grpc.user_model` affordance is rewired to populate/query this shape. |
| Cache scoping | **Substrate-shared (default).** Offload planner stays keyed on `(op, inputs, graph_version)` — no `agent_id`. | No code change to `OperationPlanner`. Closes the question. |
| User-facing surface | **`apps/commonplace`** (extend; don't create new app). | S5 adds a Theorem chat view *inside* the existing Commonplace surface, reusing its user, local-first infra, and UI conventions. Avoids parallel-app drift. |
| Qwen and Mistral in the room | Treat Qwen and Mistral as resident provider participants when their keys are loaded, not only as direct smoke-test targets. | The room runner uses the configured composed-agent path; provider env files set `THEOREM_AGENT_HEADS=qwen,mistral` and `THEOREM_AGENT_ROOM_RUNNER=1` for local/general room runs. |

## 7. Cross-references

- Composed-agent: [`../composed-agent/README.md`](../composed-agent/README.md), [`implementation-plan.md`](../composed-agent/implementation-plan.md), [`lane-b-next-plan.md`](../composed-agent/lane-b-next-plan.md), [`lane-split.md`](../composed-agent/lane-split.md)
- Compute-offload: [`../compute-offload/RECONCILED-PLAN.md`](../compute-offload/RECONCILED-PLAN.md)
- Harness notes (in the shared substrate):
  - `doc_5db60e5151ee1d0d` — compute-offload reconciled plan
  - `doc_71d4aa79acef78f0` — field-grounded refinements
  - `doc_5719cbfde278bae6` — correction: planner already built
  - `doc_6f06c46662cc18f7` — correction: "multiple models, one agent" beyond MoA
- Code:
  - `rustyredcore_THG/crates/theorem-harness-core/src/agent_binding.rs` (Lane A kernel)
  - `rustyredcore_THG/crates/theorem-harness-runtime/src/composed_agent.rs` (runtime + persistence)
  - `rustyredcore_THG/crates/rustyred-thg-affordances/` (charter + affordances)
  - `rustyredcore_THG/crates/rustyred-thg-offload/` (operation-level offload planner)

## 8. The thesis in one sentence
**The substrate is built; the agent isn't presented yet.** Agent Theorem is the user-presentation layer that turns a working multi-model binding into "Theorem, who knows you" — with persona, user model, memory continuity, and a surface to talk to. The novel primitive (binding-as-identity over multiple LLM substrates) is verified to have no shipped precedent.
