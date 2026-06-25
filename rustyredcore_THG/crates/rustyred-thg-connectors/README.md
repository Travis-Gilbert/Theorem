# rustyred-thg-connectors

Live MCP connector transport: connect to an external MCP server, list its tools, register each as a learnable `Affordance` graph node, and invoke a selected tool under policy. The outbound mirror of `rustyred-thg-mcp` (which exposes the graph as MCP tools). Sync and tokio-free.

## Key API

- Protocol (`protocol.rs`): `initialize_params`, `tools_list_params`, `tools_call_params`, `parse_initialize`/`InitializeInfo`, `parse_tools_list`/`ToolDescriptor`, `parse_tool_call_result`/`ToolCallOutcome`, `tool_manifest_from_descriptor`, `writeback_policy_from_hints` (MCP `readOnlyHint`/`destructiveHint` to `read-only`/`destructive`/`write`/`unknown`; un-annotated maps to `unknown`).
- Transport (`transport.rs`): `McpTransport` trait, `StdioTransport<R,W>` (newline-delimited JSON over a child's stdin/stdout) with `spawn_stdio`, and `HttpTransport` (with SSE) via `connect_http`. `connect_transport`, `ConnectedTransport`, `ConnectionTarget`, `ConnectorAuth`.
- Bridge (`bridge.rs`): `connect_and_register`, `connect_and_register_with_target` (persists the reach on the `Connector` node), `connect_target`.
- Invoke (`invoke.rs`): `InvokePolicy` (`DryRun` default, `FireAllowlist(Vec<String>)`), `plan_invocation` (store reads only), `fire_over_transport`, `invoke_affordance` (full gated bridge).
- Errors: `ConnectorError`.

Path deps: `rustyred-thg-affordances`, `rustyred-thg-core`, `ureq` (rustls).

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-connectors
```

Tests under `src/tests/` (protocol, transport, bridge, invoke) are in-memory/fake-transport. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
