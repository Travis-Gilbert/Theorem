# RustyRed dbt PG-Wire Fixture

This dbt project targets `rustyred-thg-pg-server` through the stock
`dbt-postgres` adapter. The profile reads `RUSTYRED_DBT_PG_PORT` and defaults to
`6543`; `RUSTYRED_DBT_PG_PASSWORD` is optional because the local fixture server
uses trust-mode auth.

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

The selected `stg_memory_topic_deliberate_failure` command is expected to exit
non-zero with one failing row; it proves failing dbt grounding facts are visible
through the PG-wire surface.
