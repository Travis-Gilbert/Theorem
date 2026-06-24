//! Native Rust fractal expansion over RustyRed and RustyWeb.
//!
//! A fractal expansion run is corpus growth, not graph-only retrieval. The
//! public runner in this crate always builds a web crawl request and ingests
//! admitted web graph state as a lower-trust, quarantined tier.

use std::collections::BTreeSet;
use std::sync::Arc;

use rustyred_membrane::recover::{admit_to_budget, emit_receipt};
use rustyred_membrane::{
    Candidate, Handle, MembraneReceipt, ScoreContext, Scorer, Source, SourceArm,
};
use rustyred_rerank::{LexicalCrossEncoder, RerankScorer, GTE_RERANKER_MODERNBERT_BASE};
use rustyred_thg_core::{GraphMutation, GraphStore, NodeQuery, ThgError, ThgResult};
use rustyred_web::{
    build_v2_fixture_crawl, configured_qwen3_embedding_4b_client_from_env, embed_crawl_graph_pages,
    fanout_search_providers, relevant_excerpt_lexical, search_substrate, CrawlEmbeddingReceipt,
    CrawlRequest, CrawlRunOutput, FetchCascade, FetchTierResult, FetchedPage, SearchAcquisition,
    SearchOptions, SearchOpts, SearchProvider, LABEL_PAGE, QWEN3_EMBEDDING_4B_MODEL_ID,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const DEFAULT_FRACTAL_EMBEDDER_MODEL: &str = QWEN3_EMBEDDING_4B_MODEL_ID;
pub const OPEN_WEB_UNVERIFIED_TRUST_TIER: &str = "open_web_unverified";
pub const DEFAULT_OPEN_WEB_CONFIDENCE_CEILING: f32 = 0.35;
/// Max query-ranked passages kept per admitted page for the excerpt.
pub const FRACTAL_EXCERPT_MAX_PASSAGES: usize = 3;
/// Max total characters across an admitted page's excerpt passages. Matches the
/// provider snippet budget so the excerpt is a drop-in upgrade over the snippet,
/// not a heavier payload.
pub const FRACTAL_EXCERPT_MAX_CHARS: usize = 600;
pub const DEFAULT_FRACTAL_BUDGET_TOKENS: usize = 2_000;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FractalExpansionRequest {
    pub run_id: String,
    pub tenant_id: String,
    pub query: String,
    pub web_seed_urls: Vec<String>,
    pub top_k: usize,
    pub frontier_limit: usize,
    pub web_seed_limit: usize,
    #[serde(default = "default_fractal_budget_tokens")]
    pub budget_tokens: usize,
    #[serde(default = "default_fractal_mmr_lambda")]
    pub mmr_lambda: f32,
    pub embedder_model: Option<String>,
    pub actor_id: Option<String>,
}

impl FractalExpansionRequest {
    pub fn normalized(mut self) -> Self {
        self.run_id = self.run_id.trim().to_string();
        self.tenant_id = self.tenant_id.trim().to_string();
        self.query = self.query.trim().to_string();
        self.web_seed_urls = self
            .web_seed_urls
            .into_iter()
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty())
            .collect();
        self.top_k = self.top_k.max(1);
        self.frontier_limit = self.frontier_limit.max(1);
        self.web_seed_limit = self.web_seed_limit.max(1);
        self.budget_tokens = self.budget_tokens.max(1);
        self.mmr_lambda = self.mmr_lambda.clamp(0.0, 1.0);
        self.embedder_model = self
            .embedder_model
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty())
            .or_else(|| Some(DEFAULT_FRACTAL_EMBEDDER_MODEL.to_string()));
        self.actor_id = self
            .actor_id
            .map(|actor| actor.trim().to_string())
            .filter(|actor| !actor.is_empty());
        self
    }
}

const fn default_fractal_budget_tokens() -> usize {
    DEFAULT_FRACTAL_BUDGET_TOKENS
}

const fn default_fractal_mmr_lambda() -> f32 {
    rustyred_membrane::scorer::DEFAULT_MMR_LAMBDA
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FractalFrontierHit {
    pub node_id: String,
    pub url: String,
    pub title: String,
    pub score: f64,
    pub ring: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FractalExpansionReceipt {
    pub run_id: String,
    pub tenant_id: String,
    pub query: String,
    pub embedder_model: String,
    pub graph_exhausted: bool,
    pub web_reached: bool,
    pub web_seed_urls: Vec<String>,
    pub frontier: Vec<FractalFrontierHit>,
    pub crawl_receipt_id: String,
    pub admitted_pages: usize,
    pub applied_writes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_receipt: Option<CrawlEmbeddingReceipt>,
    #[serde(default)]
    pub provider_candidates: Vec<FractalProviderCandidate>,
    #[serde(default)]
    pub provider_receipts: Vec<FractalProviderReceipt>,
    /// Seed URLs whose fetch failed in the live runner. Lets a run that reaches no
    /// pages report honestly (web_reached:false + these failures) instead of a
    /// false web_reached:true on zero fetches.
    #[serde(default)]
    pub web_seed_failures: Vec<String>,
    /// Query-ranked passages extracted from each admitted page body (the "scrape
    /// the relevant pieces" excerpt). Empty passages for a page mean no content
    /// matched -- the consumer falls back to the provider snippet.
    #[serde(default)]
    pub admitted_page_excerpts: Vec<FractalPageExcerpt>,
    #[serde(default)]
    pub admitted_context: Vec<Candidate>,
    #[serde(default)]
    pub deferred_handles: Vec<Handle>,
    #[serde(default)]
    pub tokens_admitted: usize,
    #[serde(default)]
    pub tokens_deferred: usize,
    #[serde(default)]
    pub reranker_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membrane_receipt: Option<MembraneReceipt>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FractalProviderCandidate {
    pub url: String,
    pub score: f64,
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FractalPageExcerpt {
    pub url: String,
    /// Best-first query-ranked passages extracted from the page body.
    #[serde(default)]
    pub passages: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FractalProviderReceipt {
    pub provider: String,
    pub status: String,
    pub returned_candidates: usize,
    pub admitted_candidates: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn run_fixture_fractal_expansion<S: GraphStore>(
    store: &mut S,
    request: FractalExpansionRequest,
    fetched_pages: &[FetchedPage],
) -> ThgResult<FractalExpansionReceipt> {
    let scorer = default_fractal_scorer();
    run_fixture_fractal_expansion_with_scorer(
        store,
        request,
        fetched_pages,
        &scorer,
        scorer.version(),
    )
}

pub fn run_fixture_fractal_expansion_with_scorer<S: GraphStore>(
    store: &mut S,
    request: FractalExpansionRequest,
    fetched_pages: &[FetchedPage],
    scorer: &dyn Scorer,
    reranker_version: impl Into<String>,
) -> ThgResult<FractalExpansionReceipt> {
    let request = request.normalized();
    validate_request(&request)?;

    let frontier = rerank_frontier(graph_frontier(store, &request), &request.query, scorer);
    let web_seed_urls = web_seeds_from_frontier(&request, &frontier);
    if web_seed_urls.is_empty() {
        return Err(ThgError::new(
            "fractal_web_seed_required",
            "fractal expansion cannot terminate at the graph frontier",
        ));
    }

    let PageAdmissionResult {
        admitted_pages,
        admitted_page_excerpts,
        admission,
        membrane_receipt,
    } = admit_pages_to_budget(
        store,
        &request,
        fetched_pages,
        scorer,
        reranker_version.into(),
    )?;
    let mut output = build_admitted_pages_output(&request, &web_seed_urls, &admitted_pages)?;
    let web_reached = !fetched_pages.is_empty();
    apply_admitted_pages(
        store,
        &request,
        web_seed_urls,
        frontier,
        &mut output,
        None,
        web_reached,
        Vec::new(),
        admitted_page_excerpts,
        admission,
        Some(membrane_receipt),
    )
}

pub async fn run_fractal_expansion<S: GraphStore>(
    store: &mut S,
    request: FractalExpansionRequest,
    cascade: &FetchCascade,
    max_bytes: usize,
) -> ThgResult<FractalExpansionReceipt> {
    let scorer = default_fractal_scorer();
    run_fractal_expansion_with_scorer(
        store,
        request,
        cascade,
        max_bytes,
        &scorer,
        scorer.version(),
    )
    .await
}

pub async fn run_fractal_expansion_with_scorer<S: GraphStore>(
    store: &mut S,
    request: FractalExpansionRequest,
    cascade: &FetchCascade,
    max_bytes: usize,
    scorer: &dyn Scorer,
    reranker_version: impl Into<String>,
) -> ThgResult<FractalExpansionReceipt> {
    let request = request.normalized();
    validate_request(&request)?;

    let frontier = rerank_frontier(graph_frontier(store, &request), &request.query, scorer);
    let web_seed_urls = web_seeds_from_frontier(&request, &frontier);
    if web_seed_urls.is_empty() {
        return Err(ThgError::new(
            "fractal_web_seed_required",
            "fractal expansion cannot terminate at the graph frontier",
        ));
    }

    // Fetch seeds concurrently (bounded) instead of one-at-a-time: wall-clock
    // becomes the slowest single fetch, not the sum of all of them. Results are
    // re-sorted into the original seed order, so `fetched` / `web_seed_failures`
    // -- and the receipt built from them -- stay byte-identical to the previous
    // sequential version (parity-safe).
    use futures_util::stream::StreamExt;
    const FETCH_CONCURRENCY: usize = 8;
    // Own each seed URL (`cloned`) so the per-fetch future borrows nothing from
    // the stream iterator: a borrowed item would impose a higher-ranked lifetime
    // bound the future cannot satisfy once the caller drives it under
    // `tokio::spawn`. Cloning a handful of short URL strings is negligible.
    let mut indexed: Vec<_> = futures_util::stream::iter(web_seed_urls.iter().cloned().enumerate())
        .map(|(idx, url)| async move {
            let outcome = match cascade.fetch_with_promotion(&url, max_bytes).await {
                Ok(result) => Ok(fetched_page_from_tier_result(&url, result)),
                // Record the failed seed instead of swallowing it, so a run that
                // reaches no pages reports honestly rather than as a false success.
                Err(error) => Err(format!("{url}: {error}")),
            };
            (idx, outcome)
        })
        .buffer_unordered(FETCH_CONCURRENCY)
        .collect()
        .await;
    indexed.sort_by_key(|(idx, _)| *idx);

    let mut fetched = Vec::new();
    let mut web_seed_failures = Vec::new();
    for (_, outcome) in indexed {
        match outcome {
            Ok(page) => fetched.push(page),
            Err(failure) => web_seed_failures.push(failure),
        }
    }
    let web_reached = !fetched.is_empty();

    let PageAdmissionResult {
        admitted_pages,
        admitted_page_excerpts,
        admission,
        membrane_receipt,
    } = admit_pages_to_budget(store, &request, &fetched, scorer, reranker_version.into())?;
    let mut output = build_admitted_pages_output(&request, &web_seed_urls, &admitted_pages)?;
    let embedding_receipt = maybe_embed_live_crawl_output(&mut output).await?;
    apply_admitted_pages(
        store,
        &request,
        web_seed_urls,
        frontier,
        &mut output,
        embedding_receipt,
        web_reached,
        web_seed_failures,
        admitted_page_excerpts,
        admission,
        Some(membrane_receipt),
    )
}

pub async fn run_fractal_expansion_with_search_providers<S: GraphStore>(
    store: &mut S,
    request: FractalExpansionRequest,
    cascade: &FetchCascade,
    max_bytes: usize,
    providers: &[Arc<dyn SearchProvider>],
    search_opts: SearchOpts,
) -> ThgResult<FractalExpansionReceipt> {
    let scorer = default_fractal_scorer();
    run_fractal_expansion_with_search_providers_and_scorer(
        store,
        request,
        cascade,
        max_bytes,
        providers,
        search_opts,
        &scorer,
        scorer.version(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn run_fractal_expansion_with_search_providers_and_scorer<S: GraphStore>(
    store: &mut S,
    request: FractalExpansionRequest,
    cascade: &FetchCascade,
    max_bytes: usize,
    providers: &[Arc<dyn SearchProvider>],
    search_opts: SearchOpts,
    scorer: &dyn Scorer,
    reranker_version: impl Into<String>,
) -> ThgResult<FractalExpansionReceipt> {
    let (request, acquisition) =
        request_with_provider_seeds(request.normalized(), providers, search_opts, scorer).await;
    let mut receipt = run_fractal_expansion_with_scorer(
        store,
        request,
        cascade,
        max_bytes,
        scorer,
        reranker_version,
    )
    .await?;
    attach_acquisition(&mut receipt, acquisition);
    Ok(receipt)
}

pub async fn run_fixture_fractal_expansion_with_search_providers<S: GraphStore>(
    store: &mut S,
    request: FractalExpansionRequest,
    fetched_pages: &[FetchedPage],
    providers: &[Arc<dyn SearchProvider>],
    search_opts: SearchOpts,
) -> ThgResult<FractalExpansionReceipt> {
    let scorer = default_fractal_scorer();
    run_fixture_fractal_expansion_with_search_providers_and_scorer(
        store,
        request,
        fetched_pages,
        providers,
        search_opts,
        &scorer,
        scorer.version(),
    )
    .await
}

pub async fn run_fixture_fractal_expansion_with_search_providers_and_scorer<S: GraphStore>(
    store: &mut S,
    request: FractalExpansionRequest,
    fetched_pages: &[FetchedPage],
    providers: &[Arc<dyn SearchProvider>],
    search_opts: SearchOpts,
    scorer: &dyn Scorer,
    reranker_version: impl Into<String>,
) -> ThgResult<FractalExpansionReceipt> {
    let (request, acquisition) =
        request_with_provider_seeds(request.normalized(), providers, search_opts, scorer).await;
    let mut receipt = run_fixture_fractal_expansion_with_scorer(
        store,
        request,
        fetched_pages,
        scorer,
        reranker_version,
    )?;
    attach_acquisition(&mut receipt, acquisition);
    Ok(receipt)
}

pub fn open_web_pages_for_tenant<S: GraphStore>(store: &S, tenant_id: &str) -> Vec<String> {
    let mut pages = store.query_nodes(NodeQuery::label(LABEL_PAGE).with_limit(10_000));
    pages.sort_by(|a, b| a.id.cmp(&b.id));
    pages
        .into_iter()
        .filter(|node| prop_str(&node.properties, "tenant_id") == Some(tenant_id))
        .filter(|node| {
            prop_str(&node.properties, "trust_tier") == Some(OPEN_WEB_UNVERIFIED_TRUST_TIER)
        })
        .filter_map(|node| prop_str(&node.properties, "url").map(str::to_string))
        .collect()
}

async fn request_with_provider_seeds(
    mut request: FractalExpansionRequest,
    providers: &[Arc<dyn SearchProvider>],
    search_opts: SearchOpts,
    scorer: &dyn Scorer,
) -> (FractalExpansionRequest, SearchAcquisition) {
    let mut opts = search_opts.normalized();
    opts.limit = opts.limit.max(request.web_seed_limit.max(1));
    let mut acquisition = fanout_search_providers(providers, &request.query, opts).await;
    rerank_provider_candidates(&mut acquisition, scorer);
    merge_provider_seeds(&mut request, &acquisition);
    (request, acquisition)
}

fn default_fractal_scorer() -> RerankScorer {
    RerankScorer::web(Box::new(LexicalCrossEncoder::new(
        GTE_RERANKER_MODERNBERT_BASE,
    )))
}

fn rerank_provider_candidates(acquisition: &mut SearchAcquisition, scorer: &dyn Scorer) {
    let active = Vec::new();
    let ctx = ScoreContext::new(&acquisition.query, &active).without_redundancy();
    acquisition.candidates.sort_by(|left, right| {
        let left_score = scorer.score(&provider_candidate_for_rerank(left), &ctx);
        let right_score = scorer.score(&provider_candidate_for_rerank(right), &ctx);
        right_score
            .partial_cmp(&left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.normalized_url.cmp(&right.normalized_url))
    });
}

fn provider_candidate_for_rerank(candidate: &rustyred_web::RankedSearchCandidate) -> Candidate {
    let title = candidate.candidate.title.clone().unwrap_or_default();
    let snippet = candidate.candidate.snippet.clone().unwrap_or_default();
    let text = join_nonempty(&[&title, &snippet, &candidate.candidate.url]);
    let mut out = Candidate::new(
        format!(
            "fractal:provider:{}",
            stable_digest(&candidate.normalized_url)
        ),
        text,
        1,
    )
    .with_source_arm(SourceArm::Web);
    out.ppr_proximity = candidate.score as f32;
    out.metadata
        .insert("url".to_string(), candidate.candidate.url.clone());
    out
}

fn merge_provider_seeds(request: &mut FractalExpansionRequest, acquisition: &SearchAcquisition) {
    let mut seen: BTreeSet<String> = request.web_seed_urls.iter().cloned().collect();
    for candidate in &acquisition.candidates {
        if request.web_seed_urls.len() >= request.web_seed_limit {
            break;
        }
        let url = candidate.candidate.url.trim();
        if !url.is_empty() && seen.insert(url.to_string()) {
            request.web_seed_urls.push(url.to_string());
        }
    }
}

fn attach_acquisition(receipt: &mut FractalExpansionReceipt, acquisition: SearchAcquisition) {
    receipt.provider_candidates = acquisition
        .candidates
        .into_iter()
        .map(|candidate| FractalProviderCandidate {
            url: candidate.candidate.url,
            score: candidate.score,
            sources: candidate.sources,
            title: candidate.candidate.title,
            snippet: candidate.candidate.snippet,
        })
        .collect();
    receipt.provider_receipts = acquisition
        .providers
        .into_iter()
        .map(|provider| FractalProviderReceipt {
            provider: provider.provider,
            status: provider.status,
            returned_candidates: provider.returned_candidates,
            admitted_candidates: provider.admitted_candidates,
            error: provider.error,
        })
        .collect();
}

fn validate_request(request: &FractalExpansionRequest) -> ThgResult<()> {
    if request.run_id.is_empty() {
        return Err(ThgError::new(
            "invalid_fractal_expansion",
            "run_id is required",
        ));
    }
    if request.tenant_id.is_empty() {
        return Err(ThgError::new(
            "invalid_fractal_expansion",
            "tenant_id is required",
        ));
    }
    if request.query.is_empty() {
        return Err(ThgError::new(
            "invalid_fractal_expansion",
            "query is required",
        ));
    }
    Ok(())
}

fn graph_frontier<S: GraphStore>(
    store: &S,
    request: &FractalExpansionRequest,
) -> Vec<FractalFrontierHit> {
    let search = search_substrate(store, &request.query, SearchOptions::default());
    search
        .hits
        .into_iter()
        .filter(|hit| {
            store
                .get_node(&hit.node_id)
                .and_then(|node| prop_str(&node.properties, "tenant_id"))
                == Some(request.tenant_id.as_str())
        })
        .take(request.frontier_limit)
        .map(|hit| FractalFrontierHit {
            node_id: hit.node_id,
            url: hit.url,
            title: hit.title,
            score: hit.match_score,
            ring: hit.ring,
        })
        .collect()
}

fn rerank_frontier(
    mut frontier: Vec<FractalFrontierHit>,
    query: &str,
    scorer: &dyn Scorer,
) -> Vec<FractalFrontierHit> {
    let active = Vec::new();
    let ctx = ScoreContext::new(query, &active).without_redundancy();
    frontier.sort_by(|left, right| {
        let left_score = scorer.score(&frontier_candidate(left), &ctx);
        let right_score = scorer.score(&frontier_candidate(right), &ctx);
        right_score
            .partial_cmp(&left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    frontier
}

fn frontier_candidate(hit: &FractalFrontierHit) -> Candidate {
    let text = join_nonempty(&[&hit.title, &hit.url]);
    let mut candidate =
        Candidate::new(hit.node_id.clone(), text, 1).with_source_arm(SourceArm::Web);
    candidate.ppr_proximity = hit.score.clamp(0.0, 1.0) as f32;
    candidate
        .metadata
        .insert("pool".to_string(), "fractal_frontier".to_string());
    candidate
}

fn web_seeds_from_frontier(
    request: &FractalExpansionRequest,
    frontier: &[FractalFrontierHit],
) -> Vec<String> {
    let mut seeds = Vec::new();
    let mut seen = BTreeSet::new();
    for seed in &request.web_seed_urls {
        if seen.insert(seed.clone()) {
            seeds.push(seed.clone());
        }
    }
    for hit in frontier {
        if !hit.url.trim().is_empty() && seen.insert(hit.url.clone()) {
            seeds.push(hit.url.clone());
        }
    }
    seeds.into_iter().take(request.web_seed_limit).collect()
}

fn rank_and_sanitize_pages(query: &str, pages: &[FetchedPage], top_k: usize) -> Vec<FetchedPage> {
    let terms = query
        .split_whitespace()
        .map(|term| term.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut scored = pages
        .iter()
        .cloned()
        .map(|mut page| {
            page.body = sanitize_web_body(&page.body);
            let haystack = format!("{} {}", page.url, page.body).to_ascii_lowercase();
            let score = terms
                .iter()
                .filter(|term| !term.is_empty() && haystack.contains(term.as_str()))
                .count();
            (score, page.url.clone(), page)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored
        .into_iter()
        .filter(|(score, _, page)| *score > 0 || page.status == 200)
        .take(top_k.max(1))
        .map(|(_, _, page)| page)
        .collect()
}

struct PageAdmissionResult {
    admitted_pages: Vec<FetchedPage>,
    admitted_page_excerpts: Vec<FractalPageExcerpt>,
    admission: rustyred_membrane::Admission,
    membrane_receipt: MembraneReceipt,
}

fn admit_pages_to_budget<S: GraphStore>(
    store: &mut S,
    request: &FractalExpansionRequest,
    fetched_pages: &[FetchedPage],
    scorer: &dyn Scorer,
    reranker_version: String,
) -> ThgResult<PageAdmissionResult> {
    let mut candidates = rustyred_hipporag::retrieve(
        store,
        rustyred_hipporag::HippoQuery {
            text: &request.query,
            top_k: request.web_seed_limit.max(request.top_k),
            include_hubs: true,
        },
    );
    candidates.extend(
        fetched_pages
            .iter()
            .map(|page| fractal_page_candidate(&request.query, page))
            .collect::<Vec<_>>(),
    );
    let candidates_scored = candidates.len();
    let active = Vec::new();
    let ctx = ScoreContext::new(&request.query, &active).with_mmr_lambda(request.mmr_lambda);
    let admission = admit_to_budget(store, candidates, scorer, &ctx, request.budget_tokens)
        .map_err(graph_error_to_thg)?;

    let admitted_urls = admission
        .admitted
        .iter()
        .filter_map(|candidate| candidate.metadata.get("url").cloned())
        .collect::<BTreeSet<_>>();
    let admitted_pages =
        rank_and_sanitize_pages(&request.query, fetched_pages, fetched_pages.len())
            .into_iter()
            .filter(|page| admitted_urls.contains(&page.url))
            .collect::<Vec<_>>();
    let admitted_page_excerpts = admitted_pages
        .iter()
        .map(|page| page_excerpt(&request.query, page, fetched_pages))
        .collect::<Vec<_>>();

    let membrane_receipt = MembraneReceipt {
        source: Source::Web,
        candidates_scored,
        tokens_admitted: admission.tokens_admitted,
        tokens_deferred: admission.tokens_deferred,
        reranker_version,
        task_token_delta_vs_baseline: Some(admission.tokens_deferred as i64),
    };
    emit_receipt(store, &membrane_receipt).map_err(graph_error_to_thg)?;

    Ok(PageAdmissionResult {
        admitted_pages,
        admitted_page_excerpts,
        admission,
        membrane_receipt,
    })
}

fn build_admitted_pages_output(
    request: &FractalExpansionRequest,
    web_seed_urls: &[String],
    admitted_pages: &[FetchedPage],
) -> ThgResult<CrawlRunOutput> {
    let mut crawl_request = CrawlRequest::new(request.run_id.clone(), web_seed_urls.to_vec());
    crawl_request.scope.namespace = format!("fractal:{}", request.tenant_id);
    crawl_request.scope.source_graph = "rustyred_fractal_expansion".to_string();
    crawl_request.scope.source_license = OPEN_WEB_UNVERIFIED_TRUST_TIER.to_string();
    crawl_request.scope.actor_id = request.actor_id.clone().unwrap_or_default();

    let mut output = build_v2_fixture_crawl(crawl_request, &admitted_pages)
        .map_err(|error| ThgError::new("fractal_web_crawl_failed", error.to_string()))?;
    annotate_open_web_batch(&mut output.graph.batch.mutations, request);
    Ok(output)
}

async fn maybe_embed_live_crawl_output(
    output: &mut CrawlRunOutput,
) -> ThgResult<Option<CrawlEmbeddingReceipt>> {
    let Some(embedder) = configured_qwen3_embedding_4b_client_from_env()
        .map_err(|error| ThgError::new("fractal_embedding_config_failed", error.to_string()))?
    else {
        return Ok(None);
    };
    let receipt = embed_crawl_graph_pages(&mut output.graph, &embedder)
        .await
        .map_err(|error| ThgError::new("fractal_embedding_failed", error.to_string()))?;
    Ok(Some(receipt))
}

// Internal run-state threader: the arg count is the cost of keeping the receipt
// build in one place rather than scattering it across the runners.
#[allow(clippy::too_many_arguments)]
fn apply_admitted_pages<S: GraphStore>(
    store: &mut S,
    request: &FractalExpansionRequest,
    web_seed_urls: Vec<String>,
    frontier: Vec<FractalFrontierHit>,
    output: &mut CrawlRunOutput,
    embedding_receipt: Option<CrawlEmbeddingReceipt>,
    web_reached: bool,
    web_seed_failures: Vec<String>,
    admitted_page_excerpts: Vec<FractalPageExcerpt>,
    admission: rustyred_membrane::Admission,
    membrane_receipt: Option<MembraneReceipt>,
) -> ThgResult<FractalExpansionReceipt> {
    let writes = output
        .graph
        .apply_to_store(store)
        .map_err(|error| ThgError::new(error.code, error.message))?;

    Ok(FractalExpansionReceipt {
        run_id: request.run_id.clone(),
        tenant_id: request.tenant_id.clone(),
        query: request.query.clone(),
        embedder_model: request
            .embedder_model
            .clone()
            .unwrap_or_else(|| DEFAULT_FRACTAL_EMBEDDER_MODEL.to_string()),
        // Stays true: we only reach here after the graph frontier was exhausted
        // into web seeds (otherwise the runners return fractal_web_seed_required).
        graph_exhausted: true,
        // ACTUAL successful fetches, not the attempt: a run where every seed
        // failed reports web_reached:false, never a silent zero-page success.
        web_reached,
        web_seed_urls,
        frontier,
        crawl_receipt_id: output.receipt.receipt_id.clone(),
        admitted_pages: output.receipt.counters.fetched_pages,
        applied_writes: writes.len(),
        embedding_receipt,
        provider_candidates: Vec::new(),
        provider_receipts: Vec::new(),
        web_seed_failures,
        admitted_page_excerpts,
        admitted_context: admission.admitted,
        deferred_handles: admission.deferred,
        tokens_admitted: admission.tokens_admitted,
        tokens_deferred: admission.tokens_deferred,
        reranker_version: membrane_receipt
            .as_ref()
            .map(|receipt| receipt.reranker_version.clone())
            .unwrap_or_default(),
        membrane_receipt,
    })
}

fn fetched_page_from_tier_result(seed_url: &str, result: FetchTierResult) -> FetchedPage {
    let body = String::from_utf8_lossy(&result.html_bytes).into_owned();
    FetchedPage::with_status(
        if result.final_url.trim().is_empty() {
            seed_url
        } else {
            &result.final_url
        },
        body,
        result.http_status,
    )
}

fn fractal_page_candidate(query: &str, page: &FetchedPage) -> Candidate {
    let text = fractal_candidate_text(query, page);
    let mut candidate = Candidate::new(
        format!("fractal:page:{}", stable_digest(&page.url)),
        text.clone(),
        approximate_tokens(&text),
    )
    .with_source_arm(SourceArm::Web);
    candidate.ppr_proximity = if page.status == 200 { 1.0 } else { 0.25 };
    candidate.epistemic.source_reliability = Some(if page.status == 200 { 0.6 } else { 0.2 });
    candidate.epistemic.support_ratio = Some(1.0);
    candidate
        .metadata
        .insert("url".to_string(), page.url.clone());
    candidate
        .metadata
        .insert("pool".to_string(), "fractal_fetched".to_string());
    if let Some(host) = host_key(&page.url) {
        candidate = candidate.with_redundancy_key(host);
    }
    candidate
}

fn fractal_candidate_text(query: &str, page: &FetchedPage) -> String {
    let passages = relevant_excerpt_lexical(
        query,
        &page.body,
        FRACTAL_EXCERPT_MAX_PASSAGES,
        FRACTAL_EXCERPT_MAX_CHARS,
    )
    .into_iter()
    .map(|passage| passage.text)
    .collect::<Vec<_>>();
    let body = if passages.is_empty() {
        sanitize_web_body(&page.body)
            .chars()
            .take(FRACTAL_EXCERPT_MAX_CHARS)
            .collect::<String>()
    } else {
        passages.join("\n")
    };
    join_nonempty(&[&page.url, &body])
}

fn page_excerpt(
    query: &str,
    admitted_page: &FetchedPage,
    raw_pages: &[FetchedPage],
) -> FractalPageExcerpt {
    let raw = raw_pages
        .iter()
        .find(|page| page.url == admitted_page.url)
        .map(|page| page.body.as_str())
        .unwrap_or(admitted_page.body.as_str());
    let passages = relevant_excerpt_lexical(
        query,
        raw,
        FRACTAL_EXCERPT_MAX_PASSAGES,
        FRACTAL_EXCERPT_MAX_CHARS,
    )
    .into_iter()
    .map(|passage| passage.text)
    .collect::<Vec<_>>();
    FractalPageExcerpt {
        url: admitted_page.url.clone(),
        passages,
    }
}

fn sanitize_web_body(body: &str) -> String {
    body.replace("<script", "&lt;script")
        .replace("</script>", "&lt;/script&gt;")
}

fn approximate_tokens(text: &str) -> usize {
    (text.chars().count() / 4).max(1)
}

fn join_nonempty(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn stable_digest(input: &str) -> String {
    blake3::hash(input.as_bytes()).to_hex().to_string()
}

fn host_key(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
}

fn graph_error_to_thg(error: rustyred_thg_core::GraphStoreError) -> ThgError {
    ThgError::new(error.code, error.message)
}

fn annotate_open_web_batch(mutations: &mut [GraphMutation], request: &FractalExpansionRequest) {
    for mutation in mutations {
        match mutation {
            GraphMutation::NodeUpsert(node) => {
                let props = object_props(&mut node.properties);
                props.insert("tenant_id".to_string(), json!(request.tenant_id));
                props.insert(
                    "trust_tier".to_string(),
                    json!(OPEN_WEB_UNVERIFIED_TRUST_TIER),
                );
                props.insert("quarantine".to_string(), json!(true));
                props.insert(
                    "confidence_ceiling".to_string(),
                    json!(DEFAULT_OPEN_WEB_CONFIDENCE_CEILING),
                );
                props.insert(
                    "fractal_expansion_run_id".to_string(),
                    json!(request.run_id),
                );
                props.insert(
                    "embedder_model".to_string(),
                    json!(request
                        .embedder_model
                        .as_deref()
                        .unwrap_or(DEFAULT_FRACTAL_EMBEDDER_MODEL)),
                );
            }
            GraphMutation::EdgeUpsert(edge) => {
                let props = object_props(&mut edge.properties);
                props.insert("tenant_id".to_string(), json!(request.tenant_id));
                props.insert(
                    "fractal_expansion_run_id".to_string(),
                    json!(request.run_id),
                );
            }
        }
    }
}

fn object_props(value: &mut Value) -> &mut serde_json::Map<String, Value> {
    if !value.is_object() {
        *value = json!({});
    }
    value.as_object_mut().expect("object value just created")
}

fn prop_str<'a>(properties: &'a Value, key: &str) -> Option<&'a str> {
    properties.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rustyred_thg_core::InMemoryGraphStore;
    use rustyred_web::{
        build_v2_fixture_crawl, CrawlRequest, FetchedPage, SearchCandidate, SearchOpts,
        SearchProvider, StaticSearchProvider,
    };

    use super::*;

    #[test]
    fn fixture_fractal_expansion_reaches_web_and_quarantines_ingest() {
        let mut store = InMemoryGraphStore::new();
        let mut initial = build_v2_fixture_crawl(
            CrawlRequest::new(
                "initial",
                vec!["https://example.com/rustyweb-skill".to_string()],
            ),
            &[FetchedPage::html(
                "https://example.com/rustyweb-skill",
                "<html><body>RustyWeb skill generation source</body></html>",
            )],
        )
        .unwrap();
        let initial_request = FractalExpansionRequest {
            run_id: "initial-fractal".to_string(),
            tenant_id: "theorem".to_string(),
            query: "rustyweb skill".to_string(),
            web_seed_urls: Vec::new(),
            top_k: 2,
            frontier_limit: 4,
            web_seed_limit: 4,
            budget_tokens: DEFAULT_FRACTAL_BUDGET_TOKENS,
            mmr_lambda: rustyred_membrane::scorer::DEFAULT_MMR_LAMBDA,
            embedder_model: None,
            actor_id: None,
        }
        .normalized();
        annotate_open_web_batch(&mut initial.graph.batch.mutations, &initial_request);
        initial.graph.apply_to_store(&mut store).unwrap();

        let receipt = run_fixture_fractal_expansion(
            &mut store,
            FractalExpansionRequest {
                run_id: "fractal-fixture".to_string(),
                tenant_id: "theorem".to_string(),
                query: "rustyweb skill".to_string(),
                web_seed_urls: vec!["https://example.com/new-grounding".to_string()],
                top_k: 2,
                frontier_limit: 4,
                web_seed_limit: 4,
                budget_tokens: DEFAULT_FRACTAL_BUDGET_TOKENS,
                mmr_lambda: rustyred_membrane::scorer::DEFAULT_MMR_LAMBDA,
                embedder_model: None,
                actor_id: Some("test".to_string()),
            },
            &[FetchedPage::html(
                "https://example.com/new-grounding",
                "<html><body>RustyWeb skill grounding with executable scripts</body></html>",
            )],
        )
        .unwrap();

        assert!(receipt.graph_exhausted);
        assert!(receipt.web_reached);
        assert_eq!(receipt.embedder_model, DEFAULT_FRACTAL_EMBEDDER_MODEL);
        assert!(!receipt.frontier.is_empty());
        assert!(receipt
            .web_seed_urls
            .contains(&"https://example.com/new-grounding".to_string()));
        assert_eq!(receipt.admitted_pages, 1);
        assert!(receipt.applied_writes > 0);

        let open_web_pages = open_web_pages_for_tenant(&store, "theorem");
        assert!(open_web_pages.contains(&"https://example.com/new-grounding".to_string()));
    }

    #[test]
    fn fixture_fractal_expansion_attaches_query_ranked_excerpt() {
        let mut store = InMemoryGraphStore::new();
        let mut initial = build_v2_fixture_crawl(
            CrawlRequest::new(
                "excerpt-initial",
                vec!["https://example.com/seed".to_string()],
            ),
            &[FetchedPage::html(
                "https://example.com/seed",
                "<html><body>tokio asynchronous runtime seed page</body></html>",
            )],
        )
        .unwrap();
        let initial_request = FractalExpansionRequest {
            run_id: "excerpt-initial-fractal".to_string(),
            tenant_id: "theorem".to_string(),
            query: "tokio async runtime".to_string(),
            web_seed_urls: Vec::new(),
            top_k: 2,
            frontier_limit: 4,
            web_seed_limit: 4,
            budget_tokens: DEFAULT_FRACTAL_BUDGET_TOKENS,
            mmr_lambda: rustyred_membrane::scorer::DEFAULT_MMR_LAMBDA,
            embedder_model: None,
            actor_id: None,
        }
        .normalized();
        annotate_open_web_batch(&mut initial.graph.batch.mutations, &initial_request);
        initial.graph.apply_to_store(&mut store).unwrap();

        let receipt = run_fixture_fractal_expansion(
            &mut store,
            FractalExpansionRequest {
                run_id: "excerpt-fractal".to_string(),
                tenant_id: "theorem".to_string(),
                query: "tokio async runtime".to_string(),
                web_seed_urls: vec!["https://example.com/doc".to_string()],
                top_k: 2,
                frontier_limit: 4,
                web_seed_limit: 4,
                budget_tokens: DEFAULT_FRACTAL_BUDGET_TOKENS,
                mmr_lambda: rustyred_membrane::scorer::DEFAULT_MMR_LAMBDA,
                embedder_model: None,
                actor_id: Some("test".to_string()),
            },
            &[FetchedPage::html(
                "https://example.com/doc",
                "<html><head><script>var leak = 1;</script></head><body>\
                 <nav>Home About Contact</nav>\
                 <p>The Tokio runtime is an asynchronous runtime for Rust that \
                 provides the building blocks for writing network services.</p>\
                 <p>This unrelated paragraph is about baking sourdough bread with \
                 a long overnight fermentation in a hot cast iron dutch oven.</p>\
                 </body></html>",
            )],
        )
        .unwrap();

        let excerpt = receipt
            .admitted_page_excerpts
            .iter()
            .find(|excerpt| excerpt.url == "https://example.com/doc")
            .expect("an excerpt for the admitted page");
        // The query-relevant passage is selected...
        assert!(excerpt
            .passages
            .iter()
            .any(|passage| passage.contains("asynchronous runtime")));
        // ...and boilerplate (script body, off-topic paragraph) is excluded.
        assert!(excerpt
            .passages
            .iter()
            .all(|passage| !passage.contains("sourdough") && !passage.contains("leak")));
    }

    #[test]
    fn fixture_fractal_expansion_reports_zero_fetch_honestly() {
        // A graph frontier exists (so web seeds are generated and the run does NOT
        // error fractal_web_seed_required), but no pages are fetched (every seed
        // failed). The receipt must report web_reached:false + admitted_pages:0 --
        // never a false web_reached:true on zero pages (the P1 empty-fetch bug).
        let mut store = InMemoryGraphStore::new();
        let mut initial = build_v2_fixture_crawl(
            CrawlRequest::new(
                "initial-empty",
                vec!["https://example.com/seed-source".to_string()],
            ),
            &[FetchedPage::html(
                "https://example.com/seed-source",
                "<html><body>grounding source for the zero-fetch case</body></html>",
            )],
        )
        .unwrap();
        let initial_request = FractalExpansionRequest {
            run_id: "initial-empty-fractal".to_string(),
            tenant_id: "theorem".to_string(),
            query: "grounding".to_string(),
            web_seed_urls: Vec::new(),
            top_k: 2,
            frontier_limit: 4,
            web_seed_limit: 4,
            budget_tokens: DEFAULT_FRACTAL_BUDGET_TOKENS,
            mmr_lambda: rustyred_membrane::scorer::DEFAULT_MMR_LAMBDA,
            embedder_model: None,
            actor_id: None,
        }
        .normalized();
        annotate_open_web_batch(&mut initial.graph.batch.mutations, &initial_request);
        initial.graph.apply_to_store(&mut store).unwrap();

        let receipt = run_fixture_fractal_expansion(
            &mut store,
            FractalExpansionRequest {
                run_id: "fractal-zero-fetch".to_string(),
                tenant_id: "theorem".to_string(),
                query: "grounding".to_string(),
                web_seed_urls: vec!["https://example.com/dead-seed".to_string()],
                top_k: 2,
                frontier_limit: 4,
                web_seed_limit: 4,
                budget_tokens: DEFAULT_FRACTAL_BUDGET_TOKENS,
                mmr_lambda: rustyred_membrane::scorer::DEFAULT_MMR_LAMBDA,
                embedder_model: None,
                actor_id: Some("test".to_string()),
            },
            &[], // every seed failed to fetch -> no admitted pages
        )
        .unwrap();

        // The graph WAS exhausted into web seeds (honest, structural)...
        assert!(receipt.graph_exhausted);
        assert!(
            !receipt.web_seed_urls.is_empty(),
            "seeds were generated; the run did not terminate at the graph"
        );
        // ...but web_reached is honest about the zero fetch.
        assert!(
            !receipt.web_reached,
            "zero fetched pages must report web_reached=false, not a false success"
        );
        assert_eq!(receipt.admitted_pages, 0);
    }

    #[test]
    fn fixture_fractal_expansion_refuses_graph_only_terminal_state() {
        let mut store = InMemoryGraphStore::new();
        let error = run_fixture_fractal_expansion(
            &mut store,
            FractalExpansionRequest {
                run_id: "fractal-no-web".to_string(),
                tenant_id: "theorem".to_string(),
                query: "missing frontier".to_string(),
                web_seed_urls: Vec::new(),
                top_k: 2,
                frontier_limit: 4,
                web_seed_limit: 4,
                budget_tokens: DEFAULT_FRACTAL_BUDGET_TOKENS,
                mmr_lambda: rustyred_membrane::scorer::DEFAULT_MMR_LAMBDA,
                embedder_model: None,
                actor_id: None,
            },
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "fractal_web_seed_required");
    }

    #[test]
    fn fixture_fractal_expansion_ignores_cross_tenant_frontier() {
        let mut store = InMemoryGraphStore::new();
        let mut other_tenant = build_v2_fixture_crawl(
            CrawlRequest::new(
                "other-initial",
                vec!["https://example.com/other-tenant-skill".to_string()],
            ),
            &[FetchedPage::html(
                "https://example.com/other-tenant-skill",
                "<html><body>RustyWeb skill generation source</body></html>",
            )],
        )
        .unwrap();
        let other_request = FractalExpansionRequest {
            run_id: "other-fractal".to_string(),
            tenant_id: "other".to_string(),
            query: "rustyweb skill".to_string(),
            web_seed_urls: Vec::new(),
            top_k: 2,
            frontier_limit: 4,
            web_seed_limit: 4,
            budget_tokens: DEFAULT_FRACTAL_BUDGET_TOKENS,
            mmr_lambda: rustyred_membrane::scorer::DEFAULT_MMR_LAMBDA,
            embedder_model: None,
            actor_id: None,
        }
        .normalized();
        annotate_open_web_batch(&mut other_tenant.graph.batch.mutations, &other_request);
        other_tenant.graph.apply_to_store(&mut store).unwrap();

        let error = run_fixture_fractal_expansion(
            &mut store,
            FractalExpansionRequest {
                run_id: "theorem-fractal".to_string(),
                tenant_id: "theorem".to_string(),
                query: "rustyweb skill".to_string(),
                web_seed_urls: Vec::new(),
                top_k: 2,
                frontier_limit: 4,
                web_seed_limit: 4,
                budget_tokens: DEFAULT_FRACTAL_BUDGET_TOKENS,
                mmr_lambda: rustyred_membrane::scorer::DEFAULT_MMR_LAMBDA,
                embedder_model: None,
                actor_id: None,
            },
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "fractal_web_seed_required");
    }

    #[tokio::test]
    async fn fixture_fractal_expansion_uses_provider_candidates_as_web_seeds() {
        let mut store = InMemoryGraphStore::new();
        let providers: Vec<Arc<dyn SearchProvider>> = vec![Arc::new(StaticSearchProvider::new(
            "brave",
            vec![SearchCandidate {
                url: "https://example.com/provider-grounding".to_string(),
                title: Some("Provider grounding".to_string()),
                snippet: Some("search provider candidate".to_string()),
                source: "brave".to_string(),
                rank: 1,
            }],
        ))];

        let receipt = run_fixture_fractal_expansion_with_search_providers(
            &mut store,
            FractalExpansionRequest {
                run_id: "provider-fractal".to_string(),
                tenant_id: "theorem".to_string(),
                query: "provider grounding".to_string(),
                web_seed_urls: Vec::new(),
                top_k: 2,
                frontier_limit: 4,
                web_seed_limit: 4,
                budget_tokens: DEFAULT_FRACTAL_BUDGET_TOKENS,
                mmr_lambda: rustyred_membrane::scorer::DEFAULT_MMR_LAMBDA,
                embedder_model: None,
                actor_id: Some("test".to_string()),
            },
            &[FetchedPage::html(
                "https://example.com/provider-grounding",
                "<html><body>Provider grounding search candidate body</body></html>",
            )],
            &providers,
            SearchOpts::default(),
        )
        .await
        .unwrap();

        assert!(receipt.web_reached);
        assert_eq!(
            receipt.web_seed_urls,
            vec!["https://example.com/provider-grounding".to_string()]
        );
        assert_eq!(receipt.provider_candidates.len(), 1);
        assert_eq!(receipt.provider_candidates[0].sources, vec!["brave"]);
        assert_eq!(receipt.provider_receipts.len(), 1);
        assert_eq!(receipt.provider_receipts[0].status, "ok");

        let open_web_pages = open_web_pages_for_tenant(&store, "theorem");
        assert!(open_web_pages.contains(&"https://example.com/provider-grounding".to_string()));
    }

    #[tokio::test]
    async fn provider_frontier_uses_reranker_before_seed_merge() {
        let mut store = InMemoryGraphStore::new();
        let providers: Vec<Arc<dyn SearchProvider>> = vec![Arc::new(StaticSearchProvider::new(
            "brave",
            vec![
                SearchCandidate {
                    url: "https://example.com/generic".to_string(),
                    title: Some("Generic result".to_string()),
                    snippet: Some("popular but off topic".to_string()),
                    source: "brave".to_string(),
                    rank: 1,
                },
                SearchCandidate {
                    url: "https://example.com/modernbert".to_string(),
                    title: Some("ModernBERT reranker".to_string()),
                    snippet: Some("fast sequence classification reranker".to_string()),
                    source: "brave".to_string(),
                    rank: 2,
                },
            ],
        ))];

        let receipt = run_fixture_fractal_expansion_with_search_providers(
            &mut store,
            FractalExpansionRequest {
                run_id: "provider-rerank-fractal".to_string(),
                tenant_id: "theorem".to_string(),
                query: "modernbert reranker".to_string(),
                web_seed_urls: Vec::new(),
                top_k: 2,
                frontier_limit: 4,
                web_seed_limit: 1,
                budget_tokens: DEFAULT_FRACTAL_BUDGET_TOKENS,
                mmr_lambda: rustyred_membrane::scorer::DEFAULT_MMR_LAMBDA,
                embedder_model: None,
                actor_id: Some("test".to_string()),
            },
            &[FetchedPage::html(
                "https://example.com/modernbert",
                "<html><body>ModernBERT reranker guide</body></html>",
            )],
            &providers,
            SearchOpts {
                provider_limit: 2,
                limit: 2,
                rrf_k: 60,
                provider_timeout_ms: SearchOpts::default().provider_timeout_ms,
            },
        )
        .await
        .unwrap();

        assert_eq!(
            receipt.web_seed_urls,
            vec!["https://example.com/modernbert".to_string()]
        );
        assert_eq!(
            receipt.provider_candidates[0].url,
            "https://example.com/modernbert"
        );
    }

    #[test]
    fn fixture_fractal_expansion_gates_and_recovers_deferred_context() {
        let mut store = InMemoryGraphStore::new();
        let receipt = run_fixture_fractal_expansion(
            &mut store,
            FractalExpansionRequest {
                run_id: "fractal-gate".to_string(),
                tenant_id: "theorem".to_string(),
                query: "modernbert reranker".to_string(),
                web_seed_urls: vec![
                    "https://example.com/modernbert".to_string(),
                    "https://example.com/qwen".to_string(),
                ],
                top_k: 2,
                frontier_limit: 4,
                web_seed_limit: 4,
                budget_tokens: 16,
                mmr_lambda: 0.7,
                embedder_model: None,
                actor_id: Some("test".to_string()),
            },
            &[
                FetchedPage::html(
                    "https://example.com/modernbert",
                    "<html><body>ModernBERT reranker uses sequence classification for fast retrieval ranking.</body></html>",
                ),
                FetchedPage::html(
                    "https://example.com/qwen",
                    "<html><body>Qwen causal language model reranker is slower on the hot path.</body></html>",
                ),
            ],
        )
        .unwrap();

        assert_eq!(
            receipt.membrane_receipt.as_ref().unwrap().source,
            Source::Web
        );
        assert_eq!(
            receipt.membrane_receipt.as_ref().unwrap().candidates_scored,
            2
        );
        assert!(receipt.tokens_admitted <= 16);
        assert!(!receipt.deferred_handles.is_empty());
        let recovered =
            rustyred_membrane::context_fetch(&store, &receipt.deferred_handles[0]).unwrap();
        assert!(!recovered.is_empty());
        assert!(receipt.reranker_version.contains("membrane-v1"));
    }
}
