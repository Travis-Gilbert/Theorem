# Connect a client over MCP

The primary way to use Theorem's Harness is as an [MCP](https://modelcontextprotocol.io) server. An MCP-capable client (Claude Code, Codex, the claude.ai connector) connects once and gets the whole [tool catalog](../reference/mcp-tools.md) — memory, coordination, jobs, graph queries, code intelligence, and browsing.

## Two ways to connect

**Over stdio (local agents).** The Harness MCP server is the `rustyred-thg-mcp` crate. The packaged distribution is the `theorems-harness` plugin, which bundles the MCP server together with its skills and commands; pointing a local MCP client at that plugin is the simplest path and is how Claude Code and Codex consume the Harness day to day.

**Over HTTP (remote clients).** The graph engine exposes a streamable MCP endpoint at `POST /mcp` (JSON-RPC). Two discovery manifests advertise it:

```bash
curl localhost:7777/.well-known/mcp/rustyred_thg.json   # tools, resources, capabilities
curl localhost:7777/.well-known/agent.json              # agent integration manifest
```

Either way, the client performs the standard MCP handshake (`initialize` -> `notifications/initialized` -> `tools/list`) and the catalog appears.

## What you get

After `tools/list`, the tools in the [catalog](../reference/mcp-tools.md) are callable. The headline groups:

- **Memory** — `remember`, `recall`, `encode` ([guide](persist-memory.md)).
- **Coordination** — `coordination_room`, `coordination_intent`, `coordinate`, `mentions` ([guide](coordinate-agents.md)).
- **Jobs** — `job_submit`, `job_list`, `job_note`, `job_archive`.
- **Code intelligence** — `compute_code`, `code_ingest`, `harness_kg_*`.
- **Graph** — neighbors, algorithms, vector/full-text/spatial search, versioning.

A tenant scopes everything; tools take a `tenant` (or `tenant_slug`) argument, defaulting to `default`.

## Read-only mode

The MCP server can run read-only. In that mode the read tools work, but any write tool (`remember`, `coordinate`, `coordination_intent`, `job_submit`, ...) returns a structured `mcp_read_only` error instead of mutating the graph. This is the safe default for exposing the server to an untrusted client; enable writes deliberately.

## Make external tools learnable

The Harness can also consume *other* MCP servers and turn their tools into learnable affordances — graph nodes the system tracks and learns to select. Register one with the harness server:

```bash
curl -s localhost:50080/connectors/register \
  -H 'content-type: application/json' \
  -d '{ "tenant": "default", "server_id": "my-tools",
        "target": { "type": "stdio", "command": "my-mcp-server", "args": [] } }'
```

The harness spawns the target, runs the MCP handshake, lists its tools, and registers each as an affordance under `(tenant, server_id)`. List them back with `GET /connectors?tenant=default`. Over time the system learns which affordance to reach for from invocation outcomes. See the [first-job guide](first-job.md) for the full connector flow.

## Which surface should I use?

- **MCP** (this page) — an agent client with tools wired in. The default.
- **HTTP** — a language-agnostic network API. See the [HTTP API](../reference/api-http.md).
- **SDK** — embed the harness in-process in Rust, Node, or Swift. See the [SDKs](../reference/sdks.md).
