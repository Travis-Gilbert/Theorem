use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::rc::Rc;

use lol_html::{element, HtmlRewriter, Settings};
use rustyred_thg_core::{
    EdgeRecord, GraphMutation, GraphMutationBatch, GraphStore, GraphStoreError, GraphStoreResult,
    GraphWriteResult, NodeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;

pub const LABEL_CRAWL_RUN: &str = "CrawlRun";
pub const LABEL_FETCH_ATTEMPT: &str = "FetchAttempt";
pub const LABEL_DOMAIN: &str = "Domain";
pub const LABEL_PAGE: &str = "Page";
pub const LABEL_CONTENT_SNAPSHOT: &str = "ContentSnapshot";
pub const LABEL_ROBOTS_POLICY: &str = "RobotsPolicy";

pub const EDGE_FETCHED: &str = "FETCHED";
pub const EDGE_RESULTED_IN: &str = "RESULTED_IN";
pub const EDGE_HAS_SNAPSHOT: &str = "HAS_SNAPSHOT";
pub const EDGE_LINKS_TO: &str = "LINKS_TO";
pub const EDGE_ON_DOMAIN: &str = "ON_DOMAIN";
pub const EDGE_ROBOTS_APPLIED: &str = "ROBOTS_APPLIED";
pub const EDGE_CANONICAL_OF: &str = "CANONICAL_OF";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CrawlConfig {
    pub run_id: String,
    pub namespace: String,
    pub user_agent: String,
}

impl Default for CrawlConfig {
    fn default() -> Self {
        Self {
            run_id: "fixture-run".to_string(),
            namespace: "link".to_string(),
            user_agent: "RustyWeb/0.1 fixture".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FixturePage {
    pub url: String,
    pub status: u16,
    pub body: String,
    pub content_type: String,
    pub fetched_at: String,
}

impl FixturePage {
    pub fn html(url: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            status: 200,
            body: body.into(),
            content_type: "text/html; charset=utf-8".to_string(),
            fetched_at: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CrawlCounters {
    pub fetched_pages: usize,
    pub discovered_pages: usize,
    pub domains: usize,
    pub snapshots: usize,
    pub links: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrawlGraph {
    pub run_id: String,
    pub namespace: String,
    pub counters: CrawlCounters,
    pub batch: GraphMutationBatch,
}

impl CrawlGraph {
    pub fn apply_to_store(
        &self,
        store: &mut impl GraphStore,
    ) -> GraphStoreResult<Vec<GraphWriteResult>> {
        apply_batch_to_store(store, &self.batch)
    }

    pub fn nodes(&self) -> Vec<NodeRecord> {
        self.batch
            .mutations
            .iter()
            .filter_map(|mutation| match mutation {
                GraphMutation::NodeUpsert(node) => Some(node.clone()),
                GraphMutation::EdgeUpsert(_) => None,
            })
            .collect()
    }

    pub fn edges(&self) -> Vec<EdgeRecord> {
        self.batch
            .mutations
            .iter()
            .filter_map(|mutation| match mutation {
                GraphMutation::NodeUpsert(_) => None,
                GraphMutation::EdgeUpsert(edge) => Some(edge.clone()),
            })
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RustyWebError {
    InvalidUrl { url: String, reason: String },
    HtmlParse { reason: String },
}

impl fmt::Display for RustyWebError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl { url, reason } => {
                write!(f, "invalid URL {url:?}: {reason}")
            }
            Self::HtmlParse { reason } => write!(f, "HTML parse failed: {reason}"),
        }
    }
}

impl std::error::Error for RustyWebError {}

pub type RustyWebResult<T> = Result<T, RustyWebError>;

pub fn build_fixture_crawl_graph(
    config: CrawlConfig,
    pages: &[FixturePage],
) -> RustyWebResult<CrawlGraph> {
    let run_node_id = crawl_run_id(&config.run_id);
    let mut nodes: BTreeMap<String, NodeRecord> = BTreeMap::new();
    let mut edges: BTreeMap<String, EdgeRecord> = BTreeMap::new();
    let mut fetched_page_ids = BTreeSet::new();
    let mut snapshot_ids = BTreeSet::new();
    let mut link_count = 0usize;

    insert_node(
        &mut nodes,
        NodeRecord::new(
            run_node_id.clone(),
            [LABEL_CRAWL_RUN],
            json!({
                "run_id": config.run_id,
                "namespace": config.namespace,
                "user_agent": config.user_agent,
                "engine": "rustyweb_fixture_v0"
            }),
        ),
    );

    for (index, fixture) in pages.iter().enumerate() {
        let canonical_url = canonicalize_url(&fixture.url)?;
        let page_node_id = page_id(&canonical_url);
        let domain = domain_for_url(&canonical_url)?;
        let domain_node_id = domain_id(&domain);
        let robots_id = robots_policy_id(&domain);
        let attempt_id = fetch_attempt_id(&config.run_id, index, &fixture.url);
        let content_hash = blake3_hash(fixture.body.as_bytes());
        let snapshot_id = content_snapshot_id(&content_hash);
        let fetched_at = if fixture.fetched_at.is_empty() {
            Value::Null
        } else {
            Value::String(fixture.fetched_at.clone())
        };

        fetched_page_ids.insert(page_node_id.clone());
        snapshot_ids.insert(snapshot_id.clone());

        insert_domain_node(&mut nodes, &domain, &domain_node_id, &config.namespace);
        insert_robots_node(&mut nodes, &domain, &robots_id, &config.namespace);
        insert_page_node(
            &mut nodes,
            &canonical_url,
            &page_node_id,
            &domain,
            &config.namespace,
            "fetched",
        );
        insert_node(
            &mut nodes,
            NodeRecord::new(
                attempt_id.clone(),
                [LABEL_FETCH_ATTEMPT],
                json!({
                    "url": fixture.url,
                    "canonical_url": canonical_url,
                    "status": fixture.status,
                    "content_type": fixture.content_type,
                    "body_bytes": fixture.body.len(),
                    "success": (200..400).contains(&fixture.status),
                    "fetched_at": fetched_at,
                    "namespace": config.namespace,
                }),
            ),
        );
        insert_node(
            &mut nodes,
            NodeRecord::new(
                snapshot_id.clone(),
                [LABEL_CONTENT_SNAPSHOT],
                json!({
                    "content_hash": content_hash,
                    "hash_algorithm": "blake3",
                    "byte_len": fixture.body.len(),
                    "text": html_to_text(&fixture.body),
                    "namespace": config.namespace,
                }),
            ),
        );

        insert_edge(
            &mut edges,
            &run_node_id,
            EDGE_FETCHED,
            &attempt_id,
            json!({}),
        );
        insert_edge(
            &mut edges,
            &attempt_id,
            EDGE_RESULTED_IN,
            &page_node_id,
            json!({"status": fixture.status}),
        );
        insert_edge(
            &mut edges,
            &page_node_id,
            EDGE_HAS_SNAPSHOT,
            &snapshot_id,
            json!({}),
        );
        insert_edge(
            &mut edges,
            &page_node_id,
            EDGE_ON_DOMAIN,
            &domain_node_id,
            json!({}),
        );
        insert_edge(
            &mut edges,
            &attempt_id,
            EDGE_ROBOTS_APPLIED,
            &robots_id,
            json!({"policy": "fixture_assumed_allowed"}),
        );

        if fixture.url != canonical_url {
            let alias_id = page_alias_id(&fixture.url);
            insert_node(
                &mut nodes,
                NodeRecord::new(
                    alias_id.clone(),
                    [LABEL_PAGE],
                    json!({
                        "url": fixture.url,
                        "canonical_url": canonical_url,
                        "page_state": "alias",
                        "namespace": config.namespace,
                    }),
                ),
            );
            insert_edge(
                &mut edges,
                &alias_id,
                EDGE_CANONICAL_OF,
                &page_node_id,
                json!({}),
            );
        }

        for target_url in extract_links(&canonical_url, &fixture.body)? {
            let target_domain = domain_for_url(&target_url)?;
            let target_domain_node_id = domain_id(&target_domain);
            let target_page_node_id = page_id(&target_url);
            insert_domain_node(
                &mut nodes,
                &target_domain,
                &target_domain_node_id,
                &config.namespace,
            );
            insert_robots_node(
                &mut nodes,
                &target_domain,
                &robots_policy_id(&target_domain),
                &config.namespace,
            );
            insert_page_node(
                &mut nodes,
                &target_url,
                &target_page_node_id,
                &target_domain,
                &config.namespace,
                "discovered",
            );
            insert_edge(
                &mut edges,
                &target_page_node_id,
                EDGE_ON_DOMAIN,
                &target_domain_node_id,
                json!({}),
            );
            if insert_edge(
                &mut edges,
                &page_node_id,
                EDGE_LINKS_TO,
                &target_page_node_id,
                json!({"source": "html_a_href"}),
            ) {
                link_count += 1;
            }
        }
    }

    let mut mutations = Vec::with_capacity(nodes.len() + edges.len());
    mutations.extend(nodes.values().cloned().map(GraphMutation::NodeUpsert));
    mutations.extend(edges.values().cloned().map(GraphMutation::EdgeUpsert));

    let discovered_pages = nodes
        .values()
        .filter(|node| node.labels.iter().any(|label| label == LABEL_PAGE))
        .count();
    let domains = nodes
        .values()
        .filter(|node| node.labels.iter().any(|label| label == LABEL_DOMAIN))
        .count();

    Ok(CrawlGraph {
        run_id: config.run_id,
        namespace: config.namespace,
        counters: CrawlCounters {
            fetched_pages: fetched_page_ids.len(),
            discovered_pages,
            domains,
            snapshots: snapshot_ids.len(),
            links: link_count,
        },
        batch: GraphMutationBatch { mutations },
    })
}

pub fn apply_batch_to_store(
    store: &mut impl GraphStore,
    batch: &GraphMutationBatch,
) -> GraphStoreResult<Vec<GraphWriteResult>> {
    let mut writes = Vec::with_capacity(batch.mutations.len());
    for mutation in &batch.mutations {
        match mutation {
            GraphMutation::NodeUpsert(node) => writes.push(store.upsert_node(node.clone())?),
            GraphMutation::EdgeUpsert(edge) => writes.push(store.upsert_edge(edge.clone())?),
        }
    }
    Ok(writes)
}

pub fn canonicalize_url(raw: &str) -> RustyWebResult<String> {
    let mut url = Url::parse(raw).map_err(|err| RustyWebError::InvalidUrl {
        url: raw.to_string(),
        reason: err.to_string(),
    })?;
    url.set_fragment(None);
    url.set_username("").ok();
    url.set_password(None).ok();
    if matches!(url.scheme(), "http" | "https") {
        Ok(url.to_string())
    } else {
        Err(RustyWebError::InvalidUrl {
            url: raw.to_string(),
            reason: format!("unsupported scheme {}", url.scheme()),
        })
    }
}

pub fn extract_links(base_url: &str, html: &str) -> RustyWebResult<Vec<String>> {
    let base = Url::parse(base_url).map_err(|err| RustyWebError::InvalidUrl {
        url: base_url.to_string(),
        reason: err.to_string(),
    })?;
    let links = Rc::new(RefCell::new(BTreeSet::new()));
    let links_for_handler = Rc::clone(&links);
    let mut rewriter = HtmlRewriter::new(
        Settings {
            element_content_handlers: vec![element!("a[href]", move |el| {
                let Some(raw_href) = el.get_attribute("href") else {
                    return Ok(());
                };
                if let Ok(mut joined) = base.join(&raw_href) {
                    joined.set_fragment(None);
                    if matches!(joined.scheme(), "http" | "https") {
                        links_for_handler.borrow_mut().insert(joined.to_string());
                    }
                }
                Ok(())
            })],
            ..Settings::default()
        },
        |_chunk: &[u8]| {},
    );
    rewriter
        .write(html.as_bytes())
        .map_err(|err| RustyWebError::HtmlParse {
            reason: err.to_string(),
        })?;
    rewriter.end().map_err(|err| RustyWebError::HtmlParse {
        reason: err.to_string(),
    })?;
    let extracted = links.borrow().iter().cloned().collect();
    Ok(extracted)
}

fn insert_node(nodes: &mut BTreeMap<String, NodeRecord>, node: NodeRecord) {
    nodes.insert(node.id.clone(), node);
}

fn insert_domain_node(
    nodes: &mut BTreeMap<String, NodeRecord>,
    domain: &str,
    id: &str,
    namespace: &str,
) {
    insert_node(
        nodes,
        NodeRecord::new(
            id.to_string(),
            [LABEL_DOMAIN],
            json!({"domain": domain, "namespace": namespace}),
        ),
    );
}

fn insert_robots_node(
    nodes: &mut BTreeMap<String, NodeRecord>,
    domain: &str,
    id: &str,
    namespace: &str,
) {
    insert_node(
        nodes,
        NodeRecord::new(
            id.to_string(),
            [LABEL_ROBOTS_POLICY],
            json!({
                "domain": domain,
                "namespace": namespace,
                "policy_state": "fixture_assumed_allowed",
            }),
        ),
    );
}

fn insert_page_node(
    nodes: &mut BTreeMap<String, NodeRecord>,
    url: &str,
    id: &str,
    domain: &str,
    namespace: &str,
    page_state: &str,
) {
    nodes.entry(id.to_string()).or_insert_with(|| {
        NodeRecord::new(
            id.to_string(),
            [LABEL_PAGE],
            json!({
                "url": url,
                "domain": domain,
                "namespace": namespace,
                "page_state": page_state,
            }),
        )
    });
}

fn insert_edge(
    edges: &mut BTreeMap<String, EdgeRecord>,
    from_id: &str,
    edge_type: &str,
    to_id: &str,
    properties: Value,
) -> bool {
    let id = edge_id(from_id, edge_type, to_id);
    let inserted = !edges.contains_key(&id);
    edges.insert(
        id.clone(),
        EdgeRecord::new(id, from_id, edge_type, to_id, properties),
    );
    inserted
}

fn domain_for_url(raw: &str) -> RustyWebResult<String> {
    let url = Url::parse(raw).map_err(|err| RustyWebError::InvalidUrl {
        url: raw.to_string(),
        reason: err.to_string(),
    })?;
    url.host_str()
        .map(|host| host.to_ascii_lowercase())
        .ok_or_else(|| RustyWebError::InvalidUrl {
            url: raw.to_string(),
            reason: "missing host".to_string(),
        })
}

fn html_to_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn crawl_run_id(run_id: &str) -> String {
    format!("crawl_run:{}", stable_part(run_id))
}

fn fetch_attempt_id(run_id: &str, index: usize, url: &str) -> String {
    format!(
        "fetch_attempt:{}",
        stable_part(&format!("{run_id}:{index}:{url}"))
    )
}

fn domain_id(domain: &str) -> String {
    format!("domain:{domain}")
}

fn page_id(url: &str) -> String {
    format!("page:{}", stable_part(url))
}

fn page_alias_id(url: &str) -> String {
    format!("page_alias:{}", stable_part(url))
}

fn content_snapshot_id(content_hash: &str) -> String {
    format!("content_snapshot:{content_hash}")
}

fn robots_policy_id(domain: &str) -> String {
    format!("robots_policy:{}", stable_part(domain))
}

fn edge_id(from_id: &str, edge_type: &str, to_id: &str) -> String {
    format!(
        "edge:{}",
        stable_part(&format!("{from_id}:{edge_type}:{to_id}"))
    )
}

fn stable_part(value: &str) -> String {
    blake3_hash(value.as_bytes())[..24].to_string()
}

fn blake3_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

impl From<RustyWebError> for GraphStoreError {
    fn from(err: RustyWebError) -> Self {
        GraphStoreError::new("rustyweb_error", err.to_string())
    }
}
