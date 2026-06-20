# The harness coordination tenant is `Travis-Gilbert`, NOT the Railway app name `rustyredcore-theorem-production` -- the app name silently parks you in an isolated partition

**Kind:** postmortem
**Captured:** 2026-06-20
**Session signature:** `claude-code:travisgilbert (SPEC-2 Item domain + SPEC-3 registry; harness comms diagnosis)`
**Domain tags:** coordination, harness, tenant, plugins, codex, multi-head

## Trigger

Asked to diagnose why Claude Code and Codex "can't talk to each other" in the harness. I checked `mentions`, `read_intents_for_room`, and `read_messages_for_room` under `tenant_slug = "rustyredcore-theorem-production"` (the value my memory note `harness-room-tenant-resolution` told me to pass), saw ZERO codex activity, and -- across this and several prior CC sessions -- concluded "Codex is git-only, never posts to the room." That conclusion was FALSE. Under the canonical tenant `Travis-Gilbert` there were 20+ `codex` -> `claude-code` mentions (lane intents, work announcements, a full P1/P2 peer review). The `agent:theorem` binding head-set included `codex` ONLY under `Travis-Gilbert`; under `rustyredcore-theorem-production` it was `[claude, claude-code, claude-code:travisgilbert, deepseek]` -- no codex.

Root cause: `rustyredcore-theorem-production` is the **Railway app/deployment name** (`rustyredcore-theorem-production.up.railway.app`), not a tenant. The harness MCP partitions by whatever `tenant_slug` you pass and silently creates a fresh partition for an unknown slug -- so passing the app name produced NO error (the 404 my memory was "fixing" vanished); it just isolated me in an empty partition no other head reads. The canonical tenant -- set in Codex's `~/.codex/config.toml` `[shell_environment_policy.set] THEOREM_TENANT_ID = "Travis-Gilbert"`, and stated in the plugin docs ("the production harness tenant is `Travis-Gilbert`") -- is where Codex and Claude.ai actually coordinate. The wrong-tenant memory propagated the false "Codex git-only" belief into several sibling memory notes over many sessions.

## Rule

Coordinate under the canonical tenant `Travis-Gilbert` (the backend lowercases it to `travis-gilbert`); never pass the Railway app name `rustyredcore-theorem-production` as a tenant. Before concluding another head "isn't coordinating," verify YOUR OWN read addressing (tenant / room_id / actor) first -- a harness read that returns empty is as likely a wrong-coordinate read as a silent peer. A workaround that makes an error vanish by moving you somewhere valid-but-wrong (here, a fresh empty partition) is worse than the error: the 404 was correctly reporting an unresolved tenant.

## Evidence

- Under `rustyredcore-theorem-production`: `coordination_intent` binding `["claude","claude-code","claude-code:travisgilbert","deepseek"]` (no codex), `presence(codex)=null`, my `@codex` mentions piling unread.
- Under `Travis-Gilbert`: binding `["Codex","claude","claude-code","codex","deepseek"]`, ~25KB of `codex`->`claude-code` mentions incl peer-review `msg_9916f23263d72d5c`.
- Canonical source: `~/.codex/config.toml:613`; plugin `theorems-harness/scripts/lib.sh` `theorem_tenant()` (no default -- empty unless `THEOREM_TENANT_ID` set).
- Corrected the propagation source: memory note `harness-room-tenant-resolution.md`.

## Encoded in

- `docs/learnings/2026-06-20-harness-coordination-wrong-tenant-not-app-name.md` (this file)
