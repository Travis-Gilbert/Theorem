# rustyred-thg-compat-server

A minimal standalone HTTP control server over the THG-Core executor, for compatibility and embedding. Hand-rolled HTTP/1.1 (no axum), thread-per-connection.

## Binary

`rustyred-thg-compat-server` (`src/main.rs`). Config from env plus `--host`/`--port` args:

- `RUSTYRED_THG_HOST` (default 127.0.0.1, or 0.0.0.0 if `PORT` set)
- `RUSTYRED_THG_PORT` / `PORT` (default 7379)
- `RUSTYRED_THG_STORE` (`memory` default, or `redis`), `RUSTYRED_THG_REDIS_URL` / `REDIS_URL`, `RUSTYRED_THG_REDIS_KEY`

Prints `RUSTYRED_THG_SERVER_READY <addr>`. Redis mode uses `RedisThgStore` plus `StoreBackedThgExecutor`; otherwise `InMemoryThgExecutor`.

## Routes

`GET /health`, `GET /ready`, `GET /v1/state/hash`, `GET /v1/runs/<run_id>`, `POST /v1/command` (`{command, args|payload}`), `POST /v1/batch` (`{commands:[...]}`).

Library entry: `serve(listener, SharedExecutor)`, `handle_http_request(raw, SharedExecutor) -> String`. Path dep: `rustyred-thg-core` (feature `redis-store`).

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-compat-server
```

Tests are inline in `lib.rs` (command endpoint and HTTP-vs-direct state-hash parity). No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
