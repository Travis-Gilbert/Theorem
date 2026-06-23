//! In-process engine holder + store bridge for slice 1.
//!
//! The engine OWNS a single in-memory substrate. Owning it directly (rather
//! than materializing `InMemoryGraphStore::from_snapshot(tenant.graph_snapshot())`
//! per request the way the multi-tenant reference does at
//! `rustyred-thg-server/src/router.rs:1410-1419`) is the slice-1 stand-in for
//! the tenant-store bridge: `search_substrate` borrows `&InMemoryGraphStore`,
//! so the store must outlive the borrow. A long-lived owned store satisfies
//! that without per-request snapshot copies.
//!
//! HONEST DEFAULT: the substrate starts EMPTY. No fabricated seed data. Search
//! over an empty substrate returns zero hits, which is the truthful "no prior
//! knowledge yet" state, not a bug. Populating the substrate (crawl/ingest, or
//! swapping `InMemoryGraphStore::default()` for a durable
//! `RedCoreGraphStore::open(data_dir)`) is a named follow-up, not slice 1.
//!
//! Tenant scoping is deferred: the SearchService proto carries user_id /
//! session_id but NO tenant_id, and the civic backend dials this as a trusted
//! server-to-server hop, so per-tenant store routing is out of scope here.

use rustyred_thg_core::InMemoryGraphStore;

/// Holds the in-process substrate the SearchService searches against.
pub struct Engine {
    store: InMemoryGraphStore,
}

impl Engine {
    /// Construct the engine with an empty substrate (the honest slice-1 default).
    pub fn new() -> Self {
        Self {
            store: InMemoryGraphStore::default(),
        }
    }

    /// Borrow the substrate. The borrow is valid for the duration of a
    /// `search_substrate(store, ...)` / `personalized_pagerank(...)` call
    /// because the `Engine` (and thus the store) outlives every handler call
    /// (the service holds the engine behind an `Arc`).
    pub fn store(&self) -> &InMemoryGraphStore {
        &self.store
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
