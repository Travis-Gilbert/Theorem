# Superseded Spec: REST-First Capability Surface and the theorem CLI

Status: superseded by `SPEC-GRAPHQL-CONTRACT-AND-CLI.md` on 2026-06-29.

This file is intentionally a tombstone. The REST-first plan correctly identified the CLI and routing ergonomics, but it incorrectly made REST `/cap/<name>` the canonical capability contract before accounting for the existing GraphQL agent and CommonPlace consumer surfaces.

Do not implement a new REST-first capability server from this file. The current plan is:

- GraphQL schema execution and shared handlers/domain services are the canonical capability contract.
- MCP, CommonPlace, the proxy membrane, and any REST convenience routes are projections of that contract.
- REST remains useful for Anthropic Messages, OpenAI Responses, health/status, and optional generated convenience wrappers over GraphQL.
- `theorem up`, `theorem status`, `theorem doctor`, `theorem connect`, `theorem connect --check`, and `theorem disconnect` are owned by `SPEC-GRAPHQL-CONTRACT-AND-CLI.md`.

Historical CLI findings from the REST-first draft were migrated into `SPEC-GRAPHQL-CONTRACT-AND-CLI.md`, including:

- Codex CLI can route through `openai_base_url`.
- Codex Desktop does not route through `codex app -c openai_base_url=...`; it needs user-level config plus restart/open.
- Claude Code routes through `ANTHROPIC_BASE_URL`.
- Claude Desktop uses Gateway mode, not MCP HTTP server config.
- Routing proof should use proxy `/status` counters.
