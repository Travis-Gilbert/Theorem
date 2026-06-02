# Connector Transport: Live MCP -> Affordance Registration

**Status:** implementation plan, 2026-06-02. Source: [`spec.md`](spec.md) +
[`implementation-plan.md`](implementation-plan.md). This plan closes the named seam
left open by the affordances crate (`fbd71e6`):

> **Live MCP `tools/list` ingestion.** `register_connector` takes a `ConnectorManifest`
> (the normalized `{name, description, input_schema}` shape the MCP crate already emits
> via `tool_definitions`). Wiring a live MCP server's `tools/list` into a manifest is a
> thin integration in `rustyred-thg-mcp` or the Python MCP layer; this crate is the
> registry core with the manifest as the contract boundary.

**One line:** build the outbound MCP client that connects to a real MCP server, performs
the handshake, lists its tools, and feeds them through `affordances::register_connector`
so each tool becomes a learnable `Affordance` node. This is the transport half the
affordance layer needs to actually carry real connectors, not hand-fed manifests.

## What exists vs what this builds

`rustyred-thg-affordances` (`fbd71e6`) is the **substrate + learning half**: connectors
and tools as graph nodes, idempotent registration that preserves learned fitness, PPR +
cosine selection, invocation receipts, Pairformer training export. It is pure, sync, and
fully unit-tested (15 tests). Its contract boundary is `ConnectorManifest` (a handed tool
catalog).

`rustyred-thg-mcp` (`0.5.0`) is the **inbound** MCP adapter: a sync JSON-RPC dispatcher
that exposes the graph as MCP tools (`tools/list` -> `tool_definitions`, `tools/call` ->
`call_tool`). No tokio, no async, no outbound client.

This plan builds the **outbound mirror**: connect to an *external* MCP server and pull its
tools into the substrate. Same wire format as the inbound adapter, opposite direction.

## Architecture decision: a new sibling crate

`crates/rustyred-thg-connectors`, depending on `rustyred-thg-affordances` +
`rustyred-thg-core` + `serde`/`serde_json`. No tokio.

Why a new crate, not a module in `rustyred-thg-mcp` (which the seam note also allows):
- `rustyred-thg-mcp` is the inbound server (6000+ lines). Adding an outbound client +
  process spawning there mixes inbound/outbound concerns and would couple the server crate
  to `rustyred-thg-affordances`. A sibling crate keeps the dependency direction clean
  (`connectors -> affordances -> core`) and the server crate untouched.
- It mirrors the isolation discipline the affordances plan itself chose ("a new crate
  touches exactly one shared line, the workspace `members` list"). The affordances crate
  stays byte-for-byte unchanged in slice 1, so its 15 tests are unaffected.
- Folding into `rustyred-thg-mcp` later (one MCP-protocol crate, client + server) remains a
  promotion candidate; isolation wins now.

**No async, matching every sibling crate.** MCP stdio framing is newline-delimited JSON
over a child process's stdin/stdout. `std::process::Command` + `BufRead`/`Write` is enough;
the framing is generic over `BufRead + Write` so it tests over in-memory `Cursor` with no
process spawn. The codebase's pure-logic-tested + thin-I/O-shell pattern (harness-server
lib, the inbound MCP dispatcher) carries over exactly.

## Module layout

```
crates/rustyred-thg-connectors/
  Cargo.toml          deps: rustyred-thg-affordances, rustyred-thg-core, serde, serde_json
  src/lib.rs          module decls + re-exports + error type + #[cfg(test)] test decls
  src/protocol.rs     PURE: JSON-RPC envelope + MCP request builders + response parsers;
                      ToolDescriptor; parse_tools_list; ToolDescriptor -> ToolManifest;
                      connector_manifest(); parse_initialize; parse_tool_call_result
  src/transport.rs    McpTransport trait (request/notify); StdioTransport<R: BufRead, W: Write>
                      (newline-framed JSON-RPC + id correlation, skips notifications);
                      ConnectionTarget::Stdio { command, args, env }; spawn() constructor
  src/bridge.rs       connect_and_register(transport, store, server_id, tenant, label, actor):
                      initialize -> notifications/initialized -> tools/list -> manifest ->
                      affordances::register_connector
  src/tests/protocol_test.rs
  src/tests/transport_test.rs
  src/tests/bridge_test.rs
```

## The contract mapping (MCP tool -> ToolManifest)

The MCP `tools/list` result is `{ "tools": [ { "name", "description", "inputSchema" }, ... ] }`
(MCP uses camelCase `inputSchema`). Map per tool into the existing `ToolManifest`:

| MCP field | ToolManifest field |
|---|---|
| `name` | `name` (and `label` defaults to `name`) |
| `description` | `description` |
| `inputSchema` | `input_schema` |
| (none) | `permissions`, `cost`, `writeback_policy`, `tags` default empty |
| (none) | `description_embedding = None` (caller/embedder supplies later; selection degrades to structural PPR, per the affordances plan's "Text embedder" seam) |

`connector_manifest(tenant, server_id, label, descriptors)` assembles the
`ConnectorManifest` the existing `register_connector` already consumes. No change to the
affordances crate.

## Build slices (dependency order)

### Slice 1 (this plan) -- connect + register

**S1.1 protocol.rs (pure).** Acceptance:
- `tools_list_request()` / `initialize_request(client_info)` / `tools_call_request(name, args)`
  build valid JSON-RPC 2.0 request bodies (no id; the transport assigns ids).
- `parse_tools_list(result)` returns `Vec<ToolDescriptor>` from a fixture `{ "tools": [...] }`,
  tolerant of missing `description`/`inputSchema`.
- `ToolDescriptor -> ToolManifest` and `connector_manifest(...)` produce the right
  `ConnectorManifest` (name/description/input_schema mapped, embedding None).
- `parse_initialize` reads server name/version; `parse_tool_call_result` reads content +
  `isError`.

**S1.2 transport.rs.** Acceptance:
- `McpTransport::request(method, params)` over a `StdioTransport<Cursor, Vec<u8>>` writes a
  newline-framed JSON-RPC request with an incrementing id and returns the matching
  response's `result`; a JSON-RPC `error` maps to `ConnectorError`.
- Interleaved server notifications / mismatched-id lines are skipped until the matching id
  arrives (correlation correctness), tested over in-memory buffers.
- `ConnectionTarget::Stdio { command, args, env }` + `StdioTransport::spawn(target)` wires a
  child process's stdin/stdout (the only untested line; everything above tests over Cursor).

**S1.3 bridge.rs.** Acceptance:
- `connect_and_register(transport, store, server_id, tenant, label, actor)` performs
  `initialize` -> `notifications/initialized` -> `tools/list`, builds the manifest, and calls
  `affordances::register_connector`, returning its `ConnectorRegisterResult`.
- End-to-end test with a `FakeTransport` (canned `initialize` + `tools/list` results) against
  an `InMemoryGraphStore`: after the call, the graph holds one `Connector` node + one
  `Affordance` node per tool, queryable by the affordances crate's own helpers. This is the
  proof that a live `tools/list` becomes learnable affordance nodes with zero hand-authoring.

**S1.4 wiring.** Append the crate to `rustyredcore_THG/Cargo.toml` `[workspace].members`;
`cargo test -p rustyred-thg-connectors` green and `-p rustyred-thg-affordances` still green;
update `CLAUDE.md` + `AGENTS.md` crate tables.

## Named gaps / deferred slices (surfaced, not buried)

- **Invoke bridge (Phase 3 of the affordances plan).** On affordance selection, call
  `tools/call` on the owning server and feed `affordances::record_invocation`. Requires
  persisting the `ConnectionTarget` so a selected affordance can reach its server again
  (an additive optional field on `ConnectorManifest`/the `Connector` node, taken when slice 2
  lands; deferred now to keep slice 1 zero-touch on the affordances crate). **Not a silent
  cut:** slice 1 deliberately stops at registration so the learning loop can be exercised
  against real tool catalogs before wiring live invocation.
- **Driving surface.** Nothing yet exposes "connect this server" to an operator or the app.
  The home is an admin MCP tool on `rustyred-thg-mcp` (`connector/register`) or an HTTP route
  on `theorem-harness-server`. Slice 3.
- **iOS consumption.** The Connectors surface in the app stays an honest empty state until a
  driving surface + at least one registered connector exist. No connector UI ships before the
  backend is real (No-Fake-UI rule).
- **HTTP/SSE transport.** Slice 1 ships stdio (the dominant local-MCP transport). The
  `McpTransport` trait makes an HTTP/SSE transport a drop-in later.
- **Embeddings.** Tool-description embeddings are not produced here (no Rust text embedder in
  core, per the affordances plan). Affordances register with `embedding = None`; selection
  degrades to structural PPR + fitness, which is functional day one.
```
