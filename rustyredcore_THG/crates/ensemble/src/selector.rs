//! The budgeted capability selector (slice S2) + trust gating (slice S3).
//!
//! [`select`] is a PURE, deterministic function: given a task, a budget, an explicit set of
//! candidate [`CapabilityPack`]s, and priors, it returns a replayable [`EnsembleDecision`]. Keeping
//! the candidate set explicit (rather than querying the store inside the scorer) is what makes the
//! core replayable: the same inputs always produce the same decision, with no store, clock, or map
//! iteration order in the loop.
//!
//! The heavy scoring (embeddings, PPR, MAP-Elites priors) stays in the offline Python learning
//! workbench. That workbench writes the `priors` this selector reads (content-addressed by
//! `pack_content_hash`). When a learned prior is absent the selector falls back to a cheap,
//! deterministic lexical overlap between the task and the pack's title/description/capabilities --
//! the same cold-start posture `rustyred-thg-affordances` uses beneath the tool layer.
//!
//! [`select_from_store`] is the thin store-backed wrapper: it gathers candidates from the registry
//! via [`crate::registry::list_packs`] and then calls the pure [`select`].

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::decision::{EnsembleDecision, RejectedCandidate, SelectedCapability};
use crate::outcomes::{
    effective_pack_fitness_from_node, PACK_SEQUENCED_WITH, PACK_SERVED_TASK, TASK_IN_DOMAIN,
};
use crate::registry::{
    domain_node_id, list_packs, pack_node_id, CapabilityPack, EnsembleGraphStore, EnsembleResult,
    PackKind, TrustTier, PACK_EXPOSES_AFFORDANCE, PACK_IN_DOMAIN,
};
use crate::trust::{meets_floor, parse_trust_floor, trust_rank, trust_score};
use rustyred_thg_affordances::{
    affordance_nodes, task_type_node_id, Affordance, AffordanceGraphStore, CapabilityScope,
    DEFAULT_COLD_START_SCORE, DEFAULT_MIN_FITNESS, PRODUCED_OUTCOME, SEQUENCED_WITH, SERVED_TASK,
};
use rustyred_thg_core::{
    cached_single_seed_personalized_pagerank, merge_ppr_scores, personalized_pagerank, EdgeRecord,
    GraphSnapshot, GraphStore,
};

/// Cost charged to a pack when neither the priors nor the pack spec name one.
const DEFAULT_COST_UNITS: u64 = 1;
/// Blend weight on the learned prior when one is present.
const DEFAULT_PRIOR_WEIGHT: f64 = 0.7;
/// Blend weight on the lexical-overlap fallback when a prior is present.
const DEFAULT_LEXICAL_WEIGHT: f64 = 0.3;
/// Weight applied to the trust-tier bonus (a bounded add-on, not a dominator).
const DEFAULT_TRUST_WEIGHT: f64 = 0.2;
/// Minimum token length kept by the tokenizer (drops "a", "to", "is", ...).
const MIN_TOKEN_LEN: usize = 3;

/// A request to the budgeted selector.
///
/// `candidates` is explicit so [`select`] stays pure and replayable; [`select_from_store`] fills it
/// from the registry first. `priors` is the JSON the offline workbench writes. Recognized keys (all
/// optional): `pack_scores: { hash: f64 }`, `pack_costs: { hash: u64 }`, `prior_weight: f64`,
/// `lexical_weight: f64`, `trust_weight: f64`, `min_trust: "first_party"`, `kinds: [str]`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EnsembleSelectRequest {
    pub task: String,
    #[serde(default)]
    pub budget_units: Option<u64>,
    #[serde(default)]
    pub max_selected: Option<usize>,
    #[serde(default)]
    pub candidates: Vec<CapabilityPack>,
    #[serde(default)]
    pub priors: Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UnifiedSelectionRequest {
    pub task: String,
    #[serde(default)]
    pub budget_units: Option<u64>,
    #[serde(default)]
    pub max_selected: Option<usize>,
    #[serde(default)]
    pub pack_kind: Option<PackKind>,
    #[serde(default)]
    pub priors: Value,
    #[serde(default)]
    pub scope: CapabilityScope,
    #[serde(default)]
    pub min_fitness: Option<f32>,
    #[serde(default)]
    pub k: Option<usize>,
    #[serde(default)]
    pub domain_refs: Vec<String>,
    #[serde(default)]
    pub domain_weight: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnifiedSelectionEntry {
    pub entry_type: String,
    pub id: String,
    pub label: String,
    pub score: f64,
    pub reason: String,
    #[serde(default)]
    pub pack_content_hash: Option<String>,
    #[serde(default)]
    pub affordance_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnifiedSelectionResult {
    pub task: String,
    pub live_graph_version: u64,
    pub entries: Vec<UnifiedSelectionEntry>,
    pub ensemble_decision: EnsembleDecision,
}

/// Internal scored candidate. Carries the cloned pack plus the deterministic score breakdown.
struct Scored {
    pack: CapabilityPack,
    score: f64,
    relevance: f64,
    cost: u64,
}

/// Run the budgeted selector over an explicit candidate set. Pure and deterministic: identical
/// inputs yield an identical [`EnsembleDecision`] (and thus identical `content_address`).
pub fn select(request: &EnsembleSelectRequest) -> EnsembleDecision {
    let priors = &request.priors;
    let prior_weight = prior_f64(priors, "prior_weight", DEFAULT_PRIOR_WEIGHT);
    let lexical_weight = prior_f64(priors, "lexical_weight", DEFAULT_LEXICAL_WEIGHT);
    let trust_weight = prior_f64(priors, "trust_weight", DEFAULT_TRUST_WEIGHT);
    let trust_floor = priors
        .get("min_trust")
        .and_then(Value::as_str)
        .map(parse_trust_floor)
        .unwrap_or(0);
    let kind_filter: Option<BTreeSet<String>> =
        priors.get("kinds").and_then(Value::as_array).map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(canonical_kind_label)
                .filter(|s| !s.is_empty())
                .collect()
        });

    let task_tokens: BTreeSet<String> = tokenize(&request.task);

    // Score every candidate deterministically.
    let mut scored: Vec<Scored> = request
        .candidates
        .iter()
        .map(|pack| {
            let lexical = lexical_overlap(&task_tokens, pack);
            let relevance = match pack_prior_score(priors, &pack.pack_content_hash) {
                Some(prior) => prior_weight * prior + lexical_weight * lexical,
                None => lexical,
            };
            let score = relevance + trust_weight * trust_score(&pack.trust);
            let cost = pack_cost(priors, pack);
            Scored {
                pack: pack.clone(),
                score: round6(score),
                relevance: round6(relevance),
                cost,
            }
        })
        .collect();

    // Deterministic order: score DESC, then content hash ASC as a stable tie-break.
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.pack.pack_content_hash.cmp(&b.pack.pack_content_hash))
    });

    let mut selected: Vec<SelectedCapability> = Vec::new();
    let mut rejected: Vec<RejectedCandidate> = Vec::new();
    let mut spent: u64 = 0;
    let mut any_unverified = false;

    for s in scored {
        let kind = canonical_kind_label(&s.pack.kind);
        let hash = s.pack.pack_content_hash.clone();

        // Intrinsic gates first (kind, trust), then capacity gates (cap, budget).
        if let Some(ref kinds) = kind_filter {
            if !kinds.contains(&kind.to_ascii_lowercase()) {
                rejected.push(reject(kind, hash, "kind not requested".to_string()));
                continue;
            }
        }
        if !meets_floor(&s.pack.trust, trust_floor) {
            rejected.push(reject(
                kind,
                hash,
                format!(
                    "below trust floor (rank {} < {})",
                    trust_rank(&s.pack.trust),
                    trust_floor
                ),
            ));
            continue;
        }
        if let Some(cap) = request.max_selected {
            if selected.len() >= cap {
                rejected.push(reject(kind, hash, "selection cap reached".to_string()));
                continue;
            }
        }
        if let Some(budget) = request.budget_units {
            if spent.saturating_add(s.cost) > budget {
                let remaining = budget.saturating_sub(spent);
                rejected.push(reject(
                    kind,
                    hash,
                    format!("over budget (cost {}, {} remaining)", s.cost, remaining),
                ));
                // Keep walking: a cheaper, lower-scored candidate may still fit the remainder.
                continue;
            }
        }

        spent = spent.saturating_add(s.cost);
        if matches!(s.pack.trust, TrustTier::Unverified) {
            any_unverified = true;
        }
        selected.push(SelectedCapability {
            kind,
            pack_content_hash: hash,
            reason: format!(
                "score {:.4} (relevance {:.4}), cost {}",
                s.score, s.relevance, s.cost
            ),
            score: s.score,
            cost_units: s.cost,
        });
    }

    let risk = risk_summary(&selected, spent, request.budget_units, any_unverified);

    EnsembleDecision {
        task: request.task.clone(),
        budget_units: request.budget_units,
        spent_units: spent,
        selected,
        rejected,
        risk,
        priors: request.priors.clone(),
    }
}

/// Store-backed convenience: gather candidates from the registry (optionally filtered to one kind),
/// then run the pure [`select`]. The `candidates` field of `request` is overwritten with the
/// registry result.
pub fn select_from_store<S: EnsembleGraphStore>(
    store: &S,
    tenant: &str,
    kind: Option<PackKind>,
    mut request: EnsembleSelectRequest,
) -> EnsembleResult<EnsembleDecision> {
    request.candidates = list_packs(store, tenant, kind)?;
    if let Some(snapshot) = store.pack_graph_snapshot()? {
        let prior = live_selection_prior_from_snapshot(&snapshot, tenant, &request.task, &[]);
        request.priors = merge_live_pack_scores(
            request.priors,
            &request.candidates,
            &prior.pack_scores,
            prior.graph_version,
        );
    }
    Ok(select(&request))
}

pub fn select_unified_from_store<S>(
    store: &S,
    tenant: &str,
    request: UnifiedSelectionRequest,
) -> EnsembleResult<UnifiedSelectionResult>
where
    S: EnsembleGraphStore + AffordanceGraphStore + GraphStore,
{
    let task = request.task.trim().to_string();
    let prior = live_selection_prior(store, tenant, &task, &request.domain_refs)?;
    let candidates = list_packs(store, tenant, request.pack_kind)?;
    let pack_priors = merge_live_pack_scores(
        request.priors.clone(),
        &candidates,
        &prior.pack_scores,
        prior.graph_version,
    );
    let ensemble_decision = select(&EnsembleSelectRequest {
        task: task.clone(),
        budget_units: request.budget_units,
        max_selected: request.max_selected,
        candidates: candidates.clone(),
        priors: pack_priors,
    });
    let pack_by_hash = candidates
        .into_iter()
        .map(|pack| (pack.pack_content_hash.clone(), pack))
        .collect::<BTreeMap<_, _>>();

    let mut entries = Vec::new();
    for selected in &ensemble_decision.selected {
        let Some(pack) = pack_by_hash.get(&selected.pack_content_hash) else {
            continue;
        };
        entries.push(UnifiedSelectionEntry {
            entry_type: "pack".to_string(),
            id: pack_node_id(&pack.tenant_slug, &pack.pack_content_hash),
            label: pack.title.clone(),
            score: selected.score,
            reason: selected.reason.clone(),
            pack_content_hash: Some(pack.pack_content_hash.clone()),
            affordance_id: None,
        });
    }

    let min_fitness = request.min_fitness.unwrap_or(DEFAULT_MIN_FITNESS);
    for node in affordance_nodes(store)
        .map_err(|error| crate::registry::EnsembleError::InvalidPack(format!("{error:?}")))?
    {
        let affordance = match Affordance::from_node_record(&node) {
            Ok(affordance) => affordance,
            Err(_) => continue,
        };
        if affordance.tenant_id != tenant {
            continue;
        }
        if !request.scope.admits(&affordance) {
            continue;
        }
        let fitness = rustyred_thg_affordances::effective_affordance_fitness_from_node(&node);
        if fitness < min_fitness {
            continue;
        }
        let structural = prior.ppr.get(&node.id).copied().unwrap_or(0.0) + DEFAULT_COLD_START_SCORE;
        let score = round6(structural * fitness as f64);
        entries.push(UnifiedSelectionEntry {
            entry_type: "affordance".to_string(),
            id: node.id.clone(),
            label: if affordance.label.trim().is_empty() {
                affordance.affordance_id.clone()
            } else {
                affordance.label.clone()
            },
            score,
            reason: format!(
                "live structural prior {:.4} * fitness {:.4}",
                structural, fitness
            ),
            pack_content_hash: None,
            affordance_id: Some(affordance.affordance_id),
        });
    }

    entries.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.entry_type.cmp(&right.entry_type))
            .then_with(|| left.id.cmp(&right.id))
    });
    entries.truncate(request.k.unwrap_or(20).max(1));

    Ok(UnifiedSelectionResult {
        task,
        live_graph_version: prior.graph_version,
        entries,
        ensemble_decision,
    })
}

fn reject(kind: String, hash: String, reason: String) -> RejectedCandidate {
    RejectedCandidate {
        kind,
        pack_content_hash: hash,
        reason,
    }
}

/// Lowercased alphanumeric tokens of length >= [`MIN_TOKEN_LEN`], deduplicated into a set so the
/// overlap score is order-independent.
fn tokenize(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| t.len() >= MIN_TOKEN_LEN)
        .collect()
}

/// Union of the pack's title, description, declared capabilities, and kind, tokenized.
fn pack_tokens(pack: &CapabilityPack) -> BTreeSet<String> {
    let mut tokens = tokenize(&pack.title);
    tokens.extend(tokenize(&pack.description));
    tokens.extend(tokenize(&pack.kind));
    if let Some(caps) = pack.spec.get("capabilities").and_then(Value::as_array) {
        for cap in caps.iter().filter_map(Value::as_str) {
            tokens.extend(tokenize(cap));
        }
    }
    tokens
}

/// Fraction of the task's tokens covered by the pack, in `[0.0, 1.0]`. Recall-style coverage: "how
/// much of what the task asks for does this pack speak to". Deterministic from the token sets.
fn lexical_overlap(task_tokens: &BTreeSet<String>, pack: &CapabilityPack) -> f64 {
    if task_tokens.is_empty() {
        return 0.0;
    }
    let pack_tokens = pack_tokens(pack);
    let hits = task_tokens
        .iter()
        .filter(|t| pack_tokens.contains(*t))
        .count();
    hits as f64 / task_tokens.len() as f64
}

fn prior_f64(priors: &Value, key: &str, default: f64) -> f64 {
    priors.get(key).and_then(Value::as_f64).unwrap_or(default)
}

fn pack_prior_score(priors: &Value, hash: &str) -> Option<f64> {
    priors.get("pack_scores")?.get(hash)?.as_f64()
}

/// Cost units for a pack: priors `pack_costs[hash]` first, then the spec's `cost_units`, then the
/// default. Always deterministic.
fn pack_cost(priors: &Value, pack: &CapabilityPack) -> u64 {
    if let Some(cost) = priors
        .get("pack_costs")
        .and_then(|m| m.get(&pack.pack_content_hash))
        .and_then(Value::as_u64)
    {
        return cost;
    }
    if let Some(cost) = pack.spec.get("cost_units").and_then(Value::as_u64) {
        return cost;
    }
    DEFAULT_COST_UNITS
}

/// Round to 6 decimal places so serialized scores are stable across runs/platforms, keeping the
/// decision's `content_address` reproducible despite floating-point arithmetic.
fn round6(x: f64) -> f64 {
    (x * 1_000_000.0).round() / 1_000_000.0
}

/// A coarse, deterministic risk label derived from budget pressure and the trust mix of the
/// *selected* packs.
fn risk_summary(
    selected: &[SelectedCapability],
    spent: u64,
    budget: Option<u64>,
    any_unverified: bool,
) -> String {
    if selected.is_empty() {
        return "none_selected".to_string();
    }
    let budget_pressure = match budget {
        Some(b) if b > 0 => spent as f64 / b as f64,
        _ => 0.0,
    };
    let pressured = budget_pressure >= 0.9;
    match (any_unverified, pressured) {
        (true, true) => "high".to_string(),
        (true, false) | (false, true) => "elevated".to_string(),
        (false, false) => "low".to_string(),
    }
}

fn canonical_kind_label(value: &str) -> String {
    PackKind::parse(value)
        .map(|kind| kind.as_str().to_string())
        .unwrap_or_else(|| value.trim().to_ascii_lowercase())
}

struct LiveSelectionPrior {
    ppr: HashMap<String, f64>,
    pack_scores: BTreeMap<String, f64>,
    graph_version: u64,
}

fn live_selection_prior<S: GraphStore>(
    store: &S,
    tenant: &str,
    task: &str,
    domain_refs: &[String],
) -> EnsembleResult<LiveSelectionPrior> {
    let snapshot = store.graph_snapshot()?;
    Ok(live_selection_prior_from_snapshot(
        &snapshot,
        tenant,
        task,
        domain_refs,
    ))
}

fn live_selection_prior_from_snapshot(
    snapshot: &GraphSnapshot,
    tenant: &str,
    task: &str,
    domain_refs: &[String],
) -> LiveSelectionPrior {
    let adjacency = selection_adjacency(snapshot);
    let mut ppr = HashMap::new();
    let task_node = task_type_node_id(tenant, task);
    if !task.trim().is_empty() {
        ppr = cached_single_seed_personalized_pagerank(
            "ensemble-selection-task",
            snapshot.version,
            &adjacency,
            &task_node,
            1.0,
            0.15,
            1e-5,
            20_000,
        );
    }
    let mut domain_seeds = HashMap::new();
    for domain_ref in domain_refs {
        let domain_ref = domain_ref.trim();
        if domain_ref.is_empty() {
            continue;
        }
        domain_seeds.insert(domain_node_id(tenant, domain_ref), 1.0);
    }
    if !domain_seeds.is_empty() {
        merge_ppr_scores(
            &mut ppr,
            personalized_pagerank(&adjacency, &domain_seeds, 0.15, 1e-5, 20_000),
        );
    }
    let mut pack_scores = BTreeMap::new();
    for node in snapshot.nodes.iter().filter(|node| {
        node.labels
            .iter()
            .any(|label| label == crate::registry::PACK_LABEL)
    }) {
        let Ok(pack) = serde_json::from_value::<CapabilityPack>(node.properties.clone()) else {
            continue;
        };
        if pack.tenant_slug != tenant && pack.tenant_slug != "default" {
            continue;
        }
        let structural = ppr.get(&node.id).copied().unwrap_or(0.0) + DEFAULT_COLD_START_SCORE;
        let fitness = effective_pack_fitness_from_node(node);
        pack_scores.insert(pack.pack_content_hash, round6(structural * fitness as f64));
    }
    LiveSelectionPrior {
        ppr,
        pack_scores,
        graph_version: snapshot.version,
    }
}

fn merge_live_pack_scores(
    priors: Value,
    candidates: &[CapabilityPack],
    live_scores: &BTreeMap<String, f64>,
    graph_version: u64,
) -> Value {
    let mut object = priors.as_object().cloned().unwrap_or_default();
    let prior_weight = prior_f64(
        &Value::Object(object.clone()),
        "prior_weight",
        DEFAULT_PRIOR_WEIGHT,
    );
    let lexical_weight = prior_f64(
        &Value::Object(object.clone()),
        "lexical_weight",
        DEFAULT_LEXICAL_WEIGHT,
    );
    let mut pack_scores = object
        .get("pack_scores")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for candidate in candidates {
        let Some(live_score) = live_scores.get(&candidate.pack_content_hash).copied() else {
            continue;
        };
        let blended = pack_scores
            .get(&candidate.pack_content_hash)
            .and_then(Value::as_f64)
            .map(|offline| prior_weight * offline + lexical_weight * live_score)
            .unwrap_or(live_score);
        pack_scores.insert(candidate.pack_content_hash.clone(), json!(round6(blended)));
    }
    object.insert("pack_scores".to_string(), Value::Object(pack_scores));
    object.insert("live_prior_source".to_string(), json!("graph_ppr"));
    object.insert("graph_version".to_string(), json!(graph_version));
    Value::Object(object)
}

fn selection_adjacency(snapshot: &GraphSnapshot) -> HashMap<String, Vec<(String, f64)>> {
    let mut adjacency: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for edge in &snapshot.edges {
        if edge.tombstone {
            continue;
        }
        match edge.edge_type.as_str() {
            SERVED_TASK | PACK_SERVED_TASK => {
                let weight = edge.effective_confidence().clamp(0.0, 1.0);
                adjacency
                    .entry(edge.to_id.clone())
                    .or_default()
                    .push((edge.from_id.clone(), weight));
            }
            PRODUCED_OUTCOME => {
                let weight = edge_outcome_weight(edge);
                adjacency
                    .entry(edge.from_id.clone())
                    .or_default()
                    .push((edge.to_id.clone(), weight));
                adjacency
                    .entry(edge.to_id.clone())
                    .or_default()
                    .push((edge.from_id.clone(), weight));
            }
            SEQUENCED_WITH | PACK_SEQUENCED_WITH => {
                adjacency
                    .entry(edge.from_id.clone())
                    .or_default()
                    .push((edge.to_id.clone(), 0.5));
                adjacency
                    .entry(edge.to_id.clone())
                    .or_default()
                    .push((edge.from_id.clone(), 0.5));
            }
            PACK_EXPOSES_AFFORDANCE => {
                adjacency
                    .entry(edge.from_id.clone())
                    .or_default()
                    .push((edge.to_id.clone(), 0.8));
                adjacency
                    .entry(edge.to_id.clone())
                    .or_default()
                    .push((edge.from_id.clone(), 0.8));
            }
            PACK_IN_DOMAIN | TASK_IN_DOMAIN => {
                adjacency
                    .entry(edge.from_id.clone())
                    .or_default()
                    .push((edge.to_id.clone(), 0.7));
                adjacency
                    .entry(edge.to_id.clone())
                    .or_default()
                    .push((edge.from_id.clone(), 0.9));
            }
            _ => {}
        }
    }
    adjacency
}

fn edge_outcome_weight(edge: &EdgeRecord) -> f64 {
    edge.properties
        .get("outcome_value")
        .and_then(Value::as_f64)
        .unwrap_or(0.5)
        .max(0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{register_pack, PackExposure};
    use rustyred_thg_affordances::{
        register_connector, ConnectorManifest, ToolManifest, CONNECTOR_FAMILY,
    };
    use rustyred_thg_core::InMemoryGraphStore;
    use serde_json::json;

    fn pack(
        hash: &str,
        kind: &str,
        title: &str,
        desc: &str,
        caps: &[&str],
        trust: TrustTier,
    ) -> CapabilityPack {
        CapabilityPack {
            tenant_slug: "default".to_string(),
            origin_tenant_slug: String::new(),
            pack_content_hash: hash.to_string(),
            kind: kind.to_string(),
            title: title.to_string(),
            description: desc.to_string(),
            spec: json!({ "kind": kind, "capabilities": caps }),
            trust,
            exposure: PackExposure::default(),
            source_content_hash: String::new(),
            artifact_hashes: vec![],
        }
    }

    fn first_party() -> TrustTier {
        TrustTier::FirstParty {
            passport_id: "fp-1".to_string(),
        }
    }

    fn connector_manifest() -> ConnectorManifest {
        ConnectorManifest {
            tenant_id: "default".to_string(),
            server_id: "github".to_string(),
            label: "GitHub".to_string(),
            tools: vec![ToolManifest {
                name: "search_code".to_string(),
                label: "Search code".to_string(),
                description: "search code".to_string(),
                family: CONNECTOR_FAMILY.to_string(),
                input_schema: json!({}),
                permissions: vec![],
                cost: json!({}),
                writeback_policy: "read-only".to_string(),
                tags: vec!["code".to_string()],
                description_embedding: None,
            }],
        }
    }

    #[test]
    fn lexical_fallback_ranks_relevant_pack_first() {
        let rust = pack(
            "h_rust",
            "skill",
            "Rust Engineering",
            "write and review rust cargo crates",
            &["rust", "cargo"],
            TrustTier::Unverified,
        );
        let cooking = pack(
            "h_cook",
            "skill",
            "Cooking",
            "recipes and kitchen techniques",
            &["cooking"],
            TrustTier::Unverified,
        );
        let req = EnsembleSelectRequest {
            task: "review this rust cargo workspace".to_string(),
            budget_units: None,
            max_selected: Some(1),
            candidates: vec![cooking, rust],
            priors: Value::Null,
        };
        let decision = select(&req);
        assert_eq!(decision.selected.len(), 1);
        assert_eq!(decision.selected[0].pack_content_hash, "h_rust");
        assert_eq!(decision.rejected.len(), 1);
        assert_eq!(decision.rejected[0].pack_content_hash, "h_cook");
    }

    #[test]
    fn budget_bounds_selection_and_fills_remainder() {
        // Three packs, all relevant; costs 3/2/2, budget 4. Greedy by score then cheaper fill.
        let priors = json!({
            "pack_scores": { "a": 0.9, "b": 0.8, "c": 0.7 },
            "pack_costs": { "a": 3, "b": 2, "c": 2 }
        });
        let req = EnsembleSelectRequest {
            task: "graph work".to_string(),
            budget_units: Some(4),
            max_selected: None,
            candidates: vec![
                pack(
                    "a",
                    "skill",
                    "A",
                    "graph",
                    &["graph"],
                    TrustTier::Unverified,
                ),
                pack(
                    "b",
                    "skill",
                    "B",
                    "graph",
                    &["graph"],
                    TrustTier::Unverified,
                ),
                pack(
                    "c",
                    "skill",
                    "C",
                    "graph",
                    &["graph"],
                    TrustTier::Unverified,
                ),
            ],
            priors,
        };
        let decision = select(&req);
        // a (cost 3) selected; b (cost 2) over budget (1 remaining); c (cost 2) over budget.
        assert_eq!(decision.spent_units, 3);
        assert_eq!(decision.selected.len(), 1);
        assert_eq!(decision.selected[0].pack_content_hash, "a");
        assert_eq!(decision.rejected.len(), 2);
        assert!(decision
            .rejected
            .iter()
            .all(|r| r.reason.contains("over budget")));
    }

    #[test]
    fn cheaper_lower_score_pack_fills_after_expensive_rejection() {
        let priors = json!({
            "pack_scores": { "big": 0.95, "small": 0.5 },
            "pack_costs": { "big": 10, "small": 2 }
        });
        let req = EnsembleSelectRequest {
            task: "graph".to_string(),
            budget_units: Some(3),
            max_selected: None,
            candidates: vec![
                pack(
                    "big",
                    "skill",
                    "Big",
                    "graph",
                    &["graph"],
                    TrustTier::Unverified,
                ),
                pack(
                    "small",
                    "skill",
                    "Small",
                    "graph",
                    &["graph"],
                    TrustTier::Unverified,
                ),
            ],
            priors,
        };
        let decision = select(&req);
        // big is highest score but costs 10 > 3 -> rejected; small costs 2 -> fits.
        assert_eq!(decision.selected.len(), 1);
        assert_eq!(decision.selected[0].pack_content_hash, "small");
        assert_eq!(decision.spent_units, 2);
    }

    #[test]
    fn trust_floor_excludes_unverified() {
        let priors = json!({ "min_trust": "first_party" });
        let req = EnsembleSelectRequest {
            task: "rust".to_string(),
            budget_units: None,
            max_selected: None,
            candidates: vec![
                pack(
                    "unv",
                    "skill",
                    "Unverified",
                    "rust",
                    &["rust"],
                    TrustTier::Unverified,
                ),
                pack("fp", "skill", "Trusted", "rust", &["rust"], first_party()),
            ],
            priors,
        };
        let decision = select(&req);
        assert_eq!(decision.selected.len(), 1);
        assert_eq!(decision.selected[0].pack_content_hash, "fp");
        assert_eq!(decision.rejected.len(), 1);
        assert!(decision.rejected[0].reason.contains("trust floor"));
    }

    #[test]
    fn kind_filter_rejects_unrequested_kinds() {
        let priors = json!({ "kinds": ["tool"] });
        let req = EnsembleSelectRequest {
            task: "anything".to_string(),
            budget_units: None,
            max_selected: None,
            candidates: vec![
                pack("s", "skill_pack", "S", "x", &[], TrustTier::Unverified),
                pack("t", "tool", "T", "x", &[], TrustTier::Unverified),
            ],
            priors,
        };
        let decision = select(&req);
        assert_eq!(decision.selected.len(), 1);
        assert_eq!(decision.selected[0].kind, "tool");
        assert_eq!(decision.rejected.len(), 1);
        assert_eq!(decision.rejected[0].reason, "kind not requested");
    }

    #[test]
    fn skill_pack_alias_matches_skill_filter() {
        let priors = json!({ "kinds": ["skill"] });
        let req = EnsembleSelectRequest {
            task: "graph".to_string(),
            budget_units: None,
            max_selected: None,
            candidates: vec![pack(
                "s",
                "skill_pack",
                "Skill",
                "graph",
                &["graph"],
                TrustTier::Unverified,
            )],
            priors,
        };
        let decision = select(&req);
        assert_eq!(decision.selected.len(), 1);
        assert_eq!(decision.selected[0].kind, "skill");
    }

    #[test]
    fn decision_is_deterministic_and_content_addressable() {
        let mk = || EnsembleSelectRequest {
            task: "review rust cargo crate".to_string(),
            budget_units: Some(5),
            max_selected: Some(2),
            candidates: vec![
                pack(
                    "h2",
                    "skill",
                    "Two",
                    "cargo crate",
                    &["cargo"],
                    TrustTier::Unverified,
                ),
                pack(
                    "h1",
                    "skill",
                    "One",
                    "rust review",
                    &["rust"],
                    first_party(),
                ),
            ],
            priors: json!({ "trust_weight": 0.3 }),
        };
        let d1 = select(&mk());
        let d2 = select(&mk());
        assert_eq!(d1.content_address(), d2.content_address());
    }

    #[test]
    fn select_from_store_gathers_then_selects() {
        let mut store = InMemoryGraphStore::new();
        let a = pack(
            "",
            "skill",
            "Graph Skill",
            "graph traversal",
            &["graph"],
            TrustTier::Unverified,
        );
        let b = pack(
            "",
            "tool",
            "Graph Tool",
            "graph query",
            &["graph"],
            TrustTier::Unverified,
        );
        register_pack(&mut store, a).unwrap();
        register_pack(&mut store, b).unwrap();

        // No kind filter on the store side; restrict to skills via priors `kinds`.
        let req = EnsembleSelectRequest {
            task: "graph traversal task".to_string(),
            budget_units: None,
            max_selected: None,
            candidates: vec![],
            priors: json!({ "kinds": ["skill"] }),
        };
        let decision = select_from_store(&store, "default", None, req).expect("select");
        assert_eq!(decision.selected.len(), 1);
        assert_eq!(decision.selected[0].kind, "skill");
    }

    #[test]
    fn unified_selection_returns_packs_and_affordances_with_domain_bias() {
        let mut store = InMemoryGraphStore::new();
        register_connector(&mut store, connector_manifest(), Some("test")).unwrap();

        let mut code = pack(
            "code-pack",
            "skill",
            "Code pack",
            "code graph work",
            &["code"],
            TrustTier::Unverified,
        );
        code.spec = json!({
            "kind": "skill",
            "title": "Code pack",
            "description": "code graph work",
            "capabilities": ["code"],
            "affordance_ids": ["github.search_code"],
            "domain_refs": ["code"]
        });
        register_pack(&mut store, code).unwrap();
        register_pack(
            &mut store,
            pack(
                "math-pack",
                "skill",
                "Math pack",
                "symbolic algebra",
                &["math"],
                TrustTier::Unverified,
            ),
        )
        .unwrap();

        let result = select_unified_from_store(
            &store,
            "default",
            UnifiedSelectionRequest {
                task: "neutral task".to_string(),
                max_selected: Some(1),
                k: Some(10),
                domain_refs: vec!["code".to_string()],
                scope: CapabilityScope::unrestricted("agent"),
                ..UnifiedSelectionRequest::default()
            },
        )
        .unwrap();

        assert_eq!(
            result.ensemble_decision.selected[0].pack_content_hash,
            "code-pack"
        );
        assert!(result
            .entries
            .iter()
            .any(|entry| entry.entry_type == "pack"));
        assert!(result
            .entries
            .iter()
            .any(|entry| entry.entry_type == "affordance"
                && entry.affordance_id.as_deref() == Some("github.search_code")));
    }
}
