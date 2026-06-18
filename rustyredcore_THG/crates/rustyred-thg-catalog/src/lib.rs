//! Relational catalog and cold index for the RustyRedCore-THG storage spine.
//!
//! The cold tier (the content-addressed object store in `rustyred-thg-core`)
//! holds the cold tail's payloads; THIS crate is the catalog that says which
//! ids live in which tier and at which object hash, plus the administrative
//! rows (tenants, projects, billing, auth). Per the storage-spine handoff the
//! catalog is a catalog: it stores residency and addresses, never a re-encoding
//! of the graph's nodes and edges. The cold index makes rehydration a keyed
//! lookup (id -> hash -> object), never a scan of the versioned-graph repository.
//!
//! Backing is sqlx on rust-postgres (Loco rejected; SeaORM is the escape hatch).
//! Queries are the runtime API (`sqlx::query`), not the compile-time `query!`
//! macro, so the crate builds with NO live database and no `DATABASE_URL` -- the
//! schema is validated against a real Postgres by the `#[ignore]` integration
//! tests when one is available.
//!
//! [`PostgresCatalog`] is the async surface. [`PostgresColdIndex`] is a sync
//! adapter implementing `rustyred_thg_core::ColdIndex` so the synchronous
//! eviction hot path can use Postgres directly; it bridges to the async pool via
//! `block_in_place`, so it must be constructed from (and called within) a
//! multi-threaded tokio runtime -- which is how the THG server runs.

use rustyred_thg_core::{
    ColdIndex, ColdIndexEntry, ColdScopeEntry, ColdTierKind, GraphStoreError, GraphStoreResult,
};
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;
use tokio::runtime::Handle;

/// DDL for the catalog. Idempotent (`CREATE TABLE IF NOT EXISTS`), applied in
/// order by [`PostgresCatalog::migrate`]. The cold index and cold scope tables
/// are the spine's load-bearing rows; the rest are the administrative catalog.
pub const CATALOG_MIGRATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS tenants (
        tenant_id TEXT PRIMARY KEY,
        slug TEXT NOT NULL,
        display_name TEXT,
        created_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )",
    "CREATE TABLE IF NOT EXISTS projects (
        tenant_id TEXT NOT NULL REFERENCES tenants(tenant_id) ON DELETE CASCADE,
        project_slug TEXT NOT NULL,
        display_name TEXT,
        created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
        PRIMARY KEY (tenant_id, project_slug)
    )",
    "CREATE TABLE IF NOT EXISTS billing_accounts (
        tenant_id TEXT PRIMARY KEY REFERENCES tenants(tenant_id) ON DELETE CASCADE,
        plan TEXT NOT NULL DEFAULT 'free',
        status TEXT NOT NULL DEFAULT 'active',
        updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )",
    "CREATE TABLE IF NOT EXISTS auth_principals (
        principal_id TEXT PRIMARY KEY,
        tenant_id TEXT NOT NULL REFERENCES tenants(tenant_id) ON DELETE CASCADE,
        kind TEXT NOT NULL,
        token_hash TEXT,
        scopes JSONB NOT NULL DEFAULT '[]'::jsonb,
        created_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )",
    "CREATE TABLE IF NOT EXISTS cold_index (
        id TEXT PRIMARY KEY,
        scope TEXT NOT NULL,
        tier TEXT NOT NULL,
        object_hash TEXT NOT NULL,
        commit_hash TEXT,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )",
    "CREATE INDEX IF NOT EXISTS cold_index_scope_idx ON cold_index (scope)",
    "CREATE TABLE IF NOT EXISTS cold_scope (
        scope TEXT PRIMARY KEY,
        commit_hash TEXT NOT NULL,
        node_ids JSONB NOT NULL,
        edge_ids JSONB NOT NULL,
        parked BOOLEAN NOT NULL DEFAULT true,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )",
];

/// A tenant row (the top of the multi-tenant catalog).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TenantRecord {
    pub tenant_id: String,
    pub slug: String,
    pub display_name: Option<String>,
}

/// A project row, the `project_slug` anchor scoped to a tenant.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProjectRecord {
    pub tenant_id: String,
    pub project_slug: String,
    pub display_name: Option<String>,
}

/// The async catalog surface over a Postgres pool.
#[derive(Clone, Debug)]
pub struct PostgresCatalog {
    pool: PgPool,
}

impl PostgresCatalog {
    /// Connect with a small pool. `database_url` is a standard Postgres URL.
    pub async fn connect(database_url: &str) -> GraphStoreResult<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(pg_err("connect"))?;
        Ok(Self { pool })
    }

    /// Wrap an existing pool (so the catalog shares the server's pool).
    pub fn with_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Apply [`CATALOG_MIGRATIONS`] (idempotent).
    pub async fn migrate(&self) -> GraphStoreResult<()> {
        for statement in CATALOG_MIGRATIONS {
            sqlx::query(statement)
                .execute(&self.pool)
                .await
                .map_err(pg_err("migrate"))?;
        }
        Ok(())
    }

    pub async fn upsert_tenant(&self, tenant: &TenantRecord) -> GraphStoreResult<()> {
        sqlx::query(
            "INSERT INTO tenants (tenant_id, slug, display_name) VALUES ($1, $2, $3)
             ON CONFLICT (tenant_id) DO UPDATE SET slug = EXCLUDED.slug, display_name = EXCLUDED.display_name",
        )
        .bind(&tenant.tenant_id)
        .bind(&tenant.slug)
        .bind(&tenant.display_name)
        .execute(&self.pool)
        .await
        .map_err(pg_err("upsert_tenant"))?;
        Ok(())
    }

    pub async fn list_tenants(&self) -> GraphStoreResult<Vec<TenantRecord>> {
        let rows =
            sqlx::query("SELECT tenant_id, slug, display_name FROM tenants ORDER BY tenant_id")
                .fetch_all(&self.pool)
                .await
                .map_err(pg_err("list_tenants"))?;
        rows.into_iter()
            .map(|row| {
                Ok(TenantRecord {
                    tenant_id: row.try_get("tenant_id").map_err(pg_err("row"))?,
                    slug: row.try_get("slug").map_err(pg_err("row"))?,
                    display_name: row.try_get("display_name").map_err(pg_err("row"))?,
                })
            })
            .collect()
    }

    pub async fn upsert_project(&self, project: &ProjectRecord) -> GraphStoreResult<()> {
        sqlx::query(
            "INSERT INTO projects (tenant_id, project_slug, display_name) VALUES ($1, $2, $3)
             ON CONFLICT (tenant_id, project_slug) DO UPDATE SET display_name = EXCLUDED.display_name",
        )
        .bind(&project.tenant_id)
        .bind(&project.project_slug)
        .bind(&project.display_name)
        .execute(&self.pool)
        .await
        .map_err(pg_err("upsert_project"))?;
        Ok(())
    }

    // ---- cold index (async) -------------------------------------------------

    pub async fn record_cold(&self, entry: &ColdIndexEntry) -> GraphStoreResult<()> {
        sqlx::query(
            "INSERT INTO cold_index (id, scope, tier, object_hash, commit_hash, updated_at)
             VALUES ($1, $2, $3, $4, $5, now())
             ON CONFLICT (id) DO UPDATE SET scope = EXCLUDED.scope, tier = EXCLUDED.tier,
                 object_hash = EXCLUDED.object_hash, commit_hash = EXCLUDED.commit_hash,
                 updated_at = now()",
        )
        .bind(&entry.id)
        .bind(&entry.scope)
        .bind(tier_str(entry.tier))
        .bind(&entry.object_hash)
        .bind(&entry.commit_hash)
        .execute(&self.pool)
        .await
        .map_err(pg_err("record_cold"))?;
        Ok(())
    }

    pub async fn lookup_cold(&self, id: &str) -> GraphStoreResult<Option<ColdIndexEntry>> {
        let row = sqlx::query(
            "SELECT id, scope, tier, object_hash, commit_hash FROM cold_index WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(pg_err("lookup_cold"))?;
        row.map(|row| {
            Ok(ColdIndexEntry {
                id: row.try_get("id").map_err(pg_err("row"))?,
                scope: row.try_get("scope").map_err(pg_err("row"))?,
                tier: parse_tier(&row.try_get::<String, _>("tier").map_err(pg_err("row"))?),
                object_hash: row.try_get("object_hash").map_err(pg_err("row"))?,
                commit_hash: row.try_get("commit_hash").map_err(pg_err("row"))?,
            })
        })
        .transpose()
    }

    pub async fn remove_cold(&self, id: &str) -> GraphStoreResult<()> {
        sqlx::query("DELETE FROM cold_index WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(pg_err("remove_cold"))?;
        Ok(())
    }

    pub async fn record_scope(&self, entry: &ColdScopeEntry) -> GraphStoreResult<()> {
        let node_ids = serde_json::to_value(&entry.node_ids).map_err(json_err)?;
        let edge_ids = serde_json::to_value(&entry.edge_ids).map_err(json_err)?;
        sqlx::query(
            "INSERT INTO cold_scope (scope, commit_hash, node_ids, edge_ids, parked, updated_at)
             VALUES ($1, $2, $3, $4, $5, now())
             ON CONFLICT (scope) DO UPDATE SET commit_hash = EXCLUDED.commit_hash,
                 node_ids = EXCLUDED.node_ids, edge_ids = EXCLUDED.edge_ids,
                 parked = EXCLUDED.parked, updated_at = now()",
        )
        .bind(&entry.scope)
        .bind(&entry.commit_hash)
        .bind(node_ids)
        .bind(edge_ids)
        .bind(entry.parked)
        .execute(&self.pool)
        .await
        .map_err(pg_err("record_scope"))?;
        Ok(())
    }

    pub async fn lookup_scope(&self, scope: &str) -> GraphStoreResult<Option<ColdScopeEntry>> {
        let row = sqlx::query(
            "SELECT scope, commit_hash, node_ids, edge_ids, parked FROM cold_scope WHERE scope = $1",
        )
        .bind(scope)
        .fetch_optional(&self.pool)
        .await
        .map_err(pg_err("lookup_scope"))?;
        row.map(|row| {
            let node_ids: serde_json::Value = row.try_get("node_ids").map_err(pg_err("row"))?;
            let edge_ids: serde_json::Value = row.try_get("edge_ids").map_err(pg_err("row"))?;
            Ok(ColdScopeEntry {
                scope: row.try_get("scope").map_err(pg_err("row"))?,
                commit_hash: row.try_get("commit_hash").map_err(pg_err("row"))?,
                node_ids: serde_json::from_value(node_ids).map_err(json_err)?,
                edge_ids: serde_json::from_value(edge_ids).map_err(json_err)?,
                parked: row.try_get("parked").map_err(pg_err("row"))?,
            })
        })
        .transpose()
    }

    pub async fn remove_scope(&self, scope: &str) -> GraphStoreResult<()> {
        sqlx::query("DELETE FROM cold_scope WHERE scope = $1")
            .bind(scope)
            .execute(&self.pool)
            .await
            .map_err(pg_err("remove_scope"))?;
        Ok(())
    }

    pub async fn cold_len(&self) -> GraphStoreResult<usize> {
        let row = sqlx::query("SELECT count(*) AS n FROM cold_index")
            .fetch_one(&self.pool)
            .await
            .map_err(pg_err("cold_len"))?;
        let count: i64 = row.try_get("n").map_err(pg_err("row"))?;
        Ok(count.max(0) as usize)
    }
}

/// A synchronous [`ColdIndex`] over Postgres, for the sync eviction hot path.
///
/// Bridges to the async [`PostgresCatalog`] via `block_in_place` + the captured
/// runtime [`Handle`], so it must be constructed from, and called within, a
/// multi-threaded tokio runtime (the THG server's runtime). The eviction path is
/// synchronous; this lets it persist cold residency to Postgres without the
/// caller threading async through the graph layer.
#[derive(Clone, Debug)]
pub struct PostgresColdIndex {
    catalog: PostgresCatalog,
    handle: Handle,
}

impl PostgresColdIndex {
    /// Capture the current runtime handle. Call from within a multi-threaded
    /// tokio runtime.
    pub fn new(catalog: PostgresCatalog) -> Self {
        Self {
            catalog,
            handle: Handle::current(),
        }
    }

    pub fn with_handle(catalog: PostgresCatalog, handle: Handle) -> Self {
        Self { catalog, handle }
    }

    fn block_on<F, T>(&self, future: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        // Leave the async context so we can block on the catalog query from the
        // synchronous eviction path without deadlocking the runtime worker.
        tokio::task::block_in_place(|| self.handle.block_on(future))
    }
}

impl ColdIndex for PostgresColdIndex {
    fn record(&self, entry: ColdIndexEntry) -> GraphStoreResult<()> {
        self.block_on(self.catalog.record_cold(&entry))
    }

    fn lookup(&self, id: &str) -> GraphStoreResult<Option<ColdIndexEntry>> {
        self.block_on(self.catalog.lookup_cold(id))
    }

    fn remove(&self, id: &str) -> GraphStoreResult<()> {
        self.block_on(self.catalog.remove_cold(id))
    }

    fn record_scope(&self, entry: ColdScopeEntry) -> GraphStoreResult<()> {
        self.block_on(self.catalog.record_scope(&entry))
    }

    fn scope(&self, scope: &str) -> GraphStoreResult<Option<ColdScopeEntry>> {
        self.block_on(self.catalog.lookup_scope(scope))
    }

    fn remove_scope(&self, scope: &str) -> GraphStoreResult<()> {
        self.block_on(self.catalog.remove_scope(scope))
    }

    fn len(&self) -> usize {
        self.block_on(self.catalog.cold_len()).unwrap_or(0)
    }
}

fn tier_str(tier: ColdTierKind) -> &'static str {
    match tier {
        ColdTierKind::Cold => "cold",
        ColdTierKind::Warm => "warm",
    }
}

fn parse_tier(value: &str) -> ColdTierKind {
    match value {
        "warm" => ColdTierKind::Warm,
        _ => ColdTierKind::Cold,
    }
}

fn pg_err(context: &'static str) -> impl Fn(sqlx::Error) -> GraphStoreError {
    move |error| GraphStoreError::new("catalog_pg", format!("{context}: {error}"))
}

fn json_err(error: serde_json::Error) -> GraphStoreError {
    GraphStoreError::new("catalog_json", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_present_and_idempotent_shaped() {
        // The schema is the contract; assert it covers the catalog + the cold
        // index/scope rows and is written idempotently.
        assert!(CATALOG_MIGRATIONS.iter().any(|m| m.contains("tenants")));
        assert!(CATALOG_MIGRATIONS.iter().any(|m| m.contains("projects")));
        assert!(CATALOG_MIGRATIONS
            .iter()
            .any(|m| m.contains("billing_accounts")));
        assert!(CATALOG_MIGRATIONS
            .iter()
            .any(|m| m.contains("auth_principals")));
        assert!(CATALOG_MIGRATIONS.iter().any(|m| m.contains("cold_index")));
        assert!(CATALOG_MIGRATIONS.iter().any(|m| m.contains("cold_scope")));
        assert!(CATALOG_MIGRATIONS
            .iter()
            .all(|m| m.contains("IF NOT EXISTS")));
    }

    #[test]
    fn tier_round_trips_through_text() {
        assert_eq!(tier_str(ColdTierKind::Cold), "cold");
        assert_eq!(tier_str(ColdTierKind::Warm), "warm");
        assert_eq!(parse_tier("warm"), ColdTierKind::Warm);
        assert_eq!(parse_tier("cold"), ColdTierKind::Cold);
        assert_eq!(parse_tier("unknown"), ColdTierKind::Cold);
    }

    // Live round-trip against a real Postgres. Ignored by default (needs a DB);
    // run with: THEOREM_CATALOG_TEST_URL=postgres://... cargo test -p
    // rustyred-thg-catalog -- --ignored
    #[test]
    #[ignore]
    fn live_cold_index_round_trip() {
        let url = std::env::var("THEOREM_CATALOG_TEST_URL")
            .expect("set THEOREM_CATALOG_TEST_URL to a Postgres URL");
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let catalog = PostgresCatalog::connect(&url).await.unwrap();
            catalog.migrate().await.unwrap();
            let entry = ColdIndexEntry::cold("mem:live", "theorem", "sha256:abc");
            catalog.record_cold(&entry).await.unwrap();
            let fetched = catalog.lookup_cold("mem:live").await.unwrap().unwrap();
            assert_eq!(fetched.object_hash, "sha256:abc");
            catalog.remove_cold("mem:live").await.unwrap();
            assert!(catalog.lookup_cold("mem:live").await.unwrap().is_none());
        });
    }
}
