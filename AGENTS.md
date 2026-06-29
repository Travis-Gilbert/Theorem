# AGENTS.md, Theorem

This file briefs coding agents working in this repository. CLAUDE.md carries the full project context, architecture, build, test, and layout; read it. This file carries the conventions that apply to every session regardless of the task, and they take precedence when a task tempts you to skip them.

## Start of every session

- Run `git pull` first. Commits made through the GitHub MCP land on the remote, not on this local checkout, until you pull.
- There is no top-level Cargo workspace. Use `rustyredcore_THG/` for workspace Cargo commands, or the specific standalone app manifest when the task points at one.

## Local RustyRed and model-path proxy

The practical "next session inside local RustyRed" path is the local node plus model-path proxy. Keep the source working directory as this checkout or a materialized RustyRed worktree unless a task explicitly asks for a DocTree/FUSE mount. Claude Desktop can also use this path through Desktop 3P gateway mode; do not configure it as an MCP HTTP server. Codex can use the same proxy through the OpenAI Responses surface.

- One-shot launch from this repo: `apps/theorem-proxy/scripts/start-proxied-session.sh`. It starts the local embedded RedCore/RustyRed node, starts `theorem-proxy`, and runs `claude` with `ANTHROPIC_BASE_URL` pointed at the proxy for that process.
- One-shot Codex launch from this repo: `apps/theorem-proxy/scripts/start-proxied-codex-session.sh`. It starts the local node and proxy, then runs `codex -c 'openai_base_url="http://127.0.0.1:8788/v1"'`, so Codex's Responses traffic passes through RustyRed/proxy for that process.
- Claude Desktop gateway launch from this repo: `apps/theorem-proxy/scripts/start-desktop-gateway.sh`. Export exactly one upstream credential first (`THEOREM_PROXY_UPSTREAM_API_KEY` or `THEOREM_PROXY_UPSTREAM_AUTH_TOKEN`). For subscription/OAuth tokens, also export `THEOREM_PROXY_UPSTREAM_BETA=oauth-2025-04-20`.
- First-run seed: `THEOREM_SEED=1 apps/theorem-proxy/scripts/start-proxied-session.sh`.
- Graph-memory path: `THEOREM_USE_NODE_MEMORY=1 apps/theorem-proxy/scripts/start-proxied-session.sh` uses the live node's `/mcp` retrieval path. Without it, the launcher uses the fast directory-memory fallback.
- Local node state lives on the external SSD by default (`/Volumes/SSD Samsung/theorem-local-node`). Do not treat that directory as a source checkout; it is durable RustyRed graph/file state.
- Embedded RedCore is the canonical local mode (`RUSTY_RED_MODE=embedded`). Valkey is only the Redis-wire compatibility/warm-tier path via `apps/theorem-proxy/scripts/valkey-local.sh` and `RUSTY_RED_MODE=redis`; do not describe Valkey as required for the normal local session.
- To point an existing Claude Code process at the proxy, set `ANTHROPIC_BASE_URL=http://127.0.0.1:8788`. Claude Code's gateway docs say env vars can also be persisted under the `env` key in `~/.claude/settings.json` or `.claude/settings.local.json`; keep credentials out of committed project settings. Setting only `ANTHROPIC_BASE_URL` routes model traffic through the proxy but does not replace the active Claude credential.
- To point an existing Codex process at the proxy, run with `codex -c 'openai_base_url="http://127.0.0.1:8788/v1"'` or add the same `openai_base_url` only to a temporary/profile config. Keep the normal global Codex config untouched unless the user explicitly asks for persistent routing.
- To point Claude Desktop at the proxy, use Claude Desktop 3P gateway config (`~/Library/Application Support/Claude-3p/claude_desktop_config.json` or the in-app Developer -> Configure third-party inference flow), not `mcpServers`. The Desktop config may use a harmless local gateway key; the running proxy must provide the real upstream credential through `THEOREM_PROXY_UPSTREAM_API_KEY` or `THEOREM_PROXY_UPSTREAM_AUTH_TOKEN`, so Desktop's local key is stripped before forwarding.
- The proxy makes RustyRed memory/coordination ambient for Claude Code and Codex on their model path. It does not, by itself, make Codex a second live voice inside the Claude Code UI. Cross-head copresence still needs the `theorem-receiver`/head-adapter job path or a composed Agent Theorem user surface.

## The harness (Theorems-Harness V2)

This project has a persistent memory and coordination substrate. Use it reflexively, not on request.

- The tenant slug is `Travis-Gilbert`, capitalized and hyphenated. Not lowercase, not default.
- Before answering an architecture question or a "did we decide X" question from your own training data, recall from the harness. Prior decisions, conventions, and the reasons behind them live there.
- When you make or are handed a load-bearing decision, a constraint, a convention, a thing ruled out, encode it to the harness so the next head and the next session inherit it, instead of fixing it by hand later.
- Coordinate with other heads by footprint, what you are doing and which files your hands are on, not by dividing files into rigid lanes. Heads do their best work on the same task with tight sync, not separated into worktrees that produce duplicate work.

## The grounding contract

This is the most important convention here, because it is the one most often skipped.

Agents are strong at translation and verification and weak at reconstruction. You default to training data, not the web or this codebase, unless you are given the source. A task where the answer or a checkable proxy sits in context succeeds. A task that asks you to reconstruct precise external knowledge from memory, a published architecture, a library's real API, a spec or wire format, fails in the details, and it fails silently because the output looks plausible.

So, for any task that depends on precise external knowledge:

- Read the named authoritative source before writing code. If the spec names a reference repo or file, read it at the pinned commit and bind to what is actually there. Do not reconstruct it from memory.
- If a spec names a tool, path, or signature, that is a requirement by position. If you disagree, surface the disagreement; do not silently substitute something else.
- Completion is defined by an oracle, not by the code looking right: a test that passes, a numerical parity check, a reference output matched, a conformance check. "Looks right" is not done.
- For a library port, parity-test module by module against the pinned reference, and load real reference weights and inputs rather than synthetic ones. Watch framework differences, for example the Burn Linear weight is laid out as [in, out] while PyTorch is [out, in], so transpose on load.

## Review and correctness

Do not look for problems by reading. Stand up the oracles and fix what they flag. For Rust that means miri for undefined behavior in unsafe code, proptest for invariants and round-trips, criterion for benchmarks so "slow" becomes a number, ThreadSanitizer with a stress test for data races, and a soak test watching resident memory for unbounded growth. For a database especially, correctness under concurrency and unbounded memory growth bite harder than inefficiency and are nearly invisible to eye review.

## Scope discipline

Implement what the spec says, fully. Do not insert conservative defaults that contradict the spec, do not downgrade to an MVP that was not asked for, and do not frame in-scope work as deferred. A named choice in the spec is a requirement, not a suggestion. If something genuinely cannot be done, say so plainly and name the blocker, rather than quietly shrinking the work.

## Local build/store hygiene

When disk pressure or cleanup is needed, clear rebuildable caches outside the project before clearing repo-local stores. Internal stores often carry the state needed to complete and validate the current project, and reloading them slows the work down.

## Doc-update protocol (end of every session)

Code outruns docs. If your session added, renamed, or removed a crate or app, before you end the session: update the crate or app table in `CLAUDE.md` and the matching row in `docs/site/reference/`, fix any `CLAUDE.md` section the change makes wrong, bump the README `Last sync` line if you re-synced with Theseus, then run `scripts/check-doc-drift.sh --refresh`. Encode the decision to the harness if it is load-bearing.

Detection backs the rule. `scripts/check-doc-drift.sh` compares crates and apps on disk against the `CLAUDE.md` map and a baseline. A `SessionStart` hook injects the current doc-map status into every session. A `Stop` hook flags new undocumented directories; export `THEOREM_DOC_DRIFT_BLOCK=1` to make it block until they are documented. Full guide: `docs/site/guides/doc-update-protocol.md`.
