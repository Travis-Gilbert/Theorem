use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::IpAddr;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lol_html::{element, HtmlRewriter, Settings};
use reqwest::header::CONTENT_TYPE;
use rustyred_thg_core::{
    EdgeRecord, GraphMutation, GraphMutationBatch, GraphStore, GraphStoreError, GraphStoreResult,
    GraphWriteResult, NodeRecord, Provenance, TTL_PROPERTY,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use url::Url;

pub const LABEL_CRAWL_RUN: &str = "CrawlRun";
pub const LABEL_FETCH_ATTEMPT: &str = "FetchAttempt";
pub const LABEL_DOMAIN: &str = "Domain";
pub const LABEL_PAGE: &str = "Page";
pub const LABEL_CONTENT_SNAPSHOT: &str = "ContentSnapshot";
pub const LABEL_ROBOTS_POLICY: &str = "RobotsPolicy";
pub const LABEL_CRAWL_RECEIPT: &str = "CrawlReceipt";
pub const LABEL_DISCOVERY_SEED: &str = "DiscoverySeed";

pub const EDGE_FETCHED: &str = "FETCHED";
pub const EDGE_RESULTED_IN: &str = "RESULTED_IN";
pub const EDGE_HAS_SNAPSHOT: &str = "HAS_SNAPSHOT";
pub const EDGE_LINKS_TO: &str = "LINKS_TO";
pub const EDGE_ON_DOMAIN: &str = "ON_DOMAIN";
pub const EDGE_ROBOTS_APPLIED: &str = "ROBOTS_APPLIED";
pub const EDGE_CANONICAL_OF: &str = "CANONICAL_OF";
pub const EDGE_EMITTED_RECEIPT: &str = "EMITTED_RECEIPT";
pub const EDGE_SEEDED: &str = "SEEDED";

pub type FetchedPage = FixturePage;

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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CrawlBudget {
    pub max_pages: usize,
    pub max_seconds: u64,
    pub max_depth: usize,
    pub max_bytes: usize,
}

impl Default for CrawlBudget {
    fn default() -> Self {
        Self {
            max_pages: 25,
            max_seconds: 30,
            max_depth: 2,
            max_bytes: 5 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CrawlScope {
    pub namespace: String,
    pub follow_offsite: bool,
    pub ttl_expires_at_ms: Option<i64>,
    pub source_graph: String,
    pub source_license: String,
    pub federable: bool,
    pub actor_id: String,
}

impl Default for CrawlScope {
    fn default() -> Self {
        Self {
            namespace: "link".to_string(),
            follow_offsite: true,
            ttl_expires_at_ms: None,
            source_graph: "theorem_crawler".to_string(),
            source_license: "unknown".to_string(),
            federable: false,
            actor_id: String::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CrawlRequest {
    pub run_id: String,
    pub seeds: Vec<String>,
    pub budget: CrawlBudget,
    pub scope: CrawlScope,
}

impl CrawlRequest {
    pub fn new(run_id: impl Into<String>, seeds: Vec<String>) -> Self {
        Self {
            run_id: run_id.into(),
            seeds,
            budget: CrawlBudget::default(),
            scope: CrawlScope::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UrlGuardPolicy {
    pub allow_loopback: bool,
    pub allow_private_networks: bool,
    pub block_metadata_services: bool,
}

impl Default for UrlGuardPolicy {
    fn default() -> Self {
        Self {
            allow_loopback: false,
            allow_private_networks: false,
            block_metadata_services: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveFetchOptions {
    pub user_agent: String,
    pub timeout_seconds: u64,
    pub guard_policy: UrlGuardPolicy,
}

impl Default for LiveFetchOptions {
    fn default() -> Self {
        Self {
            user_agent: "RustyWeb/0.2 live".to_string(),
            timeout_seconds: 10,
            guard_policy: UrlGuardPolicy::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CrawlReceipt {
    pub receipt_id: String,
    pub run_id: String,
    pub namespace: String,
    pub status: String,
    pub seed_count: usize,
    pub counters: CrawlCounters,
    pub graph_delta_hash: String,
    pub budget: CrawlBudget,
    pub scope: CrawlScope,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrawlRunOutput {
    pub graph: CrawlGraph,
    pub receipt: CrawlReceipt,
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
    Fetch { url: String, reason: String },
    BodyLimitExceeded { url: String, limit: usize },
    EmptySeeds,
    InvalidBudget { reason: String },
    BlockedUrl { url: String, reason: String },
}

impl fmt::Display for RustyWebError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl { url, reason } => {
                write!(f, "invalid URL {url:?}: {reason}")
            }
            Self::HtmlParse { reason } => write!(f, "HTML parse failed: {reason}"),
            Self::Fetch { url, reason } => write!(f, "fetch failed for {url:?}: {reason}"),
            Self::BodyLimitExceeded { url, limit } => {
                write!(f, "fetch body for {url:?} exceeded {limit} bytes")
            }
            Self::EmptySeeds => write!(f, "crawl request requires at least one seed URL"),
            Self::InvalidBudget { reason } => write!(f, "invalid crawl budget: {reason}"),
            Self::BlockedUrl { url, reason } => write!(f, "blocked URL {url:?}: {reason}"),
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

pub fn build_v2_fixture_crawl(
    request: CrawlRequest,
    pages: &[FetchedPage],
) -> RustyWebResult<CrawlRunOutput> {
    build_v2_fixture_crawl_with_policy(request, pages, &UrlGuardPolicy::default())
}

pub fn build_v2_fixture_crawl_with_policy(
    request: CrawlRequest,
    pages: &[FetchedPage],
    guard_policy: &UrlGuardPolicy,
) -> RustyWebResult<CrawlRunOutput> {
    let guarded_seeds = validate_crawl_request(&request, guard_policy)?;
    let (selected_pages, budget_limited) = select_budgeted_pages(&request.budget, pages);
    let status = if budget_limited {
        "budget_limited"
    } else {
        "completed"
    }
    .to_string();
    let mut graph = build_fixture_crawl_graph(
        CrawlConfig {
            run_id: request.run_id.clone(),
            namespace: request.scope.namespace.clone(),
            user_agent: "RustyWeb/0.2 fixture".to_string(),
        },
        &selected_pages,
    )?;

    add_seed_nodes(&mut graph, &request, &guarded_seeds);
    annotate_graph_for_scope(&mut graph, &request.scope);
    let graph_delta_hash = graph_delta_hash(&graph.batch);
    let receipt_id = crawl_receipt_id(&request.run_id, &graph_delta_hash);
    let receipt = CrawlReceipt {
        receipt_id,
        run_id: request.run_id.clone(),
        namespace: request.scope.namespace.clone(),
        status,
        seed_count: guarded_seeds.len(),
        counters: graph.counters.clone(),
        graph_delta_hash,
        budget: request.budget.clone(),
        scope: request.scope.clone(),
    };
    add_receipt_node(&mut graph, &receipt);

    Ok(CrawlRunOutput { graph, receipt })
}

pub async fn run_live_crawl(request: CrawlRequest) -> RustyWebResult<CrawlRunOutput> {
    run_live_crawl_with_options(request, &LiveFetchOptions::default()).await
}

pub async fn run_live_crawl_with_options(
    request: CrawlRequest,
    options: &LiveFetchOptions,
) -> RustyWebResult<CrawlRunOutput> {
    let pages = fetch_seed_pages(&request, options).await?;
    build_v2_fixture_crawl_with_policy(request, &pages, &options.guard_policy)
}

pub async fn fetch_seed_pages(
    request: &CrawlRequest,
    options: &LiveFetchOptions,
) -> RustyWebResult<Vec<FetchedPage>> {
    let seeds = validate_crawl_request(request, &options.guard_policy)?;
    let mut effective_options = options.clone();
    effective_options.timeout_seconds = options.timeout_seconds.min(request.budget.max_seconds);
    let client = build_live_http_client(&effective_options)?;
    let mut pages = Vec::new();
    let mut remaining_bytes = request.budget.max_bytes;

    for seed in seeds.into_iter().take(request.budget.max_pages) {
        if remaining_bytes == 0 {
            break;
        }
        let page = fetch_one_page(&client, &seed, remaining_bytes).await?;
        remaining_bytes = remaining_bytes.saturating_sub(page.body.len());
        pages.push(page);
    }

    Ok(pages)
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

pub fn validate_crawl_request(
    request: &CrawlRequest,
    policy: &UrlGuardPolicy,
) -> RustyWebResult<Vec<String>> {
    if request.seeds.is_empty() {
        return Err(RustyWebError::EmptySeeds);
    }
    if request.budget.max_pages == 0 {
        return Err(RustyWebError::InvalidBudget {
            reason: "max_pages must be greater than zero".to_string(),
        });
    }
    if request.budget.max_seconds == 0 {
        return Err(RustyWebError::InvalidBudget {
            reason: "max_seconds must be greater than zero".to_string(),
        });
    }
    if request.budget.max_bytes == 0 {
        return Err(RustyWebError::InvalidBudget {
            reason: "max_bytes must be greater than zero".to_string(),
        });
    }

    let mut seeds = BTreeSet::new();
    for seed in &request.seeds {
        seeds.insert(guarded_canonicalize_url(seed, policy)?);
    }
    Ok(seeds.into_iter().collect())
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

pub fn guarded_canonicalize_url(raw: &str, policy: &UrlGuardPolicy) -> RustyWebResult<String> {
    let canonical = canonicalize_url(raw)?;
    guard_canonical_url(&canonical, policy)?;
    Ok(canonical)
}

pub fn guard_canonical_url(canonical: &str, policy: &UrlGuardPolicy) -> RustyWebResult<()> {
    let url = Url::parse(canonical).map_err(|err| RustyWebError::InvalidUrl {
        url: canonical.to_string(),
        reason: err.to_string(),
    })?;
    let host =
        url.host_str()
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| RustyWebError::InvalidUrl {
                url: canonical.to_string(),
                reason: "missing host".to_string(),
            })?;

    if policy.block_metadata_services && is_metadata_service_host(&host) {
        return Err(RustyWebError::BlockedUrl {
            url: canonical.to_string(),
            reason: "metadata service host".to_string(),
        });
    }
    if host == "localhost" && !policy.allow_loopback {
        return Err(RustyWebError::BlockedUrl {
            url: canonical.to_string(),
            reason: "loopback hostname".to_string(),
        });
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if let Some(reason) = blocked_ip_reason(ip, policy) {
            return Err(RustyWebError::BlockedUrl {
                url: canonical.to_string(),
                reason: reason.to_string(),
            });
        }
    }

    Ok(())
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

pub fn graph_delta_hash(batch: &GraphMutationBatch) -> String {
    let bytes = serde_json::to_vec(batch).unwrap_or_default();
    blake3_hash(&bytes)
}

fn select_budgeted_pages(budget: &CrawlBudget, pages: &[FetchedPage]) -> (Vec<FetchedPage>, bool) {
    let mut selected = Vec::new();
    let mut consumed_bytes = 0usize;
    let mut budget_limited = pages.len() > budget.max_pages;

    for page in pages.iter().take(budget.max_pages) {
        let next_bytes = page.body.len();
        if consumed_bytes.saturating_add(next_bytes) > budget.max_bytes {
            budget_limited = true;
            break;
        }
        consumed_bytes += next_bytes;
        selected.push(page.clone());
    }

    (selected, budget_limited)
}

fn add_seed_nodes(graph: &mut CrawlGraph, request: &CrawlRequest, seeds: &[String]) {
    let run_node_id = crawl_run_id(&request.run_id);
    for seed in seeds {
        let seed_id = discovery_seed_id(seed);
        graph
            .batch
            .mutations
            .push(GraphMutation::NodeUpsert(NodeRecord::new(
                seed_id.clone(),
                [LABEL_DISCOVERY_SEED],
                json!({
                    "url": seed,
                    "namespace": request.scope.namespace,
                    "seed_hash": stable_part(seed),
                }),
            )));
        graph
            .batch
            .mutations
            .push(GraphMutation::EdgeUpsert(EdgeRecord::new(
                edge_id(&run_node_id, EDGE_SEEDED, &seed_id),
                run_node_id.clone(),
                EDGE_SEEDED,
                seed_id,
                json!({"source": "crawl_request"}),
            )));
    }
}

fn add_receipt_node(graph: &mut CrawlGraph, receipt: &CrawlReceipt) {
    let run_node_id = crawl_run_id(&receipt.run_id);
    let mut properties = serde_json::to_value(receipt).unwrap_or_else(|_| {
        json!({
            "receipt_id": receipt.receipt_id,
            "run_id": receipt.run_id,
            "status": receipt.status,
        })
    });
    let props = properties_object(&mut properties);
    insert_scope_properties(props, &receipt.scope);
    props.insert(
        "graph_delta_hash_algorithm".to_string(),
        Value::String("blake3".to_string()),
    );

    graph
        .batch
        .mutations
        .push(GraphMutation::NodeUpsert(NodeRecord::new(
            receipt.receipt_id.clone(),
            [LABEL_CRAWL_RECEIPT],
            properties,
        )));
    graph.batch.mutations.push(GraphMutation::EdgeUpsert(
        EdgeRecord::new(
            edge_id(&run_node_id, EDGE_EMITTED_RECEIPT, &receipt.receipt_id),
            run_node_id,
            EDGE_EMITTED_RECEIPT,
            receipt.receipt_id.clone(),
            json!({
                "receipt_id": receipt.receipt_id,
                "graph_delta_hash": receipt.graph_delta_hash,
            }),
        )
        .with_provenance(provenance_for_scope(&receipt.scope)),
    ));
}

fn annotate_graph_for_scope(graph: &mut CrawlGraph, scope: &CrawlScope) {
    for mutation in &mut graph.batch.mutations {
        match mutation {
            GraphMutation::NodeUpsert(node) => {
                let props = properties_object(&mut node.properties);
                insert_scope_properties(props, scope);
            }
            GraphMutation::EdgeUpsert(edge) => {
                let props = properties_object(&mut edge.properties);
                insert_scope_properties(props, scope);
                edge.provenance = Some(provenance_for_scope(scope));
            }
        }
    }
}

fn properties_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = json!({});
    }
    value.as_object_mut().expect("object inserted above")
}

fn insert_scope_properties(properties: &mut Map<String, Value>, scope: &CrawlScope) {
    properties.insert(
        "source_graph".to_string(),
        Value::String(scope.source_graph.clone()),
    );
    properties.insert(
        "source_license".to_string(),
        Value::String(scope.source_license.clone()),
    );
    properties.insert("federable".to_string(), Value::Bool(scope.federable));
    properties.insert(
        "admission_tier".to_string(),
        Value::String("advisory".to_string()),
    );
    properties.insert(
        "follow_offsite".to_string(),
        Value::Bool(scope.follow_offsite),
    );
    if let Some(expires_at_ms) = scope.ttl_expires_at_ms {
        properties.insert(TTL_PROPERTY.to_string(), json!(expires_at_ms));
    }
    if !scope.actor_id.is_empty() {
        properties.insert(
            "actor_id".to_string(),
            Value::String(scope.actor_id.clone()),
        );
    }
}

fn provenance_for_scope(scope: &CrawlScope) -> Provenance {
    Provenance {
        source_id: Some(scope.source_graph.clone()),
        timestamp: None,
        method: Some("rustyweb_v2".to_string()),
    }
}

fn is_metadata_service_host(host: &str) -> bool {
    matches!(
        host,
        "169.254.169.254" | "169.254.170.2" | "metadata" | "metadata.google.internal"
    )
}

fn blocked_ip_reason(ip: IpAddr, policy: &UrlGuardPolicy) -> Option<&'static str> {
    match ip {
        IpAddr::V4(ip) => {
            if policy.block_metadata_services
                && (ip.octets() == [169, 254, 169, 254] || ip.octets() == [169, 254, 170, 2])
            {
                return Some("metadata service ip address");
            }
            if ip.is_loopback() && !policy.allow_loopback {
                return Some("loopback ip address");
            }
            if (ip.is_private() || ip.is_link_local() || ip.is_unspecified())
                && !policy.allow_private_networks
            {
                return Some("private, link-local, or unspecified ip address");
            }
        }
        IpAddr::V6(ip) => {
            if ip.is_loopback() && !policy.allow_loopback {
                return Some("loopback ip address");
            }
            if (ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_unspecified())
                && !policy.allow_private_networks
            {
                return Some("private, link-local, or unspecified ip address");
            }
        }
    }
    None
}

fn build_live_http_client(options: &LiveFetchOptions) -> RustyWebResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(options.user_agent.clone())
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(options.timeout_seconds))
        .build()
        .map_err(|err| RustyWebError::Fetch {
            url: "<client>".to_string(),
            reason: err.to_string(),
        })
}

async fn fetch_one_page(
    client: &reqwest::Client,
    canonical_url: &str,
    max_bytes: usize,
) -> RustyWebResult<FetchedPage> {
    let response = client
        .get(canonical_url)
        .send()
        .await
        .map_err(|err| RustyWebError::Fetch {
            url: canonical_url.to_string(),
            reason: err.to_string(),
        })?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let body = read_limited_body(response, canonical_url, max_bytes).await?;

    Ok(FetchedPage {
        url: canonical_url.to_string(),
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
        content_type,
        fetched_at: current_unix_ms_string(),
    })
}

async fn read_limited_body(
    mut response: reqwest::Response,
    url: &str,
    limit: usize,
) -> RustyWebResult<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|err| RustyWebError::Fetch {
        url: url.to_string(),
        reason: err.to_string(),
    })? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(RustyWebError::BodyLimitExceeded {
                url: url.to_string(),
                limit,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn current_unix_ms_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string())
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

fn discovery_seed_id(seed: &str) -> String {
    format!("discovery_seed:{}", stable_part(seed))
}

fn crawl_receipt_id(run_id: &str, graph_delta_hash: &str) -> String {
    format!(
        "crawl_receipt:{}",
        stable_part(&format!("{run_id}:{graph_delta_hash}"))
    )
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
