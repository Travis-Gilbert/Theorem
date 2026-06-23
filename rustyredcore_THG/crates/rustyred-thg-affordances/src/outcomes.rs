//! Invocation receipts: the audit trail and the training signal in one write.
//!
//! Every affordance call records the candidate set considered, the affordance
//! selected, the graph version the decision was made against, and the outcome,
//! as an `InvocationReceipt` node plus `SERVED_TASK` / `PRODUCED_OUTCOME` /
//! `SEQUENCED_WITH` edges, and updates the selected affordance's fitness (an
//! EWMA that decays over time). This is the receipt pattern from hot inference,
//! pointed at tool calls: it produces the audit trail immediately and the
//! Pairformer's training corpus over time.

use serde_json::{json, Map};

use rustyred_thg_core::{
    now_ms, stable_hash, GraphMutation, GraphMutationBatch, NodeQuery, NodeRecord, ThgError,
    ThgResult,
};
use theorem_harness_core::AffordanceReceipt;

use crate::types::{
    affordance_node_id, edge_with_affordance_provenance, invocation_receipt_node_id, property_f32,
    property_i64, task_type_node_id, thg_error_from_store, Affordance, AffordanceGraphStore,
    InvocationRecordRequest, InvocationRecordResult, AFFORDANCE_LABEL, DEFAULT_BASE_FITNESS,
    DEFAULT_FITNESS_EPSILON, DEFAULT_HALF_LIFE_DAYS, INVOCATION_RECEIPT_LABEL, PRODUCED_OUTCOME,
    SEQUENCED_WITH, SERVED_TASK, TASK_TYPE_LABEL, THG_AFFORDANCE_SOURCE,
};

/// Record one affordance invocation: receipt node + edges + fitness update.
pub fn record_invocation<S: AffordanceGraphStore>(
    store: &mut S,
    req: InvocationRecordRequest,
    actor: Option<&str>,
) -> ThgResult<InvocationRecordResult> {
    let tenant_id = req.tenant_id.trim().to_string();
    let task_type = req.task_type.trim().to_string();
    let selected_id = req.selected_affordance_id.trim().to_string();
    if task_type.is_empty() {
        return Err(ThgError::new("invalid_invocation", "task_type is required"));
    }
    if selected_id.is_empty() {
        return Err(ThgError::new(
            "invalid_invocation",
            "selected_affordance_id is required",
        ));
    }

    let selected_node_id = affordance_node_id(&tenant_id, &selected_id);
    let selected_node = store
        .get_node(&selected_node_id)
        .map_err(thg_error_from_store)?
        .ok_or_else(|| {
            ThgError::new(
                "affordance_not_found",
                format!("selected affordance {selected_id} is not registered"),
            )
        })?;
    let selected = Affordance::from_node_record(&selected_node)?;

    // The graph version the selection decision was made against.
    let graph_version = store.snapshot().map_err(thg_error_from_store)?.version;
    let recorded_at_ms = req.recorded_at_ms.unwrap_or_else(now_ms);
    let value = req.outcome_value.clamp(0.0, 1.0);
    let weight = req.outcome_weight.max(0.0);
    let fitness_observed = !is_fitness_neutral_outcome(&req.outcome_label);

    // Fitness EWMA (mirrors the adapter catalog): heavier observations move
    // the estimate more; the estimate decays over time on read.
    let old_fitness = effective_affordance_fitness_from_node(&selected_node);
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

    let task_type_node = task_type_node_id(&tenant_id, &task_type);

    // Content-addressed receipt (reuses the harness-core affordance receipt).
    let input_hash = stable_hash(json!({
        "task_type": task_type,
        "query_text": req.query_text,
    }));
    let candidate_node_refs = req
        .candidate_affordance_ids
        .iter()
        .map(|id| affordance_node_id(&tenant_id, id))
        .collect::<Vec<_>>();
    let mut payload = Map::new();
    payload.insert("task_type".to_string(), json!(task_type));
    payload.insert(
        "candidate_affordance_ids".to_string(),
        json!(req.candidate_affordance_ids),
    );
    payload.insert("selected_affordance_id".to_string(), json!(selected_id));
    payload.insert("outcome_value".to_string(), json!(value));
    payload.insert("outcome_weight".to_string(), json!(weight));
    payload.insert("outcome_label".to_string(), json!(req.outcome_label));
    payload.insert("fitness_observed".to_string(), json!(fitness_observed));
    payload.insert("graph_version".to_string(), json!(graph_version));
    payload.insert("recorded_at_ms".to_string(), json!(recorded_at_ms));
    let receipt = AffordanceReceipt::new(&selected.server_id, &selected_id, input_hash, payload)
        .with_input_node_refs(candidate_node_refs);
    let receipt_hash = receipt.receipt_hash.clone();
    let receipt_node_id = invocation_receipt_node_id(&tenant_id, &receipt_hash);

    // Build mutations.
    let mut mutations = Vec::new();

    mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
        &task_type_node,
        [TASK_TYPE_LABEL],
        json!({
            "tenant_id": tenant_id,
            "task_type": task_type,
            "source": THG_AFFORDANCE_SOURCE,
        }),
    )));

    let receipt_value = serde_json::to_value(&receipt)
        .map_err(|err| ThgError::new("receipt_serialize_failed", err.to_string()))?;
    mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
        &receipt_node_id,
        [INVOCATION_RECEIPT_LABEL],
        json!({
            "tenant_id": tenant_id,
            "receipt_hash": receipt_hash,
            "task_type": task_type,
            "selected_affordance_id": selected_id,
            "candidate_affordance_ids": req.candidate_affordance_ids,
            "outcome_value": value,
            "outcome_weight": weight,
            "outcome_label": req.outcome_label,
            "fitness_observed": fitness_observed,
            "graph_version": graph_version,
            "recorded_at_ms": recorded_at_ms,
            "receipt": receipt_value,
            "source": THG_AFFORDANCE_SOURCE,
        }),
    )));

    // Update the selected affordance's fitness only for tool-quality observations.
    if fitness_observed {
        let mut updated_node = selected_node.clone();
        updated_node.properties["fitness"] = json!(updated_fitness);
        updated_node.properties["fitness_updated_at_ms"] = json!(recorded_at_ms);
        if updated_node
            .properties
            .get("fitness_half_life_days")
            .is_none()
        {
            updated_node.properties["fitness_half_life_days"] = json!(DEFAULT_HALF_LIFE_DAYS);
        }
        mutations.push(GraphMutation::NodeUpsert(updated_node));
    }

    // SERVED_TASK: selected affordance -> task type (stable; strengthens via
    // the affordance node fitness rather than per-edge weight).
    let served_task_edge_id = served_task_edge_id(&selected_node_id, &task_type_node);
    mutations.push(GraphMutation::EdgeUpsert(edge_with_affordance_provenance(
        served_task_edge_id.clone(),
        &selected_node_id,
        SERVED_TASK,
        &task_type_node,
        json!({
            "tenant_id": tenant_id,
            "last_outcome_value": value,
            "recorded_at_ms": recorded_at_ms,
        }),
        actor,
    )));

    // PRODUCED_OUTCOME: selected affordance -> receipt (one per invocation).
    let produced_outcome_edge_id = produced_outcome_edge_id(&selected_node_id, &receipt_node_id);
    mutations.push(GraphMutation::EdgeUpsert(edge_with_affordance_provenance(
        produced_outcome_edge_id.clone(),
        &selected_node_id,
        PRODUCED_OUTCOME,
        &receipt_node_id,
        json!({
            "tenant_id": tenant_id,
            "outcome_value": value,
            "outcome_weight": weight,
            "recorded_at_ms": recorded_at_ms,
        }),
        actor,
    )));

    // SEQUENCED_WITH: previous selection -> this selection (if present and
    // registered).
    let mut sequenced_edge_id = None;
    if let Some(previous_id) = req
        .previous_affordance_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let previous_node_id = affordance_node_id(&tenant_id, previous_id);
        if store
            .get_node(&previous_node_id)
            .map_err(thg_error_from_store)?
            .is_some()
        {
            let edge_id = sequenced_with_edge_id(&previous_node_id, &selected_node_id);
            mutations.push(GraphMutation::EdgeUpsert(edge_with_affordance_provenance(
                edge_id.clone(),
                &previous_node_id,
                SEQUENCED_WITH,
                &selected_node_id,
                json!({
                    "tenant_id": tenant_id,
                    "task_type": task_type,
                    "recorded_at_ms": recorded_at_ms,
                }),
                actor,
            )));
            sequenced_edge_id = Some(edge_id);
        }
    }

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;

    Ok(InvocationRecordResult {
        receipt_node_id,
        receipt_hash,
        served_task_edge_id,
        produced_outcome_edge_id,
        sequenced_with_edge_id: sequenced_edge_id,
        effective_fitness: updated_fitness,
        graph_version,
        transaction,
    })
}

/// Time-decayed effective fitness read off an affordance node. With no recorded
/// update, returns the stored fitness clamped above the epsilon floor.
pub fn effective_affordance_fitness_from_node(node: &NodeRecord) -> f32 {
    let stored = property_f32(&node.properties, "fitness")
        .unwrap_or(DEFAULT_BASE_FITNESS)
        .clamp(0.0, 1.0);
    let Some(updated_at_ms) = property_i64(&node.properties, "fitness_updated_at_ms") else {
        return stored.clamp(DEFAULT_FITNESS_EPSILON, 1.0);
    };
    let half_life_days = property_f32(&node.properties, "fitness_half_life_days")
        .unwrap_or(DEFAULT_HALF_LIFE_DAYS)
        .max(1.0);
    let age_ms = now_ms().saturating_sub(updated_at_ms).max(0) as f32;
    let half_life_ms = half_life_days * 86_400_000.0;
    let decay = 0.5_f32.powf(age_ms / half_life_ms);
    (DEFAULT_FITNESS_EPSILON + (stored - DEFAULT_FITNESS_EPSILON) * decay)
        .clamp(DEFAULT_FITNESS_EPSILON, 1.0)
}

/// All affordance nodes in the store (used by selection and export).
pub fn affordance_nodes<S: AffordanceGraphStore>(store: &S) -> ThgResult<Vec<NodeRecord>> {
    store
        .query_nodes(NodeQuery::label(AFFORDANCE_LABEL).with_limit(10_000))
        .map_err(thg_error_from_store)
}

fn served_task_edge_id(affordance_node_id: &str, task_type_node_id: &str) -> String {
    format!("edge:{affordance_node_id}:served_task:{task_type_node_id}")
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

fn produced_outcome_edge_id(affordance_node_id: &str, receipt_node_id: &str) -> String {
    format!("edge:{affordance_node_id}:produced_outcome:{receipt_node_id}")
}

fn sequenced_with_edge_id(previous_node_id: &str, selected_node_id: &str) -> String {
    format!("edge:{previous_node_id}:sequenced_with:{selected_node_id}")
}
