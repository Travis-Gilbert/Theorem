//! The WEB arm of SPEC-CONTEXT-MEMBRANE-1.0: search -> graph-gated context.
//!
//! [`web_search_graph`] is the fast path the MCP `web_search_graph` tool wraps.
//! It does NOT block on page extraction. In one pass it:
//!
//!   1. fans the query out across the configured providers and RRF-merges the
//!      result (snippets) -- the FRESH pool;
//!   2. reads the WARM substrate subgraph for the same query through
//!      [`search_substrate`] over the passed-in `GraphStore` (pages that a
//!      prior `FetchCompletionHook` extraction already left behind) -- the WARM
//!      pool, carrying real graph proximity;
//!   3. builds a [`rustyred_membrane::Candidate`] for both pools, deduped across
//!      them (warm wins on collision, since it has graph proximity);
//!   4. optionally runs a [`ListwiseReranker`] over the candidate set and stamps
//!      its rank as a scorer-visible signal;
//!   5. gates the unified pool through [`admit_to_budget`], which persists every
//!      DEFERRED candidate byte-exact so [`context_fetch`] recovers it -- the
//!      Ariadne recovery property the spec requires;
//!   6. emits a content-addressed [`MembraneReceipt`] (`Source::Web`) into the
//!      store;
//!   7. returns [`WebSearchGraph`] plus a FIRE-AND-FORGET fetch closure.
//!
//! The fetch closure ([`WebSearchGraph::into_fetch_task`]) fetches the top-K
//! result URLs through [`FetchCascade`] and writes `Page` nodes with
//! `state="fetched"` (plus their `ContentSnapshot` via `HAS_SNAPSHOT`). Writing
//! `state="fetched"` is exactly the transition [`crate::crawl_hooks`]'s
//! `fetch_completion_hook` watches for, so extraction runs reactively AFTER the
//! response is already assembled. The caller `tokio::spawn`s the returned future;
//! the membrane [`Admission`] is built and returned without ever awaiting it.
//! That is what makes the SECOND query for the same topic admit warm graph
//! context the first query seeded.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rustyred_membrane::recover::{admit_to_budget, emit_receipt};
use rustyred_membrane::{
    Candidate, Handle, MembraneReceipt, ScoreContext, Scorer, Source, SourceArm,
};
use rustyred_rerank::{stamp_listwise_rank, ListwiseRankScorer, ListwiseReranker};
use rustyred_thg_core::GraphStore;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::fetch_cascade::{FetchCascade, FetchCascadeOptions};
use crate::search::{
    fanout_search_providers, search_substrate, RankedSearchCandidate, SearchAcquisition,
    SearchOptions, SearchOpts, SearchProvider, SearchProviderReceipt, SubstrateSearch,
};
use crate::{CrawlConfig, FixturePage, RustyWebResult, SearchHit};

/// Default fetch body cap for the fire-and-forget warming pass.
const DEFAULT_FETCH_MAX_BYTES: usize = 1_048_576;
/// Default fetch timeout (seconds) for the warming pass.
const DEFAULT_FETCH_TIMEOUT_SECONDS: u64 = 15;
/// User agent the warming pass fetches under.
const FETCH_USER_AGENT: &str = "RustyWeb/0.2 membrane-warm";
/// Namespace the warming pass writes Page nodes under (matches the crawler scope).
const WARM_NAMESPACE: &str = "open_web_unverified";

/// Knobs for one [`web_search_graph`] invocation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebSearchGraphOptions {
    /// Provider fan-out + RRF acquisition knobs (FRESH pool).
    pub acquisition: SearchOpts,
    /// Warm-substrate read knobs (WARM pool). A runtime knob, not part of the
    /// serialized response contract, so it is skipped (defaults on deserialize).
    #[serde(skip)]
    pub substrate: SearchOptions,
    /// Token budget the membrane gate fills. Overflow defers (recoverable).
    pub budget_tokens: usize,
    /// Cross-pool redundancy penalty handed to the [`ScoreContext`].
    pub redundancy_penalty: f32,
    /// How many top result URLs the fire-and-forget pass fetches + writes back
    /// as `state="fetched"` Page nodes (firing the extraction hook).
    pub fetch_top_k: usize,
}

impl Default for WebSearchGraphOptions {
    fn default() -> Self {
        Self {
            acquisition: SearchOpts::default(),
            substrate: SearchOptions::default(),
            budget_tokens: 2_000,
            redundancy_penalty: 0.15,
            fetch_top_k: 5,
        }
    }
}

/// The result of the WEB arm fast path: the gated context plus the receipts and
/// a stable reference to the touched subgraph. Serde-serializable so it rides
/// the MCP `web_search_graph` response unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebSearchGraph {
    /// The normalized query the gate ran against.
    pub query: String,
    /// Candidates that fit the budget, in admitted order. Each carries its
    /// source pool (`pool=fresh|warm`) and provenance in `metadata`.
    pub admitted_context: Vec<Candidate>,
    /// Lossless handles for everything that overflowed the budget. Each is
    /// byte-exact recoverable via `rustyred_membrane::context_fetch`.
    pub deferred_handles: Vec<Handle>,
    /// Content-addressed digest of the touched subgraph (query + fresh urls +
    /// warm page node ids), stable across identical inputs.
    pub subgraph_ref: String,
    /// Per-provider receipts: a failed provider is recorded, not fatal.
    pub providers: Vec<SearchProviderReceipt>,
    pub tokens_admitted: usize,
    pub tokens_deferred: usize,
    /// The reranker version the membrane receipt was stamped with.
    pub reranker_version: String,
    /// The membrane receipt emitted into the store (also returned for callers
    /// that want it without a store read).
    pub receipt: MembraneReceipt,
    /// The top result URLs the fire-and-forget pass will fetch + write back.
    /// Exposed so the MCP layer can report what is warming.
    pub fetch_seed_urls: Vec<String>,
}

impl WebSearchGraph {
    /// Build the fire-and-forget warming task: fetch the top-K result URLs and
    /// write each as a `state="fetched"` Page node (plus its `ContentSnapshot`).
    /// This is intentionally NOT awaited inside [`web_search_graph`]; the caller
    /// `tokio::spawn`s it so extraction runs reactively after the response is
    /// already assembled. The future is `Send + 'static`; it owns its own store
    /// handle so it can outlive the request.
    ///
    /// `store` is the shared GraphStore the warm pages must land in. Callers
    /// typically pass a clonable handle (e.g. `Arc<Mutex<Store>>` wrapped in a
    /// small adapter); the closure receives `&mut S` so it can `apply_to_store`.
    pub fn fetch_seed_urls(&self) -> &[String] {
        &self.fetch_seed_urls
    }
}

/// The WEB arm fast path. Generic over the `GraphStore` so the same entry serves
/// the in-memory test store, a tenant-scoped `RedCoreGraphStore`, or any backing.
///
/// `providers` is the configured fan-out set (use
/// [`crate::configured_search_providers_from_env`]); `scorer` is normally
/// `RerankScorer::web(Box::new(LexicalCrossEncoder::new(...)))` (relevance-
/// dominant); `listwise` is optional, and when present its candidate-set order is
/// converted into a scorer-visible rank feature before the shared budget gate.
///
/// This runs the provider fan-out + RRF, the warm-substrate read, the membrane
/// gate, and the receipt emit, then returns. It does NOT fetch or extract pages:
/// that is the caller's `tokio::spawn` of [`warm_pages_task`].
///
/// `reranker_version` is the version string stamped onto the [`MembraneReceipt`];
/// pass `scorer.version()` when the scorer is a `RerankScorer` (the `Scorer`
/// trait itself does not expose a version, so the caller threads it in).
pub async fn web_search_graph<S: GraphStore>(
    store: &mut S,
    providers: &[Arc<dyn SearchProvider>],
    query: &str,
    options: &WebSearchGraphOptions,
    scorer: &dyn Scorer,
    listwise: Option<&dyn ListwiseReranker>,
    reranker_version: impl Into<String>,
) -> RustyWebResult<WebSearchGraph> {
    // (a) FRESH pool: provider fan-out + RRF (async, store-free). Per-provider
    // failure is recorded in the receipts and does not fail the whole search
    // (see fanout_search_providers).
    let acquisition = fanout_search_providers(providers, query, options.acquisition.clone()).await;
    // (b)-(g) are synchronous store work. Splitting them out lets an async caller
    // (the product server) run the fan-out OUTSIDE the tenant store lock, then
    // drive the gate inside a brief synchronous lock scope, so no store guard is
    // ever held across the fan-out `.await`.
    gate_search_graph(
        store,
        acquisition,
        options,
        scorer,
        listwise,
        reranker_version,
    )
}

/// Synchronous store-side half of the web arm: the warm-substrate read, candidate
/// unification across the fresh + warm pools, the optional listwise rank stamp,
/// the membrane gate, and the receipt emit, over an
/// already-fanned-out [`SearchAcquisition`]. Split from [`web_search_graph`] so
/// an async caller can run the network fan-out without holding a non-`Send` store
/// guard across the await, then call this inside a brief synchronous lock scope.
pub fn gate_search_graph<S: GraphStore>(
    store: &mut S,
    acquisition: SearchAcquisition,
    options: &WebSearchGraphOptions,
    scorer: &dyn Scorer,
    listwise: Option<&dyn ListwiseReranker>,
    reranker_version: impl Into<String>,
) -> RustyWebResult<WebSearchGraph> {
    // (b) WARM pool: the substrate subgraph for the query, read over the store.
    let substrate = search_substrate(store, &acquisition.query, options.substrate.clone());

    // (c) Build candidates for both pools, deduped across them.
    let pools = unify_pools(&acquisition, &substrate, options.redundancy_penalty);
    let UnifiedPools {
        candidates,
        fresh_urls,
        warm_ids,
        warm_urls,
    } = pools;
    let candidates_scored = candidates.len();

    let active: Vec<String> = Vec::new();
    let ctx = ScoreContext::new(&acquisition.query, &active)
        .with_redundancy_penalty(options.redundancy_penalty);

    let base_reranker_version = reranker_version.into();
    let listwise_model_id = listwise.map(|reranker| reranker.model_id().to_string());
    let candidates = if let Some(listwise) = listwise {
        stamp_listwise_rank(listwise.rerank(candidates, scorer, &ctx))
    } else {
        candidates
    };
    let listwise_scorer = listwise_model_id
        .as_ref()
        .map(|_| ListwiseRankScorer::new(scorer));
    let gate_scorer: &dyn Scorer = match listwise_scorer.as_ref() {
        Some(rank_scorer) => rank_scorer,
        None => scorer,
    };

    // (d)-(e) Gate the unified pool. A listwise reranker feeds this gate by way
    // of the `ListwiseRankScorer` wrapper above, so diversity can change the
    // admitted set instead of merely reordering already-admitted context.
    let admission = admit_to_budget(store, candidates, gate_scorer, &ctx, options.budget_tokens)
        .map_err(|error| crate::RustyWebError::Fetch {
            url: acquisition.query.clone(),
            reason: format!("membrane admit failed: {error:?}"),
        })?;

    // (f) Emit a content-addressed Web receipt into the store. The single-gate
    // saving is the tokens an ungated admit-all run would have placed in the
    // window but the gate deferred (baseline = admitted + deferred, so delta =
    // deferred). This is the per-gate proxy for the cost lever; a true per-task
    // no-gate baseline is a deployed cross-run measurement. Consistent with the
    // code arm.
    let reranker_version = match listwise_model_id {
        Some(model_id) => format!("{base_reranker_version}+listwise:{model_id}"),
        None => base_reranker_version,
    };
    let receipt = MembraneReceipt {
        source: Source::Web,
        candidates_scored,
        tokens_admitted: admission.tokens_admitted,
        tokens_deferred: admission.tokens_deferred,
        reranker_version: reranker_version.clone(),
        task_token_delta_vs_baseline: Some(admission.tokens_deferred as i64),
    };
    emit_receipt(store, &receipt).map_err(|error| crate::RustyWebError::Fetch {
        url: acquisition.query.clone(),
        reason: format!("membrane receipt emit failed: {error:?}"),
    })?;

    // (g) Assemble the response. The fetch seed urls drive the fire-and-forget
    // warming pass; they are FRESH-pool urls the warm pool does not already
    // cover (no point re-fetching pages already in the substrate).
    let fetch_seed_urls = fetch_seed_urls(&acquisition, &warm_urls, options.fetch_top_k);
    let subgraph_ref = subgraph_ref(&acquisition.query, &fresh_urls, &warm_ids);

    Ok(WebSearchGraph {
        query: acquisition.query.clone(),
        admitted_context: admission.admitted,
        deferred_handles: admission.deferred,
        subgraph_ref,
        providers: acquisition.providers,
        tokens_admitted: admission.tokens_admitted,
        tokens_deferred: admission.tokens_deferred,
        reranker_version,
        receipt,
        fetch_seed_urls,
    })
}

/// The fire-and-forget warming pass: fetch each url through [`FetchCascade`] and
/// write it as a `state="fetched"` Page node (plus its `ContentSnapshot`). The
/// `state="fetched"` write is what [`crate::crawl_hooks::fetch_completion_hook`]
/// reacts to, so entity extraction / classification run AFTER this returns,
/// outside the search fast path. Failures per url are swallowed (best-effort
/// warming): one dead url must not poison the warm corpus.
///
/// Call this from a `tokio::spawn` AFTER [`web_search_graph`] returns. It is a
/// free function (not a method) so it can own its store handle and be `'static`.
pub async fn warm_pages_task<S: GraphStore>(
    store: &mut S,
    urls: &[String],
    run_id: impl Into<String>,
) -> usize {
    let cascade = match FetchCascade::new(FetchCascadeOptions::http2_only(
        FETCH_USER_AGENT.to_string(),
        DEFAULT_FETCH_TIMEOUT_SECONDS,
    )) {
        Ok(cascade) => cascade,
        Err(_) => return 0,
    };

    let mut fetched = Vec::new();
    for url in urls {
        let canonical = match crate::canonicalize_url(url) {
            Ok(canonical) => canonical,
            Err(_) => continue,
        };
        match cascade
            .fetch_with_promotion(&canonical, DEFAULT_FETCH_MAX_BYTES)
            .await
        {
            Ok(result) if (200..400).contains(&result.http_status) => {
                let body = String::from_utf8_lossy(&result.html_bytes).into_owned();
                fetched.push(FixturePage {
                    url: if result.final_url.trim().is_empty() {
                        canonical
                    } else {
                        result.final_url.clone()
                    },
                    status: result.http_status,
                    body,
                    content_type: if result.content_type.trim().is_empty() {
                        "text/html".to_string()
                    } else {
                        result.content_type.clone()
                    },
                    fetched_at: String::new(),
                });
            }
            _ => continue,
        }
    }

    write_fetched_pages(store, fetched, run_id.into())
}

/// Pure write seam: turn already-fetched pages into `state="fetched"` Page nodes
/// in the store, reusing the crawler's `build_fixture_crawl_graph` ->
/// `apply_to_store` path (so the write shape is identical to a real crawl and
/// the extraction hook fires). Returns the number of pages written. Split out so
/// the fire-and-forget seam is testable without live network.
pub fn write_fetched_pages<S: GraphStore>(
    store: &mut S,
    pages: Vec<FixturePage>,
    run_id: String,
) -> usize {
    if pages.is_empty() {
        return 0;
    }
    let config = CrawlConfig {
        run_id,
        namespace: WARM_NAMESPACE.to_string(),
        user_agent: FETCH_USER_AGENT.to_string(),
    };
    match crate::build_fixture_crawl_graph(config, &pages) {
        Ok(graph) => match graph.apply_to_store(store) {
            Ok(_) => pages.len(),
            Err(_) => 0,
        },
        Err(_) => 0,
    }
}

/// The deduped candidate pool plus the bookkeeping the response needs.
struct UnifiedPools {
    candidates: Vec<Candidate>,
    /// FRESH-pool normalized urls, in contribution order.
    fresh_urls: Vec<String>,
    /// WARM-pool page node ids that contributed.
    warm_ids: Vec<String>,
    /// WARM-pool canonical urls (for fetch-seed dedupe against the fresh pool).
    warm_urls: BTreeSet<String>,
}

/// Merge the FRESH (provider) and WARM (substrate) pools into one candidate set,
/// deduped across them.
///
/// Dedupe key: a fresh result and a warm page that resolve to the same canonical
/// url are the same thing; the WARM one wins because it carries real graph
/// proximity (the fresh one only has provider rank).
fn unify_pools(
    acquisition: &SearchAcquisition,
    substrate: &SubstrateSearch,
    redundancy_penalty: f32,
) -> UnifiedPools {
    let mut by_url: BTreeMap<String, Candidate> = BTreeMap::new();
    let mut warm_ids = Vec::new();
    let mut warm_urls = BTreeSet::new();
    let mut fresh_urls = Vec::new();

    // WARM first so it wins collisions.
    let max_match = substrate
        .hits
        .iter()
        .map(|hit| hit.match_score)
        .fold(0.0_f64, f64::max);
    for hit in &substrate.hits {
        let url_key = warm_url_key(hit);
        warm_ids.push(hit.node_id.clone());
        warm_urls.insert(url_key.clone());
        let candidate = warm_candidate(hit, max_match);
        by_url.insert(url_key, candidate);
    }

    // FRESH next; skip a url the warm pool already covers.
    let max_score = acquisition
        .candidates
        .iter()
        .map(|candidate| candidate.score)
        .fold(0.0_f64, f64::max);
    for ranked in &acquisition.candidates {
        let url_key = ranked.normalized_url.clone();
        fresh_urls.push(url_key.clone());
        if by_url.contains_key(&url_key) {
            // Warm copy already present; do not double-count.
            continue;
        }
        by_url.insert(url_key, fresh_candidate(ranked, max_score));
    }

    let _ = redundancy_penalty; // applied via ScoreContext, kept here for clarity.
    UnifiedPools {
        candidates: by_url.into_values().collect(),
        fresh_urls,
        warm_ids,
        warm_urls,
    }
}

/// A FRESH-pool candidate: provider snippet, no graph proximity, redundancy key
/// = url host so same-site results collapse at the gate.
fn fresh_candidate(ranked: &RankedSearchCandidate, max_score: f64) -> Candidate {
    let title = ranked.candidate.title.clone().unwrap_or_default();
    let snippet = ranked.candidate.snippet.clone().unwrap_or_default();
    let text = join_nonempty(&[&title, &snippet]);
    let token_count = approximate_tokens(&text);
    let mut candidate = Candidate::new(
        web_result_node_id(&ranked.normalized_url),
        text,
        token_count,
    )
    .with_source_arm(SourceArm::Web);
    candidate.ppr_proximity = 0.0;
    candidate.epistemic.source_reliability =
        Some((ranked.sources.len() as f32 / 3.0).clamp(0.0, 1.0));
    candidate.epistemic.support_ratio = Some((ranked.sources.len() as f32 / 2.0).clamp(0.0, 1.0));
    if max_score > 0.0 {
        candidate.metadata.insert(
            "provider_score_norm".to_string(),
            format!("{:.6}", ranked.score / max_score),
        );
    }
    if let Some(host) = host_key(&ranked.normalized_url) {
        candidate = candidate.with_redundancy_key(host);
    }
    candidate
        .metadata
        .insert("pool".to_string(), "fresh".to_string());
    candidate
        .metadata
        .insert("url".to_string(), ranked.candidate.url.clone());
    candidate
        .metadata
        .insert("normalized_url".to_string(), ranked.normalized_url.clone());
    candidate
        .metadata
        .insert("sources".to_string(), ranked.sources.join(","));
    candidate
}

/// A WARM-pool candidate: the extracted page snippet with REAL graph proximity
/// (the substrate search's PPR-aware `match_score`, normalized to 0..1).
fn warm_candidate(hit: &SearchHit, max_match: f64) -> Candidate {
    let text = join_nonempty(&[&hit.title, &hit.snippet]);
    let token_count = approximate_tokens(&text);
    let mut candidate =
        Candidate::new(hit.node_id.clone(), text, token_count).with_source_arm(SourceArm::Web);
    candidate.ppr_proximity = if max_match > 0.0 {
        (hit.match_score / max_match).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    // A corpus page (already fetched + extracted) is more reliable than a fresh
    // snippet; a frontier page (discovered, unfetched) less so.
    candidate.epistemic.source_reliability =
        Some(if hit.provenance == "corpus" { 0.9 } else { 0.4 });
    candidate.epistemic.support_ratio = Some(1.0);
    if let Some(host) = host_key(&hit.url) {
        candidate = candidate.with_redundancy_key(host);
    }
    candidate
        .metadata
        .insert("pool".to_string(), "warm".to_string());
    candidate
        .metadata
        .insert("url".to_string(), hit.url.clone());
    candidate
        .metadata
        .insert("ring".to_string(), hit.ring.to_string());
    candidate
        .metadata
        .insert("ring_label".to_string(), hit.ring_label.clone());
    candidate
        .metadata
        .insert("provenance".to_string(), hit.provenance.clone());
    candidate
        .metadata
        .insert("graph_score".to_string(), format!("{:.6}", hit.match_score));
    candidate
}

/// The dedupe key for a warm hit. Uses the SAME normalization the fresh pool's
/// `RankedSearchCandidate.normalized_url` uses (lowercase host, strip default
/// ports + fragment), so warm and fresh hash into one key space and a page
/// present in both collapses to a single candidate (warm wins, since it carries
/// real graph proximity). Falls back to the page node id when the url will not
/// parse. Previously this used `canonicalize_url`, which does not lowercase the
/// host or strip default ports, so a mixed-case host or an explicit :443 leaked
/// the same page in twice (once warm, once fresh at proximity 0).
fn warm_url_key(hit: &SearchHit) -> String {
    crate::search::normalize_candidate_url(&hit.url).unwrap_or_else(|| hit.node_id.clone())
}

/// The top result urls the fire-and-forget pass should fetch: FRESH-pool urls in
/// rank order whose canonical form the WARM pool does NOT already cover, capped
/// at `top_k`. No point re-fetching a page already in the substrate.
fn fetch_seed_urls(
    acquisition: &SearchAcquisition,
    warm_urls: &BTreeSet<String>,
    top_k: usize,
) -> Vec<String> {
    acquisition
        .candidates
        .iter()
        .filter(|ranked| {
            let canonical = crate::search::normalize_candidate_url(&ranked.candidate.url)
                .unwrap_or_else(|| ranked.normalized_url.clone());
            !warm_urls.contains(&canonical) && !warm_urls.contains(&ranked.normalized_url)
        })
        .take(top_k)
        .map(|ranked| ranked.candidate.url.clone())
        .collect()
}

fn web_result_node_id(normalized_url: &str) -> String {
    format!(
        "web:result:{}",
        blake3::hash(normalized_url.as_bytes()).to_hex()
    )
}

/// A stable, content-addressed reference to the subgraph this search touched:
/// the query plus the fresh urls plus the warm page node ids. Identical inputs
/// produce an identical ref.
fn subgraph_ref(query: &str, fresh_urls: &[String], warm_ids: &[String]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(query.as_bytes());
    hasher.update(b"\x00fresh\x00");
    let mut fresh_sorted: Vec<&String> = fresh_urls.iter().collect();
    fresh_sorted.sort();
    for url in fresh_sorted {
        hasher.update(url.as_bytes());
        hasher.update(b"\x00");
    }
    hasher.update(b"\x00warm\x00");
    let mut warm_sorted: Vec<&String> = warm_ids.iter().collect();
    warm_sorted.sort();
    for id in warm_sorted {
        hasher.update(id.as_bytes());
        hasher.update(b"\x00");
    }
    format!("web:subgraph:{}", hasher.finalize().to_hex())
}

fn host_key(url: &str) -> Option<String> {
    Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
}

fn join_nonempty(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Token estimate: ~chars/4, floor 1 (mirrors the spec's stated rule).
fn approximate_tokens(text: &str) -> usize {
    (text.chars().count() / 4).max(1)
}

#[cfg(test)]
mod tests {
    use rustyred_membrane::context_fetch;
    use rustyred_rerank::{LexicalCrossEncoder, NoopListwiseReranker, RerankScorer};
    use rustyred_thg_core::InMemoryGraphStore;

    use crate::search::{SearchProviderError, StaticSearchProvider};
    use crate::{build_fixture_crawl_graph, CrawlConfig, FixturePage, SearchCandidate};

    use super::*;

    fn web_scorer() -> RerankScorer {
        RerankScorer::web(Box::new(LexicalCrossEncoder::new("lexical-offline")))
    }

    /// A provider that always errors, to prove per-provider failure is recorded
    /// and the whole search still succeeds (acceptance #5).
    #[derive(Clone, Debug)]
    struct FailingProvider {
        name: String,
    }

    impl SearchProvider for FailingProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn search<'a>(
            &'a self,
            _query: &'a str,
            _opts: &'a SearchOpts,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<Vec<SearchCandidate>, SearchProviderError>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                Err(SearchProviderError::new(
                    self.name.clone(),
                    "quota exceeded",
                ))
            })
        }
    }

    #[derive(Clone, Debug)]
    struct ReverseListwiseReranker;

    impl ListwiseReranker for ReverseListwiseReranker {
        fn model_id(&self) -> &str {
            "test-listwise"
        }

        fn rerank(
            &self,
            mut candidates: Vec<Candidate>,
            _scorer: &dyn Scorer,
            _ctx: &ScoreContext<'_>,
        ) -> Vec<Candidate> {
            candidates.reverse();
            candidates
        }
    }

    fn fixture_searxng_provider() -> Arc<dyn SearchProvider> {
        // Stand in for a self-hosted SearXNG endpoint via the StaticSearchProvider
        // fixture (no network), named "searxng" so the receipt reads true.
        Arc::new(StaticSearchProvider::new(
            "searxng",
            vec![
                SearchCandidate {
                    url: "https://example.com/modernbert".to_string(),
                    title: Some("ModernBERT reranker".to_string()),
                    snippet: Some("fast sequence classification reranker".to_string()),
                    source: "searxng".to_string(),
                    rank: 1,
                },
                SearchCandidate {
                    url: "https://example.com/qwen".to_string(),
                    title: Some("Qwen reranker".to_string()),
                    snippet: Some("slow causal language model reranker".to_string()),
                    source: "searxng".to_string(),
                    rank: 2,
                },
            ],
        ))
    }

    #[tokio::test]
    async fn fanout_continues_when_one_provider_errors() {
        // Acceptance #5: a SearXNG-style provider plus a failing provider; the
        // failure is a recorded receipt, not a failed whole search.
        let providers: Vec<Arc<dyn SearchProvider>> = vec![
            fixture_searxng_provider(),
            Arc::new(FailingProvider {
                name: "brave".to_string(),
            }),
        ];
        let acquisition =
            fanout_search_providers(&providers, "fast reranker", SearchOpts::default()).await;

        assert_eq!(acquisition.providers.len(), 2);
        let searxng = acquisition
            .providers
            .iter()
            .find(|receipt| receipt.provider == "searxng")
            .expect("searxng receipt present");
        assert_eq!(searxng.status, "ok");
        let brave = acquisition
            .providers
            .iter()
            .find(|receipt| receipt.provider == "brave")
            .expect("brave receipt present");
        assert_eq!(brave.status, "error");
        assert_eq!(brave.error.as_deref(), Some("quota exceeded"));
        // The good provider's candidates survived the bad one's failure.
        assert!(!acquisition.candidates.is_empty());
    }

    #[tokio::test]
    async fn web_search_graph_gates_within_budget_and_handles_recover() {
        // Acceptance #2 reuse: an Admission within budget whose deferred handles
        // recover byte-exact through context_fetch.
        let mut store = InMemoryGraphStore::new();
        let providers: Vec<Arc<dyn SearchProvider>> = vec![fixture_searxng_provider()];
        let scorer = web_scorer();
        let options = WebSearchGraphOptions {
            budget_tokens: 6,
            fetch_top_k: 2,
            ..WebSearchGraphOptions::default()
        };

        let version = scorer.version();
        let result = web_search_graph(
            &mut store,
            &providers,
            "fast sequence reranker",
            &options,
            &scorer,
            Some(&NoopListwiseReranker),
            version,
        )
        .await
        .expect("web search graph");

        assert_eq!(result.receipt.source, Source::Web);
        assert!(result.tokens_admitted <= 6);
        assert!(!result.deferred_handles.is_empty());
        assert_eq!(
            result
                .deferred_handles
                .iter()
                .map(|handle| handle.token_count)
                .sum::<usize>(),
            result.tokens_deferred
        );
        // Every deferred handle recovers byte-exact from the store.
        for handle in &result.deferred_handles {
            let recovered = context_fetch(&store, handle).expect("deferred recovers");
            assert!(!recovered.is_empty());
        }
        assert!(result.subgraph_ref.starts_with("web:subgraph:"));
        // The membrane receipt landed in the store too.
        let receipt_node = store.get_node(&format!(
            "membrane:receipt:{}",
            result.receipt.content_address()
        ));
        assert!(receipt_node.is_some());
    }

    #[tokio::test]
    async fn listwise_rerank_feeds_gate_selection() {
        let mut store = InMemoryGraphStore::new();
        let providers: Vec<Arc<dyn SearchProvider>> = vec![Arc::new(StaticSearchProvider::new(
            "searxng",
            vec![
                SearchCandidate {
                    url: "https://a.example/first".to_string(),
                    title: Some("Alpha".to_string()),
                    snippet: None,
                    source: "searxng".to_string(),
                    rank: 1,
                },
                SearchCandidate {
                    url: "https://b.example/second".to_string(),
                    title: Some("Alpha".to_string()),
                    snippet: None,
                    source: "searxng".to_string(),
                    rank: 2,
                },
            ],
        ))];
        let scorer = web_scorer();
        let options = WebSearchGraphOptions {
            budget_tokens: 1,
            fetch_top_k: 0,
            ..WebSearchGraphOptions::default()
        };

        let result = web_search_graph(
            &mut store,
            &providers,
            "alpha",
            &options,
            &scorer,
            Some(&ReverseListwiseReranker),
            scorer.version(),
        )
        .await
        .expect("web search graph");

        assert_eq!(result.admitted_context.len(), 1);
        assert!(result.reranker_version.contains("listwise:test-listwise"));
        assert!(result.admitted_context[0]
            .metadata
            .get("url")
            .map(|url| url.contains("b.example/second"))
            .unwrap_or(false));
        assert!(result.admitted_context[0]
            .metadata
            .contains_key(rustyred_rerank::LISTWISE_RANK_SCORE_KEY));
    }

    #[tokio::test]
    async fn second_query_admits_warm_subgraph_context() {
        // Acceptance #4: seed the store with an already-extracted Page about the
        // query (what a prior FetchCompletionHook pass would have written), then
        // assert the search admits it from the WARM subgraph -- WITHOUT any
        // provider returning it.
        let mut store = InMemoryGraphStore::new();

        // A prior crawl extracted a page about ModernBERT into the substrate.
        let graph = build_fixture_crawl_graph(
            CrawlConfig {
                run_id: "warm-seed".to_string(),
                namespace: WARM_NAMESPACE.to_string(),
                user_agent: "test".to_string(),
            },
            &[FixturePage::html(
                "https://warm.example.com/modernbert-guide",
                "<html><body><h1>ModernBERT</h1><p>ModernBERT is a fast sequence \
                 classification reranker used in retrieval pipelines.</p></body></html>",
            )],
        )
        .expect("fixture crawl graph");
        graph.apply_to_store(&mut store).expect("seed warm page");

        // The provider pool returns something UNRELATED, so any warm admission
        // must come from the substrate, not the fresh pool.
        let providers: Vec<Arc<dyn SearchProvider>> = vec![Arc::new(StaticSearchProvider::new(
            "searxng",
            vec![SearchCandidate {
                url: "https://unrelated.example.org/weather".to_string(),
                title: Some("Weather".to_string()),
                snippet: Some("local forecast".to_string()),
                source: "searxng".to_string(),
                rank: 1,
            }],
        ))];
        let scorer = web_scorer();
        let options = WebSearchGraphOptions {
            budget_tokens: 4_000,
            ..WebSearchGraphOptions::default()
        };

        let version = scorer.version();
        let result = web_search_graph(
            &mut store,
            &providers,
            "modernbert reranker",
            &options,
            &scorer,
            None,
            version,
        )
        .await
        .expect("web search graph");

        let warm_admitted = result
            .admitted_context
            .iter()
            .find(|candidate| candidate.metadata.get("pool").map(String::as_str) == Some("warm"));
        let warm_admitted = warm_admitted.expect("a warm-pool candidate was admitted");
        assert!(warm_admitted
            .metadata
            .get("url")
            .map(|url| url.contains("warm.example.com"))
            .unwrap_or(false));
        // The warm candidate carries real graph proximity (PPR-aware), not 0.
        assert!(warm_admitted.ppr_proximity > 0.0);
    }

    #[test]
    fn write_fetched_pages_lands_state_fetched_pages_for_the_hook() {
        // The fire-and-forget write seam (tested without network): fetched pages
        // become state="fetched" Page nodes the FetchCompletionHook watches.
        let mut store = InMemoryGraphStore::new();
        let written = write_fetched_pages(
            &mut store,
            vec![FixturePage::html(
                "https://example.com/fetched-page",
                "<html><body>hello membrane</body></html>",
            )],
            "warm-run".to_string(),
        );
        assert_eq!(written, 1);

        // The page is in the substrate and search_substrate can read it back.
        let found = search_substrate(&store, "membrane", SearchOptions::default());
        assert!(found
            .hits
            .iter()
            .any(|hit| hit.url.contains("fetched-page")));
    }
}
