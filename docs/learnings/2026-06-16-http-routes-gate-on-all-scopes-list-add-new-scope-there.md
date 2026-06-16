# A `rustyred-thg-server` HTTP route gating on a scope that is NOT in `auth.rs::ALL_SCOPES` returns 403 even with `require_auth=false`: dev mode grants ALL_SCOPES, but only the scopes literally enumerated there

**Kind:** gotcha
**Captured:** 2026-06-16
**Session signature:** `claude:travisgilbert (agent-space-viewport / transport)`
**Domain tags:** rust, axum, auth, scopes, rustyred-thg-server, 403

## Trigger

The Agent Space design specified scope `coordination:read` for the new
`/v1/agent-space/{stream,snapshot}` routes, so the handler called
`require_scope(&headers, &state.config.api_tokens, "coordination:read", require_auth)`.
The two HTTP integration tests used `memory_product_state()` which sets
`require_auth=false`. Both returned **403**, not 200/400:

```
assertion `left == right` failed
  left: 403
 right: 200
```

`require_scope` calls `authenticate(..., require_auth=false)` which returns a dev
`AuthContext` whose scopes are `ALL_SCOPES` -- a FIXED 16-entry array in `auth.rs`.
`coordination:read` was not in it, so the membership check failed -> 403. The
sibling SSE route `/v1/coordination/events` had sidestepped this by gating on
`graph:read` (which IS in ALL_SCOPES). Fix: add `"coordination:read"` to
`ALL_SCOPES` (bump `[&str; 16]` -> `[&str; 17]`). It is also already an
established scope in the MCP-context layer (`McpRequestContext::with_scopes(["coordination:read"])`),
just not in the HTTP token vocabulary.

## Rule

`require_auth=false` does NOT mean "all scopes pass" -- it means the request is
granted exactly the scopes in `auth.rs::ALL_SCOPES`. Before gating a new HTTP route
on a scope string, confirm that string is in `ALL_SCOPES`; if not, add it (and
update the `[&str; N]` length). HTTP token scopes (`ALL_SCOPES` / per-token
`ApiToken.scopes`) are a SEPARATE vocabulary from MCP-context scopes
(`McpRequestContext::with_scopes`); a scope existing in the MCP layer does not make
it valid for an HTTP `require_scope` gate. A 403 on a route under `require_auth=false`
almost always means an unlisted scope, not an auth problem.

## Evidence

- `auth.rs`: `const ALL_SCOPES: [&str; 16]` lacked `coordination:read`; dev context
  is `scopes: ALL_SCOPES.iter()...collect()`.
- Existing `coordination_events` handler gates on `"graph:read"`; tests went green
  after adding `coordination:read` to ALL_SCOPES.

## Encoded in

- `docs/learnings/2026-06-16-http-routes-gate-on-all-scopes-list-add-new-scope-there.md` (this file)
