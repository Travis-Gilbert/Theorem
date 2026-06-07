# Theorems Harness ChatGPT App handoff

Status: started 2026-06-07. First compatibility slice landed locally in
`rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs`: all advertised MCP tools now expose
`outputSchema`, and ChatGPT-flagged browser/code tools expose concrete receipt-oriented schemas.

## Goal

Turn Theorems Harness into a ChatGPT app by keeping the existing remote MCP endpoint and adding a
small ChatGPT-facing facade over the native substrate. ChatGPT should compile, recall, audit, render,
and propose memory updates without seeing raw Redis, raw graph internals, or direct canonical memory
mutation.

## Current finding

The screenshot warning is not a transport failure. ChatGPT Developer Mode scanned the MCP actions,
but tools such as `browse_for_me`, `browse_with_me`, and `code_search` were missing `outputSchema`.
OpenAI's Apps SDK docs describe `outputSchema` as the schema for returned `structuredContent`, and
separate `structuredContent`, `content`, and `_meta` for model-visible JSON, narration, and
widget-only detail.

The current native MCP still advertises many substrate tools. That is useful for Codex/Claude-style
power use, but the ChatGPT app should expose a smaller app surface.

## App archetype

Primary archetype: `interactive-decoupled`.

Reason: The first version can work as tool-only MCP, but the target experience wants a Context
Cockpit widget that renders context artifacts, token ledger, provenance, and proposed memory patches.
Data tools should return compact `structuredContent`; the widget can receive larger detail through
`_meta` later.

## Tool surface

Use these high-level app tools first:

| Tool | Read/write | Purpose |
|------|------------|---------|
| `theorem.prepare_context` | read-like, creates artifact | Compile a task-scoped context artifact. |
| `theorem.recall_memory` | read | Search saved contexts, postmortems, handoffs, and memory docs. |
| `theorem.hydrate_memory` | read | Hydrate selected handles after recall or prepare. |
| `theorem.search_artifacts` | read | Find prior ContextArtifacts and receipts. |
| `theorem.propose_memory_patch` | write-gated | Create a MemoryPatch proposal, not a canonical write. |
| `theorem.record_outcome` | write-gated | Record a run outcome or use receipt. |
| `theorem.render_context` | app/UI | Attach the Context Cockpit widget for an artifact. |

Do not expose raw graph/database tools as the default ChatGPT app actions.

## Implementation checklist

- [x] Add `outputSchema` to native MCP tool descriptors so ChatGPT Developer Mode no longer flags
  missing output schema on scan.
- [x] Add regression coverage that every listed tool has an output schema in read-only and write
  mode.
- [x] Add targeted coverage for `browse_for_me`, `browse_with_me`, and `code_search`.
- [ ] Add an app-profile filter or registration path that exposes only the ChatGPT facade tools.
- [ ] Implement `theorem.prepare_context` over the existing context compiler/artifact path.
- [ ] Implement `theorem.recall_memory` and `theorem.hydrate_memory` over native memory recall.
- [ ] Implement `theorem.propose_memory_patch` as proposal-only; do not direct-write canon.
- [ ] Add OAuth or bearer auth with tenant/user scopes: `context:read`, `context:compile`,
  `context:write_patch`, `context:export`.
- [ ] Register `ui://theorem/context-cockpit-v1.html` as an Apps SDK resource with
  `text/html;profile=mcp-app`.
- [ ] Return compact `structuredContent`, human `content`, and widget-only `_meta` for app tools.
- [ ] Re-scan in ChatGPT Developer Mode and verify the draft app actions.

## Acceptance criteria

- ChatGPT can scan the remote MCP endpoint without output-schema warnings for app-facing tools.
- The app-facing action list is small enough that the model naturally chooses context/memory tools
  instead of low-level graph verbs.
- Read tools are marked read-only. Write-gated tools are not marked read-only and create proposals or
  receipts, not hidden destructive mutations.
- A context artifact response includes artifact id, title, task type, capsule preview, token ledger,
  included/excluded counts, risks, actions, and widget resource URI.
- Large atom lists, graph paths, and provenance details are not placed in model-visible
  `structuredContent` by default.

## Validation receipts

- `cargo test -p rustyred-thg-mcp chatgpt_flagged_tools_advertise_receipt_output_schemas`: passed.
- `cargo test -p rustyred-thg-mcp tools_list_exposes_read_only_graph_tools`: passed.
- `cargo test -p rustyred-thg-mcp tools_list_exposes_native_coordination_write_tools_when_enabled`:
  passed.
- `cargo test -p rustyred-thg-mcp`: passed, 42 tests.
- `git diff --check`: passed.

## Source docs checked

- https://developers.openai.com/apps-sdk/build/mcp-server
- https://developers.openai.com/apps-sdk/reference
- https://developers.openai.com/apps-sdk/plan/tools
- https://help.openai.com/en/articles/12584461-developer-mode-apps-and-full-mcp-connectors-in-chatgpt-beta
