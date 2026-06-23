# Theorem Coding Harness - Design

Date: 2026-06-22
Status: design (validated in brainstorm, grounded against the code)
Sibling context: `SPEC-HARNESS-CONSOLE.md` (the web surface; this engine is what it drives)

## 1. What it is

The coding harness is the **head-execution engine + governance spine** the console
spec defers to ("the console is a view and a control surface over the harness and
the engine"). It owns: running a model head against a technical task, governing what
that head may do, remembering across runs, recording to RustyRed.

Decisions from the brainstorm:

- **One shared spine, many heads.** agentd-as-Gemma is just the local head in a
  roster: GLM 5.2, DeepSeek, Mistral, Codex, OpenAI, MiniMax 3.0.
- **The product is the coding interface where people complete technical tasks.**
- **Interactive-primary, can dispatch.** A live session you steer turn-by-turn that
  can also fire autonomous background tasks (receiver/dispatch already exist).
- **First surface = the harness-console (web)**, and the console **also embeds a
  TUI**. The engine is surface-agnostic (exposes heads over the SDK/ACP seam).
- **Governance is layered, not sole.** The constitution/hooks/memory are *enforced*
  in harness code (model-agnostic, so swapping heads can't reorder the law). The
  console *reinforces* it (surfaces authority order, hook decisions, attribution).
- **Velt ships in this build** (cursors/presence only). It was deferred by a prior
  session; it is in scope here. Agents render as labeled live cursors in the
  collaborative surface (one cursor per agent in a doc; one voice in chat).

## 2. What already exists (~70%)

Grounded against the tree. Reuse these; do not rewrite them.

| Capability | Where | State |
|---|---|---|
| ApiHead turn loop | `theorem-agentd/src/turn_loop.rs::run_once`, `model.rs::OpenAiModelClient` | Real. Single-tool-per-turn, OpenAI-compatible chat completions. |
| SpawnHead | `theorem-receiver/src/head.rs::HeadAdapter` (Claude/Codex) | Real. `program/build_args` + inherited detect/strip_env/parse_receipt. |
| Subscription-bridge auth | `receiver/src/spawn.rs` `strip_env` removes `ANTHROPIC_API_KEY` | Done. CLI subscription login wins over metered key. |
| Composed-head invoke | `theorem-harness-core/src/head_invocation.rs::HeadInvoker` | Trait + FakeHeadInvoker stub. |
| Runtime-recipe heads | `receiver/src/config.rs::HeadRuntimeRecipe` (codex/aider/openhands) | Defined, **not wired into run_job**. |
| **Constitution floor** | `theorem-harness-core/src/alignment.rs::evaluate_publication`, `agent_binding.rs::ActionTierPolicy` | **Partial** - invariants + 3 action tiers exist, gate publication only. |
| Charter | `theorem-agentd/src/tools.rs::charter()` | Hardcoded into the system-prompt string. |
| Memory recall/consolidate/decay | `rustyred-thg-memory/src/lib.rs::{recall,consolidate,decay,read_epistemic_shadow}` | Real. Recall is explicit-call only; no passive layer. |
| OS sandbox | `receiver/src/sandbox_exec.rs::OpenSandboxRuntime` | Real but **cloud-only** (HTTP to OpenSandbox). No local bwrap/Seatbelt. |
| Coordination | streams/rooms/footprint (memory index) | Done, ahead of the jcode borrow. |
| SDK turn primitives | `theorem-harness/src/{run,stream,event,session}.rs` | RunHandle/RunStream/Event/Session - building blocks, no turn API. |
| agent-grep | `compute_code` / CodeCrawler | Done. |
| Console scaffold | `apps/harness-console` (agent/memory/rooms/runs/skills/keys/providers/connections/canvas/inbox + tokens + island + `HarnessClient` mock+live) | Rich scaffold; binding fixture already has 3 heads. |

## 3. The genuine build: the spine (3 things)

Verified by adversarial refutation:

1. **Constitution (promote + layer, not greenfield).** `alignment.rs` already
   enforces the epistemic floor ("publication only HARDER, never easier"; every
   claim carries provenance) and `ActionTierPolicy` has 3 tiers. Missing: a
   *layered* authority order (global law -> project law -> current request -> live
   evidence) enforced for **every head's every turn** (not just composed-agent
   publication), with the law in harness code so model swaps can't reorder it.
2. **Passive memory (mechanism over existing substrate).** `recall()` exists and
   `turn_loop` even calls it at turn-start, but: it's an explicit best_effort tool
   call, there is no relevance gate (it injects whatever returns), seeding is the
   raw prompt, and there is no ambient consolidation. Missing: a passive layer with
   a relevance gate + turn-context seeding + ambient `consolidate()` cadence +
   epistemic floor via `read_epistemic_shadow`.
3. **Tool-lifecycle hooks (new, small).** No allow/deny/ask anywhere. `turn_loop`
   goes `validate_call` (schema only) -> `call_tool` with no decision point.
   `receiver --permission-mode` is a flag to the spawned CLI, not a harness hook.
   `rustyred-thg-core/src/hooks.rs` is post-commit graph mutation - different thing.
   Missing: a PreToolUse/PostToolUse allow/deny/ask gate above both transports.

## 4. Architecture: the Head trait + roster-as-config

The roster is mostly **config, not new impls** - DeepSeek/Mistral/MiniMax/GLM/OpenAI
are all OpenAI-compatible, which `OpenAiModelClient` already speaks.

```
HeadRegistry
  ApiHead   (agentd OpenAiModelClient)  <- one config entry per: gemma(local),
                                            glm-5.2, deepseek, mistral, minimax, openai
  SpawnHead (receiver HeadAdapter)      <- claude, codex (CLI, sub-bridge auth)
  -> all selected behind one interface; the spine wraps the interface, once.
```

Per the grounding's own recommendation: **do not merge `HeadAdapter` into the Head
trait** - keep spawn vs api separate, compose them in the registry. `HeadInvoker`
(harness-core) is the composition seam. Extend `ModelConfig` with a `HeadKind` enum;
`from_config` dispatches.

## 5. Milestones

### Engine (Rust) - the coding harness proper

- **M1 - Head roster unification.** HeadRegistry over `OpenAiModelClient` (N api
  heads as config) + `HeadAdapter` (2 spawn heads). Wire the dormant
  `HeadRuntimeRecipe` path in `receiver/run_job`. Reuse: `model.rs::ModelClient`,
  `head.rs::HeadAdapter`, `head_invocation.rs::HeadInvoker`, `config.rs::HeadRuntimeRecipe`.
- **M2 - Constitution.** A `Constitution` struct = layered authority order. Global
  law = lift `alignment.rs` invariants + `ActionTierPolicy` + `charter()`. Project
  law = a `.theorem/constitution.json` (protected_invariants, branch_policy,
  verification_policy, escalate_when). Enforce in `turn_loop` per turn for every
  head; carry a `policy_decision` on `TransitionResult`. Add a drift test asserting
  the order cannot be reordered. Reuse: `alignment.rs::evaluate_publication`,
  `agent_binding.rs::ActionTierPolicy`, `types.rs::GuardViolation` (+ `policy_layer` field).
- **M3 - Passive memory.** Turn turn-start recall into a passive layer: relevance
  gate on `RankedMemory.score` (floor), turn-context seeding via `recall_seeds`,
  ambient `consolidate()` on a cadence, epistemic floor via `read_epistemic_shadow`.
  No tool call surfaced to the head. Reuse: `recall`, `consolidate`,
  `read_epistemic_shadow`, `RankedMemory`.
- **M4 - Tool hooks.** allow/deny/ask `PreToolUse`/`PostToolUse` at the `turn_loop`
  dispatch point (api heads) + a hook point covering spawned heads (receiver
  run_job). Decisions are constitution-driven and audited. Reuse:
  `state_machine.rs::validate_toolkit_payload` pattern, `GuardViolation`.
- **M5 - ACP turn seam.** Head-facing "send a turn (messages, context, constraints)
  -> stream of `AgentEvent`" API + a typed `AgentEvent` union (Text/ToolCall/
  StateChange/Error/MemoryOp/Complete) + a server-side async wrapper. Reuse:
  `RunHandle`, `RunStream`, `Event`, `Session`, `IdempotencyToken`, `CancelToken`.

### Console (consumer) - reinforce, don't own

- **C1 - Wire the agent surface to a real turn.** `lib/seams/agents/acp-registry.ts`
  + point `runAgent` at M5. The Thread/Composer/RunTrace/HeadPanel already exist.
- **C2 - One voice + reinforcement.** `HeadAttributionDrawer` (operator unmask) +
  announce-primary overlap guard; surface the constitution authority order and each
  hook's allow/deny/ask decision (reinforce, not own).
- **C3 - Embedded TUI.** A terminal surface in the console (xterm.js) driving a head
  turn over the same ACP seam. Sibling of the existing Yrs/CodeMirror editor.
- **C4 - Collaborative surface with live presence (Velt - IN SCOPE this build).**
  Velt was deferred by a prior session; it ships here. The multi-agent document
  surface (console spec section 4) with **Velt for cursors + presence ONLY**, Yrs
  for free text, RustyRed graph CRDT for structure - the copresence boundary already
  set. Each head and the user render as a labeled live cursor (spec section 3: one
  cursor per agent in a document; one voice in chat). Reuse: existing
  `CollaborativeEditor.tsx` / `LiveCursors.tsx` / `collab.ts` (Velt presence partly
  wired) + the `theorem-copresence` crate (structure CRDT + Yrs text regions).

The remaining console spec sections (7/8/9/10/11/12: footprint panel, scheduled
tasks, remote, history, native mount, plugin SDK) are the console's own roadmap -
tracked there, not re-planned here.

## 6. Tracer-bullet first slice

Thin end-to-end path before the spine thickens it: **M1 (roster as config) + M5
(turn seam) + C1 (wire console)** => a user picks GLM 5.2 in the console, types a
technical task, a real head turn streams back. Then M2/M3/M4 make every such turn
governed, remembering, and hooked - each is additive over a working path.

## 7. Sequencing

```
M1 roster ---> M5 turn seam ---> C1 console wire   [TRACER: pick head, run a task]
                                       |
M2 constitution --+                    |
M3 passive memory --+--> wraps every turn (additive)
M4 tool hooks    --+
                                       v
                                  C2 one-voice + reinforce, C3 TUI
```

Artifacts to fold in late (low priority): mermaid-rs renderer (scene-os),
agent-grep (already covered by `compute_code`). Local sandbox backend
(bwrap/Seatbelt) is a named follow-up - today's sandbox is cloud-only OpenSandbox.

## 8. Open decisions (announce-and-proceed unless flagged)

- Project-law file format/location (`.theorem/constitution.json` proposed).
- Whether tool hooks live in agentd only first (api heads) then receiver (spawn),
  or both at once. Lean: agentd first (the tracer head path), receiver next.
- AgentEvent union exact variants (start from the 6 above).
