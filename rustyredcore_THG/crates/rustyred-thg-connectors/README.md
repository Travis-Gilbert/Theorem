# rustyred-thg-connectors

Live MCP connector transport: connect to an external MCP server, list its tools, and register them as learnable Affordance graph nodes via rustyred-thg-affordances.

## What it is

Live MCP connector transport.

The outbound mirror of `rustyred-thg-mcp` (the inbound adapter that exposes
the graph as MCP tools): connect to an *external* MCP server, perform the
handshake, list its tools, and feed them through
`rustyred_thg_affordances::register_connector` so each tool becomes a
learnable `Affordance` graph node. This is the transport half the affordance
layer needs to carry real connectors instead of hand-fed manifests.

Sync and tokio-free, matching every sibling crate. MCP stdio framing is
newline-delimited JSON over a child process's stdin/stdout, so the protocol
layer (`protocol`) is pure and the transport (`transport`) is a thin
`BufRead + Write` shell. Plan: docs/plans/mcp-learning-layer/connector-transport-plan.md.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-connectors
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
