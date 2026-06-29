# Execution Spec: GraphQL Capability Contract and the theorem CLI

Date: 2026-06-29. Register: execution. Read `CONVENTIONS.md` first; its rules apply. Parents: `NORTH-STAR-RUSTYRED-MULTIMODEL.md`, `SPEC-GRAPHQL-MCP.md`, `SPEC-LOCAL-PROXY-MVP.md`, `SPEC-PROXY-RESIDENT-CAPABILITIES.md`, `SPEC-PROXY-PROVE-AND-PRUNE.md`, `rustyred-thg-mcp`, `apps/commonplace-api`, `apps/theorem-proxy`, and the local node / model-path proxy work.

This supersedes `SPEC-REST-FIRST-CAPABILITIES-AND-CLI.md`. The earlier REST-first plan correctly captured the CLI and connection ergonomics, but it made `/cap/<name>` the canonical capability wire contract before confirming the GraphQL surfaces already in the repo. This spec keeps the CLI work and changes the capability center of gravity to GraphQL-contract-first, REST-compatible.

## Purpose

Use one typed contract for the substrate capability surface, with thin projections for each consumer. The substrate already has that pattern in GraphQL:

- `rustyred-thg-mcp/src/graphql` is the agent profile reached through `graphql_query`, `graphql_mutate`, and `graphql_introspect`.
- `apps/commonplace-api` is the consumer profile for front ends, built on async-graphql and axum over the same substrate crates.
- The model-path proxy/membrane can become a third consumer of the same contract instead of calling private handlers, MCP, or a new REST-only surface.

The CLI remains the product ergonomics layer: `theorem up`, `theorem status`, `theorem doctor`, `theorem connect`, `theorem connect --check`, and `theorem disconnect` absorb each agent's configuration ceremony.

## Governing Principle

The crate-private payload handlers are the single source of capability behavior. The typed GraphQL schema is the canonical contract. MCP, CommonPlace, the proxy membrane, and any REST convenience routes are projections of that contract.

REST is still useful, but it is not the source of truth. REST stays for:

- provider-required model endpoints such as Anthropic Messages and OpenAI Responses,
- status and health endpoints,
- simple browser/curl/webhook convenience calls,
- optional generated `/cap/<name>` wrappers that execute the matching GraphQL operation.

REST must not introduce capability logic, state, schemas, or acceptance tests that can drift from the GraphQL contract.

## What Exists

- `apps/theorem-proxy`: local model-path proxy serving Anthropic Messages (`/v1/messages`) and OpenAI Responses (`/v1/responses`) on localhost, with ambient memory injection and `/status` counters.
- Anthropic Messages routing is working in the current local proxy setup; keep that proof separate from GraphQL capability wiring. Vendor model endpoints remain their native REST/SSE protocols.
- The proxy status oracle: `openai_responses_seen`, `anthropic_messages_seen`, `last_request_at`, and related counters are the proof surface for routing.
- `rustyred-thg-mcp/src/graphql`: typed agent GraphQL profile exposed through `graphql_query`, `graphql_mutate`, and `graphql_introspect`.
- `apps/commonplace-api`: typed consumer GraphQL profile over the CommonPlace object model, gated by per-instance API keys.
- Existing Codex findings from 2026-06-29: Codex CLI can route through `openai_base_url`; Codex Desktop cannot be routed by `codex app -c`; user-level Codex config is required.
- Existing Claude findings: Claude Code can be routed with `ANTHROPIC_BASE_URL`; Claude Desktop must use Desktop Gateway mode rather than an MCP HTTP server.

## Non-Duplication Decisions

- This spec owns `theorem connect`, `theorem up`, `theorem status`, `theorem doctor`, and `theorem disconnect`. Do not create a separate Codex Desktop launcher/config lane outside this CLI surface.
- `SPEC-REST-FIRST-CAPABILITIES-AND-CLI.md` is superseded. Do not build a new `rustyred-thg-http-server` as the canonical capability surface unless a later proof shows the GraphQL contract cannot cover the use case.
- `/cap/<name>` is allowed only as a generated REST-compatible facade over GraphQL. It is not a separate handler registry or capability definition surface.
- MCP stays thin. The agent MCP face should expose GraphQL transport and any required flat tools, but capability behavior lives behind the shared schema/handlers.
- The proxy membrane reaches capabilities through GraphQL, in-process when co-located and over HTTP GraphQL when remote. It should not call private handlers or MCP for the canonical path.
- ChatGPT subscription-native Codex routing remains an explicit bridge gate, separate from API/local OpenAI Responses routing.

## Deliverables

### 1. Membrane Reaches Capabilities Through GraphQL

Build: the proxy membrane in `apps/theorem-proxy` reaches capabilities by executing against the substrate GraphQL schema, in-process with `schema.execute` when co-located and over the served GraphQL endpoint when not. Ambient injection, resident affordance execution, coordination recency checks, and verification offload cross this typed, introspectable boundary.

Acceptance: an ambient injection in the membrane is served by GraphQL execution. The same operation returns equivalent results through the membrane, through MCP GraphQL, and through the CommonPlace-facing GraphQL profile where the domain overlaps. Verify the three-way parity with a fixture-backed operation.

### 2. One Schema/Handler Contract, No Drift

Build: confirm and hold the invariant that the agent profile, consumer profile, and membrane all resolve through shared payload handlers or shared domain services. If the agent and consumer profiles expose different field names, that is acceptable only when they wrap the same behavior and test against the same fixture.

Acceptance: adding or changing a capability happens once at the handler/domain layer and is visible through the relevant GraphQL profile(s) and membrane path without copying behavior into each face.

### 3. REST-Compatible Convenience Facade

Build: if `/cap/<name>` is kept, generate it from the GraphQL contract or from metadata bound to the GraphQL operation. Each route should translate JSON input into a GraphQL operation and return the GraphQL result envelope or a documented simplified JSON projection.

Acceptance: a `/cap/<name>` call, a direct GraphQL operation, and the membrane call produce equivalent results. The REST facade has no capability logic beyond auth, validation translation, operation dispatch, and response projection.

### 4. theorem CLI: Node Lifecycle

Build: `theorem up` starts the local node and its surfaces: GraphQL contract, optional REST-compatible facade, model-path proxy, OAuth/subscription bridge when available, watcher, and status service. `theorem doctor` verifies each surface and names broken links. `theorem status` lists running surfaces, connected agents, GraphQL health, optional facade health, and proxy counters.

Acceptance: `theorem up` brings the node and surfaces up; `theorem doctor` reports healthy and intentionally broken states accurately; `theorem status` shows GraphQL, the optional REST facade, Anthropic and OpenAI routing counters, and connected-agent state.

### 5. theorem CLI: connect

Build: `theorem connect <agent>` writes the agent's configuration to point at the local node and records enough state for `disconnect` to restore it.

`theorem connect claude`:

- Writes Claude Code's `ANTHROPIC_BASE_URL=http://127.0.0.1:PORT`.
- Writes or prints Claude Desktop Gateway steps: Developer Mode / third-party inference, Gateway base URL, local placeholder key, and any manual UI step the CLI cannot perform.
- Keeps real upstream credentials out of committed config. The proxy owns upstream credentials through environment or private local env files.
- Preserves the working Anthropic Messages path and verifies it through `anthropic_messages_seen`.

`theorem connect codex`:

- Writes user-level Codex config, not project `.codex/config.toml`, because provider routing keys are ignored in project-scoped config.
- For Desktop, restarts/opens the app after config is active. It must not rely on `codex app -c`.
- Provides an API/local mode backed by a custom local provider:

```toml
# managed by theorem connect codex
model_provider = "theorem"

[model_providers.theorem]
name = "Theorem local proxy"
base_url = "http://127.0.0.1:PORT/v1"
wire_api = "responses"
requires_openai_auth = false
```

- When the proxy forwards to `https://api.openai.com`, `theorem up` must require a private `THEOREM_PROXY_OPENAI_UPSTREAM_API_KEY` or equivalent upstream credential override. Without that, Codex's ChatGPT subscription token forwarded to the public OpenAI API fails with missing `api.responses.*` scopes.
- When the proxy forwards to a local OpenAI-compatible model host, no upstream API key is required.

Acceptance: `theorem connect claude` produces a Claude Code and Claude Desktop Gateway path that reaches the node. `theorem connect codex` produces a Codex CLI and Codex Desktop path that reaches the node in API/local mode. Each command prints manual steps it cannot perform and writes a reversible state record.

### 6. theorem CLI: check and disconnect

Build: `theorem connect <agent> --check` issues a path-specific probe and reports where it breaks. `theorem disconnect <agent>` restores the previous config and confirms the agent has returned to its normal provider path.

Codex check must use the proxy status oracle:

- record `openai_responses_seen`,
- run a minimal Responses request through the configured Codex path,
- read `/status` again,
- report success only when the counter increments.

Claude check must do the same with `anthropic_messages_seen`.

Acceptance: connect/check/disconnect round trips for Claude Code and Codex CLI. Desktop checks either perform a real Desktop request or report the exact manual action needed to generate one.

### 7. Codex Subscription-Native Bridge

Build: investigate and prototype a subscription-native Codex bridge separately from API/local mode. Candidate paths include Codex app-server auth surfaces, externally managed ChatGPT tokens, or a local host that speaks Codex app-server protocol while routing through RustyRed. This is not the same as forwarding ChatGPT OAuth tokens to `api.openai.com`.

Acceptance: either a subscription-backed Codex request reaches the local node and upstream without `api.responses.read/write` scope errors, or the boundary is documented with a failing proof and the product defaults to API/local mode until a supported host-token bridge exists.

## Build Table

| # | Current state | Feature | Location | Action | Desired outcome | Test |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Membrane uses directory/HTTP memory and risks private capability paths | Membrane reaches capabilities through GraphQL | `apps/theorem-proxy`, GraphQL schema modules | Build | Injection and resident calls served by schema execution, with parity proof | [-] |
| 2 | Agent and consumer profiles can drift if behavior is copied | One schema/handler contract | `rustyred-thg-mcp/src/graphql`, `apps/commonplace-api`, shared domain services | Build/verify | Capability behavior added once reaches all relevant faces | [-] |
| 3 | REST-first `/cap/<name>` could become a second contract | REST-compatible generated facade | local node HTTP surface | Build only if useful | Curl/browser routes dispatch to GraphQL and contain no behavior | [-] |
| 4 | Node lifecycle is a remembered pipeline | `theorem up`, `doctor`, `status` | `theorem` CLI | Build | One command brings the node up; status shows GraphQL, proxy, counters | [-] |
| 5 | Per-agent setup is manual and surface-specific | `theorem connect claude` and `connect codex` | `theorem` CLI | Build | One command per agent writes working reversible config | [-] |
| 6 | No verify or undo for connections | `connect --check` and `disconnect` | `theorem` CLI | Build | Check confirms routing via counters or names the break; disconnect restores | [-] |
| 7 | Codex subscription-native routing is not proven | Subscription-native bridge | `theorem` CLI / Codex app-server bridge / proxy | Research + prototype | Clear proof of support or documented boundary | [-] |

Test legend: `[-]` open, `[x]` verified against the acceptance criterion, `[~]` deferred with a reason that names a real external blocker.

## Verify First

Confirm the schema composition in `rustyred-thg-mcp/src/graphql/mod.rs`; the payload-handler or domain-service boundary each resolver wraps; the overlap and differences between the agent GraphQL profile and `apps/commonplace-api`; async-graphql in-process execution (`schema.execute`) for co-located membrane calls; the existing served GraphQL endpoint for remote membrane calls; the exact `theorem-proxy` OpenAI Responses and Anthropic Messages routes; current Codex config semantics for user-level provider routing; Claude Desktop Gateway persistence; and whether Codex app-server exposes a supported subscription-token bridge.

Build against the real source and docs, not memory. The Codex Desktop launcher trap is known: do not accept a green app launch as routing proof unless `/status.openai_responses_seen` increments. The Anthropic Messages path is currently working, but still verify it with `anthropic_messages_seen` when touching the connect/check lane.

## Where It Lands

- Canonical capability contract: GraphQL schema and shared handlers/domain services.
- Agent face: `rustyred-thg-mcp/src/graphql` via `graphql_query`, `graphql_mutate`, and `graphql_introspect`.
- Consumer face: `apps/commonplace-api`.
- Membrane through the contract: `apps/theorem-proxy`, executing GraphQL in-process when co-located and over HTTP when remote.
- REST-compatible facade: optional generated wrapper over GraphQL; no new source-of-truth logic.
- CLI lifecycle and connection management: the `theorem` CLI surface, with compatibility shims from current `theorem-proxy` commands while binary naming is reconciled.
- Subscription-native research: documented under this spec until it has a passing proof.
