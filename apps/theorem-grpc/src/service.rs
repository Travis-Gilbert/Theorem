//! theseus_search.v1.SearchService implementation.
//!
//! Four RPCs. NONE fabricate:
//!   - Search:     REAL graph rank via `rustyred_web::search_substrate`.
//!   - GapWalk:    REAL single-round PPR over the existing substrate via
//!     `rustyred_thg_core::personalized_pagerank`.
//!   - SourcePair: honest-EMPTY (no source/web bipartite anchoring layer yet).
//!   - Provenance: the REAL single node if it resolves, else honest-empty.
//!
//! Every engine path here is pure Rust. No PyO3, no Django, no open-web crawl.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use rustyred_thg_core::personalized_pagerank;
use rustyred_web::{search_substrate, SearchOptions, SubstrateSearch};
use tonic::{Request, Response, Status};

use crate::engine::Engine;
use crate::pb;

// PPR tuning for the GapWalk single-round expansion. Mirrors the defaults the
// substrate search itself uses internally (ACL local-push PPR). Held as local
// constants because the upstream DEFAULT_PPR_* consts are private to rustyred-web.
const GAPWALK_PPR_ALPHA: f64 = 0.15;
const GAPWALK_PPR_EPSILON: f64 = 1e-6;
const GAPWALK_PPR_MAX_PUSHES: usize = 10_000;

/// The tonic service. Holds the engine behind an `Arc` so the owned substrate
/// outlives every (borrowing) handler call.
pub struct TheoremSearchService {
    engine: Arc<Engine>,
}

impl TheoremSearchService {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self { engine }
    }
}

#[tonic::async_trait]
impl pb::SearchService for TheoremSearchService {
    /// REAL graph-grounded search. Maps `search_substrate` (sync, infallible)
    /// onto SearchResponse. mode / source_pair / bbox / time_range are
    /// accepted-and-ignored for slice 1 (see field comments below): the server
    /// always runs the same single graph-grounded round.
    async fn search(
        &self,
        request: Request<pb::SearchRequest>,
    ) -> Result<Response<pb::SearchResponse>, Status> {
        let req = request.into_inner();

        // Named binding: the store must outlive the &borrow `search_substrate`
        // takes. The engine owns it, so `store()` hands back a borrow valid for
        // the whole call (contrast the reference bridge, which returns an
        // InMemoryGraphStore BY VALUE from from_snapshot and so must bind it).
        let store = self.engine.store();

        // mode (req.mode): accepted-and-ignored. SEARCH_MODE_CIVIC_ATLAS(4) and
        // every other mode run the identical single graph-grounded round in
        // slice 1. Multi-round / source-paired behavior is a named follow-up.
        // source_pair (req.source_pair): accepted-and-ignored, same reason.
        // bbox / time_range (req.bbox, req.time_range): accepted-and-ignored.
        // search_substrate has no spatial/temporal argument; composing
        // SpatialBackend::bbox_search + TimeInterval::overlaps is a named
        // follow-up, NOT slice 1.
        let started = Instant::now();
        let result: SubstrateSearch = search_substrate(store, &req.query, SearchOptions::default());
        let latency_ms = started.elapsed().as_millis() as u64;

        let response = map::to_search_response(req.query, result, latency_ms);
        Ok(Response::new(response))
    }

    /// REAL single-round PPR over the EXISTING substrate. Does NOT crawl the
    /// open web in slice 1 (that needs robots/SSRF policy + a populated
    /// frontier). Seeds PPR with the ring-0 hits from a substrate search, runs
    /// `personalized_pagerank` over the `LINKS_TO` adjacency among hits, and
    /// returns the ranked neighbours as admitted_evidence (source="graph").
    /// Empty substrate => empty GapWalkResponse. Open-web GapWalk via
    /// BrowserSessionStore::search_or_crawl is the named enrichment for a later slice.
    async fn gap_walk(
        &self,
        request: Request<pb::GapWalkRequest>,
    ) -> Result<Response<pb::GapWalkResponse>, Status> {
        let req = request.into_inner();
        let store = self.engine.store();

        let started = Instant::now();
        let search = search_substrate(store, &req.query, SearchOptions::default());

        // Build PPR adjacency from the public LINKS_TO edges among hits, and
        // seed with the ring-0 (directly matched) hits. Both come straight off
        // the SubstrateSearch the engine just produced; no private helper needed.
        let adjacency = map::adjacency_from_links(&search);
        let seeds = map::ring0_seeds(&search);

        let admitted_evidence = if seeds.is_empty() {
            // Nothing matched (e.g. empty substrate) => honest empty result.
            Vec::new()
        } else {
            let ppr = personalized_pagerank(
                &adjacency,
                &seeds,
                GAPWALK_PPR_ALPHA,
                GAPWALK_PPR_EPSILON,
                GAPWALK_PPR_MAX_PUSHES,
            );
            map::ppr_to_results(&search, &ppr)
        };

        let latency_ms = started.elapsed().as_millis() as u64;

        let response = pb::GapWalkResponse {
            // We did not synthesize gap descriptions; surfacing fabricated gaps
            // would violate the no-fake rule. Honest empty.
            gaps: Vec::new(),
            admitted_evidence,
            rounds_executed: 1,
            hit_round_cap: false,
            latency_ms,
        };
        Ok(Response::new(response))
    }

    /// HONEST-EMPTY. There is no source/web bipartite cross-anchoring index in
    /// the substrate for slice 1, so every repeated field is empty. This is the
    /// truthful empty state, NOT a bug: real source-pairing requires the
    /// source/web anchoring layer, which is not yet ingested. A later session
    /// must not mistake this empty response for a defect.
    async fn source_pair(
        &self,
        request: Request<pb::SourcePairRequest>,
    ) -> Result<Response<pb::SourcePairResponse>, Status> {
        let _req = request.into_inner();
        Ok(Response::new(pb::SourcePairResponse {
            source_candidates: Vec::new(),
            web_candidates: Vec::new(),
            anchored_results: Vec::new(),
            exploratory_results: Vec::new(),
        }))
    }

    /// REAL node or honest-empty. If `result_id` resolves to a Page node in the
    /// store, return a single-node ProvenanceGraph rooted at it. If it does not
    /// resolve, return an empty graph with root_result_id = the requested id.
    /// Never fabricates seeded / expanded_via_ppr edges. Full provenance
    /// (building a HarnessInstantKg over the snapshot and walking explain_edge
    /// for every admitted edge) is the named enrichment.
    async fn provenance(
        &self,
        request: Request<pb::ProvenanceRequest>,
    ) -> Result<Response<pb::ProvenanceGraph>, Status> {
        let req = request.into_inner();
        let store = self.engine.store();

        let graph = map::provenance_for(store, &req.result_id);
        Ok(Response::new(graph))
    }
}

/// Pure mapping functions: SubstrateSearch -> proto messages. No I/O, no
/// fabrication. Kept private to the service module.
mod map {
    use super::*;
    use rustyred_thg_core::{InMemoryGraphStore, NodeQuery};
    use rustyred_web::{SearchHit, LABEL_PAGE};

    /// Map a finished substrate search onto the wire SearchResponse.
    ///
    /// For slice 1 ALL hits go into `prior_knowledge` ("what the substrate
    /// already knew") and `new_evidence` is empty, because no gap-walk /
    /// open-web round happened this call. `gap_closures` is empty (nothing
    /// closed a gap), `provenance_root_id` is "" (no provenance graph rooted
    /// this round; the Provenance RPC returns empty for an unknown root).
    pub fn to_search_response(
        query: String,
        search: SubstrateSearch,
        latency_ms: u64,
    ) -> pb::SearchResponse {
        let total_admitted = search.matched_count as u32;
        let total_returned = search.kept_count as u32;
        let prior_knowledge: Vec<pb::SearchResult> =
            search.hits.iter().map(hit_to_result).collect();

        pb::SearchResponse {
            query,
            total_admitted,
            total_returned,
            prior_knowledge,
            new_evidence: Vec::new(),
            gap_closures: Vec::new(),
            provenance_root_id: String::new(),
            rounds_executed: 1,
            latency_ms,
        }
    }

    /// One substrate hit -> one wire SearchResult.
    ///
    /// confidence = 0.0 is INTENTIONAL and not a fabricated 0.6: substrate
    /// search emits no epistemic confidence, and the proto documents
    /// min_confidence = 0.0 as "use orchestrator defaults" (i.e. unset). Setting
    /// a fake 0.6 would lie about epistemic strength.
    pub fn hit_to_result(hit: &SearchHit) -> pb::SearchResult {
        pb::SearchResult {
            result_id: hit.node_id.clone(),
            // Substrate nodes are Page-labelled.
            kind: "page".to_string(),
            label: hit.title.clone(),
            snippet: hit.snippet.clone(),
            relevance_score: hit.match_score,
            confidence: 0.0,
            source: "graph".to_string(),
            url: hit.url.clone(),
            closes_gap_id: String::new(),
        }
    }

    /// Build a PPR adjacency (`source -> [(target, weight)]`) from the public
    /// LINKS_TO edges that survived inside the search neighbourhood. Each edge
    /// carries unit weight; personalized_pagerank normalizes by out-degree.
    pub fn adjacency_from_links(search: &SubstrateSearch) -> HashMap<String, Vec<(String, f64)>> {
        let mut adjacency: HashMap<String, Vec<(String, f64)>> = HashMap::new();
        for link in &search.links {
            adjacency
                .entry(link.source.clone())
                .or_default()
                .push((link.target.clone(), 1.0));
        }
        adjacency
    }

    /// Seed PPR with the ring-0 (directly matched) hits, mass split evenly so
    /// the residual sums to ~1.0 as personalized_pagerank expects.
    pub fn ring0_seeds(search: &SubstrateSearch) -> HashMap<String, f64> {
        let ring0: Vec<&SearchHit> = search.hits.iter().filter(|h| h.ring == 0).collect();
        if ring0.is_empty() {
            return HashMap::new();
        }
        let mass = 1.0 / ring0.len() as f64;
        ring0
            .into_iter()
            .map(|h| (h.node_id.clone(), mass))
            .collect()
    }

    /// Turn PPR scores into ranked SearchResults (source="graph"), highest
    /// score first, deterministic on ties by node id. Only nodes that PPR
    /// touched AND that are present in the search neighbourhood are emitted, so
    /// every result carries real title/url/snippet metadata.
    pub fn ppr_to_results(
        search: &SubstrateSearch,
        ppr: &HashMap<String, f64>,
    ) -> Vec<pb::SearchResult> {
        let hit_by_id: HashMap<&str, &SearchHit> = search
            .hits
            .iter()
            .map(|h| (h.node_id.as_str(), h))
            .collect();

        let mut scored: Vec<(&SearchHit, f64)> = ppr
            .iter()
            .filter_map(|(id, score)| hit_by_id.get(id.as_str()).map(|h| (*h, *score)))
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.node_id.cmp(&b.0.node_id))
        });

        scored
            .into_iter()
            .map(|(hit, score)| {
                let mut result = hit_to_result(hit);
                // Override relevance with the PPR score from this expansion.
                result.relevance_score = score;
                result
            })
            .collect()
    }

    /// Build a single-node ProvenanceGraph if `result_id` resolves to a Page
    /// node in the store, else an empty graph rooted at the requested id.
    pub fn provenance_for(store: &InMemoryGraphStore, result_id: &str) -> pb::ProvenanceGraph {
        // Resolve by scanning Page nodes for a matching id. Bounded scan is fine
        // for slice 1 (small substrate); a future enrichment can index by id.
        let pages = store.query_nodes(NodeQuery::label(LABEL_PAGE));
        let found = pages.into_iter().find(|p| p.id == result_id);

        match found {
            Some(node) => {
                let label = title_from_node(&node);
                pb::ProvenanceGraph {
                    nodes: vec![pb::ProvenanceNode {
                        node_id: result_id.to_string(),
                        kind: "result".to_string(),
                        label,
                        metadata_json: "{}".to_string(),
                    }],
                    edges: Vec::new(),
                    root_result_id: result_id.to_string(),
                }
            }
            None => pb::ProvenanceGraph {
                nodes: Vec::new(),
                edges: Vec::new(),
                root_result_id: result_id.to_string(),
            },
        }
    }

    /// Derive a human label for a Page node from its url property (last path
    /// segment or host), falling back to the node id.
    fn title_from_node(node: &rustyred_thg_core::NodeRecord) -> String {
        let url = node
            .properties
            .get("url")
            .and_then(|v| v.as_str())
            .or_else(|| {
                node.properties
                    .get("canonical_url")
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("");
        if url.is_empty() {
            return node.id.clone();
        }
        let trimmed = url.trim_end_matches('/');
        match trimmed.rsplit('/').next() {
            Some(seg) if !seg.is_empty() => seg.to_string(),
            _ => url.to_string(),
        }
    }
}
