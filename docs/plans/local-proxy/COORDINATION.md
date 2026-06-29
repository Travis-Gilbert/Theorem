# Local-proxy plan: lane split (CC <-> Codex)

Date: 2026-06-28. Channel: this file + commit messages (the harness coordination remote is
`remote_unavailable`; git is the fallback channel). Source of truth for who owns what so the
two heads do not overwrite each other. Update it when a lane changes hands.

## Ownership

| Lane | Owner | State |
| --- | --- | --- |
| Phase A.1 Valkey on SSD | CC | DONE (`scripts/valkey-local.sh`, AOF-restart verified; daemon mode added for local service starts) |
| Phase A.3 substrate `HttpMemorySource` | CC | DONE (mock-`/mcp` proven; commit `63be1d038`) |
| Phase A.2 local node on SSD (`rustyred-thg-server`) | CC | PROVEN locally by Codex smoke: `/ready` reports embedded RedCore at `/Volumes/SSD Samsung/theorem-local-node`; final landing still CC/rebase lane |
| Phase A.4 local memory + coordination | CC | COORDINATION PROVEN locally by Codex smoke: `coordination_record` -> `coordination_context` survived node restart; memory live-provider smoke still separate |
| Phase B.1 ranked injection wired to substrate + remove `recall` | Codex | open |
| Phase B.2 tool-surface pruning (+ the usage audit) | Codex | audit in progress |
| Phase B.3 staleness-aware memory + memory-CI | Codex | open |
| Phase B.4 proxy-mediated proactive coordination | Codex | has `local-proxy-codex-presence` |
| Phase B.5 built-in measurement + `doctor` readout | Codex | open |
| Phase C.1 install ergonomics (brew/curl/`wrap claude`) | CC | DONE local (`wrap` + `doctor` + `scripts/install.sh` from-source); brew tap + release-binary `curl\|sh` is the outward-facing publish step (gated) |
| Phase C.2 remaining MVP (D2 membrane, D4 parity, D6 sidecar, D7 two-token) | CC | DONE (D2 + D4 + D7 + D6 desktop sidecar all landed) |
| Phase C.3 one-click onboarding | CC | IN PROGRESS (`theorem-proxy doctor` chain-check done; site download / `theorem login` remaining) |
| Phase C.4 resident capabilities (affordance exec, cascade, verify offload) | Codex | has `proxy-resident-capabilities` (merged #67) |
| `fix-proxy-timeout-mcp-latency` -> main | Codex | branch `5df900607`, unmerged |

**UPDATE 2026-06-28: lanes dropped (Travis).** Both heads now work the whole plan; the
ownership table above is historical context, not a fence. The one rule that stays: reconcile,
never overwrite ([[parallel-head-shipped-the-same-capability-reconcile-dont-overwrite]]) --
check git for the other head's live work before touching shared files. (Original split:
Codex = Phase B + resident-capabilities; CC = Phase A remainder + Phase C.)

## Cross-lane dependencies

- **B.4 (Codex) depends on A.2/A.4 (CC).** Proactive coordination needs the proxy to see
  multi-head traffic against a *local* coordination graph that is actually up. CC's A.2 node
  (rustyred-thg-server on the SSD, serving the `coordination_*` MCP tools at `/mcp`) is that
  substrate. Handoff: CC stands up the node + proves `coordination_record`/`coordination_context`
  land locally; Codex builds the proxy-side recency push on top.
- **B.1 (Codex) and A.3 (CC) meet at the `MemorySource` seam.** CC's `HttpMemorySource` already
  retrieves over the node (`hippo_retrieve`); B.1's "wire ranked injection to substrate retrieval"
  is the same seam. Reuse `HttpMemorySource`, do not fork a second retrieval path. `recall`-tool
  removal is B.1's (the MCP manifest in `rustyred-thg-mcp` / the plugin), Codex's lane.

## Local mode clarification (Codex smoke, 2026-06-28)

- The runnable local node mode is **embedded**: `rustyred-thg-server` opens durable RedCore under
  `/Volumes/SSD Samsung/theorem-local-node`, and `/ready` reports `mode=embedded`,
  `durability=aof_everysec`.
- Valkey is installed/runnable and proven persistent on `/Volumes/SSD Samsung/theorem-valkey`, but
  in the current server code it is the `RUSTY_RED_MODE=redis` / `RedisGraphStore` compatibility
  path. It is **not** automatically a warm tier beside embedded RedCore in this slice.
- `RUSTY_RED_MODE=redis` remains useful for Redis/Valkey compatibility checks, but embedded RedCore
  is the canonical local graph substrate for memory, coordination, instant KG, and the proxy link.

## Open reconcile (CC's branch -> main)

CC's substrate-memory work is committed local-only on `prove-and-prune-substrate-memory`
(`63be1d038`). PR #69 is already MERGED to main, and main's `apps/theorem-proxy` is byte-identical
to the original #69 commit, so CC's base == main's proxy. To land cleanly:

- Rebase `prove-and-prune-substrate-memory` onto `origin/main`: the whole-proxy files in the commit
  are identical to main's and become no-ops; only the true delta (memory.rs HttpMemorySource,
  main.rs `--memory-url`, Cargo `ureq`, valkey script, substrate test, CLAUDE.md row, plans) lands.
- Codex's `fix-proxy-timeout-mcp-latency` (`17e005054`, lib.rs total-timeout removal) is unmerged and
  does NOT touch CC's delta files (memory.rs/main.rs/Cargo), so the two compose without conflict.
  Whichever merges second just rebases.

Nothing is pushed yet; push topology is Travis's call.

## Gated / cross-surface (named blockers, not silent cuts)

- **D6 Commonplace sidecar (spec deliverable 6): DONE (CC, lanes-off).** `apps/desktop` now
  spawns `theorem-proxy` in-process on launch (port 17891, ambient memory from the local node)
  and aborts it on teardown, plus `connect_claude_code` / `disconnect_claude_code`
  (merge-preserving `ANTHROPIC_BASE_URL` write into `~/.claude/settings.json`) +
  `theorem_proxy_status`, with `commands.ts` wrappers. In-process spawn (mirrors
  `start_local_node`), not a Tauri externalBin. `cargo check` + clippy green. Remaining UI: a
  Connect button wired to `connectClaudeCode()`.
- **C.1 brew tap: artifacts written (CC).** `.github/workflows/release-proxy.yml` (per-platform
  binary build on a `proxy-v*` tag) + `apps/theorem-proxy/packaging/homebrew/theorem-proxy.rb` +
  a README. Remaining is Travis's GitHub-side (the README spells it out): tag a release, create the
  `homebrew-theorem` tap repo, paste the released `sha256`s. Caveat: the repo is private, so the
  release assets must be made public for `brew install` to fetch them.
- **C.3 `theorem login` + site download + Connect-button parity:** `login` needs an account ->
  substrate-key backend to authenticate against (the harness remote, currently degraded); the
  site download needs a web distribution surface. Both depend on external infra, not local code.
  `theorem-proxy doctor` (the chain-check half of C.3) is done; B.5 will add its value readout (Codex).
- **C.4 resident capabilities (affordance exec / cascade / verify offload):** Codex owns it
  (`proxy-resident-capabilities`, merged #67).
- **Binary naming:** reconciled -- the CLI entry is `theorem-proxy` (`proxy` / `wrap` / `doctor`);
  the local-proxy specs now use that command surface.
