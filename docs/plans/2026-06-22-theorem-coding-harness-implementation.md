# Theorem Coding Harness Implementation Plan

> **For Claude:** REQUIRED: Use /execute-plan to implement this plan task-by-task.

**Goal:** Build the governance spine + multi-head roster that turns the existing harness crates into one coding interface, driven first by the harness-console.

**Architecture:** Heads resolve through `AgentBinding` + `AgentHeadRegistry` and execute through `RealHeadInvoker`/`ProviderHeadInvoker` (one trait, every provider). The SDK persists each turn as a run. A layered `Constitution`, a passive-memory layer, and allow/deny/ask tool hooks wrap every turn in harness code; the console reinforces (does not own) governance.

**Tech Stack:** Rust (no root workspace - `cd rustyredcore_THG && cargo test -p <crate>`); Next.js 16 / React 19 console (`apps/harness-console`); RustyRed substrate; Velt (presence only) + Yrs (text) + graph CRDT (structure).

**Conventions:** Commit with scoped pathspec (`git commit -m "..." -- <paths>`), conventional `type(scope): desc`, no Co-Authored-By. Codex is often live - claim a seam, commit only your paths.

---

## PHASE 0 - Tracer: ALREADY BUILT (verify + configure, do not rebuild)

Grounding (2026-06-22) found the entire api-head tracer already shipped. Verified
by reading the code + the offline `head_invoker` tests passing. **Do not build a
new HeadRegistry/HeadKind/ApiHeadInvoker - they exist.**

What exists and where:
- **Roster** = `theorem-harness-core::{AgentBinding, AgentHeadRegistry, AgentHead}`
  (heads carry provider/model/`HeadTransport`/credential_ref). Adding a head is data.
- **Real invoker** = `theorem-harness-runtime::head_invoker` -
  `RealHeadInvoker`/`ProviderHeadInvoker` with live HTTP profiles for anthropic,
  deepseek, mistral, minimax, zhipu/glm, openai, ai21, gemma (`head_invoker/api.rs`),
  `CredentialResolver` (`env:NAME`/`secret:NAME`), `EndpointMap::from_env`.
- **Turn** = `composed_agent.rs::run_composed_agent` -> `run_intra_agent_loop_with_invoker`;
  `default_theorem_binding` builds the binding from `THEOREM_AGENT_HEADS` env.
- **Transport** = `composed_agent_run` MCP tool + HTTP route (`rustyred-thg-server/src/router.rs:870`), read-only-gated.
- **Console** = `apps/harness-console/src/lib/harness/mcp.ts:62` `runAgent` already
  calls `composed_agent_run` live (mock fallback in `mock.ts`).

### V0.1 - Prove a real turn (config only, needs one key)

```
THEOREM_LIVE_PROVIDER_TEST=1 THEOREM_AGENT_HEADS=mistral MISTRAL_API_KEY=... \
  cargo test -p theorem-harness-runtime --test live_single_head_smoke -- --ignored --nocapture
```
(DeepSeek: `THEOREM_AGENT_HEADS=deepseek DEEPSEEK_API_KEY=...`. Two heads
`mistral,deepseek` also lets the >=2-head consensus gate publish. Local Gemma:
`THEOREM_AGENT_HEADS=gemma THEOREM_AGENT_HEAD_GEMMA_TRANSPORT=local THEOREM_LOCAL_OPENAI_URL=...`.)
Smoke committed: `tests/live_single_head_smoke.rs` (self-skips without the env).

### V0.2 - Console live wiring (genuine remaining work)
- Point the console at a running harness server: `NEXT_PUBLIC_HARNESS_SOURCE=live`
  + the MCP/HTTP base URL; server env carries `THEOREM_AGENT_HEADS` + the key(s).
- Head selector in `Composer`/`HeadPanel` (binding already lists heads).
- Verify the agent surface renders a real turn end to end; screenshot.
- Acceptance: pick a configured head in the console, type a task, a real turn returns.

### V0.3 - Streaming (DEFERRED, not needed for the tracer)
`composed_agent_run` returns a final result, not a token stream. A typed
`AgentEvent` stream (Text/ToolCall/StateChange/Error/Complete) over the SDK
`RunStream` is a real enhancement - build only when the UX needs live per-head
streaming. The console renders the final reconciled message today.

---

## PHASE 1 - The spine (the real engine build; wraps every turn, additive)

Each task: failing test -> implement -> green -> commit. Expand to bite-sized when reached.
NOTE the constitution is NOT greenfield (see design doc): `alignment.rs` already
enforces the epistemic floor and `agent_binding.rs::ActionTierPolicy` has 3 tiers.

### M1b: Spawn heads in the roster (Codex/Claude CLIs)
- **Files:** a `SpawnHeadInvoker` (impl `core::HeadInvoker`) over `theorem-receiver::HeadAdapter`; wire the dormant `HeadRuntimeRecipe` in `receiver.rs::run_job`.
- **Acceptance:** a binding head with transport that resolves to a spawn lands a Codex/Claude CLI turn (sub-bridge auth preserved) and returns a receipt, behind the same `HeadInvoker` the api path uses.

### M2: Layered Constitution (promote + layer)
- **Files:** `theorem-harness-core/src/constitution.rs`; enforce per-turn in `run_intra_agent_loop_with_invoker`; extend `types.rs::{GuardViolation (+policy_layer), TransitionResult (+policy_decision)}`.
- **Acceptance:** ordered ladder (global law from `alignment.rs` invariants + `ActionTierPolicy` + charter -> project law `.theorem/constitution.json` -> request -> live evidence); enforced for **every head every turn**; epistemic floor unconditional; **drift test** the order can't be reordered; each turn carries a `policy_decision`.
- **Reuse:** `alignment.rs::evaluate_publication`, `agent_binding.rs::ActionTierPolicy`.

### M3: Passive memory layer
- **Files:** `rustyred-thg-memory/src/passive.rs`; call site in the turn loop's context assembly.
- **Acceptance:** relevant memories auto-injected with **no tool call**; relevance gate on `RankedMemory.score`; turn-context seeding via `recall_seeds`; ambient `consolidate()` on a cadence; epistemic floor via `read_epistemic_shadow`.
- **Reuse:** `recall`, `consolidate`, `read_epistemic_shadow`.

### M4: Tool-lifecycle hooks (allow/deny/ask)
- **Files:** `theorem-harness-core/src/tool_hooks.rs`; gate point in the invoker's tool dispatch (api + spawn).
- **Acceptance:** a `PreToolUse` allow/deny/ask gate between decision and execution; constitution-driven (M2) + audited; `PostToolUse` annotate/validate. Distinct from `rustyred-thg-core/src/hooks.rs` (post-commit graph mutation).
- **Reuse:** `GuardViolation`, `state_machine.rs::validate_toolkit_payload` pattern.

---

## PHASE 2 - Console reinforcement

### C2: One voice + governance reinforcement
- `HeadAttributionDrawer.tsx` + operator unmask; announce-primary overlap guard; surface the constitution authority order + each hook's allow/deny/ask decision. Reuse `Thread.tsx`, `RunTrace.tsx`, the replay ledger.

### C3: Embedded TUI
- `app/(console)/terminal/` + xterm.js driving a head turn over the same path as `runAgent`; full-bleed in `Shell`.

### C4: Collaborative surface with Velt (Velt ships now)
- `components/collab/DocumentCanvas.tsx`, `lib/collab/{presence-velt,text-yrs,structure-crdt}.ts`. Velt for cursors + presence ONLY, Yrs text, RustyRed graph CRDT structure; each head + user a labeled cursor; no lost writes. Reuse existing `CollaborativeEditor.tsx`/`LiveCursors.tsx`/`collab.ts` + `theorem-copresence`.

---

## Out of scope (console's own roadmap)
Console spec sections 7/8/9/10/11/12 (footprint beyond C2, scheduled tasks, remote, history, native mount, plugin SDK). Tracked in the console plan.

## Named follow-ups
V0.3 AgentEvent streaming. Local sandbox backend (bwrap/Seatbelt) - today's `sandbox_exec.rs` is cloud-only OpenSandbox. mermaid-rs (scene-os). agent-grep already covered by `compute_code`.
