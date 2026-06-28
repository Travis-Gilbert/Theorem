# Local-proxy plan: lane split (CC <-> Codex)

Date: 2026-06-28. Channel: this file + commit messages (the harness coordination remote is
`remote_unavailable`; git is the fallback channel). Source of truth for who owns what so the
two heads do not overwrite each other. Update it when a lane changes hands.

## Ownership

| Lane | Owner | State |
| --- | --- | --- |
| Phase A.1 Valkey warm tier on SSD | CC | DONE (`scripts/valkey-local.sh`, AOF-restart verified) |
| Phase A.3 substrate `HttpMemorySource` | CC | DONE (mock-`/mcp` proven; commit `63be1d038`) |
| Phase A.2 local node on SSD (`rustyred-thg-server`) | CC | IN PROGRESS (this session) |
| Phase A.4 local memory + coordination | CC | IN PROGRESS (rides A.2) |
| Phase B.1 ranked injection wired to substrate + remove `recall` | Codex | open |
| Phase B.2 tool-surface pruning (+ the usage audit) | Codex | audit in progress |
| Phase B.3 staleness-aware memory + memory-CI | Codex | open |
| Phase B.4 proxy-mediated proactive coordination | Codex | has `local-proxy-codex-presence` |
| Phase B.5 built-in measurement + `doctor` readout | Codex | open |
| Phase C.1 install ergonomics (brew/curl/`wrap claude`) | CC | IN PROGRESS (`theorem-proxy wrap` done; brew tap + curl-installer remaining) |
| Phase C.2 remaining MVP (D2 membrane, D4 parity, D6 sidecar, D7 two-token) | CC | IN PROGRESS (D2 membrane + D4 parity + D7 two-token done; D6 sidecar remaining) |
| Phase C.3 one-click onboarding | CC | IN PROGRESS (`theorem doctor` chain-check done; site download / `theorem login` remaining) |
| Phase C.4 resident capabilities (affordance exec, cascade, verify offload) | Codex | has `proxy-resident-capabilities` (merged #67) |
| `fix-proxy-timeout-mcp-latency` -> main | Codex | branch `5df900607`, unmerged |

Rule of thumb: **Codex owns all of Phase B + the resident-capabilities lane; CC owns Phase A
remainder + Phase C install/MVP/onboarding.** Stay out of each other's branches; reconcile,
never overwrite ([[parallel-head-shipped-the-same-capability-reconcile-dont-overwrite]]).

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
