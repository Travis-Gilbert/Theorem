# Theorem Coding Harness Implementation Plan

> **For Claude:** REQUIRED: Use /execute-plan to implement this plan task-by-task.

**Goal:** Build the governance spine + multi-head roster that turns the existing harness crates into one coding interface, driven first by the harness-console.

**Architecture:** A `HeadRegistry` in `theorem-harness-core` selects among heads that all `impl HeadInvoker`; an `ApiHeadInvoker` (agentd) and `SpawnHeadInvoker` (receiver) supply the two transports. The SDK's `turn()` drives any `&dyn HeadInvoker` over `RunHandle`, streaming a typed `AgentEvent`. A layered `Constitution`, a passive-memory layer, and allow/deny/ask tool hooks wrap every turn in harness code; the console reinforces (does not own) governance.

**Tech Stack:** Rust (no root workspace - `cd rustyredcore_THG && cargo test -p <crate>`); Next.js 16 / React 19 console (`apps/harness-console`); RustyRed substrate; Velt (presence only) + Yrs (text) + graph CRDT (structure).

**Granularity note:** Phase 0 (tracer) is bite-sized and built now. Phases 1-2 are task-level with acceptance criteria + reuse surfaces; expand each to bite-sized via /write-plan when reached (plans lag code - do not over-spec what rots).

**Conventions:** Commit with scoped pathspec (`git commit -- <paths>`), conventional `type(scope): desc`, no Co-Authored-By. Codex is often live in this repo - claim a seam, commit only your paths.

---

## PHASE 0 - Tracer bullet (M1 api path + M5 + C1)

End state: in the console, pick a configured head (Mistral / DeepSeek / agentd-local
Gemma - GLM 5.2 has no key yet), type a technical task, a real head turn streams back.

### Task 1: HeadSpec + HeadRegistry (core)

**Files:**
- Create: `rustyredcore_THG/crates/theorem-harness-core/src/head_registry.rs`
- Modify: `rustyredcore_THG/crates/theorem-harness-core/src/lib.rs` (add `pub mod head_registry;`)
- Reuse: `head_invocation.rs::HeadInvoker` (existing trait), `ResolvedHead` types
- Test: same file `#[cfg(test)]`

**Step 1 - Failing test:** roster parses; registry lists + gets by id.
```rust
#[test]
fn roster_lists_api_and_spawn_heads() {
    let toml = r#"
        [[head]]
        id = "mistral"
        kind = "api"
        base_url = "https://api.mistral.ai/v1"
        model = "mistral-small-latest"
        api_key_env = "MISTRAL_API_KEY"

        [[head]]
        id = "codex"
        kind = "spawn"
        program = "codex"
    "#;
    let reg = HeadRegistry::from_toml_str(toml).expect("parse");
    assert_eq!(reg.list().len(), 2);
    let mistral = reg.get("mistral").expect("mistral present");
    assert_eq!(mistral.kind, HeadKind::Api);
    assert_eq!(mistral.base_url.as_deref(), Some("https://api.mistral.ai/v1"));
    assert_eq!(reg.get("codex").unwrap().kind, HeadKind::Spawn);
}
```

**Step 2 - Run, expect fail:** `cd rustyredcore_THG && cargo test -p theorem-harness-core head_registry`

**Step 3 - Implement:** `HeadKind { Api, Spawn }`, `HeadSpec { id, kind, base_url, model, api_key_env, program, args }` (serde, all-optional except id/kind), `HeadRegistry(BTreeMap<String, HeadSpec>)` with `from_toml_str`, `list`, `get`. No transport logic yet - pure config.

**Step 4 - Run, expect pass.**

**Step 5 - Commit:** `git commit -- rustyredcore_THG/crates/theorem-harness-core/src/head_registry.rs rustyredcore_THG/crates/theorem-harness-core/src/lib.rs -m "feat(harness-core): head roster registry (HeadSpec/HeadKind/HeadRegistry)"`

### Task 2: ApiHeadInvoker (agentd impls core::HeadInvoker)

**Files:**
- Create: `rustyredcore_THG/crates/theorem-agentd/src/api_head.rs`
- Modify: `theorem-agentd/src/lib.rs` (`pub mod api_head;`)
- Reuse: `model.rs::{ModelClient, OpenAiModelClient, from_config}`, `turn_loop.rs::run_once`, `config.rs::ModelConfig`; `theorem-harness-core::HeadInvoker`
- Test: same file

**Step 1 - Failing test:** an `ApiHeadInvoker` built from a `HeadSpec` implements `HeadInvoker::invoke` and returns a receipt for a stubbed model (use the existing `RuleModelClient` path so no network).
```rust
#[test]
fn api_head_invoker_runs_a_turn_with_rule_client() {
    let spec = HeadSpec::api("local-rule"); // helper -> ModelProvider::Rule
    let invoker = ApiHeadInvoker::from_spec(&spec).expect("build");
    let req = HeadInvocationRequest::task("write a hello world");
    let receipt = invoker.invoke(req).expect("invoke");
    assert!(!receipt.output_summary.is_empty());
}
```

**Step 2 - Run, expect fail.**

**Step 3 - Implement:** `ApiHeadInvoker` holds a `ModelClient` (built via `from_config` from the spec) + a `ToolCatalog` + `McpRouter`. `impl HeadInvoker` calls `run_once`, maps its result into a `HeadInvocationReceipt`. Add `HeadSpec::api`/`HeadSpec::to_model_config` helpers in core (or a small adapter in agentd).

**Step 4 - Run, expect pass.** `cargo test -p theorem-agentd api_head`

**Step 5 - Commit** (paths: api_head.rs, lib.rs).

### Task 3: AgentEvent + turn() seam (SDK)

**Files:**
- Create: `rustyredcore_THG/crates/theorem-harness/src/agent_event.rs`, `rustyredcore_THG/crates/theorem-harness/src/turn.rs`
- Modify: `theorem-harness/src/lib.rs`
- Reuse: `run.rs::RunHandle`, `stream.rs::RunStream`, `event.rs::Event`, `idempotency.rs`, `cancel.rs`, `theorem-harness-core::HeadInvoker`
- Test: same files

**Step 1 - Failing test (mapping):** `Event -> AgentEvent`.
```rust
#[test]
fn event_text_maps_to_agentevent_text() {
    let ev = Event::fake_text("hello"); // test helper over EventState
    assert!(matches!(AgentEvent::from_event(&ev), AgentEvent::Text { content } if content == "hello"));
}
```

**Step 2 - Failing test (turn):** driving a `FakeHeadInvoker` through `turn()` yields Text then Complete.
```rust
#[test]
fn turn_streams_text_then_complete() {
    let mut store = InMemoryGraphStore::new();
    let head = FakeHeadInvoker::returning("done");
    let events = run_turn_collect(&mut store, &head, TurnRequest::task("x"));
    assert!(matches!(events.last().unwrap(), AgentEvent::Complete { .. }));
}
```

**Step 3 - Implement:** `AgentEvent { Text, ToolCall, StateChange, Error, MemoryOp, Complete }` + `from_event`. `turn(store, head: &dyn HeadInvoker, req) -> RunHandle`: `RunHandle::start` (idempotency from req id), invoke the head, `append` events, expose via `RunStream`. Synchronous core; the server wraps async.

**Step 4 - Run, expect pass.** `cargo test -p theorem-harness agent_event turn`

**Step 5 - Commit.**

### Task 4: Turn transport route (MCP/HTTP)

**Files:**
- Modify: `rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs` (add a `head_turn` tool/route) OR the harness HTTP server `apps/theorem-harness-server`
- Reuse: `handle_mcp_request`, `RunStream::events_since` for poll-streaming
- Test: a request JSON in, AgentEvent deltas out

**Acceptance:** a `head_turn { head_id, task, idempotency }` call starts a run and returns AgentEvent deltas readable by cursor (reuse the stream_read poll pattern). Keep read-only-mode rules.

**Commit** when green.

### Task 5: Console ACP seam + wire agent surface (C1)

**Files:**
- Create: `apps/harness-console/src/lib/seams/agents/acp-registry.ts`
- Modify: `apps/harness-console/src/lib/harness/{client.ts,mcp.ts,mock.ts}`, `app/(console)/agent/page.tsx`, `components/agent/{Composer,HeadPanel}.tsx`
- Reuse: `HarnessClient`, `mcp.ts::callTool/graphql`, `useHarnessStream.ts`, fixtures (`BINDING` already has heads)

**Steps:**
1. `acp-registry.ts`: `listAgents()` (roster), `connectAgent(id)`, `sendTurn(agentId, turn)` returning an `AgentEvent` async-iterable mirroring the Rust enum. Mock impl first (deterministic), then live impl calling Task 4's route via `mcp.ts`.
2. Head selector in `HeadPanel`/`Composer` populated from `listAgents()`; GLM 5.2 selectable.
3. `runAgent` -> `sendTurn`; `Thread` renders streamed `AgentEvent`s (Text appends, ToolCall folds into trace).
4. Verify in browser via preview tools (start, pick glm-5.2, send a task, see streamed turn). Screenshot proof.

**Commit** (scoped to console paths).

**PHASE 0 DONE = tracer green:** pick a configured head (Mistral/DeepSeek/agentd), type a task, a real turn streams back.

---

## PHASE 1 - The spine (wraps every turn, additive)

Each task: failing test -> implement -> green -> commit. Expand to bite-sized when reached.

### M1b: SpawnHeadInvoker + roster spawn path
- **Files:** `theorem-receiver/src/spawn_head.rs` (impl `core::HeadInvoker` over `HeadAdapter`); wire dormant `HeadRuntimeRecipe` in `receiver.rs::run_job`.
- **Acceptance:** `reg.get("codex")` resolves to a `SpawnHeadInvoker`; a turn via Codex spawns the CLI (sub-bridge auth preserved) and returns a receipt. Runtime-recipe heads (aider/openhands) selectable by config.
- **Reuse:** `head.rs::{HeadAdapter, adapter_for}`, `spawn.rs`, `config.rs::HeadRuntimeRecipe`.

### M2: Layered Constitution (promote + layer)
- **Files:** `theorem-harness-core/src/constitution.rs`; modify `turn_loop.rs` (agentd) + `turn.rs` (SDK) to enforce per turn; modify `types.rs::{GuardViolation (+policy_layer), TransitionResult (+policy_decision)}`.
- **Acceptance criteria:**
  1. `Constitution` holds an ordered ladder: global law (lifted from `alignment.rs::evaluate_publication` invariants + `agent_binding.rs::ActionTierPolicy` + agentd `charter()`), project law (`.theorem/constitution.json`: protected_invariants/branch_policy/verification_policy/escalate_when), current request, live evidence.
  2. Enforced once per turn for **every head** (api + spawn), not just composed-agent publication.
  3. Lower layers yield to higher; the epistemic floor (every claim carries provenance; "publication only HARDER") is unconditional.
  4. **Drift test** asserts the authority order cannot be reordered (compile-time order constant + a test that a reordered config is rejected).
  5. Each turn carries a `policy_decision` (which layer permitted/denied) on the persisted event.
- **Reuse:** `alignment.rs::evaluate_publication`, `agent_binding.rs::ActionTierPolicy`, `state_machine.rs` guard pattern.

### M3: Passive memory layer
- **Files:** `rustyred-thg-memory/src/passive.rs` (new); modify agentd `turn_loop.rs` startup-recall call site.
- **Acceptance criteria:**
  1. Relevant memories auto-injected into turn context with **no tool call surfaced** to the head.
  2. **Relevance gate** on `RankedMemory.score` (floor) before injection - low-signal memories dropped (no token waste).
  3. Turn-context seeding (not raw prompt) via `recall_seeds`.
  4. Ambient `consolidate()` on a cadence (idempotent per key).
  5. Epistemic floor: gate on `read_epistemic_shadow` source_kind/warrant; provenance surfaced.
- **Reuse:** `recall`, `consolidate`, `read_epistemic_shadow`, `RankedMemory`, `recall_seeds`, `observe_payload`.

### M4: Tool-lifecycle hooks (allow/deny/ask)
- **Files:** `theorem-harness-core/src/tool_hooks.rs` (new); modify agentd `turn_loop.rs` dispatch (api heads) + receiver `run_job` (spawn heads).
- **Acceptance criteria:**
  1. A `PreToolUse` gate runs between model decision and `call_tool`, returning allow / deny / ask.
  2. Decisions are constitution-driven (M2) and audited (a receipt names the rule).
  3. `PostToolUse` hook can annotate/validate the result.
  4. Distinct from `rustyred-thg-core/src/hooks.rs` (post-commit graph mutation) - this gates execution.
- **Reuse:** `GuardViolation`, `state_machine.rs::validate_toolkit_payload` pattern.

---

## PHASE 2 - Console reinforcement

### C2: One voice + governance reinforcement
- **Files:** `apps/harness-console/src/components/agent/HeadAttributionDrawer.tsx`; modify `Thread.tsx`; `lib/seams/agents/coordination-room.ts`.
- **Acceptance:** end user sees one voice across a multi-head turn; operator toggle reveals per-head attribution + tool calls + cost from the replay ledger; announce-primary overlap guard resolves divergence pre-render; the constitution authority order + each hook's allow/deny/ask decision are surfaced (reinforce, not own).
- **Reuse:** `Thread.tsx`, `RunTrace.tsx`, the replay ledger via `getRun`.

### C3: Embedded TUI
- **Files:** `apps/harness-console/src/app/(console)/terminal/` + an xterm.js component.
- **Acceptance:** a terminal surface drives a head turn over the same ACP seam (Phase 0 Task 5); sibling of the Yrs/CodeMirror editor; full-bleed registered in `Shell`.
- **Reuse:** `acp-registry.ts::sendTurn`, `Shell` full-bleed set, design tokens.

### C4: Collaborative surface with Velt (Velt ships now)
- **Files:** `apps/harness-console/src/components/collab/DocumentCanvas.tsx`, `lib/collab/{presence-velt.ts,text-yrs.ts,structure-crdt.ts}`.
- **Acceptance:** human + >=2 agents edit one document concurrently; **Velt for cursors + presence ONLY**, Yrs for free text, RustyRed graph CRDT for structure; each head + user a labeled live cursor (one cursor per agent in a doc; one voice in chat); no lost writes under concurrent edits.
- **Reuse:** existing `CollaborativeEditor.tsx`/`LiveCursors.tsx`/`collab.ts` (Velt partly wired), `theorem-copresence` crate (structure CRDT + Yrs text regions).

---

## Out of scope (console's own roadmap)
Console spec sections 7 (footprint panel beyond C2), 8 (scheduled tasks), 9 (remote), 10 (history timeline), 11 (workspace watch/native mount), 12 (plugin SDK). Tracked in the console plan, not here.

## Named follow-ups
Local sandbox backend (bwrap/Seatbelt) - today's `sandbox_exec.rs` is cloud-only OpenSandbox. mermaid-rs renderer (scene-os). agent-grep already covered by `compute_code`.
