# rustyred-thg-pg-server

Postgres wire-protocol server for RustyRedCore-THG native views. SQL lands as a real `tokio-postgres` client would send it; the relational surface lowers it to the core planner.

## Binary

`rustyred-thg-pg-server` (`src/main.rs`). Bind env `RUSTYRED_THG_PG_ADDR` (default `127.0.0.1:6543`); prints `RUSTYRED_THG_PG_READY <addr>`. Mode env `RUSTYRED_THG_PG_MODE`:

- `relational` (default): planner-backed native views via `serve_relational` over `demo_native_store()`.
- `executor`: the legacy `ThgExecutor` command surface via `serve`.

Trust-mode auth (no password); SSL/GSS requests are answered `N`.

## Two surfaces

- `relational_server.rs` (the spec surface): `serve_relational(listener, SharedRelationalStore)`, `execute_native_sql(sql, &RelationalStore) -> Result<PgQueryResult, PgError>`, `demo_native_store()`. Parses with `sqlparser`, lowers to planner `QueryIr`, executes via `execute_query`. Supports simple and extended protocol (Parse/Bind/Describe/Execute/Sync), INNER JOIN on equality, WHERE (eq/range/BETWEEN/prefix-LIKE), `ORDER BY`/`LIMIT`/`OFFSET`/`GROUP BY` with `count/sum/min/max/avg` (incl. `count(DISTINCT)`), and `EXPLAIN <select>` (shows the access path, e.g. `time_series`). Real pg OIDs (text/int8/float8/bool). Of the modality predicates, only `time_range(col, lo, hi)` is indexed; `knn`/`geo_within`/`text_match` are recognized but rejected rather than returning unfiltered rows.
- `lib.rs` (the executor surface): `serve(listener, SharedExecutor)`, `execute_executor_sql`, `describe_executor_sql`, `execute_relational_sql(sql, &RelationalStore)`, `PgColumn`, `PgQueryResult`. Supports `SELECT state_hash()`, constant SELECTs, and `SELECT ... FROM nodes|graph_nodes [WHERE label=..] [LIMIT n]`.

Path dep: `rustyred-thg-core` (feature `redis-store`). Other: `postgres-types`, `sqlparser 0.62`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-pg-server
RUSTYRED_THG_PG_ADDR=127.0.0.1:6543 cargo run -p rustyred-thg-pg-server
```

`tests/pg_wire_acceptance.rs` drives a real `tokio-postgres` client against an in-process server (no `#[ignore]`).

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
