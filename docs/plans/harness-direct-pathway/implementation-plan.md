# Harness Direct Pathway: room-integrated spawn (implementation plan)

Status: implemented on branch harness-direct-pathway, 2026-06-03. Source spec:
`harness-direct-pathway-spec.md`. Executor: Claude Code, theorem repo, by hand (the bootstrap build
that makes hands-free spawn work).

## Architecture decision

The spawn verb lives in the sync, HTTP-free MCP crate (`rustyred-thg-mcp`). HTTP cannot run there, so
the GitHub dispatch is injected through the existing `McpGraphBackend` DI seam: a new
`dispatch_handoff` trait method defaults to an "unsupported" error (matching
`append_harness_transition` / `harness_run_detail`), and the concrete server backend
(`rustyred-thg-server` `ProductMcpBackend`, which depends on reqwest) overrides it with the real
`repository_dispatch` POST. This keeps the verb unit-testable with a mock backend and keeps reqwest
out of the MCP crate. `handle_mcp_request` is sync but called from async Axum handlers, so the server
impl fires reqwest on a dedicated thread with its own current-thread runtime (no nested runtime).

The CoordinationRecord is reused as-is: `record_type = "event"`, `summary = intent`, and
`metadata = { dispatch_id, owner, repo, branch, surface: "spawned", kind: "spawn", status:
"running", executor: "github_actions" }`. No new node type or edge; `write_coordination_record` +
`COORDINATION_RECORD_OF` are the existing machinery. The dispatch correlation id is ours (GitHub's
`repository_dispatch` returns 204, no id); the follow-up webhook maps PR -> dispatch_id.

Config: `THEOREM_HANDOFF_GITHUB_TOKEN` (fallback `GITHUB_TOKEN`) for the dispatch POST; owner/repo
default `Travis-Gilbert/theorem`, overridable per call. Token absent => the verb still writes the
room-visible record but marks status `dispatch_failed` and returns the reason (honest, not silent).

## Checklist (every item backreferences a spec section)

- [x] BE-1 (spec "The design" first; "Ready intent" (1)): `spawn_session` writes a room-visible
  CoordinationRecord (dispatch id, intent summary, repo, branch, surface "spawned", status "running")
  via `write_coordination_record`, reusing CoordinationRecord + COORDINATION_RECORD_OF.
- [x] BE-2 (spec "The design" second; "Ready intent" (2)): verb fires `repository_dispatch`
  (event_type `theorem-handoff`, client_payload `{ intent, branch }`) via `backend.dispatch_handoff`,
  NOT the Railway runner. Default-unsupported in the MCP crate; reqwest POST in `rustyred-thg-server`.
- [x] BE-3 (spec "The design" third; "Ready intent" (3)): `update_spawn_record_status(dispatch_id,
  status, pr_url)` implemented as the PR-opened webhook seam. The webhook HTTP route is the follow-up.
- [x] BE-4 (spec "Non-goals"; "Sequencing"): Railway runner path left intact; untouched.
- [x] BE-5 (spec "Ready intent"): tool registration with a flat, OpenAI-safe schema (no top-level
  anyOf) + dispatch match arm + read-only / coordination-policy gating like other write verbs.
- [x] T-1 (spec "Ready intent"): unit test that the spawn verb writes a room-retrievable
  CoordinationRecord (mock backend whose `dispatch_handoff` records the call). Green.
- [x] SH-1 (spec "Ready intent"): branch harness-direct-pathway, PR opened. Do NOT merge.

## Out of scope (named, not silently cut)

- The webhook HTTP route on the harness service (spec "Sequencing" second): only the update function
  ships now; the route is the explicit follow-up the spec authorizes.

## Validation

- `cargo test -p rustyred-thg-mcp` : spawn_session_writes_room_visible_coordination_record green;
  the schema guard (assert_no_top_level_schema_combinators) accepts the new flat spawn_session schema.
- `cargo check -p rustyred-thg-server` : the reqwest dispatch_handoff impl typechecks.
