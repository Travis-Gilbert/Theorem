# theorem-proxy

A local Theorem node that sits on the model path. It speaks the Anthropic Messages
API for Claude clients and the OpenAI Responses API for Codex clients on
`localhost`, so sessions can route through RustyRed/proxy while the substrate
injects memory and coordination ambiently, with no MCP tool calls. This is the
third surface of the harness inversion (file watcher, phone control, now the
model path): capabilities become products of the environment, not tools the
agent elects to call.

## Status

**Deliverable 1 shipped** (`SPEC-LOCAL-PROXY-MVP` D1): faithful passthrough plus
Codex/OpenAI sibling surface.
`POST /v1/messages` forwards to the upstream (default `https://api.anthropic.com`),
streaming and non-streaming, with headers, body, and the SSE event stream preserved
byte-for-byte -- nothing is parsed or mutated, so `tool_use` ids, the
`anthropic-beta` header (incl. the OAuth subscription capability), and prompt-cache
breakpoints all survive. `GET /v1/models` is forwarded too, so Desktop gateway
clients can discover models.

`POST /v1/responses` forwards to the OpenAI upstream (default
`https://api.openai.com`) for Codex. Ambient memory injection appends to the latest
Responses `input` user item and preserves the prefix. `/openai/v1/models` is the
unambiguous OpenAI model-discovery route if a client needs it; `/v1/models` remains
the Claude Desktop gateway route. Tested against mock upstreams; the full live
acceptance (a real multi-turn tool-calling session) is a manual run -- see below.

## Run it

```bash
theorem-proxy proxy            # serves on http://127.0.0.1:8788
# in another shell:
export ANTHROPIC_BASE_URL=http://127.0.0.1:8788
claude                         # this session now routes through the proxy
```

`--port` and `--upstream` are configurable. CPU-only, no model download, no config
file.

### Codex Responses mode

Codex uses the OpenAI Responses surface. Start the proxy, then point Codex at the
proxy's OpenAI-compatible `/v1` base URL for that process:

```bash
theorem-proxy proxy --memory-url http://127.0.0.1:8380/mcp
codex -c 'openai_base_url="http://127.0.0.1:8788/v1"'
```

This repo also has a one-shot launcher:

```bash
apps/theorem-proxy/scripts/start-proxied-codex-session.sh
```

For Codex Desktop, the app opener starts or reuses the local RustyRed node and
proxy, then opens this workspace:

```bash
apps/theorem-proxy/scripts/start-proxied-codex-app.sh --restart-codex
```

The `--restart-codex` flag quits Codex first so the workspace open happens from
a clean app process. On current Codex Desktop 0.142.3, `codex app -c openai_base_url=...`
does not route model traffic. Desktop routing belongs in the GraphQL-contract-first
`theorem connect codex` path, which will write reversible user-level Codex config
and verify `/status.openai_responses_seen`. You can also double-click the
transitional opener:

```text
apps/theorem-proxy/scripts/RustyRed-Codex.command
```

By default, Codex keeps using its normal OpenAI auth and the proxy forwards that
credential only to `https://api.openai.com`. For sidecar/local-key modes, set the
proxy-owned upstream key and let the client send a harmless local bearer:

```bash
export THEOREM_PROXY_OPENAI_UPSTREAM_API_KEY=...
theorem-proxy proxy --memory-url http://127.0.0.1:8380/mcp
```

### Resident provider participants

Qwen and Mistral can also be loaded as room participants for the harness runner,
separate from routing Codex or Claude through the proxy. Keep the private keys in
`~/.theorem-proxy/qwen.env` and `~/.theorem-proxy/mistral.env`, then source:

```bash
source apps/theorem-proxy/scripts/agent-room-participants-env.sh
```

The helper exports `THEOREM_AGENT_HEADS=qwen,mistral`,
`THEOREM_AGENT_ROOM_RUNNER=1`, `QWEN_MODEL=qwen3.7-max`, and
`MISTRAL_MODEL=mistral-small-latest` for the current shell. Start the harness
server or local room from that same shell; a room mention to `@theorem` can then
wake the configured binding and publish a room contribution.

### Claude Desktop gateway mode

Claude Desktop does not use `ANTHROPIC_BASE_URL` for its ordinary chat surface. Use
Claude Desktop 3P gateway mode and point its gateway base URL at:

```text
http://127.0.0.1:8788
```

Use a harmless local gateway key in Desktop, then start the proxy with the real
upstream credential in the proxy environment. The proxy strips Desktop's local key
before forwarding:

```bash
export THEOREM_PROXY_UPSTREAM_API_KEY=...
theorem-proxy proxy --memory-url http://127.0.0.1:8380/mcp
```

For a Claude subscription/OAuth token created with `claude setup-token`, use:

```bash
export THEOREM_PROXY_UPSTREAM_AUTH_TOKEN=...
export THEOREM_PROXY_UPSTREAM_BETA=oauth-2025-04-20
theorem-proxy proxy --memory-url http://127.0.0.1:8380/mcp
```

Keep those upstream credentials out of Desktop config and project files.

This repo also has a launcher that starts the local RustyRed node and proxy for
Desktop:

```bash
export THEOREM_PROXY_UPSTREAM_API_KEY=...
apps/theorem-proxy/scripts/start-desktop-gateway.sh
```

## Where it sits (the spec stack)

- `SPEC-LOCAL-PROXY-MVP.md` -- the governing spec. D1 (this passthrough) is shipped;
  D2 (native-tool membrane), D3 (ambient memory + directive injection, the value),
  D5 (brew/curl install), D6 (Commonplace sidecar) build on it.
- `SPEC-PROXY-RESIDENT-CAPABILITIES.md` -- transparent affordance execution, the
  cascade, and verification offload, once the proxy exists.
- `docs/plans/local-proxy/SPEC-PROXY-PROVE-AND-PRUNE.md` -- the five changes that make
  the harness ambient *well* and *measurable*: relevance-ranked memory injection
  (retire wholesale `MEMORY.md`, remove `recall`), tool-surface pruning, proxy-mediated
  proactive coordination, staleness-aware memory + memory CI, and built-in measurement
  with a value readout in `theorem-proxy doctor`.
- `SPEC-ONECLICK-ONBOARDING.md` -- the one-click distribution (brew, site,
  `theorem-proxy doctor`, and account login) around the binary; chrome that needs this binary first.

## Notes

- Standalone Cargo root (bare `[workspace]`), like `apps/theorem-grpc`. Isolated from
  the `rustyredcore_THG` workspace; builds independently.
- Binary naming (`theorem` vs `rustyred`) is unreconciled across the specs; the crate
  is `theorem-proxy` for now and trivially renamed.
