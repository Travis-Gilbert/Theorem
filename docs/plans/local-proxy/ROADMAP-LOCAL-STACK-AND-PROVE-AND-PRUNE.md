# Roadmap: local Theorem stack (Valkey + SSD + proxy), then Prove-and-Prune, then D5 + remaining

Date: 2026-06-28. Register: execution roadmap (sequences existing specs; details live in the specs it references). Read `CONVENTIONS.md` first.

This is the resume artifact: a fresh, low-context session can execute it top to bottom. Each phase names what exists, what to verify first, the concrete steps, and the acceptance.

## Why this order, and the usage signal

Travis's API usage spiked to ~33% in one day (very high even for him). The direct token reducers are **Prove-and-Prune D1** (relevance-ranked memory injection instead of the wholesale `MEMORY.md` dump injected every turn) and **Prove-and-Prune D2** (prune the dozens-of-tools surface advertised every turn down to the ~3 genuine actions). The requested order is local stack first, then Prove-and-Prune, then D5. That order is honored below, BUT: **Prove-and-Prune D2 (tool pruning) is the fastest standalone usage win and does not require the local stack** -- if the usage bleed is urgent, pull B.2 forward ahead of Phase A.

## Current state (grounded, 2026-06-28)

- `apps/theorem-proxy` built: SPEC-LOCAL-PROXY-MVP **D1** (faithful Anthropic Messages passthrough: streaming + non-streaming, headers/body/SSE preserved byte-for-byte) **and D3** (cache-stable ambient memory injection: relevance-ranked `MemorySource` seam, injection appended only to the last user message so system+tools stay byte-identical, deterministic re-serialization so a second identical request hits the prompt cache, fail-open). PR #69 (`theorem-proxy-mvp` -> `main`) is merged on `origin/main`.
- Phase B.1 local start: the proxy now has `HttpMemorySource` behind `--memory-url` / `THEOREM_PROXY_MEMORY_URL`, calling the local node's hidden `hippo_retrieve` MCP intercept. `recall` is hidden from `rustyred-thg-mcp` `tools/list`, while the compatibility call handler remains. The product server also keeps `hippo_retrieve` callable but no longer advertises it.
- The proxy's fallback `MemorySource` is `DirectoryMemorySource` (a dir of `*.md`); live use should prefer `THEOREM_PROXY_MEMORY_URL` once Phase A's local node is running.
- Specs: `docs/plans/local-proxy/SPEC-PROXY-PROVE-AND-PRUNE.md` (the 5 deliverables). Source specs `SPEC-LOCAL-PROXY-MVP.md`, `SPEC-PROXY-RESIDENT-CAPABILITIES.md`, `SPEC-ONECLICK-ONBOARDING.md` still in `~/Downloads` -- MOVE them into `docs/plans/local-proxy/`.
- Builds route to the SSD via `rustyredcore_THG/.cargo/config.toml` (gitignored, `target-dir = /Volumes/SSD Samsung/theorem-recon-target`). Other standalone crates (`apps/theorem-proxy`) need their own `CARGO_TARGET_DIR=` prefix or a per-crate `.cargo/config.toml`.
- Substrate facts: `RedCoreGraphStore` = durable file-backed in-process store (AOF + snapshots; `open(data_dir, RedCoreOptions)`). `RedisGraphStore` = connects to a Redis/Valkey server (Valkey is Redis wire-compatible) but is a `RUSTY_RED_MODE=redis` compatibility path, not the canonical local graph path. `rustyred-thg-memory` = graph-native memory with `recall`/`encode`, bitemporal validity (`valid_at_ms`/`invalid_at_ms`), decay. `rustyred-embedded` = in-process embedded engine + a stdio MCP binary over a local data dir. Server surfaces: `rustyred-thg-server` (HTTP/gRPC/MCP), `theorem-grpc`, `theorem-harness-server`, `theorem-localmodel` (local model host, blocking).
- Codex local smoke (2026-06-28): Valkey on `127.0.0.1:6391` persisted a probe key through restart with AOF under `/Volumes/SSD Samsung/theorem-valkey`; `rustyred-thg-server` on `127.0.0.1:8380` reported `mode=embedded` and `data_dir=/Volumes/SSD Samsung/theorem-local-node`; `tools/list` advertised `coordination_record`/`coordination_context` and hid `recall`/`hippo_retrieve`; a `coordination_record` in `room:local-proxy-smoke` was visible through `coordination_context` after node restart.

## Phase A -- local Theorem node on the SSD (Valkey compatibility + proxy)

Goal: a local running Theorem node whose durable graph state lives on the SSD, with the proxy pointed at its live memory, so memory + coordination are local (bypassing the degraded Railway remote) and the harness runs in-process. Valkey is available on the SSD for Redis compatibility/cache checks; embedded RedCore is the canonical local graph substrate in this slice. This is the substrate that makes Prove-and-Prune real.

Verify first (next session, before building): which server surface is "the local node" -- likely `rustyred-embedded` (already in-process over a local dir) or `rustyred-thg-server`; whether `valkey-server` is installed (`which valkey-server`); the `RedCoreOptions` data-dir + the `RedisGraphStore` connection contract; and how a `MemorySource` impl in `apps/theorem-proxy` reaches the local node's memory (in-process link vs a local socket).

- **A.1 Valkey on the SSD.** Install `valkey-server` (brew); a `valkey.conf` with `dir /Volumes/SSD Samsung/theorem-valkey`, `appendonly yes`, a fixed port; a launch script. Acceptance: `valkey-server` runs, persists AOF to the SSD, `PING` -> `PONG`, survives restart. Status: proven locally on port `6391`; script now supports daemon mode via `THEOREM_VALKEY_DAEMONIZE=true`.
- **A.2 Local substrate on the SSD.** Stand up `rustyred-thg-server` in `RUSTY_RED_MODE=embedded` with its `RedCoreGraphStore`/object-store `data_dir` on the SSD. Acceptance: the node starts, writes land on the SSD, survive a process restart (recover from AOF/snapshot). Status: `/ready` proves embedded RedCore on `/Volumes/SSD Samsung/theorem-local-node`; coordination write/read survived node restart.
- **A.3 Proxy -> local memory.** Add a substrate-backed `MemorySource` to `apps/theorem-proxy` that reads the local node's memory (`recall` over RedCore), replacing the directory default for live use. Acceptance: the proxy injects from live local memory, not a static dir.
- **A.4 Harness memory + coordination local.** Route `encode`/`recall`/coordination to the local node instead of Railway. Acceptance: a session's memory writes/reads and coordination land on the local node; `Coordination Context` is no longer `remote_unavailable`.

Note: Phase A is the largest and most underspecified. Open it with a short design pass (which surface; how Valkey slots as the warm tier; in-process vs socket for the proxy link). Decisions are CC's to make and coordinate with Codex, not to block on.

## Phase B -- Prove-and-Prune (the usage fix; `SPEC-PROXY-PROVE-AND-PRUNE.md`)

Ordered by usage leverage.

- **B.1 (spec D1) Relevance-ranked injection wired to the substrate + remove `recall`.** Started locally: proxy `HttpMemorySource` calls hidden local-node `hippo_retrieve`, and `recall` is no longer advertised in MCP tools. Remaining live acceptance: run a proxied Claude/provider session against the local node and confirm prompt-cache usage on two identical requests.
- **B.2 (spec D2) Tool-surface pruning.** From the usage audit (Codex is running one), cut never-called tools from the advertised set; move context-injection "tools" to proxy injection. Acceptance per spec D2. **Biggest immediate token-tax reducer; pull forward if usage is urgent.**
- **B.3 (spec D4) Staleness-aware memory + memory CI.** Wire the existing `rustyred-thg-memory` validity layer to the ambient path; periodic memory-CI marks dead memories (referenced file/symbol gone). Acceptance per spec D4.
- **B.4 (spec D3) Proxy-mediated proactive coordination.** Needs Phase A's local multi-head traffic. Push a "head B edited file X 90s ago" heads-up before an action targets a contended file. Acceptance per spec D3.
- **B.5 (spec D5) Built-in measurement + `theorem-proxy doctor` readout.** The proxy A/Bs injection on/off; tracks rediscovery rate, collisions prevented, tokens saved; `doctor` surfaces it. This is the answer to "does the harness help" with numbers. Acceptance per spec D5.

## Phase C -- D5 install ergonomics + remaining proxy MVP

- **C.1 SPEC-LOCAL-PROXY-MVP D5:** brew tap + `curl ... | sh` + `theorem-proxy wrap -- claude` (one-command connect) + CPU-only default. Turns the local stack into one command instead of the manual `ANTHROPIC_BASE_URL` export.
- **C.2 Remaining MVP:** D2 native-tool membrane (sample/defer large tool outputs via `tool_result_fetch`), D4 tool-call parity through the proxy, D6 Commonplace sidecar auto-launch, D7 two-token separation (Anthropic credential local-only, harness key to Railway).
- **C.3 SPEC-ONECLICK-ONBOARDING:** site download + three-step copy-paste, `theorem login` (account -> substrate key), `theorem-proxy doctor` chain-check (+ the value readout from B.5), Connect-button parity.
- **C.4 SPEC-PROXY-RESIDENT-CAPABILITIES:** transparent affordance execution (proxy resolves harness affordances itself), the cascade (route easy turns to local Gemma), verification offload (advisory checks against the graph/symbolic layer).

## Cross-cutting follow-ups

- **Doc-drift:** `apps/theorem-proxy` (and `commonplace-desktop-runtime`) are undocumented in `CLAUDE.md`. Add `theorem-proxy` to the app table + README "Last sync" line as part of PR #69 (its CLAUDE.md is `origin/main`'s, so no Codex collision), then `scripts/check-doc-drift.sh --refresh`.
- **Move source specs** from `~/Downloads` into `docs/plans/local-proxy/`.
- **Naming:** reconcile `theorem` vs `rustyred` for the binary across the specs.
- **Build target:** keep `CARGO_TARGET_DIR` on the SSD for every crate.

## Resume pointers

- Memory: `[[harness-local-proxy]]` (the inversion + what's built + this roadmap).
- The five Prove-and-Prune deliverables with full acceptance: `SPEC-PROXY-PROVE-AND-PRUNE.md` (same dir).
- PR #69 (theorem-proxy D1+D3) is merged on `origin/main`; this branch carries the Phase B.1 follow-on.
