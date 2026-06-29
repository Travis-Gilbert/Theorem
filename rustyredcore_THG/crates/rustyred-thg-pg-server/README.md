# rustyred-thg-pg-server

Postgres wire-protocol server for RustyRedCore-THG native views. SQL lands as a real Postgres client would send it; the relational surface lowers native `SELECT` queries to the core planner and serves the dbt Postgres adapter's catalog, materialization, grounding, and lineage paths.

## Binary

`rustyred-thg-pg-server` (`src/main.rs`). Bind env `RUSTYRED_THG_PG_ADDR` (default `127.0.0.1:6543`); prints `RUSTYRED_THG_PG_READY <addr>`. Mode env `RUSTYRED_THG_PG_MODE`:

- `relational` (default): planner-backed native views via `serve_relational` over `demo_native_store()`.
- `executor`: the legacy `ThgExecutor` command surface via `serve`.

Trust-mode auth (no password); SSL/GSS requests are answered `N`.

## Two surfaces

- `relational_server.rs` (the spec surface): `serve_relational(listener, SharedRelationalStore)`, `execute_native_sql(sql, &RelationalStore) -> Result<PgQueryResult, PgError>`, `execute_dbt_sql(sql, &mut RelationalStore) -> Result<PgQueryResult, PgError>`, `demo_native_store()`. Parses with `sqlparser`, lowers native `SELECT` queries to planner `QueryIr`, executes via `execute_query`, and handles dbt's adapter SQL around that planner core. Supports simple and extended protocol (Parse/Bind/Describe/Execute/Sync), INNER JOIN on equality, WHERE (eq/range/BETWEEN/prefix-LIKE), `ORDER BY`/`LIMIT`/`OFFSET`/`GROUP BY` with `count/sum/min/max/avg` (incl. `count(DISTINCT)`), and `EXPLAIN <select>` (shows the access path, e.g. `time_series`). Real pg OIDs (text/int8/float8/bool). Of the modality predicates, only `time_range(col, lo, hi)` is indexed; `knn`/`geo_within`/`text_match` are recognized but rejected rather than returning unfiltered rows.
- `lib.rs` (the executor surface): `serve(listener, SharedExecutor)`, `execute_executor_sql`, `describe_executor_sql`, `execute_relational_sql(sql, &RelationalStore)`, `PgColumn`, `PgQueryResult`. Supports `SELECT state_hash()`, constant SELECTs, and `SELECT ... FROM nodes|graph_nodes [WHERE label=..] [LIMIT n]`.

## dbt target

The relational surface answers the dbt Postgres adapter's connection checks, catalog introspection, table/view materialization, incremental append path, and generic tests. dbt-built relations land as relational-store facts; receipt relations expose the proof trail:

- `dbt_grounding_facts`: dbt generic test receipts with `model_id`, `assertion`, `outcome`, and `observed_count`.
- `dbt_model_lineage`: dbt model/source lineage.
- `dbt_substrate_provenance`: substrate materialization provenance beside the dbt lineage.

The fixture project in `dbt_project/` targets the local PG-wire endpoint through the stock `dbt-postgres` adapter:

```bash
cd rustyredcore_THG
RUSTYRED_THG_PG_ADDR=127.0.0.1:6543 cargo run -p rustyred-thg-pg-server

cd crates/rustyred-thg-pg-server/dbt_project
RUSTYRED_DBT_PG_PORT=6543 dbt debug --profiles-dir profiles
RUSTYRED_DBT_PG_PORT=6543 dbt parse --profiles-dir profiles
RUSTYRED_DBT_PG_PORT=6543 dbt run --profiles-dir profiles
RUSTYRED_DBT_PG_PORT=6543 dbt test --profiles-dir profiles --exclude stg_memory_topic_deliberate_failure
RUSTYRED_DBT_PG_PORT=6543 dbt test --profiles-dir profiles --select stg_memory_topic_deliberate_failure
```

The selected `stg_memory_topic_deliberate_failure` test is expected to fail with one row; it verifies that failing grounding facts are recorded.

Path dep: `rustyred-thg-core` (feature `redis-store`). Other: `postgres-types`, `sqlparser 0.62`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-pg-server
RUSTYRED_THG_PG_ADDR=127.0.0.1:6543 cargo run -p rustyred-thg-pg-server
```

`tests/pg_wire_acceptance.rs` drives a real `tokio-postgres` client against an in-process server (no `#[ignore]`).

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
