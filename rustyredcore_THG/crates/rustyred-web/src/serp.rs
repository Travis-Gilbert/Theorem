//! SERP rendering — turn a [`SubstrateSearch`] into the browser's results PAGE.
//!
//! The browser's SERP (search engine results page) displays results as a node-
//! and-edge graph, not a list of links: the matched pages plus their `LINKS_TO`
//! neighbourhood, laid out as concentric relevance rings. This module produces
//! that page. The browser's `load_web_resource` hook intercepts a search URL,
//! calls [`crate::search_substrate`], passes the result here, and serves the
//! returned HTML, exactly as `apps/browser` already intercepts and serves its
//! smoke page.
//!
//! The page is a single self-contained HTML document (`serp.html`, embedded via
//! `include_str!`): no bundler, no npm, no external assets, so the Servo embedder
//! serves it directly. The only dynamic part is the search payload, injected in
//! place of a `null` marker.
//!
//! Security: the SERP renders titles, urls, and snippets that came from CRAWLED
//! pages, which are untrusted. Two defenses, both required:
//!   1. `serp.html` sets every piece of page text via `textContent` /
//!      `createElement`, never `innerHTML` (no DOM-injection).
//!   2. [`serp_payload_json`] escapes `<`, `>`, `&` to their `\uXXXX` forms so a
//!      title containing `</script>` cannot break out of the `<script>` block
//!      the payload is injected into (no script-injection).

use crate::search::SubstrateSearch;

/// The self-contained SERP page. The `null` payload marker is replaced at render
/// time. Served verbatim (empty state) when no search has run.
const SERP_TEMPLATE: &str = include_str!("serp.html");

/// The exact line in the template that carries the payload placeholder.
const PAYLOAD_MARKER: &str = "var SERP_DATA = null; // __SERP_DATA__";

/// Serialize a search to a `<script>`-safe JSON literal.
///
/// `serde_json` does not escape `<`/`>`/`&` by default, so a crawled page whose
/// title contains `</script>` would close the script tag the payload lives in.
/// We escape those three to their JSON `\uXXXX` forms — still valid JSON (so it
/// re-parses), still renders as the original character in JS, but inert as HTML.
pub fn serp_payload_json(search: &SubstrateSearch) -> String {
    let raw = serde_json::to_string(search).unwrap_or_else(|_| "null".to_string());
    raw.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

/// Render the browser's SERP for a substrate search: the self-contained graph
/// page with the search payload injected.
pub fn render_serp_html(search: &SubstrateSearch) -> String {
    let payload = serp_payload_json(search);
    let injected = format!("var SERP_DATA = {payload};");
    SERP_TEMPLATE.replacen(PAYLOAD_MARKER, &injected, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{SearchHit, SearchLink};

    fn hit(node_id: &str, url: &str, title: &str, ring: usize, score: u32) -> SearchHit {
        SearchHit {
            node_id: node_id.to_string(),
            url: url.to_string(),
            title: title.to_string(),
            snippet: String::new(),
            ring,
            ring_label: match ring {
                0 => "match",
                1 => "adjacent",
                _ => "nearby",
            }
            .to_string(),
            match_score: score,
        }
    }

    fn sample() -> SubstrateSearch {
        SubstrateSearch {
            query: "apple".to_string(),
            hits: vec![
                hit("p1", "http://ex.com/apple", "apple", 0, 3),
                hit("p2", "http://ex.com/orchard", "orchard", 1, 0),
            ],
            links: vec![SearchLink {
                source: "p1".to_string(),
                target: "p2".to_string(),
            }],
            matched_count: 1,
            kept_count: 2,
        }
    }

    #[test]
    fn render_injects_the_payload_and_consumes_the_marker() {
        let html = render_serp_html(&sample());
        assert!(html.contains("var SERP_DATA = {"), "payload injected");
        assert!(!html.contains("// __SERP_DATA__"), "marker consumed");
        assert!(html.contains("http://ex.com/apple"), "matched url present");
        assert!(html.contains("orchard"), "neighbour present");
    }

    #[test]
    fn payload_is_valid_json_after_escaping() {
        let json = serp_payload_json(&sample());
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("escaped payload must remain valid JSON");
        assert_eq!(parsed["query"], "apple");
        assert_eq!(parsed["hits"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn script_breakout_is_neutralized() {
        // A crawled page whose title tries to close the script tag and inject.
        let mut malicious = sample();
        malicious.hits[0].title = "</script><img src=x onerror=alert(1)>".to_string();
        let html = render_serp_html(&malicious);
        // The dangerous literal must NOT appear; the escaped form must.
        assert!(
            !html.contains("</script><img"),
            "raw script breakout must not survive into the page"
        );
        assert!(
            html.contains("\\u003c/script\\u003e\\u003cimg"),
            "the title is present, but escaped"
        );
    }

    #[test]
    fn empty_search_renders_a_valid_page() {
        let empty = SubstrateSearch {
            query: "quantum".to_string(),
            hits: vec![],
            links: vec![],
            matched_count: 0,
            kept_count: 0,
        };
        let html = render_serp_html(&empty);
        assert!(html.contains("var SERP_DATA = {"));
        assert!(html.contains("\"kept_count\":0"));
        assert!(html.contains("<!doctype html>"));
    }
}
