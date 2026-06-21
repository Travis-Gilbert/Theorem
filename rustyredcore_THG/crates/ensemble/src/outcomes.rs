//! Pack invocation receipts and live pack fitness.
//!
//! This mirrors `rustyred-thg-affordances::record_invocation` at the pack layer.
//! The pure selector stays replayable; this module records what happened after
//! a replayable `EnsembleDecision` was used.

use std::collections::BTreeMap;

use rustyred_thg_affordances::{
    edge_with_affordance_provenance, task_type_node_id, DEFAULT_BASE_FITNESS, TASK_TYPE_LABEL,
};
use rustyred_thg_core::{now_ms, EdgeRecord, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use theorem_harness_core::stable_value_hash;

use crate::decision::EnsembleDecision;
use crate::registry::{
    domain_node_id, get_pack, pack_node_id, EnsembleError, EnsembleGraphStore, EnsembleResult,
    DOMAIN_MAP_LABEL,
};

const DEFAULT_FITNESS_EPSILON: f32 = 0.05;
const DEFAULT_HALF_LIFE_DAYS: f32 = 30.0;

pub const PACK_INVOCATION_RECEIPT_LABEL: &str = "PackInvocationReceipt";
pub const PACK_SERVED_TASK: &str = "PACK_SERVED_TASK";
pub const PACK_SEQUENCED_WITH: &str = "PACK_SEQUENCED_WITH";
pub const TASK_IN_DOMAIN: &str = "TASK_IN_DOMAIN";
pub const ENSEMBLE_SOURCE: &str = "ensemble";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PackInvocationRecordRequest {
    pub tenant_slug: String,
    pub decision: EnsembleDecision,
    pub outcome_value: f32,
    pub outcome_weight: f32,
    #[serde(default)]
    pub outcome_label: String,
    #[serde(default)]
    pub previous_pack_content_hash: Option<String>,
    #[serde(default)]
    pub domain_refs: Vec<String>,
    #[serde(default)]
    pub recorded_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PackInvocationRecordResult {
    pub receipt_node_id: String,
    pub receipt_hash: String,
    pub served_task_edge_ids: Vec<String>,
    pub sequenced_with_edge_id: Option<String>,
    pub task_domain_edge_ids: Vec<String>,
    pub effective_fitness: BTreeMap<String, f32>,
    pub graph_version: u64,
}

pub fn record_pack_invocation<S: EnsembleGraphStore>(
    store: &mut S,
    req: PackInvocationRecordRequest,
    actor: Option<&str>,
) -> EnsembleResult<PackInvocationRecordResult> {
    let tenant = normalize_tenant(&req.tenant_slug);
    let task = req.decision.task.trim().to_string();
    if task.is_empty() {
        return Err(EnsembleError::InvalidPack(
            "decision.task is required to record pack invocation".to_string(),
        ));
    }
    if req.decision.selected.is_empty() {
        return Err(EnsembleError::InvalidPack(
            "decision.selected is required to record pack invocation".to_string(),
        ));
    }

    let recorded_at_ms = req.recorded_at_ms.unwrap_or_else(now_ms);
    let value = req.outcome_value.clamp(0.0, 1.0);
    let weight = req.outcome_weight.max(0.0);
    let fitness_observed = !is_fitness_neutral_outcome(&req.outcome_label);
    let task_node = task_type_node_id(&tenant, &task);
    store.pack_upsert_node(NodeRecord::new(
        task_node.clone(),
        [TASK_TYPE_LABEL],
        json!({
            "tenant_id": tenant,
            "tenant_slug": tenant,
            "task_type": task,
            "source": ENSEMBLE_SOURCE,
        }),
    ))?;

    let graph_version = req
        .decision
        .priors
        .get("graph_version")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let receipt_hash = stable_value_hash(&json!({
        "tenant_slug": tenant,
        "decision": req.decision,
        "outcome_value": value,
        "outcome_weight": weight,
        "outcome_label": req.outcome_label,
        "fitness_observed": fitness_observed,
        "recorded_at_ms": recorded_at_ms,
    }));
    let receipt_node_id = format!("pack_invocation_receipt:{tenant}:{receipt_hash}");
    store.pack_upsert_node(NodeRecord::new(
        receipt_node_id.clone(),
        [PACK_INVOCATION_RECEIPT_LABEL],
        json!({
            "tenant_slug": tenant,
            "receipt_hash": receipt_hash,
            "task_type": task,
            "selected": req.decision.selected,
            "rejected": req.decision.rejected,
            "outcome_value": value,
            "outcome_weight": weight,
            "outcome_label": req.outcome_label,
            "fitness_observed": fitness_observed,
            "recorded_at_ms": recorded_at_ms,
            "graph_version": graph_version,
            "source": ENSEMBLE_SOURCE,
        }),
    ))?;

    let mut served_task_edge_ids = Vec::new();
    let mut effective_fitness = BTreeMap::new();
    for selected in &req.decision.selected {
        let Some(pack) = get_pack(store, &tenant, &selected.pack_content_hash)? else {
            continue;
        };
        let pack_node = pack_node_id(&pack.tenant_slug, &pack.pack_content_hash);
        let Some(mut node) = store.pack_get_node(&pack_node)? else {
            continue;
        };
        let old_fitness = effective_pack_fitness_from_node(&node);
        let alpha = if weight <= 0.0 {
            0.0
        } else {
            (weight / (weight + 4.0)).clamp(0.0, 1.0)
        };
        let updated_fitness = if fitness_observed {
            (old_fitness + alpha * (value - old_fitness)).clamp(DEFAULT_FITNESS_EPSILON, 1.0)
        } else {
            old_fitness
        };
        if fitness_observed {
            node.properties["fitness"] = json!(updated_fitness);
            node.properties["fitness_updated_at_ms"] = json!(recorded_at_ms);
            if node.properties.get("fitness_half_life_days").is_none() {
                node.properties["fitness_half_life_days"] = json!(DEFAULT_HALF_LIFE_DAYS);
            }
            store.pack_upsert_node(node)?;
        }
        effective_fitness.insert(pack.pack_content_hash.clone(), updated_fitness);

        let edge_id = format!("{pack_node}|{PACK_SERVED_TASK}|{task_node}");
        store.pack_upsert_edge(edge_with_affordance_provenance(
            edge_id.clone(),
            &pack_node,
            PACK_SERVED_TASK,
            &task_node,
            json!({
                "tenant_slug": tenant,
                "task_type": task,
                "last_outcome_value": value,
                "recorded_at_ms": recorded_at_ms,
                "source": ENSEMBLE_SOURCE,
            }),
            actor,
        ))?;
        served_task_edge_ids.push(edge_id);
    }

    let sequenced_with_edge_id = if let Some(previous) = req
        .previous_pack_content_hash
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let Some(first_selected) = req.decision.selected.first() else {
            return Ok(PackInvocationRecordResult {
                receipt_node_id,
                receipt_hash,
                served_task_edge_ids,
                sequenced_with_edge_id: None,
                task_domain_edge_ids: Vec::new(),
                effective_fitness,
                graph_version,
            });
        };
        let Some(previous_pack) = get_pack(store, &tenant, previous)? else {
            return Ok(PackInvocationRecordResult {
                receipt_node_id,
                receipt_hash,
                served_task_edge_ids,
                sequenced_with_edge_id: None,
                task_domain_edge_ids: Vec::new(),
                effective_fitness,
                graph_version,
            });
        };
        let Some(current_pack) = get_pack(store, &tenant, &first_selected.pack_content_hash)?
        else {
            return Ok(PackInvocationRecordResult {
                receipt_node_id,
                receipt_hash,
                served_task_edge_ids,
                sequenced_with_edge_id: None,
                task_domain_edge_ids: Vec::new(),
                effective_fitness,
                graph_version,
            });
        };
        let previous_node =
            pack_node_id(&previous_pack.tenant_slug, &previous_pack.pack_content_hash);
        let current_node = pack_node_id(&current_pack.tenant_slug, &current_pack.pack_content_hash);
        let edge_id = format!("{previous_node}|{PACK_SEQUENCED_WITH}|{current_node}");
        store.pack_upsert_edge(edge_with_affordance_provenance(
            edge_id.clone(),
            previous_node,
            PACK_SEQUENCED_WITH,
            current_node,
            json!({
                "tenant_slug": tenant,
                "task_type": task,
                "recorded_at_ms": recorded_at_ms,
                "source": ENSEMBLE_SOURCE,
            }),
            actor,
        ))?;
        Some(edge_id)
    } else {
        None
    };

    let mut task_domain_edge_ids = Vec::new();
    for domain_ref in clean_strings(req.domain_refs) {
        let domain_node = domain_node_id(&tenant, &domain_ref);
        if store.pack_get_node(&domain_node)?.is_none() {
            store.pack_upsert_node(NodeRecord::new(
                domain_node.clone(),
                [DOMAIN_MAP_LABEL],
                json!({
                    "tenant_slug": tenant,
                    "scope_kind": "domain",
                    "scope_ref": domain_ref,
                    "map_kind": "DomainMap",
                    "source": ENSEMBLE_SOURCE,
                }),
            ))?;
        }
        let edge_id = format!("{task_node}|{TASK_IN_DOMAIN}|{domain_node}");
        store.pack_upsert_edge(EdgeRecord::new(
            edge_id.clone(),
            task_node.clone(),
            TASK_IN_DOMAIN,
            domain_node,
            json!({
                "tenant_slug": tenant,
                "domain_ref": domain_ref,
                "task_type": task,
                "recorded_at_ms": recorded_at_ms,
                "source": ENSEMBLE_SOURCE,
            }),
        ))?;
        task_domain_edge_ids.push(edge_id);
    }

    Ok(PackInvocationRecordResult {
        receipt_node_id,
        receipt_hash,
        served_task_edge_ids,
        sequenced_with_edge_id,
        task_domain_edge_ids,
        effective_fitness,
        graph_version,
    })
}

pub fn effective_pack_fitness_from_node(node: &NodeRecord) -> f32 {
    let stored = node
        .properties
        .get("fitness")
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .unwrap_or_else(|| compound_standing_fitness(node))
        .clamp(0.0, 1.0);
    let Some(updated_at_ms) = node
        .properties
        .get("fitness_updated_at_ms")
        .and_then(Value::as_i64)
    else {
        return stored.clamp(DEFAULT_FITNESS_EPSILON, 1.0);
    };
    let half_life_days = node
        .properties
        .get("fitness_half_life_days")
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .unwrap_or(DEFAULT_HALF_LIFE_DAYS)
        .max(1.0);
    let age_ms = now_ms().saturating_sub(updated_at_ms).max(0) as f32;
    let half_life_ms = half_life_days * 86_400_000.0;
    let decay = 0.5_f32.powf(age_ms / half_life_ms);
    (DEFAULT_FITNESS_EPSILON + (stored - DEFAULT_FITNESS_EPSILON) * decay)
        .clamp(DEFAULT_FITNESS_EPSILON, 1.0)
}

/// Live-fitness fallback for skill-pack nodes that carry compound standing instead of a
/// top-level scalar `fitness`. The harness Compound spine writes
/// `metadata.fitness.compound.{run_count, positive_count}` onto the dual-labeled
/// `["CapabilityPack", "SkillPack"]` node when a skill is applied and its run closes. We read
/// it as a Laplace-smoothed success rate with the base fitness as the prior: a never-used
/// pack reads exactly `DEFAULT_BASE_FITNESS`, each positive compound run raises it, and a
/// repeatedly-unhelpful pack sinks below the prior. This is no new ranking -- it only
/// projects the existing compound standing into the fitness contract the selector already
/// reads, so a skill use compounds into ensemble selection like a tool use does.
fn compound_standing_fitness(node: &NodeRecord) -> f32 {
    let Some(compound) = node
        .properties
        .get("metadata")
        .and_then(|metadata| metadata.get("fitness"))
        .and_then(|fitness| fitness.get("compound"))
    else {
        return DEFAULT_BASE_FITNESS;
    };
    let run_count = compound
        .get("run_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if run_count == 0 {
        return DEFAULT_BASE_FITNESS;
    }
    let positive_count = compound
        .get("positive_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    ((positive_count as f32 + DEFAULT_BASE_FITNESS) / (run_count as f32 + 1.0))
        .clamp(DEFAULT_FITNESS_EPSILON, 1.0)
}

fn normalize_tenant(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

fn clean_strings(values: Vec<String>) -> Vec<String> {
    let mut cleaned = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    cleaned.sort();
    cleaned.dedup();
    cleaned
}

fn is_fitness_neutral_outcome(label: &str) -> bool {
    let normalized = label.trim().to_ascii_lowercase().replace('-', "_");
    matches!(
        normalized.as_str(),
        "confirmation_required"
            | "confirmation_denied"
            | "approval_required"
            | "approval_denied"
            | "policy_denied"
            | "policy_blocked"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{register_pack, CapabilityPack, PackExposure, TrustTier};
    use crate::selector::{select_from_store, EnsembleSelectRequest};
    use rustyred_thg_core::InMemoryGraphStore;

    fn pack(hash: &str, title: &str) -> CapabilityPack {
        CapabilityPack {
            tenant_slug: "default".to_string(),
            origin_tenant_slug: String::new(),
            pack_content_hash: hash.to_string(),
            kind: "skill".to_string(),
            title: title.to_string(),
            description: title.to_string(),
            spec: json!({ "kind": "skill", "title": title, "capabilities": [title] }),
            trust: TrustTier::default(),
            exposure: PackExposure::default(),
            source_content_hash: String::new(),
            artifact_hashes: vec![],
        }
    }

    #[test]
    fn pack_outcome_lifts_later_store_selection_without_offline_priors() {
        let mut store = InMemoryGraphStore::new();
        register_pack(&mut store, pack("search", "Search")).unwrap();
        register_pack(&mut store, pack("other", "Other")).unwrap();

        let decision = select_from_store(
            &store,
            "default",
            None,
            EnsembleSelectRequest {
                task: "lookup task".to_string(),
                max_selected: Some(1),
                ..EnsembleSelectRequest::default()
            },
        )
        .unwrap();
        record_pack_invocation(
            &mut store,
            PackInvocationRecordRequest {
                tenant_slug: "default".to_string(),
                decision: EnsembleDecision {
                    task: "lookup task".to_string(),
                    selected: vec![crate::SelectedCapability {
                        kind: "skill".to_string(),
                        pack_content_hash: "search".to_string(),
                        reason: "fixture".to_string(),
                        score: 1.0,
                        cost_units: 1,
                    }],
                    ..decision
                },
                outcome_value: 1.0,
                outcome_weight: 10.0,
                outcome_label: "success".to_string(),
                previous_pack_content_hash: None,
                domain_refs: Vec::new(),
                recorded_at_ms: None,
            },
            Some("test"),
        )
        .unwrap();

        let selected = select_from_store(
            &store,
            "default",
            None,
            EnsembleSelectRequest {
                task: "lookup task".to_string(),
                max_selected: Some(1),
                priors: Value::Null,
                ..EnsembleSelectRequest::default()
            },
        )
        .unwrap();
        assert_eq!(selected.selected[0].pack_content_hash, "search");
    }

    #[test]
    fn pack_fitness_decays_like_affordance_fitness() {
        let fresh = NodeRecord::new(
            "pack:fresh",
            ["CapabilityPack"],
            json!({ "fitness": 1.0, "fitness_updated_at_ms": now_ms() }),
        );
        let stale = NodeRecord::new(
            "pack:stale",
            ["CapabilityPack"],
            json!({
                "fitness": 1.0,
                "fitness_updated_at_ms": 0,
                "fitness_half_life_days": 1.0
            }),
        );

        assert!(
            effective_pack_fitness_from_node(&stale) < effective_pack_fitness_from_node(&fresh)
        );
    }
}
