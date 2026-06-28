# Execution Spec: Local Model Proxy (Anthropic Messages surface, native-tool membrane, ambient injection) + Install and Launch Ergonomics

Date: 2026-06-27. Register: execution. Read `CONVENTIONS.md` first; its rules apply.

## Purpose

Put the substrate on the model path. Today the harness reaches the model only as MCP tools the agent elects to call. This spec adds a local proxy that sits on every Claude Code (and any Anthropic-Messages client) turn, in both directions, on the user's own machine. It serves three new behaviors at launch: the membrane gates and samples the agent's native tool outputs (Read, Bash, Grep, Edit) before they reach the model, relevant memory and directives inject ambiently on the incoming turn, and tool-calling continues to work unchanged. It also makes the local node runnable by a stranger: a single binary, one command, point the agent at it. This is the third surface of the same inversion already shipped on the file surface (notify watcher) and the human surface (phone control): capabilities are environment, not tools.

## Governing principle

The proxy is a local Theorem node on the user's machine. The user's model credential and the model call never leave that machine. The proxy holds two separate tokens: the Anthropic credential, used to reach api.anthropic.com and never sent anywhere else, and the harness key, used to reach the Railway shared substrate for memory and nothing else. The proxy injects and gates at a cache-stable position so Anthropic's prompt cache keeps hitting. Nothing in the static request prefix (system, tools) is ever mutated.

## What exists (verified or known; do not rebuild)

- `theorem-agentd`: local daemon with an OpenAI-compatible model loop, a schema-guarded MCP tool host, a receiver sidecar, capture and relay, and a compute-offload ledger. The proxy surface lands here.
- The context membrane (`SPEC-CONTEXT-MEMBRANE-1.0`): admission gate, token budget, and deferred handles persisted byte-exact and recoverable. The web arm is `web_search_graph`. The recoverable-handle retrieval primitive is `tool_result_fetch` (fetch a byte slice of a tool result past the boundary budget). The native-tool gate extends this membrane; locate its home crate before extending.
- The copresence and watcher layers shipped this session: `theorem-copresence` presence and footprints, and the notify watcher in `commonplace-desktop-runtime` that keeps the graph current from filesystem events. The proxy serves from the graph the watcher keeps fresh.
- The shared substrate on Railway: multi-tenant memory, the affordance router (`tool_search` / `invoke`), and the federation layer. The local node connects up with the harness key.
- The harness identity onboarding: the RustyRed site issues an API key and a tenant. That key is the substrate token. It is not the Anthropic credential.

## Deliverables

### 1. Anthropic Messages surface on `theorem-agentd`
Build: a local HTTP endpoint that speaks the Anthropic Messages API (`POST /v1/messages`), streaming and non-streaming, accepts the full request body (system, messages, tools), and forwards to the configured upstream (default `https://api.anthropic.com`). The client's auth header passes through untouched, whether an `x-api-key` for an API key or the OAuth bearer for subscription passthrough. The `anthropic-beta` header is forwarded intact, including the OAuth capability required for subscription routing and the prompt-caching and tool-use betas the client sends. SSE is piped straight through, never buffered. Every field is preserved, in particular `tool_use` ids, since a stripped id breaks multi-turn tool calls.
Acceptance: Claude Code with `ANTHROPIC_BASE_URL` set to the local endpoint completes a multi-turn, tool-calling session end to end, with streaming intact and no dropped tool ids. Verify by running a real session that reads a file, runs a command, and edits, and confirming identical behavior to the direct path.

### 2. Membrane over native tool outputs
Build: intercept `tool_result` blocks in the incoming request (the agent's prior Read, Bash, Grep, Edit outputs arrive as `tool_result` content on the next request). Route them through the membrane's admission gate with smart sampling: keep error and anomaly items and a diversity-preserving subset, defer the redundant remainder behind a byte-exact recoverable handle reachable through the existing `tool_result_fetch` primitive. Predicate-style pruning runs here too, so Bash and Grep output is pruned before the model sees it. Gate list, carried from the membrane discipline: never touch user messages, never touch the system prefix or the tools array, protect the most recent turns, and fail open, so any sampling that would grow the payload returns the original.
Acceptance: a turn whose tool output is a large array reaches the model as a sampled subset with a retrieval marker, the error and anomaly items survive sampling, the model can retrieve the deferred remainder through the handle, and a payload that does not compress is passed unchanged. Verify with one large Grep result and one small one.

### 3. Ambient memory and directive injection
Build: on the incoming turn, run retrieval over the user's tenant (the index context path, `hippo_retrieve` or the equivalent) and inject the relevant memory and any active skill-pack directives at a cache-stable suffix, the latest user turn or a trailing position, never into system or tools. This is the proactive-context move: the substrate injects before the model sees the turn, with no election. Handle the watcher lag: a graph update is asynchronous to the turn, so injected state may trail the agent's own just-made edit; do not assume the graph reflects the turn in progress.
Acceptance: a turn that references prior project context receives the relevant memory injected at the suffix, the injection does not mutate the cached prefix, and a second identical request still hits the provider prompt cache. Verify the cache hit by inspecting the usage block across two requests.

### 4. Tool-call parity and tool search
Build: preserve existing MCP tool-calling through the proxy at parity. Because the base URL is non-first-party, MCP tool search is disabled by default; when the proxy forwards `tool_reference` blocks, set `ENABLE_TOOL_SEARCH=true` so discovery behaves as it does on the direct path.
Acceptance: every MCP tool that works on the direct path works through the proxy, and tool search behaves identically. Verify by exercising one MCP tool and confirming the tools list is unchanged.

### 5. Standalone install and launch ergonomics
Build: a single prebuilt binary per platform (macOS arm64, macOS x64, Linux x64, Linux arm64) published on GitHub releases, a one-line install script (`curl ... | sh`) that fetches the right binary and puts it on PATH, and a Homebrew tap. The user-facing command starts the local node and the Messages surface on localhost with zero config: a default data directory, a default port, and a configurable data path so it can point at an external volume. A convenience command sets `ANTHROPIC_BASE_URL` for a spawned Claude Code session and starts the proxy, so the user runs one command and keeps their normal Claude Code. The command surface is `rustyred proxy` to start the node and `rustyred wrap claude` for the spawn convenience; align the binary name with the repo's actual CLI entry. Runtime-asset default: a bare `rustyred proxy` runs CPU-only with no model download and no ONNX fetch on first run; ML features are opt-in. The data path and the cold tier and sidecar location are configurable to a chosen volume.
Acceptance: on a clean machine, the install script produces a working binary, `rustyred proxy` starts and serves on localhost with no downloaded assets and no config file, and `rustyred wrap claude` launches a Claude Code session already pointed at the proxy. Verify on a machine that has never run the binary, offline.

### 6. Commonplace-bundled path
Build: Commonplace ships the proxy binary as a sidecar and starts it on app launch (the Tauri sidecar or Electron child-process mechanism, whichever Commonplace uses), and a "Connect Claude Code" control writes `ANTHROPIC_BASE_URL` into the Claude Code settings or the spawned session env. The user double-clicks Commonplace and the proxy is running.
Acceptance: launching Commonplace starts the proxy with no terminal step, the Connect control points Claude Code at it, and a session runs through it. Verify by launching the app and starting a Claude Code session.

### 7. Substrate connection and credential separation
Build: the local node authenticates to the Railway shared substrate with the harness key for memory and federation calls only. The Anthropic credential is used solely to reach the upstream model endpoint and is never transmitted to Railway or logged. Two tokens, two destinations, never crossed.
Acceptance: substrate calls carry the harness key, the model call carries the Anthropic credential, and a log and traffic inspection confirms the Anthropic credential never leaves the machine except to the upstream model endpoint. Verify by inspecting outbound requests during one session.

## Build Table

| # | Current state | Feature | Location | Action | Desired outcome | Test |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | OpenAI-compatible loop only | Anthropic Messages surface, streaming + non-streaming, faithful passthrough | `theorem-agentd` | Build | Claude Code runs a full tool-calling session through localhost identical to direct | [-] |
| 2 | Membrane gates own tool JSON at MCP boundary only | Membrane over native tool outputs with smart sampling + recoverable handle | membrane crate (locate) + `tool_result_fetch` | Build | Large native tool output reaches model sampled, remainder retrievable, errors kept, fail-open | [-] |
| 3 | Memory reached only by elected MCP call | Ambient memory + directive injection at cache-stable suffix | `theorem-agentd` | Build | Relevant memory injected pre-model, prefix cache still hits | [-] |
| 4 | MCP tools called directly | Tool-call parity through proxy + `ENABLE_TOOL_SEARCH` | `theorem-agentd` | Build | Every MCP tool and tool search behaves as on direct path | [-] |
| 5 | No documented local run path | Standalone binary, curl install, brew tap, `rustyred proxy` / `rustyred wrap claude`, CPU-only default | release CI + CLI entry | Build | Stranger installs a binary, runs one command, points agent at localhost, offline | [-] |
| 6 | Commonplace ships without the proxy | Sidecar + auto-launch + Connect Claude Code control | `commonplace-desktop-runtime` + Commonplace app | Build | Double-click app, proxy running, one-click connect | [-] |
| 7 | Single-token assumptions | Two-token separation: Anthropic local-only, harness key to Railway | `theorem-agentd` | Build | Anthropic credential never leaves the machine except to upstream model | [-] |

Test legend: `[-]` open, `[x]` verified against the acceptance criterion, `[~]` deferred with a reason that names a real external blocker.

## Verify first

Confirm against current source and docs before building: the Anthropic Messages SSE event contract and the current `anthropic-beta` values Claude Code sends (including the OAuth capability flag for subscription); `theorem-agentd`'s current HTTP surface and where the OpenAI loop lives, so the Messages endpoint sits beside it cleanly; the home crate of the context membrane and the exact `tool_result_fetch` signature; the `ENABLE_TOOL_SEARCH` behavior under a non-first-party base URL and the `tool_reference` forwarding contract; and the sidecar mechanism Commonplace uses (Tauri sidecar versus Electron child process). Build against the current contracts, not assumed ones.

## What lands on this once it exists (each its own spec)

These build on the proxy and are named here so the launch boundary is clear; they are not part of this spec's deliverables.

- Transparent affordance execution: the proxy injects the harness affordances into the tools array and resolves them itself against the substrate, returning the result to context, so users get harness tools without installing the MCP.
- The cascade: route easy turns to the local Gemma or a cheap tier and escalate on isotonic-calibrated token-level confidence, transparent to the client. Gated on calibration data.
- Verification offload: the substrate checks the model's output against the graph and via Datalog and Z3 and injects corrections, advisory-first.
- The event-scored corpus: the proxy and watcher capture every session, segment at feedback boundaries into postmortem and solution atoms, weight by advantage within the neighborhood, and connect tools, skills, and outcomes in the behavior graph for the Pairformer and the GNNs. The proxy is the capture apparatus; this spec ships the capture-capable path, the scoring layer is separate.

## Where it lands

- Messages surface, ambient injection, tool parity, credential separation: `theorem-agentd`.
- Native-tool membrane gate: the context membrane's home crate, extending the admission path and reusing `tool_result_fetch`.
- Standalone install: release CI cross-compile matrix and the CLI entry binary.
- Commonplace sidecar and Connect control: `commonplace-desktop-runtime` and the Commonplace app.
- Substrate connection: the harness-key client path in `theorem-agentd` to the Railway substrate.
