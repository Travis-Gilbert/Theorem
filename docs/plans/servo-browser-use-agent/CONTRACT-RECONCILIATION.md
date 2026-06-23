# Contract reconciliation: theorem-browser-agent vs Theseus apps/notebook

Status: resolution **B** (adapted contract, documented divergence). Authored by claude-code as the Mirror-Rule reconciliation for Codex's `theorem-browser-agent` implementation.

## Why this file exists

Acceptance criterion #1 of `HANDOFF.md` reads: "The four contracts exist as Rust types and round-trip the same JSON the Python emits." The shipped `theorem-browser-agent` crate does **not** round-trip the Python's JSON: it is a clean, browser-shaped **reimplementation** of the perceive/govern/afford shape, not a field-for-field port of `apps/notebook/{context_command,perception,action_rail}/contracts.py`.

That is a deliberate, reasonable choice (the browser agent does not need the full Theseus client-context surface), but the Mirror Rule (CLAUDE.md: "Theseus is canonical ... contract-affecting changes must stay reconciled with Theseus") forbids letting the two diverge *silently*. This file is the explicit reconciliation so the divergence is surfaced, not buried.

The handoff is itself in tension here: its own prose paraphrases the contracts (it writes `TracePolicy (graph_trace, receipts, context_preview)` and "collapse GraphLayer, drop falkor_hot"), which already departs from the Python it cites. Codex implemented the handoff's paraphrase faithfully; the Python is the thing that diverges from both.

## Where the contracts match Theseus (no divergence)

These were ported faithfully (same variants / field names):

- `PerceptionMode` (ask/browse/capture/compare/verify/monitor/act)
- `CandidateStatus` (known/local/external_unfetched/fetched_unadmitted/admitted/rejected)
- `ExecutionRoute` (ask_pipeline/capture_api/web_api/context_command_api/monitor_api/writeback_api/frontend_only/not_implemented)
- `PermissionPolicy` (all 9 fields, same names)

## Where the contracts diverge (intentional, browser-shaped)

| Contract | theorem-browser-agent | Theseus apps/notebook | Divergence rationale |
|---|---|---|---|
| `ContextCommandState` | `command_id, raw_request, surface, permission_policy, retrieval_policy, risk_mode, output_target, graph_layers, tool_scope, trace_policy` | adds `goal, query, user_id, session_id, folio_id, notebook_id, project_id, current_page, selected_text, working_set, exclusions, hot_context, canonical_context, memory_scope, warnings, metadata`; has no `raw_request`/`surface` | browser agent resolves a single raw request + surface; it does not carry the full Theseus notebook client-context. |
| `OutputTarget` | answer/report/browser_chrome/harness_receipt | answer/artifact/action_plan/citation_map/code_patch/task/report/graph_trace | browser surfaces (chrome, harness receipt) replace notebook output targets. |
| `GraphLayer` | 4 (memgraph_canonical/rustyred_hot/redis_hot/local_webdocs) | 6 (adds thg_hot, falkor_hot) | handoff-directed collapse; falkor retired, thg folded onto rustyred_hot. |
| `TracePolicy` | graph_trace/receipts/context_preview (3) | include_graph_trace/.../include_permission_explanations (5, `include_` prefix) | handoff's short names; two notebook-only trace toggles dropped. |
| `RetrievalPolicy` | mode + 4 include_* / freshness | adds max_known_objects, max_hot_context, max_web_candidates, include_browser_hot_graph | budgets live on the engine/job, not the policy. |
| `ActionCategory` | read/navigate/extract/compare/writeback/monitor/permission (7) | understand/compare/capture/verify/transform/act/monitor/learn/protect (9) | browser dispatch axis, not the notebook editorial taxonomy. |
| `ActionRisk` | read_only/external_web/hot_graph_write/canonical_write/remember/state_changing (6, semantic) | low/medium/high (3, severity) | risk is the gating axis the rail dispatches on, not a severity label. |
| `ActionStatus` | ready/needs_confirmation/blocked_policy/not_implemented | available/disabled/requires_confirmation/requires_permission | maps 1:1 in spirit; names chosen for the gate outcomes. |
| `ActionType` | 21 (the handoff list) | 25 (adds capture_selection, create_task_list, generate_citation_map, monitor_topic) | the four extra notebook actions are not yet surfaced; add when needed. |
| `CandidateKind` | 10 | 11 (has `monitor`) | `monitor` kind dropped; restore if the monitor surface needs it. |
| `ContextRef` | (absent) | central typed pointer | perception uses `PerceptionCandidate`; no separate `ContextRef`. |

## Resolution

- **Criterion #1 is reworded** for the browser agent: the four contracts exist as Rust types and round-trip **their own canonical JSON** (proven by `theorem-browser-agent`'s round-trip tests and the server/MCP surface). They are **reconciled-not-identical** to the Theseus contracts; the divergences above are intentional and enumerated here.
- The Theseus Python contracts remain canonical for the **notebook** runtime. The browser agent is a sibling projection, not a mirror, and is allowed to carry a leaner surface.
- If strict Theseus parity is later required (option A), the path is: keep every variant in the enums (so Python JSON deserializes), restore the dropped fields with defaults, and rename `TracePolicy` fields to the `include_*` form. That is a larger follow-up and is **not** done here.

## Parity fixtures

`parity-fixtures/context_command_state.json` is a golden JSON emitted by the Theseus Python `context_command` for reference. It documents the canonical Theseus shape the browser agent intentionally departs from (and is the test target if option A is ever chosen).
