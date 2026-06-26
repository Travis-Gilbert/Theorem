//! Instant structural epistemic pass for code ingestion.
//!
//! The source-agnostic writer lives in `rustyred-thg-core::epistemic`. This
//! module supplies the code-specific inputs: current-generation CodeSymbol
//! nodes, nearest-neighbor candidate pairs, and cheap doc-claim drift
//! heuristics.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use rustyred_thg_core::{
    structural_epistemic_pass, EpistemicCandidatePair, EpistemicReadout, EpistemicRelationInput,
    EpistemicRelationKind, EpistemicSourceKind, HookContext, HookError, HookHandler, HookOutcome,
    HookRegistration, MutationEvent, MutationKind, MutationMatcher, NodeQuery, NodeRecord,
    RedCoreGraphStore, StructuralEpistemicConfig, StructuralEpistemicInput,
};
use serde_json::{json, Value};

use crate::code_embed_hook::{
    default_symbol_embedder, ensure_embedding_designation, extract_float_vec, set_embedding,
    symbol_embedding_text,
};
use crate::{
    latest_repo_generations_for_tenant, normalize_tenant, property_string, property_u64,
    CALLS_SYMBOL, CODE_SYMBOL_LABEL, DECLARES_SYMBOL, DEPENDS_ON_SYMBOL, EMBEDDING_PROPERTY,
};

pub const CODE_EPISTEMIC_ENGINE: &str = "rustyred-thg-code.instant_structural_epistemic";
pub const DEFAULT_CODE_EPISTEMIC_TOP_K: usize = 8;

#[derive(Clone, Debug, Default)]
pub struct CodeEpistemicReadout {
    pub tenant_id: String,
    pub repo_id: String,
    pub generation: u64,
    pub readout: EpistemicReadout,
    pub drift: Vec<CodeDriftFinding>,
}

impl CodeEpistemicReadout {
    pub fn to_json(&self) -> Value {
        json!({
            "tenant_id": self.tenant_id,
            "repo_id": self.repo_id,
            "generation": self.generation,
            "readout": self.readout,
            "drift": self.drift.iter().map(CodeDriftFinding::to_json).collect::<Vec<_>>(),
            "checked_pair_count": self.readout.checked_pair_count,
            "candidate_pair_bound": self.readout.candidate_pair_bound,
            "contradictions": self.readout.contradictions,
            "unsupported": self.readout.unsupported,
            "orphans": self.readout.orphans,
            "chokepoints": self.readout.chokepoints,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeDriftFinding {
    pub claim_node_id: String,
    pub missing_target: String,
    pub kind: String,
    pub evidence: String,
}

impl CodeDriftFinding {
    pub fn to_json(&self) -> Value {
        json!({
            "claim_node_id": self.claim_node_id,
            "missing_target": self.missing_target,
            "kind": self.kind,
            "evidence": self.evidence,
        })
    }
}

pub fn code_epistemic_hook() -> HookRegistration {
    let handler: HookHandler = Arc::new(epistemic_handler);
    HookRegistration::new(
        "code.instant_epistemic",
        MutationMatcher::any()
            .with_kinds([MutationKind::NodeUpserted, MutationKind::EdgeUpserted])
            .with_labels([
                CODE_SYMBOL_LABEL,
                CALLS_SYMBOL,
                DEPENDS_ON_SYMBOL,
                DECLARES_SYMBOL,
            ]),
        coalesce_code_epistemic,
        handler,
    )
}

fn coalesce_code_epistemic(_event: &MutationEvent) -> Option<String> {
    Some("code-instant-epistemic".to_string())
}

fn epistemic_handler(
    ctx: &mut HookContext,
    events: &[MutationEvent],
) -> Result<HookOutcome, HookError> {
    let repos = repo_ids_from_events(ctx.store, events)?;
    let mut writes = 0usize;
    for repo_id in repos {
        let readout = run_code_epistemic_pass_for_repo(ctx.store, ctx.tenant, &repo_id, None)
            .map_err(|err| HookError::new(err.to_string()))?;
        writes += readout.readout.shadows.len()
            + readout.readout.contradictions.len()
            + readout.drift.len();
    }
    if writes == 0 {
        Ok(HookOutcome::Done)
    } else {
        Ok(HookOutcome::Wrote { mutations: writes })
    }
}

pub fn run_code_epistemic_pass_for_repo(
    store: &mut RedCoreGraphStore,
    tenant_id: &str,
    repo_id: &str,
    generation: Option<u64>,
) -> Result<CodeEpistemicReadout, crate::CodeIndexError> {
    let tenant = normalize_tenant(tenant_id);
    let repo_id = repo_id.trim();
    if repo_id.is_empty() {
        return Ok(CodeEpistemicReadout::default());
    }
    ensure_embedding_designation(store)
        .map_err(|err| crate::CodeIndexError::invalid(err.to_string()))?;
    let latest = latest_repo_generations_for_tenant(store, &tenant)?;
    let generation = generation
        .or_else(|| latest.get(repo_id).copied())
        .unwrap_or(0);
    let mut symbols = current_repo_symbols(store, &tenant, repo_id, generation)?;
    warm_missing_embeddings(store, &mut symbols)?;
    let pairs = bounded_candidate_pairs(store, &symbols, DEFAULT_CODE_EPISTEMIC_TOP_K)?;
    let (drift, drift_relations) = drift_findings(&symbols);
    let node_ids = symbols.iter().map(|node| node.id.clone()).collect();
    let readout = structural_epistemic_pass(
        store,
        StructuralEpistemicInput {
            batch_node_ids: node_ids,
            candidate_pairs: pairs,
            explicit_relations: drift_relations,
            config: StructuralEpistemicConfig {
                engine: CODE_EPISTEMIC_ENGINE.to_string(),
                candidate_top_k: DEFAULT_CODE_EPISTEMIC_TOP_K,
                ..StructuralEpistemicConfig::default()
            },
        },
    )
    .map_err(crate::CodeIndexError::from_store)?;
    Ok(CodeEpistemicReadout {
        tenant_id: tenant,
        repo_id: repo_id.to_string(),
        generation,
        readout,
        drift,
    })
}

fn repo_ids_from_events(
    store: &RedCoreGraphStore,
    events: &[MutationEvent],
) -> Result<BTreeSet<String>, HookError> {
    let mut repos = BTreeSet::new();
    for event in events {
        match event.kind {
            MutationKind::NodeUpserted => {
                let Some(node) = store.get_node(&event.id).map_err(HookError::from)? else {
                    continue;
                };
                if let Some(repo) = property_string(&node.properties, "repo_id") {
                    repos.insert(repo);
                }
            }
            MutationKind::EdgeUpserted | MutationKind::EdgeDeleted => {
                let Some(edge) = store.get_edge(&event.id).map_err(HookError::from)? else {
                    continue;
                };
                for node_id in [&edge.from_id, &edge.to_id] {
                    let Some(node) = store.get_node(node_id).map_err(HookError::from)? else {
                        continue;
                    };
                    if let Some(repo) = property_string(&node.properties, "repo_id") {
                        repos.insert(repo);
                    }
                }
            }
            MutationKind::NodeDeleted => {}
        }
    }
    Ok(repos)
}

fn current_repo_symbols(
    store: &RedCoreGraphStore,
    tenant_id: &str,
    repo_id: &str,
    generation: u64,
) -> Result<Vec<NodeRecord>, crate::CodeIndexError> {
    let nodes = store
        .query_nodes(
            NodeQuery::label(CODE_SYMBOL_LABEL)
                .with_property("tenant_id", json!(tenant_id))
                .with_property("repo_id", json!(repo_id))
                .with_limit(100_000),
        )
        .map_err(crate::CodeIndexError::from_store)?;
    let mut symbols = nodes
        .into_iter()
        .filter(|node| {
            generation == 0 || property_u64(&node.properties, "generation") == Some(generation)
        })
        .collect::<Vec<_>>();
    symbols.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(symbols)
}

fn warm_missing_embeddings(
    store: &mut RedCoreGraphStore,
    symbols: &mut [NodeRecord],
) -> Result<(), crate::CodeIndexError> {
    let embedder = default_symbol_embedder();
    for node in symbols {
        let text = symbol_embedding_text(&node.properties);
        if text.trim().is_empty() {
            continue;
        }
        let vector = embedder
            .embed_code(&text)
            .map_err(|error| crate::CodeIndexError {
                code: "code_embedding_error".to_string(),
                message: error.to_string(),
            })?;
        let up_to_date = extract_float_vec(&node.properties, EMBEDDING_PROPERTY)
            .map(|existing| vectors_close(&existing, &vector))
            .unwrap_or(false);
        if up_to_date {
            continue;
        }
        let mut updated = node.clone();
        set_embedding(&mut updated, &vector);
        store
            .upsert_node(updated.clone())
            .map_err(crate::CodeIndexError::from_store)?;
        *node = updated;
    }
    Ok(())
}

fn bounded_candidate_pairs(
    store: &RedCoreGraphStore,
    symbols: &[NodeRecord],
    top_k: usize,
) -> Result<Vec<EpistemicCandidatePair>, crate::CodeIndexError> {
    let by_id = symbols
        .iter()
        .map(|node| (node.id.clone(), node))
        .collect::<HashMap<_, _>>();
    let mut seen = BTreeSet::new();
    let mut pairs = Vec::new();
    for node in symbols {
        let Some(vector) = extract_float_vec(&node.properties, EMBEDDING_PROPERTY) else {
            continue;
        };
        let repo_id = property_string(&node.properties, "repo_id").unwrap_or_default();
        let generation = property_u64(&node.properties, "generation");
        let mut admitted = 0usize;
        let results = store
            .vector_search(
                Some(CODE_SYMBOL_LABEL),
                EMBEDDING_PROPERTY,
                &vector,
                top_k.saturating_mul(4).saturating_add(1),
            )
            .map_err(crate::CodeIndexError::from_store)?;
        for (candidate_id, _distance) in results {
            if candidate_id == node.id {
                continue;
            }
            let Some(candidate) = by_id.get(&candidate_id) else {
                continue;
            };
            if property_string(&candidate.properties, "repo_id").as_deref()
                != Some(repo_id.as_str())
            {
                continue;
            }
            if generation.is_some()
                && property_u64(&candidate.properties, "generation") != generation
            {
                continue;
            }
            let (left, right) = sorted_pair(&node.id, &candidate_id);
            if seen.insert((left.clone(), right.clone())) {
                pairs.push(EpistemicCandidatePair {
                    left_content_id: left,
                    right_content_id: right,
                });
            }
            admitted += 1;
            if admitted >= top_k {
                break;
            }
        }
    }
    if pairs.is_empty() && symbols.len() > 1 {
        for (idx, node) in symbols.iter().enumerate() {
            for candidate in symbols.iter().skip(idx + 1).take(top_k) {
                let (left, right) = sorted_pair(&node.id, &candidate.id);
                if seen.insert((left.clone(), right.clone())) {
                    pairs.push(EpistemicCandidatePair {
                        left_content_id: left,
                        right_content_id: right,
                    });
                }
            }
        }
    }
    Ok(pairs)
}

fn drift_findings(symbols: &[NodeRecord]) -> (Vec<CodeDriftFinding>, Vec<EpistemicRelationInput>) {
    let symbol_names = symbols
        .iter()
        .filter_map(|node| property_string(&node.properties, "name"))
        .collect::<BTreeSet<_>>();
    let mut drift = Vec::new();
    let mut relations = Vec::new();
    for node in symbols {
        if property_string(&node.properties, "kind").as_deref() != Some("claim") {
            continue;
        }
        let text = claim_source_text(node);
        for target in referenced_entities(&text) {
            if symbol_names.contains(&target) || is_builtin_token(&target) {
                continue;
            }
            let finding = CodeDriftFinding {
                claim_node_id: node.id.clone(),
                missing_target: target.clone(),
                kind: "missing_code_entity".to_string(),
                evidence: format!("documentation claim references missing entity `{target}`"),
            };
            relations.push(EpistemicRelationInput {
                from_content_id: node.id.clone(),
                to_content_id: node.id.clone(),
                kind: EpistemicRelationKind::Undercuts,
                confidence: 0.7,
                evidence: finding.evidence.clone(),
                source_kind: EpistemicSourceKind::Structural,
                score: None,
                model_id: None,
                calibration_version: None,
                feature_version: None,
                connection_features: None,
            });
            drift.push(finding);
        }
    }
    drift.sort_by(|left, right| {
        left.claim_node_id
            .cmp(&right.claim_node_id)
            .then_with(|| left.missing_target.cmp(&right.missing_target))
    });
    drift.dedup();
    relations.sort_by(|left, right| {
        left.from_content_id
            .cmp(&right.from_content_id)
            .then_with(|| left.to_content_id.cmp(&right.to_content_id))
            .then_with(|| left.evidence.cmp(&right.evidence))
    });
    relations.dedup_by(|left, right| {
        left.from_content_id == right.from_content_id
            && left.to_content_id == right.to_content_id
            && left.evidence == right.evidence
    });
    (drift, relations)
}

fn claim_source_text(node: &NodeRecord) -> String {
    ["snippet", "signature", "search_text", "name"]
        .into_iter()
        .filter_map(|key| property_string(&node.properties, key))
        .collect::<Vec<_>>()
        .join(" ")
}

fn referenced_entities(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for marker in [
        "references ",
        "reference ",
        "calls ",
        "uses ",
        "depends on ",
        "requires ",
    ] {
        let mut remaining = text;
        while let Some(index) = remaining.to_ascii_lowercase().find(marker) {
            let after = &remaining[index + marker.len()..];
            if let Some(entity) = first_identifier(after) {
                out.insert(entity);
            }
            remaining = after;
        }
    }
    out
}

fn first_identifier(raw: &str) -> Option<String> {
    let token = raw
        .trim_start_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != ':')
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != ':')
        .next()?
        .trim_matches('`')
        .trim();
    if token.is_empty() {
        None
    } else {
        Some(token.trim_end_matches("()").to_string())
    }
}

fn is_builtin_token(token: &str) -> bool {
    matches!(
        token,
        "Self" | "self" | "str" | "String" | "usize" | "u64" | "u32" | "i64" | "i32" | "bool"
    )
}

fn sorted_pair(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_string(), right.to_string())
    } else {
        (right.to_string(), left.to_string())
    }
}

fn vectors_close(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-6)
}
