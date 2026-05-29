use rustyred_thg_core::{InMemoryGraphStore, NodeQuery};
use rustyred_web::{
    build_fixture_crawl_graph, canonicalize_url, extract_links, CrawlConfig, FixturePage,
    EDGE_CANONICAL_OF, EDGE_HAS_SNAPSHOT, EDGE_LINKS_TO, EDGE_ON_DOMAIN, EDGE_RESULTED_IN,
    LABEL_CONTENT_SNAPSHOT, LABEL_CRAWL_RUN, LABEL_DOMAIN, LABEL_FETCH_ATTEMPT, LABEL_PAGE,
    LABEL_ROBOTS_POLICY,
};

fn label_count(graph: &rustyred_web::CrawlGraph, label: &str) -> usize {
    graph
        .nodes()
        .iter()
        .filter(|node| node.labels.iter().any(|candidate| candidate == label))
        .count()
}

fn edge_count(graph: &rustyred_web::CrawlGraph, edge_type: &str) -> usize {
    graph
        .edges()
        .iter()
        .filter(|edge| edge.edge_type == edge_type)
        .count()
}

#[test]
fn fixture_crawl_emits_v0_graph_contract_and_applies_to_store() {
    let graph = build_fixture_crawl_graph(
        CrawlConfig {
            run_id: "rw-fixture-1".to_string(),
            namespace: "link".to_string(),
            user_agent: "RustyWeb test".to_string(),
        },
        &[
            FixturePage::html(
                "https://example.com/index.html#top",
                r#"
                <html>
                  <head><title>Home</title></head>
                  <body>
                    <a href="/about">About</a>
                    <a href="https://docs.example.org/start#section">Docs</a>
                  </body>
                </html>
                "#,
            ),
            FixturePage::html(
                "https://example.com/about",
                r#"<html><body><a href="/index.html">Home</a></body></html>"#,
            ),
        ],
    )
    .unwrap();

    assert_eq!(graph.counters.fetched_pages, 2);
    assert_eq!(graph.counters.discovered_pages, 4);
    assert_eq!(graph.counters.domains, 2);
    assert_eq!(graph.counters.snapshots, 2);
    assert_eq!(graph.counters.links, 3);

    assert_eq!(label_count(&graph, LABEL_CRAWL_RUN), 1);
    assert_eq!(label_count(&graph, LABEL_FETCH_ATTEMPT), 2);
    assert_eq!(label_count(&graph, LABEL_DOMAIN), 2);
    assert_eq!(label_count(&graph, LABEL_PAGE), 4);
    assert_eq!(label_count(&graph, LABEL_CONTENT_SNAPSHOT), 2);
    assert_eq!(label_count(&graph, LABEL_ROBOTS_POLICY), 2);

    assert_eq!(edge_count(&graph, EDGE_RESULTED_IN), 2);
    assert_eq!(edge_count(&graph, EDGE_HAS_SNAPSHOT), 2);
    assert_eq!(edge_count(&graph, EDGE_ON_DOMAIN), 3);
    assert_eq!(edge_count(&graph, EDGE_LINKS_TO), 3);
    assert_eq!(edge_count(&graph, EDGE_CANONICAL_OF), 1);

    let mut store = InMemoryGraphStore::new();
    let writes = graph.apply_to_store(&mut store).unwrap();
    assert_eq!(writes.len(), graph.batch.mutations.len());
    assert_eq!(store.query_nodes(NodeQuery::label(LABEL_PAGE)).len(), 4,);
}

#[test]
fn canonicalization_and_link_extraction_are_deterministic() {
    assert_eq!(
        canonicalize_url("https://example.com/a?b=1#frag").unwrap(),
        "https://example.com/a?b=1"
    );

    let links = extract_links(
        "https://example.com/docs/index.html",
        r#"<a href="../a#x">A</a><a href="mailto:nope@example.com">Mail</a><a href="https://example.org/z">Z</a>"#,
    )
    .unwrap();
    assert_eq!(
        links,
        vec![
            "https://example.com/a".to_string(),
            "https://example.org/z".to_string(),
        ],
    );
}
