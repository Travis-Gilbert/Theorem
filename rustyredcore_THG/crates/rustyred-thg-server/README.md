# rustyred-thg-server

Product HTTP/gRPC/MCP surface over RustyRedCore-THG: tenant graph routes, query/Cypher APIs, harness coordination, browser actions, fractal expansion, and TTL sweeping.

## What it is

`rustyred-thg-server` is the product server for the RustyRed/THG graph substrate. It builds the Axum router, merges the gRPC routes, owns `AppState`, runs the TTL sweep loop, and hosts the higher-level tenant surfaces that sit over `rustyred-thg-core`, `rustyred-thg-mcp`, `theorem-harness-runtime`, RustyWeb, and the browser/fractal layers.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-server
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
