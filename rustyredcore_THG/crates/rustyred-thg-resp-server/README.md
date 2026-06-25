# rustyred-thg-resp-server

A RESP/Redis-protocol server exposing the core's scoped sorted-set commands over `OrderedIndexRegistry`. Binary-only (no lib target).

## Binary

`rustyred-thg-resp-server` (`src/main.rs`). Bind env `RUSTYRED_THG_RESP_ADDR` (default `127.0.0.1:6380`); prints `RUSTYRED_THG_RESP_READY <addr>`. Tokio multi-threaded, one task per connection. Reads both the RESP array form (`*N`/`$len`) and the whitespace command form.

## Commands

`execute_resp_command` (`protocol.rs`) implements `PING`, `ZADD`, `ZSCORE`, `ZPOPMIN`, `ZPOPMAX`, `ZRANGEBYSCORE` (`WITHSCORES`, `LIMIT offset count`), `ZREM`, `ZCARD`, `ZRANK`. Scores parse `-inf`/`+inf`/`inf` and reject NaN. Unknown commands return an explicit "scoped ZSET commands only" error. `RespValue` carries `encode()`.

Path dep: `rustyred-thg-core` (feature `redis-store`). Other: `redis-protocol 5`, `tokio`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-resp-server
RUSTYRED_THG_RESP_ADDR=127.0.0.1:6380 cargo run -p rustyred-thg-resp-server
```

Tests are inline in `protocol.rs`. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
