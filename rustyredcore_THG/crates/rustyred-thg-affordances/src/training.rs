//! Affordance-outcome training export + Pairformer writeback + validation gate.
//!
//! The learned Pairformer head is trained outside the graph engine (Modal /
//! RunPod / Theseus worker), exactly as the durable-training-substrate plan
//! draws the Rust/Python boundary. This module ships the Rust side of that
//! boundary:
//!   1. `export_affordance_training_view` emits the affordance-outcome ranking
//!      pairs (the `ranking_pairs` the durable-training-substrate plan lists),
//!      deterministically from a frozen graph snapshot.
//!   2. `register_pairformer_artifact` writes a trained selector back as a
//!      `ModelArtifact` + `EvaluationReceipt`, using the same graph labels as
//!      the adapter catalog's training substrate so a future unified model
//!      catalog reads both.
//!   3. `pairformer_validation_gate` wraps the harness-core pairformer A/B
//!      comparison (`compare_modes`) so promotion is held to the 90%-confidence
//!      bar on held-out sessions, not assumed from more invocations.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use rustyred_thg_core::{
    stable_hash, GraphMutation, GraphMutationBatch, GraphSnapshot, GraphTransaction, NodeRecord,
    ThgError, ThgResult,
};
use theorem_harness_core::{compare_modes, load_jsonl_metrics};

use crate::types::{
    edge_with_affordance_provenance, normalize_tenant_id, tenant_node_id, thg_error_from_store,
    AffordanceGraphStore, INVOCATION_RECEIPT_LABEL, THG_AFFORDANCE_SOURCE,
};

// Same label/edge strings as the adapter catalog's training substrate, so a
// future unified model-artifact catalog reads affordance and adapter models
// alike. Defined locally to avoid a code dependency on the adapters crate.
pub const MODEL_ARTIFACT_LABEL: &str = "ModelArtifact";
pub const EVALUATION_RECEIPT_LABEL: &str = "EvaluationReceipt";
pub const TRAINED_ON: &str = "TRAINED_ON";
pub const EVALUATED_BY: &str = "EVALUATED_BY";
pub const PROMOTED_TO_ACTIVE: &str = "PROMOTED_TO_ACTIVE";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AffordanceRankingPair {
    pub task_type: String,
    pub candidate_affordance_ids: Vec<String>,
    pub selected_affordance_id: String,
    pub outcome_value: f64,
    pub outcome_weight: f64,
    pub graph_version: u64,
    pub recorded_at_ms: i64,
    pub receipt_hash: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AffordanceTrainingExport {
    pub export_id: String,
    pub tenant_id: String,
    pub graph_version: u64,
    pub snapshot_hash: String,
    pub source_graph_status: String,
    pub ranking_pair_count: usize,
    pub distinct_task_types: usize,
    pub distinct_selected_affordances: usize,
    pub ranking_pairs: Vec<AffordanceRankingPair>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairformerArtifactInput {
    pub model_id: String,
    pub tenant_id: String,
    /// e.g. "pairformer" or "tool_router".
    pub model_type: String,
    pub s3_uri: String,
    pub dataset_hash: String,
    pub source_graph_version: u64,
    pub trained_on_node_ids: Vec<String>,
    pub metrics: Value,
    pub promotion_decision: String,
    pub manifest_version: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairformerWritebackResult {
    pub model_node_id: String,
    pub evaluation_node_id: String,
    pub transaction: GraphTransaction,
}

/// Emit the affordance-outcome ranking pairs from a frozen graph snapshot. Each
/// `InvocationReceipt` node is one ranking observation (the candidates that
/// were considered and the affordance that was selected, with its outcome).
/// Deterministic for the same snapshot; rejects empty.
pub fn export_affordance_training_view(
    snapshot: &GraphSnapshot,
    tenant_id: &str,
    export_id: &str,
) -> ThgResult<AffordanceTrainingExport> {
    let tenant_id = tenant_id.trim().to_string();
    let mut ranking_pairs = snapshot
        .nodes
        .iter()
        .filter(|node| {
            !node.tombstone
                && node
                    .labels
                    .iter()
                    .any(|label| label == INVOCATION_RECEIPT_LABEL)
                && property_str(&node.properties, "tenant_id") == Some(tenant_id.as_str())
        })
        .map(ranking_pair_from_receipt)
        .collect::<Vec<_>>();

    if ranking_pairs.is_empty() {
        return Err(ThgError::new(
            "empty_affordance_training_view",
            format!("no invocation receipts found for tenant {tenant_id}"),
        ));
    }

    // Deterministic ordering for reproducible exports.
    ranking_pairs.sort_by(|a, b| {
        a.recorded_at_ms
            .cmp(&b.recorded_at_ms)
            .then_with(|| a.receipt_hash.cmp(&b.receipt_hash))
    });

    let distinct_task_types = ranking_pairs
        .iter()
        .map(|pair| pair.task_type.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let distinct_selected_affordances = ranking_pairs
        .iter()
        .map(|pair| pair.selected_affordance_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .len();

    let snapshot_hash = stable_hash(json!({
        "version": snapshot.version,
        "ranking_pairs": ranking_pairs,
    }));

    Ok(AffordanceTrainingExport {
        export_id: export_id.trim().to_string(),
        tenant_id,
        graph_version: snapshot.version,
        snapshot_hash,
        source_graph_status: "frozen_snapshot".to_string(),
        ranking_pair_count: ranking_pairs.len(),
        distinct_task_types,
        distinct_selected_affordances,
        ranking_pairs,
    })
}

/// Write a trained Pairformer / tool-router selector back into the graph as a
/// `ModelArtifact` plus an `EvaluationReceipt`. Refuses to promote a model to
/// active without evaluation metrics (no model becomes active without a
/// receipt).
pub fn register_pairformer_artifact<S: AffordanceGraphStore>(
    store: &mut S,
    input: PairformerArtifactInput,
    actor: Option<&str>,
) -> ThgResult<PairformerWritebackResult> {
    let tenant_id = input.tenant_id.trim().to_string();
    let model_id = input.model_id.trim().to_string();
    let model_type = input.model_type.trim().to_string();
    let s3_uri = input.s3_uri.trim().to_string();
    let dataset_hash = input.dataset_hash.trim().to_string();
    let promotion_decision = input.promotion_decision.trim().to_string();
    let promote_to_active = promotion_decision == "active";

    if model_id.is_empty() {
        return Err(ThgError::new(
            "invalid_pairformer_artifact",
            "model_id is required",
        ));
    }
    if model_type.is_empty() {
        return Err(ThgError::new(
            "invalid_pairformer_artifact",
            "model_type is required",
        ));
    }
    if !s3_uri.starts_with("s3://") {
        return Err(ThgError::new(
            "invalid_pairformer_artifact",
            "s3_uri must point at an s3:// model artifact",
        ));
    }
    if dataset_hash.is_empty() {
        return Err(ThgError::new(
            "invalid_pairformer_artifact",
            "dataset_hash is required",
        ));
    }
    let metrics_present = input
        .metrics
        .as_object()
        .map(|object| !object.is_empty())
        .unwrap_or(false);
    if promote_to_active && !metrics_present {
        return Err(ThgError::new(
            "promotion_without_evaluation",
            "a pairformer artifact cannot be promoted to active without evaluation metrics",
        ));
    }

    let model_node_id = pairformer_model_node_id(&tenant_id, &model_id);
    let evaluation_node_id = pairformer_eval_node_id(&tenant_id, &model_id);

    let mut mutations = vec![
        GraphMutation::NodeUpsert(NodeRecord::new(
            &model_node_id,
            [MODEL_ARTIFACT_LABEL],
            json!({
                "model_id": model_id,
                "tenant_id": tenant_id,
                "model_type": model_type,
                "model_family": "affordance_selection",
                "s3_uri": s3_uri,
                "dataset_hash": dataset_hash,
                "source_graph_version": input.source_graph_version,
                "promotion_decision": promotion_decision,
                "manifest_version": input.manifest_version.max(1),
                "source": THG_AFFORDANCE_SOURCE,
            }),
        )),
        GraphMutation::NodeUpsert(NodeRecord::new(
            &evaluation_node_id,
            [EVALUATION_RECEIPT_LABEL],
            json!({
                "model_id": model_id,
                "tenant_id": tenant_id,
                "metrics": input.metrics,
                "source_graph_version": input.source_graph_version,
                "promotion_decision": promotion_decision,
                "source": THG_AFFORDANCE_SOURCE,
            }),
        )),
        GraphMutation::EdgeUpsert(edge_with_affordance_provenance(
            edge_id(&model_node_id, EVALUATED_BY, &evaluation_node_id),
            &model_node_id,
            EVALUATED_BY,
            &evaluation_node_id,
            json!({ "tenant_id": tenant_id }),
            actor,
        )),
    ];

    for target in input
        .trained_on_node_ids
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
    {
        ensure_node_exists(store, &target)?;
        mutations.push(GraphMutation::EdgeUpsert(edge_with_affordance_provenance(
            edge_id(&model_node_id, TRAINED_ON, &target),
            &model_node_id,
            TRAINED_ON,
            target,
            json!({ "tenant_id": tenant_id }),
            actor,
        )));
    }

    if promote_to_active {
        mutations.push(GraphMutation::EdgeUpsert(edge_with_affordance_provenance(
            edge_id(
                &tenant_node_id(&tenant_id),
                PROMOTED_TO_ACTIVE,
                &model_node_id,
            ),
            tenant_node_id(&tenant_id),
            PROMOTED_TO_ACTIVE,
            &model_node_id,
            json!({ "tenant_id": tenant_id }),
            actor,
        )));
    }

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;
    Ok(PairformerWritebackResult {
        model_node_id,
        evaluation_node_id,
        transaction,
    })
}

/// Held-out validation gate: compare the candidate pairformer mode against a
/// baseline across completed sessions (token cost per completed task), using
/// the harness-core Welch comparison. Returns the verdict object plus a
/// `passed` flag (the 90%-confidence bar). The selected-data caution applies:
/// successful runs are over-represented in the corpus, so the gate validates on
/// session-level outcomes, not on the count of invocations.
pub fn pairformer_validation_gate<I, T>(
    metrics_jsonl_lines: I,
    baseline_mode: Option<&str>,
    candidate_mode: Option<&str>,
) -> ThgResult<Value>
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let metrics = load_jsonl_metrics(metrics_jsonl_lines)
        .map_err(|err| ThgError::new("metrics_parse_failed", err.to_string()))?;
    let mut verdict = compare_modes(&metrics, baseline_mode, candidate_mode);
    let passed = verdict
        .get("confidence_90_bar_met")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Some(object) = verdict.as_object_mut() {
        object.insert("passed".to_string(), json!(passed));
    }
    Ok(verdict)
}

pub fn pairformer_model_node_id(tenant_id: &str, model_id: &str) -> String {
    format!(
        "pairformer_model:{}:{}",
        normalize_tenant_id(tenant_id),
        model_id.trim()
    )
}

pub fn pairformer_eval_node_id(tenant_id: &str, model_id: &str) -> String {
    format!(
        "pairformer_eval:{}:{}",
        normalize_tenant_id(tenant_id),
        model_id.trim()
    )
}

fn ranking_pair_from_receipt(node: &NodeRecord) -> AffordanceRankingPair {
    let props = &node.properties;
    AffordanceRankingPair {
        task_type: property_str(props, "task_type")
            .unwrap_or_default()
            .to_string(),
        candidate_affordance_ids: props
            .get("candidate_affordance_ids")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        selected_affordance_id: property_str(props, "selected_affordance_id")
            .unwrap_or_default()
            .to_string(),
        outcome_value: props
            .get("outcome_value")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        outcome_weight: props
            .get("outcome_weight")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        graph_version: props
            .get("graph_version")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        recorded_at_ms: props
            .get("recorded_at_ms")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        receipt_hash: property_str(props, "receipt_hash")
            .unwrap_or_default()
            .to_string(),
    }
}

fn ensure_node_exists<S: AffordanceGraphStore>(store: &S, node_id: &str) -> ThgResult<()> {
    if store
        .get_node(node_id)
        .map_err(thg_error_from_store)?
        .is_some()
    {
        Ok(())
    } else {
        Err(ThgError::new(
            "missing_graph_endpoint",
            format!("trained_on endpoint node {node_id} does not exist"),
        ))
    }
}

fn property_str<'a>(properties: &'a Value, key: &str) -> Option<&'a str> {
    properties.get(key).and_then(Value::as_str)
}

fn edge_id(from_id: &str, edge_type: &str, to_id: &str) -> String {
    format!("edge:{from_id}:{edge_type}:{to_id}")
}
