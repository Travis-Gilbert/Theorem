# rustyred-thg-server

The product HTTP, gRPC, and MCP surface over the RustyRed/THG substrate: tenant graph routes, query/Cypher, vector/full-text/spatial, harness coordination, browser actions, fractal expansion, and TTL sweeping. This is the release binary; the repo `Dockerfile` builds only this crate.

## Binaries

- `rustyred-thg-server` (`src/main.rs`): builds the axum router, merges the tonic gRPC routes onto the same TCP listener (sniffs `application/grpc*` to gRPC, everything else to HTTP), spawns the TTL sweep and the coordination wake listener, and shuts down gracefully on SIGTERM/Ctrl-C. Prints `RUSTYRED_THG_PRODUCT_READY <addr>`.
- `rustyred-thg-upgrade-format` (`src/bin/upgrade_format.rs`): migrates the RedCore on-disk format `0 -> CURRENT_FORMAT_VERSION` per tenant. `rustyred-thg-upgrade-format <data-dir> [--dry-run]`.

## Library

`lib.rs` exposes `config`, `router`, `state`; re-exports `Config` and `AppState`; and `serve_loopback(Config, oneshot::Receiver<()>)`, the embedded loopback mode the Tauri desktop app uses instead of spawning the binary.

## Config and ports

`Config::from_env` (`src/config.rs`). Each key has both a `RUSTY_RED_*` and a `RUSTYRED_THG_PRODUCT_*` (or `RUSTYRED_THG_*`) alias.

- Bind: `PORT` / `RUSTY_RED_PORT` (default 8380); host `RUSTY_RED_HOST` (default 127.0.0.1, or 0.0.0.0 when `PORT` is set).
- Storage: `RUSTY_RED_MODE` (embedded default / memory / redis), durability (`AofEverysec` default), snapshot interval, strict-ACID gates (strict-ACID requires embedded + aof_always + single-writer + serializable), tenant memory quota.
- Auth: `RUSTY_RED_REQUIRE_AUTH` (default true), `RUSTY_RED_API_TOKENS` (ed25519). CORS via `RUSTY_RED_ALLOWED_ORIGINS`.
- MCP: `RUSTY_RED_MCP_ENABLED` (default true), read-only, allow-admin, default-tenant, graphql-default-surface.
- TTL sweep: `RUSTYRED_THG_TTL_SWEEP_MS` (default 1000, must be > 0).

## Surface

- HTTP: tenant graph routes (`/v1/tenants/:tenant_id/...`), `/v1/query`, `/v1/cypher` (pest-based, `/explain`), `/v1/transactions/{begin,commit,rollback}`, `/v1/cache/*`, `/search*`, `/crawl`, `/federate/submit`, `/mcp`, `/metrics`, `/healthz`, `/ready`, `/.well-known/mcp/rustyred_thg.json`, `/.well-known/agent.json`, and more.
- gRPC: `rustyred.v1.GraphDatabase` (proto compiled at build time from `vendor/proto/rustyred/v1/rustyred.proto`), served on the shared port.

Path deps span the substrate: `rustyred-thg-core`, `rustyred-thg-mcp`, `theorem-harness-{core,runtime}`, `theorem-dispatch`, `theorem-browser-agent`, `rustyred-thg-{adapters,affordances,connectors,fractal}`, `rustyred-web`, `rustyred-hipporag`, `rustyred-membrane`, `rustyred-rerank`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-server
PORT=8380 cargo run -p rustyred-thg-server
```

Tests are module-level (`config.rs` validation/tenant-overlay, etc.); there is no top-level `tests/` dir.

Part of the `rustyredcore_THG` workspace. Has its own `Dockerfile` and `railway.toml`. See [the workspace README](../../README.md) for the crate map.
