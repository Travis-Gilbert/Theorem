# rustyred-thg-catalog

Relational catalog (tenants, projects, billing, auth) plus the cold index/scope rows for the storage spine, over sqlx/Postgres. The catalog stores residency and addresses, never a re-encoding of the graph.

## Key API

- `PostgresCatalog` (async): `connect(database_url)`, `with_pool(PgPool)`, `migrate()`, `upsert_tenant`/`list_tenants`, `upsert_project`; cold index async ops `record_cold`/`lookup_cold`/`remove_cold`/`record_scope`/`lookup_scope`/`remove_scope`/`cold_len`.
- `PostgresColdIndex`: a synchronous `ColdIndex` impl for the eviction hot path. Bridges to the async pool via `block_in_place` plus a captured `Handle`, so it must run inside a multi-threaded tokio runtime. `new(catalog)` / `with_handle(catalog, handle)`.
- `TenantRecord`, `ProjectRecord`.
- `CATALOG_MIGRATIONS: &[&str]`: idempotent DDL for `tenants`, `projects`, `billing_accounts`, `auth_principals` (scopes JSONB), `cold_index`, `cold_scope`.

Runtime `sqlx::query` (not compile-time `query!`), so the crate builds with no live DB and no `DATABASE_URL`. The hot native cold-index path lives in `rustyred-thg-core` (`OrderedColdIndex`, `NativeCatalog`); keep this crate for deployments that still want the external Postgres catalog bridge.

Path dep: `rustyred-thg-core`. Other: `sqlx 0.8` (postgres), `tokio`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-catalog
```

Tests: 2 offline (migrations present/idempotent-shaped; tier round-trip) plus `live_cold_index_round_trip` (`#[ignore]`, needs `THEOREM_CATALOG_TEST_URL`).

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
