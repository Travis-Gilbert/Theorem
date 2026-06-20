# Theorem plugin hooks call `theorem_native_call` with no tenant, so a head's automated Stop-hook mention-drain runs tenant-less even when `THEOREM_TENANT_ID` is set

**Kind:** gotcha
**Captured:** 2026-06-20
**Session signature:** `claude-code:travisgilbert (harness comms diagnosis + plugin fix 0.5.10)`
**Domain tags:** plugins, coordination, harness, tenant, bash-hooks, codex

## Trigger

Diagnosing the harness split-brain: `presence(codex)` was null and a 34KB `@codex` mention backlog sat unread under the canonical tenant -- yet Codex's *posted* messages all carried `tenant_slug: travis-gilbert`, so Codex clearly WAS resolving the tenant somewhere. The asymmetry: Codex's *model-driven* `coordinate` calls pass the tenant, but its *hook-driven* calls do not. `drain-mentions.sh:28` (the Stop hook) calls `theorem_native_call "mentions" '{actor, consume, limit}'` with NO `tenant_slug`, and `lib.sh theorem_native_call` injects none -- only `theorem_code_call` (the code-KG path) calls `theorem_tenant()`. So a head's automatic turn-end mention-drain queries the wrong/default partition and never sees pings addressed to it, even though `THEOREM_TENANT_ID` is exported in the session env. This is why "communication still wasn't happening" after the Layer-2/3 (stream-read / normalize_tenant) fixes -- those never touched the hook-call tenant omission.

## Rule

Inject the resolved tenant at the ONE choke point (`theorem_native_call`), not per-script: a jq merge `if (type=="object") and ((has("tenant_slug") or has("tenant")) | not) then . + {tenant_slug:$t} else . end` guarded on a non-empty `theorem_tenant()`. Per-script tenant args rot (every new hook reintroduces the bug); the choke point cannot. Fixed in `theorems-harness 0.5.10` (`claude-marketplace 3db0da7`). NB: running sessions keep the broken `0.5.9` cache until they resync -- the source push alone does not fix live drains.

## Evidence

- `theorems-harness/scripts/drain-mentions.sh:28` (tenant-less `mentions`); `scripts/lib.sh theorem_native_call` (no injection); `theorem_tenant` used only by `theorem_code_call`.
- Fix `claude-marketplace@3db0da7`: `lib.sh theorem_inject_tenant` + bump 0.5.10; `bash -n` clean, jq-verified inject-when-absent / leave-when-present.

## Encoded in

- `docs/learnings/2026-06-20-codex-hooks-native-calls-drop-tenant.md` (this file)
