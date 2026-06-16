//! Code-arm context membrane (SPEC-CONTEXT-MEMBRANE-1.0, acceptance #7).
//!
//! Builds [`Candidate`]s from `CodeSymbol` nodes for a repo + task, ranks them
//! through the code-arm reranker (PPR-dominant), and gates the result to a token
//! budget through the shared membrane. Two things distinguish this from a raw
//! search:
//!
//! 1. **Warm centrality as a prior, refined by task-seeded PPR.** Each
//!    candidate's `ppr_proximity` blends the `centrality` value the
//!    [`IncrementalCentralityHook`](crate::incremental_centrality_hook) warmed
//!    onto the node with a seed-conditioned `personalized_pagerank` run over the
//!    repo's call/depend subgraph seeded on the symbols the task already
//!    touches. The warm prior keeps prompt-time cost low; the seed pass tilts
//!    proximity toward the task without a cold global PPR.
//! 2. **Budget gate with recoverable overflow.** Admission runs through
//!    [`admit_to_budget`], which persists every DEFERRED candidate byte-exact as
//!    a graph-resident node so each returned [`Handle`] recovers through
//!    [`context_fetch`]. Nothing is summarized or dropped at the gate.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rustyred_membrane::{
    admit_to_budget, context_fetch, emit_receipt, Admission, Candidate, Handle, MembraneReceipt,
    ScoreContext, Source, SourceArm,
};
use rustyred_rerank::{LexicalCrossEncoder, RerankScorer};
use rustyred_thg_core::{
    personalized_pagerank, Direction, NeighborQuery, NodeQuery, RedCoreGraphStore,
};
use serde_json::{json, Value};

use crate::ensure::{ensure_repo_kg_in_store, RepoKgStatus};
use crate::repo_fetch::{is_fetchable_repo_url, RepoFetchCaps};
use crate::{
    bounded_limit, hit_from_node, latest_repo_generations, normalize_tenant, property_string,
    property_u64, record_receipt, search_code_with_store, CodeHitRecord, CodeIndexError,
    SearchCodeInput, CALLS_SYMBOL, CENTRALITY_PROPERTY, CODE_SYMBOL_LABEL, DEFAULT_LIMIT,
    DEPENDS_ON_SYMBOL,
};

pub const DEFAULT_CONTEXT_PACK_BUDGET_TOKENS: usize = 2_000;
const DEFAULT_CONTEXT_PACK_CANDIDATE_LIMIT: u64 = (DEFAULT_LIMIT * 4) as u64;

/// Lexical-cross-encoder model id stamped into the reranker version. The offline
/// deterministic default (mirrors the web arm); a learned cross-encoder swaps in
/// behind the same `CrossEncoder` seam without touching this module.
const CODE_CROSS_ENCODER_ID: &str = "lexical-cross-encoder";

// Seed-conditioned PPR bounds. Mirrors the incremental-centrality hook so the
// task pass and the warm prior speak the same graph language.
const PPR_ALPHA: f64 = 0.15;
const PPR_EPSILON: f64 = 1e-4;
const PPR_MAX_PUSHES: usize = 100_000;
const PPR_EXPANSION_DEPTH: usize = 2;
const PPR_MAX_NEIGHBORHOOD: usize = 5_000;
/// Damping gain on the task-seeded PPR lift. The warm centrality prior is a
/// floor; a node in the task neighborhood is lifted toward 1.0 through the
/// remaining headroom, scaled by this gain so a strong centrality prior still
/// outranks a pure lexical match (the code arm is centrality/PPR-dominant). At
/// 0.5 the lift of a high-mass seed equals the prior convex blend, while a
/// low-mass called neighbor still rises above an equal-prior peer.
const SEED_LIFT_GAIN: f32 = 0.5;

#[derive(Clone, Debug, Default)]
pub struct CodeContextPackInput {
    pub tenant_id: String,
    pub query: String,
    pub repo_id: String,
    pub path_prefix: String,
    pub kinds: Vec<String>,
    pub limit: u64,
    pub budget_tokens: u64,
}

#[derive(Clone, Debug)]
pub struct CodeContextPackOutput {
    pub tenant_id: String,
    pub query: String,
    pub repo_id: String,
    pub admission: Admission,
    pub receipt: MembraneReceipt,
    pub total_candidates: u64,
    pub receipt_hash: String,
    pub receipt_json: String,
}

impl CodeContextPackOutput {
    pub fn to_json(&self) -> Value {
        json!({
            "tenant_id": self.tenant_id,
            "query": self.query,
            "repo_id": self.repo_id,
            "admitted_context": self.admission.admitted,
            "deferred_handles": self.admission.deferred,
            "tokens_admitted": self.admission.tokens_admitted,
            "tokens_deferred": self.admission.tokens_deferred,
            "total_candidates": self.total_candidates,
            "receipt": self.receipt,
            "receipt_hash": self.receipt_hash,
        })
    }
}

/// The spec-named code-arm context pack result. Carries the admission summary
/// (which symbols made the budget, at `file:line`), a composed code-map
/// markdown, the recoverable deferred handles, the token accounting, the
/// reranker version, and the SHA-keyed ingest mode that produced the graph this
/// pack read.
#[derive(Clone, Debug)]
pub struct ContextPackOutput {
    pub tenant_id: String,
    pub repo_id: String,
    pub query: String,
    /// One row per admitted symbol: `{ node_id, name, file_path, line,
    /// ppr_proximity }`.
    pub admitted: Vec<AdmittedSymbol>,
    /// Markdown code map of the admitted neighborhood (signatures + locations).
    pub code_map: String,
    /// Recoverable handles for the candidates the budget deferred. Each resolves
    /// byte-exact through [`context_fetch`].
    pub deferred_handles: Vec<Handle>,
    pub tokens_admitted: usize,
    pub tokens_deferred: usize,
    pub total_candidates: usize,
    pub reranker_version: String,
    /// How the repo's code graph was entered for this pack (snapshot load vs
    /// reindex vs full ingest). `None` when the pack ran over an
    /// already-resident graph with no entry step.
    pub ingest_status: Option<RepoKgStatus>,
    pub receipt: MembraneReceipt,
    pub receipt_hash: String,
}

impl ContextPackOutput {
    pub fn to_json(&self) -> Value {
        json!({
            "tenant_id": self.tenant_id,
            "repo_id": self.repo_id,
            "query": self.query,
            "admitted": self.admitted.iter().map(AdmittedSymbol::to_json).collect::<Vec<_>>(),
            "code_map": self.code_map,
            "deferred_handles": self.deferred_handles,
            "tokens_admitted": self.tokens_admitted,
            "tokens_deferred": self.tokens_deferred,
            "total_candidates": self.total_candidates,
            "reranker_version": self.reranker_version,
            "ingest_status": self.ingest_status.as_ref().map(RepoKgStatus::to_json),
            "receipt": self.receipt,
            "receipt_hash": self.receipt_hash,
        })
    }
}

/// One admitted symbol in a [`ContextPackOutput`].
#[derive(Clone, Debug)]
pub struct AdmittedSymbol {
    pub node_id: String,
    pub name: String,
    pub file_path: String,
    pub line: u64,
    pub ppr_proximity: f32,
}

impl AdmittedSymbol {
    pub fn location(&self) -> String {
        format!("{}:{}", self.file_path, self.line)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "node_id": self.node_id,
            "name": self.name,
            "file_path": self.file_path,
            "line": self.line,
            "location": self.location(),
            "ppr_proximity": self.ppr_proximity,
        })
    }
}

/// Spec-named code-arm context pack. Optionally ensures the repo's code graph is
/// resident at `sha` first (SHA-keyed: a current-sha re-entry loads the snapshot
/// rather than re-ingesting), then builds, ranks, and budget-gates a pack for
/// `task`.
///
/// `repo_url_or_id` is treated as a clone URL when it parses as a fetchable repo
/// url, in which case the ensure step runs over `store` and the derived repo id
/// is used for the pack. Otherwise it is used directly as the repo id with no
/// ingest step (`ingest_status` is `None`).
pub fn context_pack(
    store: &mut RedCoreGraphStore,
    tenant_id: &str,
    repo_url_or_id: &str,
    sha: Option<&str>,
    task: Option<&str>,
    budget_tokens: usize,
) -> Result<ContextPackOutput, CodeIndexError> {
    let tenant = normalize_tenant(tenant_id);
    let trimmed = repo_url_or_id.trim();

    let (repo_id, ingest_status) = if is_fetchable_repo_url(trimmed) {
        let status = ensure_repo_kg_in_store(
            store,
            &tenant,
            trimmed,
            sha,
            None,
            &RepoFetchCaps::default(),
        )?;
        (crate::ensure::repo_id_from_url(trimmed), Some(status))
    } else {
        (trimmed.to_string(), None)
    };

    let query = task.unwrap_or("").trim().to_string();
    let pack = build_code_context_pack(
        store,
        CodeContextPackInput {
            tenant_id: tenant.clone(),
            query: query.clone(),
            repo_id: repo_id.clone(),
            budget_tokens: budget_tokens as u64,
            ..CodeContextPackInput::default()
        },
        ingest_status.as_ref(),
    )?;

    let admitted: Vec<AdmittedSymbol> = pack
        .admission
        .admitted
        .iter()
        .map(admitted_symbol_from_candidate)
        .collect();
    let code_map = compose_code_map(&repo_id, &query, &admitted, &pack.admission);

    Ok(ContextPackOutput {
        tenant_id: pack.tenant_id,
        repo_id: pack.repo_id,
        query: pack.query,
        admitted,
        code_map,
        deferred_handles: pack.admission.deferred,
        tokens_admitted: pack.admission.tokens_admitted,
        tokens_deferred: pack.admission.tokens_deferred,
        total_candidates: pack.total_candidates as usize,
        reranker_version: pack.receipt.reranker_version.clone(),
        ingest_status,
        receipt: pack.receipt,
        receipt_hash: pack.receipt_hash,
    })
}

/// Recover the byte-exact text behind a deferred [`Handle`] this pack produced.
/// Thin re-export of the membrane integrity-checked fetch so callers do not need
/// the membrane crate directly.
pub fn context_pack_fetch(store: &RedCoreGraphStore, handle: &Handle) -> Option<String> {
    context_fetch(store, handle)
}

pub fn code_context_pack_in_store(
    store: &mut RedCoreGraphStore,
    input: CodeContextPackInput,
) -> Result<CodeContextPackOutput, CodeIndexError> {
    build_code_context_pack(store, input, None)
}

fn build_code_context_pack(
    store: &mut RedCoreGraphStore,
    input: CodeContextPackInput,
    ingest_status: Option<&RepoKgStatus>,
) -> Result<CodeContextPackOutput, CodeIndexError> {
    let tenant_id = normalize_tenant(&input.tenant_id);
    let query = input.query.trim().to_string();
    let repo_id = input.repo_id.trim().to_string();
    let candidate_limit = if input.limit == 0 {
        DEFAULT_CONTEXT_PACK_CANDIDATE_LIMIT
    } else {
        bounded_limit(input.limit) as u64
    };
    let search = search_code_with_store(
        store,
        SearchCodeInput {
            tenant_id: tenant_id.clone(),
            query: query.clone(),
            repo_id: repo_id.clone(),
            path_prefix: input.path_prefix.clone(),
            kinds: input.kinds.clone(),
            limit: candidate_limit,
        },
    )?;

    let mut hits = search.hits;
    // Seed-conditioned PPR over the call/depend subgraph, seeded on the symbols
    // the task (the lexical search) already surfaced. Captured before the warm
    // centrality hits are folded in below, so PPR seeds stay task-driven. Empty
    // when there is no task signal, in which case proximity falls back to the
    // warm prior alone.
    let task_seeds: Vec<String> = search_seed_ids(&hits);
    let mut seen: BTreeSet<String> = hits.iter().map(|hit| hit.node_id.clone()).collect();
    let centrality_hits = centrality_seed_hits(
        store,
        &tenant_id,
        &repo_id,
        &input.path_prefix,
        &input.kinds,
        candidate_limit as usize,
        &seen,
    )?;
    for hit in centrality_hits {
        seen.insert(hit.node_id.clone());
        hits.push(hit);
    }

    let ppr_scores = task_seeded_ppr(store, &task_seeds)?;

    let max_search_score = hits.iter().map(|hit| hit.score).fold(0.0_f64, f64::max);
    let mut candidates = Vec::with_capacity(hits.len());
    for hit in &hits {
        if let Some(candidate) =
            candidate_from_code_hit(store, hit, max_search_score, &ppr_scores)?
        {
            candidates.push(candidate);
        }
    }

    let scorer = RerankScorer::code(Box::new(LexicalCrossEncoder::new(CODE_CROSS_ENCODER_ID)));
    let active = Vec::new();
    let ctx = ScoreContext::new(&query, &active);
    let budget_tokens = if input.budget_tokens == 0 {
        DEFAULT_CONTEXT_PACK_BUDGET_TOKENS
    } else {
        input.budget_tokens as usize
    };
    let total_candidates = candidates.len() as u64;
    // admit_to_budget runs the same pure fill as fill_to_budget, then persists
    // each deferred candidate byte-exact so its Handle recovers via context_fetch.
    let admission = admit_to_budget(store, candidates, &scorer, &ctx, budget_tokens)
        .map_err(CodeIndexError::from_store)?;
    let baseline_tokens = admission.tokens_admitted + admission.tokens_deferred;
    let receipt = MembraneReceipt {
        source: Source::Code,
        candidates_scored: total_candidates as usize,
        tokens_admitted: admission.tokens_admitted,
        tokens_deferred: admission.tokens_deferred,
        reranker_version: scorer.version(),
        task_token_delta_vs_baseline: Some(
            baseline_tokens as i64 - admission.tokens_admitted as i64,
        ),
    };
    let membrane_receipt_hash = emit_receipt(store, &receipt).map_err(CodeIndexError::from_store)?;
    let mut receipt_payload = json!({
        "tenant_id": tenant_id,
        "operation": "code_context_pack",
        "query": query,
        "repo_id": repo_id,
        "budget_tokens": budget_tokens,
        "total_candidates": total_candidates,
        "tokens_admitted": admission.tokens_admitted,
        "tokens_deferred": admission.tokens_deferred,
        "membrane_receipt_hash": membrane_receipt_hash,
    });
    if let (Some(status), Some(map)) = (ingest_status, receipt_payload.as_object_mut()) {
        map.insert("ingest_status".to_string(), status.to_json());
    }
    let stored_receipt = record_receipt(store, &tenant_id, "code_context_pack", &receipt_payload)?;

    Ok(CodeContextPackOutput {
        tenant_id,
        query,
        repo_id,
        admission,
        receipt,
        total_candidates,
        receipt_hash: stored_receipt.receipt_hash,
        receipt_json: stored_receipt.receipt_json,
    })
}

/// Seed ids for the task-conditioned PPR: every symbol the lexical search
/// matched (score > 0), not a top-k cut. These are the "symbols the task already
/// touches" the pass tilts proximity toward.
fn search_seed_ids(hits: &[CodeHitRecord]) -> Vec<String> {
    hits.iter()
        .filter(|hit| hit.score > 0.0)
        .map(|hit| hit.node_id.clone())
        .collect()
}

/// Run seed-conditioned `personalized_pagerank` over the repo's call/depend
/// subgraph, seeded on `seeds`. Returns an empty map when there are no seeds (no
/// task signal) so proximity falls back to the warm centrality prior alone.
/// Mirrors the incremental-centrality hook's bounded-BFS + directed adjacency so
/// the task pass and the warm prior agree on graph shape.
fn task_seeded_ppr(
    store: &RedCoreGraphStore,
    seeds: &[String],
) -> Result<HashMap<String, f64>, CodeIndexError> {
    if seeds.is_empty() {
        return Ok(HashMap::new());
    }
    let edge_types = [CALLS_SYMBOL, DEPENDS_ON_SYMBOL];

    let mut neighborhood: BTreeSet<String> = seeds.iter().cloned().collect();
    let mut frontier: Vec<String> = seeds.to_vec();
    'expand: for _ in 0..PPR_EXPANSION_DEPTH {
        let mut next = Vec::new();
        for node_id in &frontier {
            for direction in [Direction::Out, Direction::In] {
                for edge_type in edge_types {
                    let hits = store
                        .neighbors(NeighborQuery {
                            node_id: node_id.clone(),
                            direction: direction.clone(),
                            edge_type: Some(edge_type.to_string()),
                            include_expired: false,
                        })
                        .map_err(CodeIndexError::from_store)?;
                    for hit in hits {
                        if neighborhood.insert(hit.node_id.clone()) {
                            if neighborhood.len() >= PPR_MAX_NEIGHBORHOOD {
                                break 'expand;
                            }
                            next.push(hit.node_id);
                        }
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }

    let mut adjacency: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for node_id in &neighborhood {
        let mut outs = Vec::new();
        for edge_type in edge_types {
            let hits = store
                .neighbors(NeighborQuery {
                    node_id: node_id.clone(),
                    direction: Direction::Out,
                    edge_type: Some(edge_type.to_string()),
                    include_expired: false,
                })
                .map_err(CodeIndexError::from_store)?;
            for hit in hits {
                if neighborhood.contains(&hit.node_id) {
                    let weight = hit.confidence.unwrap_or(1.0).max(0.0);
                    outs.push((hit.node_id, weight));
                }
            }
        }
        adjacency.insert(node_id.clone(), outs);
    }

    let seed_map: HashMap<String, f64> = seeds
        .iter()
        .filter(|seed| neighborhood.contains(*seed))
        .map(|seed| (seed.clone(), 1.0))
        .collect();
    if seed_map.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(personalized_pagerank(
        &adjacency,
        &seed_map,
        PPR_ALPHA,
        PPR_EPSILON,
        PPR_MAX_PUSHES,
    ))
}

fn admitted_symbol_from_candidate(candidate: &Candidate) -> AdmittedSymbol {
    let meta = |key: &str| candidate.metadata.get(key).cloned().unwrap_or_default();
    AdmittedSymbol {
        node_id: candidate.node_id.clone(),
        name: meta("name"),
        file_path: meta("file_path"),
        line: meta("line").parse::<u64>().unwrap_or(0),
        ppr_proximity: candidate.ppr_proximity,
    }
}

/// Compose the admitted neighborhood into a markdown code map, mirroring the
/// grpc AM5 context_pack idiom: one line per symbol, `- \`name\` (file:line) --
/// signature`.
fn compose_code_map(
    repo_id: &str,
    query: &str,
    admitted: &[AdmittedSymbol],
    admission: &Admission,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    if query.trim().is_empty() {
        lines.push(format!("# Code context for `{repo_id}`"));
    } else {
        lines.push(format!("# Code context for `{repo_id}` -- {query}"));
    }
    lines.push(String::new());
    if admitted.is_empty() {
        lines.push("_No code neighborhood resolved within budget._".to_string());
        return lines.join("\n");
    }
    lines.push("**Admitted code (membrane-ranked):**".to_string());
    for (symbol, candidate) in admitted.iter().zip(admission.admitted.iter()) {
        let signature = candidate
            .text
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        let mut entry = format!("- `{}` ({})", symbol.name, symbol.location());
        if !signature.is_empty() && signature != symbol.name {
            entry.push_str(&format!(" -- {}", truncate(&signature, 160)));
        }
        lines.push(entry);
    }
    if admission.tokens_deferred > 0 {
        lines.push(String::new());
        lines.push(format!(
            "_{} token(s) of lower-ranked context deferred (recoverable)._",
            admission.tokens_deferred
        ));
    }
    lines.join("\n")
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

fn candidate_from_code_hit(
    store: &RedCoreGraphStore,
    hit: &CodeHitRecord,
    max_search_score: f64,
    ppr_scores: &HashMap<String, f64>,
) -> Result<Option<Candidate>, CodeIndexError> {
    let Some(node) = store
        .get_node(&hit.node_id)
        .map_err(CodeIndexError::from_store)?
    else {
        return Ok(None);
    };
    if !node.labels.iter().any(|label| label == CODE_SYMBOL_LABEL) {
        return Ok(None);
    }

    let signature = property_string(&node.properties, "signature").unwrap_or_default();
    let snippet =
        property_string(&node.properties, "snippet").unwrap_or_else(|| hit.snippet.clone());
    let text = [
        signature.as_str(),
        snippet.as_str(),
        hit.file_path.as_str(),
        hit.name.as_str(),
    ]
    .into_iter()
    .filter(|part| !part.trim().is_empty())
    .collect::<Vec<_>>()
    .join("\n");
    // Count the realized payload (the full `text` that enters the window), not
    // just signature + snippet, so the gate budget and the receipt's
    // tokens_admitted reflect the bytes actually admitted -- the cost lever stays
    // honest. No redundancy_key: for code, same-file co-location is signal, not
    // duplication, so redundancy falls back to content-based lexical overlap,
    // which still collapses genuinely near-identical boilerplate symbols.
    let token_count = text.split_whitespace().count().max(1);
    let mut candidate =
        Candidate::new(hit.node_id.clone(), text, token_count).with_source_arm(SourceArm::Code);
    let normalized_search = if max_search_score > 0.0 {
        (hit.score / max_search_score).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let centrality = node
        .properties
        .get(CENTRALITY_PROPERTY)
        .and_then(Value::as_f64)
        .map(|value| value.clamp(0.0, 1.0) as f32)
        .unwrap_or(0.0);
    // Seed-conditioned PPR for this node (0 when outside the task neighborhood).
    let seeded = ppr_scores
        .get(&hit.node_id)
        .map(|value| value.clamp(0.0, 1.0) as f32)
        .unwrap_or(0.0);
    candidate.ppr_proximity =
        combine_proximity(centrality, seeded, normalized_search);
    candidate.metadata = BTreeMap::from([
        ("repo_id".to_string(), hit.repo_id.clone()),
        ("file_id".to_string(), hit.file_id.clone()),
        ("file_path".to_string(), hit.file_path.clone()),
        ("kind".to_string(), hit.kind.clone()),
        ("name".to_string(), hit.name.clone()),
        ("line".to_string(), hit.line.to_string()),
        ("search_score".to_string(), format!("{:.6}", hit.score)),
        ("centrality".to_string(), format!("{:.6}", centrality)),
        ("ppr_seeded".to_string(), format!("{:.6}", seeded)),
    ]);
    Ok(Some(candidate))
}

/// Blend the warm centrality prior with the task-seeded PPR score into the
/// `ppr_proximity` the code arm weights heavily. When neither structural signal
/// is present, fall back to a damped normalized search score so a freshly
/// ingested (un-warmed) graph still ranks sensibly.
fn combine_proximity(centrality: f32, seeded: f32, normalized_search: f32) -> f32 {
    if centrality <= 0.0 && seeded <= 0.0 {
        return normalized_search * 0.25;
    }
    // Prior-as-floor lift: the warm centrality prior anchors proximity, and the
    // task-seeded PPR lifts a node that sits in the task neighborhood toward 1.0
    // through the remaining headroom. A node outside the neighborhood (seeded ==
    // 0) keeps its prior exactly; a positive task signal never pulls a node below
    // its prior. A convex average would (and did) mis-rank a called neighbor
    // beneath an unrelated symbol of equal centrality.
    let prior = centrality.clamp(0.0, 1.0);
    let lift = SEED_LIFT_GAIN * seeded.clamp(0.0, 1.0) * (1.0 - prior);
    (prior + lift).clamp(0.0, 1.0)
}

fn centrality_seed_hits(
    store: &RedCoreGraphStore,
    tenant_id: &str,
    repo_id: &str,
    path_prefix: &str,
    kinds: &[String],
    limit: usize,
    already_seen: &BTreeSet<String>,
) -> Result<Vec<CodeHitRecord>, CodeIndexError> {
    let latest = latest_repo_generations(store)?;
    let normalized_kinds = kinds
        .iter()
        .map(|kind| kind.trim().to_ascii_lowercase())
        .filter(|kind| !kind.is_empty())
        .collect::<BTreeSet<_>>();
    let mut query = NodeQuery::label(CODE_SYMBOL_LABEL).with_limit(100_000);
    if !repo_id.trim().is_empty() {
        query = query.with_property("repo_id", json!(repo_id.trim()));
    }
    let mut hits = store
        .query_nodes(query)
        .map_err(CodeIndexError::from_store)?
        .into_iter()
        .filter(|node| !already_seen.contains(&node.id))
        .filter(|node| node.properties.get("tenant_id").and_then(Value::as_str) == Some(tenant_id))
        .filter(|node| match property_string(&node.properties, "repo_id") {
            Some(repo_id) => latest
                .get(&repo_id)
                .map(|generation| property_u64(&node.properties, "generation") == Some(*generation))
                .unwrap_or(true),
            None => false,
        })
        .filter_map(|node| {
            let centrality = node
                .properties
                .get(CENTRALITY_PROPERTY)
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            if centrality <= 0.0 {
                return None;
            }
            let mut hit = hit_from_node(&node)?;
            if !path_prefix.trim().is_empty() && !hit.file_path.starts_with(path_prefix.trim()) {
                return None;
            }
            if !normalized_kinds.is_empty()
                && !normalized_kinds.contains(&hit.kind.to_ascii_lowercase())
            {
                return None;
            }
            hit.score = centrality;
            Some(hit)
        })
        .collect::<Vec<_>>();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.file_path.cmp(&b.file_path))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.name.cmp(&b.name))
    });
    hits.truncate(limit.saturating_sub(already_seen.len()));
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use rustyred_thg_core::{
        EdgeRecord, GraphMutation, GraphMutationBatch, NodeRecord, RedCoreGraphStore,
        RedCoreOptions,
    };

    use super::*;
    use crate::{CALLS_SYMBOL, CODE_REPO_LABEL};

    fn temp_store() -> RedCoreGraphStore {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        RedCoreGraphStore::open(
            std::env::temp_dir().join(format!("context-pack-test-{}-{nanos}", std::process::id())),
            RedCoreOptions::default(),
        )
        .unwrap()
    }

    #[test]
    fn context_pack_uses_warm_centrality_and_budgeted_membrane_admission() {
        let mut store = temp_store();
        let repo = NodeRecord::new(
            "repo:test",
            [CODE_REPO_LABEL],
            json!({
                "tenant_id": "theorem",
                "repo_id": "repo",
                "latest_generation": 1,
            }),
        );
        let central = NodeRecord::new(
            "sym:central",
            [CODE_SYMBOL_LABEL],
            json!({
                "tenant_id": "theorem",
                "repo_id": "repo",
                "file_id": "file:a",
                "file_path": "src/a.rs",
                "kind": "function",
                "name": "central_symbol",
                "language": "rust",
                "line": 1,
                "signature": "pub fn central_symbol()",
                "snippet": "unrelated body",
                "generation": 1,
                CENTRALITY_PROPERTY: 0.95,
            }),
        );
        let lexical = NodeRecord::new(
            "sym:lexical",
            [CODE_SYMBOL_LABEL],
            json!({
                "tenant_id": "theorem",
                "repo_id": "repo",
                "file_id": "file:b",
                "file_path": "src/b.rs",
                "kind": "function",
                "name": "query_symbol",
                "language": "rust",
                "line": 1,
                "signature": "pub fn query_symbol()",
                "snippet": "query parser exact match",
                "generation": 1,
                CENTRALITY_PROPERTY: 0.10,
            }),
        );
        store
            .commit_batch(GraphMutationBatch {
                mutations: vec![
                    GraphMutation::NodeUpsert(repo),
                    GraphMutation::NodeUpsert(central),
                    GraphMutation::NodeUpsert(lexical),
                ],
            })
            .unwrap();

        let output = code_context_pack_in_store(
            &mut store,
            CodeContextPackInput {
                tenant_id: "theorem".to_string(),
                query: "query parser".to_string(),
                repo_id: "repo".to_string(),
                budget_tokens: 8,
                ..CodeContextPackInput::default()
            },
        )
        .unwrap();

        assert!(output.admission.tokens_admitted <= 8);
        assert_eq!(output.receipt.source, Source::Code);
        assert_eq!(output.total_candidates, 2);
        // Warm centrality prior ranks the high-centrality symbol first even
        // though the query lexically matches the other.
        assert_eq!(output.admission.admitted[0].node_id, "sym:central");
        assert!(output.receipt.task_token_delta_vs_baseline.unwrap() >= 0);
        let receipt_node = store
            .get_node(&format!(
                "membrane:receipt:{}",
                output.receipt.content_address()
            ))
            .unwrap();
        assert!(receipt_node.is_some());
    }

    #[test]
    fn deferred_handles_recover_byte_exact_via_context_fetch() {
        let mut store = temp_store();
        let repo = NodeRecord::new(
            "repo:test",
            [CODE_REPO_LABEL],
            json!({ "tenant_id": "theorem", "repo_id": "repo", "latest_generation": 1 }),
        );
        let mut mutations = vec![GraphMutation::NodeUpsert(repo)];
        for idx in 0..4 {
            mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
                format!("sym:{idx}"),
                [CODE_SYMBOL_LABEL],
                json!({
                    "tenant_id": "theorem",
                    "repo_id": "repo",
                    "file_id": format!("file:{idx}"),
                    "file_path": format!("src/f{idx}.rs"),
                    "kind": "function",
                    "name": format!("parser_symbol_{idx}"),
                    "language": "rust",
                    "line": idx + 1,
                    "signature": format!("pub fn parser_symbol_{idx}(query: &str) -> usize"),
                    "snippet": format!("parser body number {idx} with query tokens"),
                    "generation": 1,
                    CENTRALITY_PROPERTY: 0.10 * (idx as f64 + 1.0),
                }),
            )));
        }
        store
            .commit_batch(GraphMutationBatch { mutations })
            .unwrap();

        // Tiny budget forces most candidates to defer.
        let output = code_context_pack_in_store(
            &mut store,
            CodeContextPackInput {
                tenant_id: "theorem".to_string(),
                query: "parser query".to_string(),
                repo_id: "repo".to_string(),
                budget_tokens: 6,
                ..CodeContextPackInput::default()
            },
        )
        .unwrap();

        assert!(output.admission.tokens_admitted <= 6);
        assert!(
            !output.admission.deferred.is_empty(),
            "tiny budget must defer some candidates"
        );
        // Every deferred handle recovers byte-exact from the store.
        for handle in &output.admission.deferred {
            let recovered = context_pack_fetch(&store, handle)
                .expect("deferred handle must recover from the store");
            assert_eq!(
                rustyred_membrane::Handle::from_candidate(
                    &Candidate::new(handle.node_id.clone(), recovered.clone(), handle.token_count)
                )
                .digest,
                handle.digest,
                "recovered text must re-hash to the handle digest"
            );
        }
    }

    #[test]
    fn task_seeded_ppr_lifts_a_called_neighbor_above_an_unrelated_symbol() {
        let mut store = temp_store();
        // Graph: query_seed CALLS callee_neighbor. An unrelated_symbol has the
        // same warm centrality as callee_neighbor but no task-seed proximity, so
        // the seeded PPR pass must rank the neighbor above it.
        let repo = NodeRecord::new(
            "repo:test",
            [CODE_REPO_LABEL],
            json!({ "tenant_id": "theorem", "repo_id": "repo", "latest_generation": 1 }),
        );
        let seed = NodeRecord::new(
            "sym:query_seed",
            [CODE_SYMBOL_LABEL],
            json!({
                "tenant_id": "theorem", "repo_id": "repo", "file_id": "file:a",
                "file_path": "src/a.rs", "kind": "function", "name": "query_seed",
                "language": "rust", "line": 1,
                "signature": "pub fn query_seed()", "snippet": "query parser entry",
                "generation": 1, CENTRALITY_PROPERTY: 0.05,
            }),
        );
        let neighbor = NodeRecord::new(
            "sym:callee_neighbor",
            [CODE_SYMBOL_LABEL],
            json!({
                "tenant_id": "theorem", "repo_id": "repo", "file_id": "file:b",
                "file_path": "src/b.rs", "kind": "function", "name": "callee_neighbor",
                "language": "rust", "line": 1,
                "signature": "pub fn callee_neighbor()", "snippet": "downstream helper",
                "generation": 1, CENTRALITY_PROPERTY: 0.20,
            }),
        );
        let unrelated = NodeRecord::new(
            "sym:unrelated",
            [CODE_SYMBOL_LABEL],
            json!({
                "tenant_id": "theorem", "repo_id": "repo", "file_id": "file:c",
                "file_path": "src/c.rs", "kind": "function", "name": "unrelated_symbol",
                "language": "rust", "line": 1,
                "signature": "pub fn unrelated_symbol()", "snippet": "isolated code",
                "generation": 1, CENTRALITY_PROPERTY: 0.20,
            }),
        );
        let calls = EdgeRecord::new(
            "edge:seed-calls-neighbor",
            "sym:query_seed",
            CALLS_SYMBOL,
            "sym:callee_neighbor",
            json!({ "tenant_id": "theorem", "repo_id": "repo" }),
        );
        store
            .commit_batch(GraphMutationBatch {
                mutations: vec![
                    GraphMutation::NodeUpsert(repo),
                    GraphMutation::NodeUpsert(seed),
                    GraphMutation::NodeUpsert(neighbor),
                    GraphMutation::NodeUpsert(unrelated),
                    GraphMutation::EdgeUpsert(calls),
                ],
            })
            .unwrap();

        let output = code_context_pack_in_store(
            &mut store,
            CodeContextPackInput {
                tenant_id: "theorem".to_string(),
                query: "query parser".to_string(),
                repo_id: "repo".to_string(),
                budget_tokens: 4_000,
                ..CodeContextPackInput::default()
            },
        )
        .unwrap();

        let proximity = |id: &str| {
            output
                .admission
                .admitted
                .iter()
                .find(|c| c.node_id == id)
                .map(|c| c.ppr_proximity)
                .unwrap_or(0.0)
        };
        assert!(
            proximity("sym:callee_neighbor") > proximity("sym:unrelated"),
            "task-seeded PPR must lift the called neighbor ({}) above the unrelated symbol ({})",
            proximity("sym:callee_neighbor"),
            proximity("sym:unrelated"),
        );
    }
}
