//! Persistence roundtrip for the browser page -> substrate seam.
//!
//! The point of routing browser-ingested pages to a `RedCoreGraphStore` rather
//! than an `InMemoryGraphStore` is durability: a page written through the seam
//! must survive closing and reopening the substrate. This test proves exactly
//! that — ingest a page, drop the store, reopen the same data dir, and confirm
//! the specific Page and ContentSnapshot nodes are still there.
//!
//! `InMemoryGraphStore` would fail this (it has no data dir). The test compiles
//! only because RR-1 added `impl GraphStore for RedCoreGraphStore`, which is the
//! seam that lets `ingest_loaded_pages` target the durable store at all.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rustyred_thg_core::graph_store::GraphStore;
use rustyred_thg_core::{RedCoreDurability, RedCoreOptions};
use rustyred_web::{LABEL_CONTENT_SNAPSHOT, LABEL_PAGE};
use theorem_browser_substrate::{
    durable_browser_session_with_options, ingest_loaded_pages, LoadedPage,
};

fn unique_temp_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "theorem-rr-persist-{}-{}",
        std::process::id(),
        nanos
    ))
}

// AofAlways: fsync every commit, so the page is durable before the store drops.
// This keeps the reopen deterministic (no AofEverysec fsync-timing flake).
fn durable_opts() -> RedCoreOptions {
    RedCoreOptions {
        durability: RedCoreDurability::AofAlways,
        ..RedCoreOptions::default()
    }
}

fn first_id_with_label(graph: &rustyred_web::CrawlGraph, label: &str) -> String {
    graph
        .nodes()
        .into_iter()
        .find(|node| node.labels.iter().any(|candidate| candidate == label))
        .map(|node| node.id)
        .unwrap_or_else(|| panic!("expected a node labelled {label}"))
}

#[test]
fn ingested_page_survives_store_reopen() {
    let dir = unique_temp_dir();
    let page = LoadedPage::html(
        "https://example.com/index.html",
        r#"<html><body><a href="/about">About</a></body></html>"#,
    );

    // Phase 1: ingest into a fresh durable store, capture the node ids, drop it.
    let (page_id, snapshot_id) = {
        let mut session =
            durable_browser_session_with_options(&dir, "browser-persist-test", durable_opts())
                .expect("open RedCore browser session");
        let (output, writes) = ingest_loaded_pages(
            session.store_mut(),
            "rr-persist-test",
            vec!["https://example.com/index.html".to_string()],
            &[page],
        )
        .expect("ingest should succeed");

        assert!(!writes.is_empty(), "ingest should write at least one node");
        let page_id = first_id_with_label(&output.graph, LABEL_PAGE);
        let snapshot_id = first_id_with_label(&output.graph, LABEL_CONTENT_SNAPSHOT);

        // The live store sees the nodes immediately.
        assert!(
            GraphStore::get_node(session.store(), &page_id).is_some(),
            "live store has the page"
        );
        assert!(
            GraphStore::get_node(session.store(), &snapshot_id).is_some(),
            "live store has the snapshot"
        );

        (page_id, snapshot_id)
        // store dropped here: directory lock released, AOF already fsynced.
    };

    // Phase 2: reopen the SAME data dir. The nodes must have persisted.
    {
        let session =
            durable_browser_session_with_options(&dir, "browser-persist-test", durable_opts())
                .expect("reopen RedCore browser session");
        assert!(
            GraphStore::get_node(session.store(), &page_id).is_some(),
            "page node must persist across reopen ({page_id})"
        );
        assert!(
            GraphStore::get_node(session.store(), &snapshot_id).is_some(),
            "content snapshot must persist across reopen ({snapshot_id})"
        );

        let html = session.render_search_page("example");
        assert!(
            html.contains("https://example.com/index.html"),
            "reopened browser session search should read the durable page"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}
