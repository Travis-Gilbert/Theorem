//! Servo-free page -> substrate seam.
//!
//! The substrate-native browser turns each page it loads into graph state. That
//! logic lives HERE, not in the Servo embedder crate, for two reasons:
//!
//! 1. Reuse: anything that has a loaded page (the Servo embedder's
//!    `WebViewDelegate::load_web_resource` hook, the `rustyred-web` crawler, the
//!    harness) can write it into the substrate through the same path.
//! 2. Cheap iteration: Servo is a ~30-minute build. The page->graph code is the
//!    valuable part and builds in seconds. Keeping it in its own crate (with no
//!    servo dependency) means it compiles and tests without paying the Servo tax.
//!
//! This crate NEVER depends on Servo. The embedder depends on this crate.
//!
//! The actual page->graph work is already implemented in `rustyred-web`
//! (`build_v2_fixture_crawl`: Page/Domain/ContentSnapshot/FetchAttempt nodes +
//! LINKS_TO/HAS_SNAPSHOT/ON_DOMAIN edges via `extract_links` + `canonicalize_url`
//! + blake3). This crate is the thin, engine-agnostic adapter onto it.

use std::fmt;

use rustyred_thg_core::graph_store::{GraphStore, GraphStoreError, GraphWriteResult};
use rustyred_web::{
    build_v2_fixture_crawl, render_serp_html, search_substrate, CrawlRequest, CrawlRunOutput,
    FetchedPage, RustyWebError, SearchOptions,
};

/// A browser-callable capability exposed by this seam.
///
/// These are intentionally small and static: the Servo embedder can inspect this
/// list without constructing a graph store or running a crawl, while tests can
/// lock the affordance contract that the browser is expected to have.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserAffordance {
    pub id: &'static str,
    pub provider: &'static str,
    pub label: &'static str,
    pub detail: &'static str,
}

const BROWSER_AFFORDANCES: &[BrowserAffordance] = &[
    BrowserAffordance {
        id: "rustyred.graph_write",
        provider: "rustyred",
        label: "Write loaded pages as graph mutations",
        detail: "Uses GraphStore plus GraphMutationBatch writes through the RustyRed substrate.",
    },
    BrowserAffordance {
        id: "rustyweb.page_to_graph",
        provider: "rustyweb",
        label: "Turn a loaded page into crawl graph state",
        detail: "Uses rustyred-web to emit Page, Domain, ContentSnapshot, FetchAttempt, and LINKS_TO state.",
    },
    BrowserAffordance {
        id: "rustyweb.substrate_search",
        provider: "rustyweb",
        label: "Query browser-ingested web graph state",
        detail: "Uses rustyred-web substrate search over the same graph state the browser writes.",
    },
];

pub fn browser_affordances() -> &'static [BrowserAffordance] {
    BROWSER_AFFORDANCES
}

/// A page the embedder has loaded, decoupled from any specific browser engine.
///
/// The Servo seam builds this from a `WebResourceLoad` (url + the intercepted
/// body + the response status and content-type); a crawler builds it from an
/// HTTP response. The downstream graph write is identical either way.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedPage {
    pub url: String,
    pub body: String,
    pub status: u16,
    pub content_type: String,
}

impl LoadedPage {
    pub fn new(
        url: impl Into<String>,
        body: impl Into<String>,
        status: u16,
        content_type: impl Into<String>,
    ) -> Self {
        Self {
            url: url.into(),
            body: body.into(),
            status,
            content_type: content_type.into(),
        }
    }

    /// Convenience for an HTML 200 page (the common browser case).
    pub fn html(url: impl Into<String>, body: impl Into<String>) -> Self {
        Self::new(url, body, 200, "text/html; charset=utf-8")
    }
}

/// Map an engine-agnostic [`LoadedPage`] onto rustyred-web's `FetchedPage` input.
///
/// `FetchedPage` is a type alias for `FixturePage` in rustyred-web; we set its
/// fields directly rather than going through `FixturePage::html` so the real
/// response status and content-type are preserved.
pub fn loaded_page_to_fetched_page(page: &LoadedPage) -> FetchedPage {
    FetchedPage {
        url: page.url.clone(),
        status: page.status,
        body: page.body.clone(),
        content_type: page.content_type.clone(),
        fetched_at: String::new(),
    }
}

/// Failure modes of the seam, kept distinct because a browser cares about the
/// difference: the page could not be turned into a graph (crawl/parse) vs. the
/// graph could not be written to the substrate (store).
#[derive(Debug)]
pub enum SeamError {
    Crawl(RustyWebError),
    Store(GraphStoreError),
}

impl fmt::Display for SeamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Crawl(e) => write!(f, "page->graph failed: {e}"),
            // GraphStoreError impls Debug but not Display; Debug is the only
            // formatter available for it.
            Self::Store(e) => write!(f, "graph->substrate write failed: {e:?}"),
        }
    }
}

impl std::error::Error for SeamError {}

impl From<RustyWebError> for SeamError {
    fn from(e: RustyWebError) -> Self {
        Self::Crawl(e)
    }
}

impl From<GraphStoreError> for SeamError {
    fn from(e: GraphStoreError) -> Self {
        Self::Store(e)
    }
}

/// Build the crawl graph for a batch of loaded pages WITHOUT writing it.
///
/// Returns the full V2 output: the graph plus the receipt (which carries the
/// `graph_delta_hash`). Useful when the caller wants to inspect or hash the
/// delta before committing it.
pub fn loaded_pages_to_graph(
    run_id: impl Into<String>,
    seeds: Vec<String>,
    pages: &[LoadedPage],
) -> Result<CrawlRunOutput, SeamError> {
    let fetched: Vec<FetchedPage> = pages.iter().map(loaded_page_to_fetched_page).collect();
    Ok(build_v2_fixture_crawl(
        CrawlRequest::new(run_id, seeds),
        &fetched,
    )?)
}

/// The seam: turn loaded pages into graph state AND write them to the substrate.
///
/// This is what the browser's `load_web_resource` / `notify_load_status_changed`
/// hook calls once a page has finished loading. Returns the V2 output (so the
/// caller can surface the receipt) alongside the per-mutation write results.
pub fn ingest_loaded_pages(
    store: &mut impl GraphStore,
    run_id: impl Into<String>,
    seeds: Vec<String>,
    pages: &[LoadedPage],
) -> Result<(CrawlRunOutput, Vec<GraphWriteResult>), SeamError> {
    let output = loaded_pages_to_graph(run_id, seeds, pages)?;
    let writes = output.graph.apply_to_store(store)?;
    Ok((output, writes))
}

/// Render the browser's graph-native search page from the same substrate the
/// browser writes into.
///
/// The Servo embedder calls this for its local search URL. Keeping it in the
/// Servo-free crate lets the SERP/search contract test quickly without building
/// Servo.
pub fn render_substrate_search_page(store: &impl GraphStore, query: &str) -> String {
    let search = search_substrate(store, query, SearchOptions::default());
    render_serp_html(&search)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::graph_store::InMemoryGraphStore;
    use rustyred_web::{EDGE_HAS_SNAPSHOT, EDGE_LINKS_TO, LABEL_CONTENT_SNAPSHOT, LABEL_PAGE};

    fn label_count(graph: &rustyred_web::CrawlGraph, label: &str) -> usize {
        graph
            .nodes()
            .iter()
            .filter(|n| n.labels.iter().any(|l| l == label))
            .count()
    }

    fn edge_count(graph: &rustyred_web::CrawlGraph, edge_type: &str) -> usize {
        graph
            .edges()
            .iter()
            .filter(|e| e.edge_type == edge_type)
            .count()
    }

    #[test]
    fn loaded_page_maps_status_and_content_type() {
        let page = LoadedPage::new("https://example.com/x", "<html></html>", 404, "text/plain");
        let fetched = loaded_page_to_fetched_page(&page);
        assert_eq!(fetched.url, "https://example.com/x");
        assert_eq!(fetched.status, 404);
        assert_eq!(fetched.content_type, "text/plain");
        assert_eq!(fetched.body, "<html></html>");
    }

    #[test]
    fn ingesting_a_page_writes_page_link_and_snapshot_nodes_to_the_store() {
        let pages = vec![LoadedPage::html(
            "https://example.com/index.html",
            r#"<html><body><a href="/about">About</a></body></html>"#,
        )];

        let mut store = InMemoryGraphStore::new();
        let (output, writes) = ingest_loaded_pages(
            &mut store,
            "browser-seam-test",
            vec!["https://example.com/index.html".to_string()],
            &pages,
        )
        .expect("seam ingest should succeed");

        // The loaded page became graph state: the fetched page + the discovered
        // /about link target are both Page nodes, with a snapshot and a link edge.
        assert!(
            label_count(&output.graph, LABEL_PAGE) >= 2,
            "page + discovered link target"
        );
        assert_eq!(label_count(&output.graph, LABEL_CONTENT_SNAPSHOT), 1);
        assert_eq!(edge_count(&output.graph, EDGE_LINKS_TO), 1);
        assert_eq!(edge_count(&output.graph, EDGE_HAS_SNAPSHOT), 1);

        // The graph was actually written to the substrate (one write per mutation).
        assert_eq!(
            writes.len(),
            output.graph.nodes().len() + output.graph.edges().len()
        );

        // The receipt carries a content-addressed delta hash (the audit anchor).
        assert!(!output.receipt.graph_delta_hash.is_empty());
    }

    #[test]
    fn browser_affordances_expose_rustyred_and_rustyweb() {
        let affordances = browser_affordances();
        assert!(affordances.iter().any(|item| item.provider == "rustyred"));
        assert!(affordances.iter().any(|item| item.provider == "rustyweb"));
        assert!(affordances
            .iter()
            .any(|item| item.id == "rustyweb.page_to_graph"));
    }

    #[test]
    fn substrate_search_page_renders_from_browser_written_graph_state() {
        let pages = vec![LoadedPage::html(
            "https://example.com/index.html",
            r#"<html><body><h1>Substrate browser</h1><a href="/search">Search</a></body></html>"#,
        )];

        let mut store = InMemoryGraphStore::new();
        ingest_loaded_pages(
            &mut store,
            "browser-search-test",
            vec!["https://example.com/index.html".to_string()],
            &pages,
        )
        .expect("fixture page should be written to the substrate");

        let html = render_substrate_search_page(&store, "substrate");
        assert!(html.contains("var SERP_DATA = {"));
        assert!(html.contains("https://example.com/index.html"));
        assert!(html.contains("Substrate browser"));
    }
}
