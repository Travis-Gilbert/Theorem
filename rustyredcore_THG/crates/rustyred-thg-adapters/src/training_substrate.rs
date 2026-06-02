//! Training-substrate records over RustyRed graph stores.
//!
//! This module keeps training-data registration and model writeback above the
//! core graph engine. RedCore remains the durable database; trainers consume
//! immutable graph snapshots and write evaluated artifacts back as graph
//! records.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use rustyred_thg_core::{
    stable_hash, GraphMutation, GraphMutationBatch, GraphSnapshot, GraphTransaction, NodeRecord,
    ThgError, ThgResult,
};

use crate::types::{
    edge_with_adapter_provenance, normalize_tenant_id, object_node_id, tenant_node_id,
    thg_error_from_store, AdapterGraphStore, LoraAdapter, LORA_ADAPTER_LABEL, THG_ADAPTER_SOURCE,
    TRAINED_ON,
};
use crate::upsert_adapter;

pub const OBJECT_LABEL: &str = "Object";
pub const REASONING_TRACE_LABEL: &str = "ReasoningTrace";
pub const TRACE_STEP_LABEL: &str = "TraceStep";
pub const POSTMORTEM_LABEL: &str = "Postmortem";
pub const ARTIFACT_LABEL: &str = "Artifact";
pub const TRAINING_PACK_LABEL: &str = "TrainingPack";
pub const TRAINING_EXPORT_LABEL: &str = "TrainingExport";
pub const PARAPHRASE_PAIR_LABEL: &str = "ParaphrasePair";
pub const GNN_EXPORT_LABEL: &str = "GnnExport";
pub const MODEL_ARTIFACT_LABEL: &str = "ModelArtifact";
pub const EVALUATION_RECEIPT_LABEL: &str = "EvaluationReceipt";

pub const HAS_STEP: &str = "HAS_STEP";
pub const USED_ARTIFACT: &str = "USED_ARTIFACT";
pub const PART_OF_PACK: &str = "PART_OF_PACK";
pub const HAS_TRAINING_PAIR: &str = "HAS_TRAINING_PAIR";
pub const HAS_GNN_EXPORT: &str = "HAS_GNN_EXPORT";
pub const PRODUCED_ARTIFACT: &str = "PRODUCED_ARTIFACT";
pub const EVALUATED_BY: &str = "EVALUATED_BY";
pub const PROMOTED_TO_ACTIVE: &str = "PROMOTED_TO_ACTIVE";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TrainingFixtureResult {
    pub tenant_id: String,
    pub object_node_ids: Vec<String>,
    pub reasoning_trace_node_ids: Vec<String>,
    pub postmortem_node_id: String,
    pub artifact_node_id: String,
    pub training_pack_node_id: String,
    pub paraphrase_pair_node_id: String,
    pub gnn_export_node_id: String,
    pub adapter_node_id: String,
    pub transaction: GraphTransaction,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TrainingExportCounts {
    pub nodes_total: usize,
    pub edges_total: usize,
    pub objects: usize,
    pub reasoning_traces: usize,
    pub trace_steps: usize,
    pub postmortems: usize,
    pub artifacts: usize,
    pub training_packs: usize,
    pub paraphrase_pairs: usize,
    pub gnn_exports: usize,
    pub model_artifacts: usize,
    pub evaluation_receipts: usize,
    pub lora_adapters: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TrainingExportManifest {
    pub export_id: String,
    pub tenant_id: String,
    pub graph_version: u64,
    pub snapshot_hash: String,
    pub source_graph_status: String,
    pub privacy_tiers: Vec<String>,
    pub selected_labels: Vec<String>,
    pub selected_edge_types: Vec<String>,
    pub feature_schema: Value,
    pub counts: TrainingExportCounts,
    pub reasoning_trace_ids: Vec<String>,
    pub artifact_ids: Vec<String>,
    pub paraphrase_pair_ids: Vec<String>,
    pub gnn_export_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ModelArtifactInput {
    pub model_id: String,
    pub tenant_id: String,
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
pub struct ModelWritebackResult {
    pub model_node_id: String,
    pub evaluation_node_id: String,
    pub transaction: GraphTransaction,
}

pub fn register_training_fixture<S: AdapterGraphStore>(
    store: &mut S,
    tenant_id: &str,
    actor: Option<&str>,
) -> ThgResult<TrainingFixtureResult> {
    let tenant_id = normalize_tenant_id(tenant_id);
    let object_node_ids = vec![object_node_id(1), object_node_id(2), object_node_id(3)];
    let reasoning_trace_node_ids = vec![
        reasoning_trace_node_id(&tenant_id, "trace-alpha"),
        reasoning_trace_node_id(&tenant_id, "trace-beta"),
    ];
    let postmortem_node_id = postmortem_node_id(&tenant_id, "pm-retrieval-drift");
    let artifact_node_id = artifact_node_id(&tenant_id, "s3-gnn-export-v1");
    let training_pack_node_id = training_pack_node_id(&tenant_id, "pack-fixture-v1");
    let paraphrase_pair_node_id = paraphrase_pair_node_id(&tenant_id, "pair-fixture-v1");
    let gnn_export_node_id = gnn_export_node_id(&tenant_id, "gnn-export-fixture-v1");

    let mut mutations = Vec::new();
    mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
        tenant_node_id(&tenant_id),
        ["Tenant"],
        json!({
            "tenant_id": tenant_id,
            "source": THG_ADAPTER_SOURCE,
        }),
    )));

    for (idx, node_id) in object_node_ids.iter().enumerate() {
        mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
            node_id,
            [OBJECT_LABEL],
            json!({
                "object_pk": (idx + 1) as i64,
                "tenant_id": tenant_id,
                "title": format!("Fixture object {}", idx + 1),
                "privacy_tier": "tier_2_structural",
                "embedding": [idx as f32 + 0.1, idx as f32 + 0.2, idx as f32 + 0.3],
                "source": "fixture",
            }),
        )));
    }

    for (idx, trace_id) in reasoning_trace_node_ids.iter().enumerate() {
        let slug = if idx == 0 {
            "trace-alpha"
        } else {
            "trace-beta"
        };
        mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
            trace_id,
            [REASONING_TRACE_LABEL],
            json!({
                "trace_id": slug,
                "tenant_id": tenant_id,
                "task_family": if idx == 0 { "retrieval" } else { "paraphrase" },
                "outcome": if idx == 0 { "success" } else { "needs_review" },
                "training_disposition": "candidate",
                "privacy_tier": "tier_2_structural",
                "source": "fixture",
            }),
        )));
        for step_idx in 0..2 {
            let step_id = trace_step_node_id(&tenant_id, slug, step_idx + 1);
            mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
                &step_id,
                [TRACE_STEP_LABEL],
                json!({
                    "tenant_id": tenant_id,
                    "trace_id": slug,
                    "seq": step_idx + 1,
                    "tool": if step_idx == 0 { "search" } else { "validate" },
                    "summary": format!("{slug} step {}", step_idx + 1),
                    "source": "fixture",
                }),
            )));
            mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
                edge_id(trace_id, HAS_STEP, &step_id),
                trace_id,
                HAS_STEP,
                step_id,
                json!({
                    "tenant_id": tenant_id,
                    "seq": step_idx + 1,
                }),
                actor,
            )));
        }
    }

    mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
        &postmortem_node_id,
        [POSTMORTEM_LABEL],
        json!({
            "tenant_id": tenant_id,
            "postmortem_id": "pm-retrieval-drift",
            "failure_mode": "retrieval_drift",
            "repair_pattern": "snapshot_alignment",
            "source": "fixture",
        }),
    )));
    mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
        &artifact_node_id,
        [ARTIFACT_LABEL],
        json!({
            "tenant_id": tenant_id,
            "artifact_id": "s3-gnn-export-v1",
            "uri": "s3://theseus-training/gnn-export/gnn_geomoe_embeddings.npz",
            "content_hash": "sha256:fixture-gnn-export",
            "export_family": "gnn",
            "privacy_tier": "tier_2_structural",
            "source": "fixture",
        }),
    )));
    mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
        &training_pack_node_id,
        [TRAINING_PACK_LABEL],
        json!({
            "tenant_id": tenant_id,
            "training_pack_id": "pack-fixture-v1",
            "family": "reasoning_trace_seed",
            "source": "fixture",
        }),
    )));
    mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
        &paraphrase_pair_node_id,
        [PARAPHRASE_PAIR_LABEL],
        json!({
            "tenant_id": tenant_id,
            "pair_id": "pair-fixture-v1",
            "source_text": "The answer should cite the graph snapshot.",
            "target_text": "The response should name the graph version it used.",
            "constraint": "preserve provenance",
            "privacy_tier": "tier_2_structural",
            "source": "fixture",
        }),
    )));
    mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
        &gnn_export_node_id,
        [GNN_EXPORT_LABEL],
        json!({
            "tenant_id": tenant_id,
            "export_id": "gnn-export-fixture-v1",
            "node_count": 3,
            "edge_count": 2,
            "embedding_dim": 3,
            "source": "fixture",
        }),
    )));

    for trace_id in &reasoning_trace_node_ids {
        mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            edge_id(trace_id, USED_ARTIFACT, &artifact_node_id),
            trace_id,
            USED_ARTIFACT,
            &artifact_node_id,
            json!({ "tenant_id": tenant_id }),
            actor,
        )));
        mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            edge_id(&training_pack_node_id, PART_OF_PACK, trace_id),
            training_pack_node_id.clone(),
            PART_OF_PACK,
            trace_id.clone(),
            json!({ "tenant_id": tenant_id }),
            actor,
        )));
    }
    mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
        edge_id(&training_pack_node_id, PART_OF_PACK, &artifact_node_id),
        &training_pack_node_id,
        PART_OF_PACK,
        &artifact_node_id,
        json!({ "tenant_id": tenant_id }),
        actor,
    )));
    mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
        edge_id(
            &training_pack_node_id,
            HAS_TRAINING_PAIR,
            &paraphrase_pair_node_id,
        ),
        &training_pack_node_id,
        HAS_TRAINING_PAIR,
        &paraphrase_pair_node_id,
        json!({ "tenant_id": tenant_id }),
        actor,
    )));
    mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
        edge_id(&training_pack_node_id, HAS_GNN_EXPORT, &gnn_export_node_id),
        &training_pack_node_id,
        HAS_GNN_EXPORT,
        &gnn_export_node_id,
        json!({ "tenant_id": tenant_id }),
        actor,
    )));

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;

    let adapter = LoraAdapter {
        adapter_id: "fixture-adapter-v1".to_string(),
        tenant_id: tenant_id.clone(),
        base_model_sha: "sha-base-fixture".to_string(),
        rank: 16,
        target_modules: vec!["q_proj".to_string(), "v_proj".to_string()],
        s3_uri: "s3://theseus-training/adapters/fixture-adapter-v1/adapter.safetensors".to_string(),
        training_object_ids: vec![1, 2, 3],
        version: 1,
        fitness: 0.5,
        created_at_ms: 1,
        manifest_version: 1,
    };
    let adapter_result = upsert_adapter(store, adapter, None, actor)?;

    Ok(TrainingFixtureResult {
        tenant_id,
        object_node_ids,
        reasoning_trace_node_ids,
        postmortem_node_id,
        artifact_node_id,
        training_pack_node_id,
        paraphrase_pair_node_id,
        gnn_export_node_id,
        adapter_node_id: adapter_result.node_id,
        transaction,
    })
}

pub fn export_training_snapshot(
    snapshot: &GraphSnapshot,
    tenant_id: &str,
    export_id: &str,
) -> ThgResult<TrainingExportManifest> {
    let tenant_id = normalize_tenant_id(tenant_id);
    let selected_nodes = snapshot
        .nodes
        .iter()
        .filter(|node| {
            !node.tombstone
                && property_str(&node.properties, "tenant_id") == Some(tenant_id.as_str())
        })
        .collect::<Vec<_>>();
    let selected_node_ids = selected_nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let selected_edges = snapshot
        .edges
        .iter()
        .filter(|edge| {
            !edge.tombstone
                && selected_node_ids.contains(edge.from_id.as_str())
                && selected_node_ids.contains(edge.to_id.as_str())
        })
        .collect::<Vec<_>>();

    let mut selected_labels = std::collections::BTreeSet::new();
    let mut privacy_tiers = std::collections::BTreeSet::new();
    let mut reasoning_trace_ids = Vec::new();
    let mut artifact_ids = Vec::new();
    let mut paraphrase_pair_ids = Vec::new();
    let mut gnn_export_ids = Vec::new();

    for node in &selected_nodes {
        for label in &node.labels {
            selected_labels.insert(label.clone());
        }
        if let Some(tier) = property_str(&node.properties, "privacy_tier") {
            privacy_tiers.insert(tier.to_string());
        }
        if node_has_label(node, REASONING_TRACE_LABEL) {
            reasoning_trace_ids.push(node.id.clone());
        }
        if node_has_label(node, ARTIFACT_LABEL) {
            artifact_ids.push(node.id.clone());
        }
        if node_has_label(node, PARAPHRASE_PAIR_LABEL) {
            paraphrase_pair_ids.push(node.id.clone());
        }
        if node_has_label(node, GNN_EXPORT_LABEL) {
            gnn_export_ids.push(node.id.clone());
        }
    }

    let selected_edge_types = selected_edges
        .iter()
        .map(|edge| edge.edge_type.clone())
        .collect::<std::collections::BTreeSet<_>>();

    if selected_nodes.is_empty() {
        return Err(ThgError::new(
            "empty_training_snapshot",
            format!("no training substrate nodes found for tenant {tenant_id}"),
        ));
    }

    Ok(TrainingExportManifest {
        export_id: export_id.trim().to_string(),
        tenant_id,
        graph_version: snapshot.version,
        snapshot_hash: snapshot_training_hash(snapshot),
        source_graph_status: "frozen_snapshot".to_string(),
        privacy_tiers: privacy_tiers.into_iter().collect(),
        selected_labels: selected_labels.into_iter().collect(),
        selected_edge_types: selected_edge_types.into_iter().collect(),
        feature_schema: json!({
            "object.embedding": {
                "type": "float32",
                "dimension": 3,
                "source": "node.properties.embedding"
            },
            "reasoning_trace.outcome": {
                "type": "categorical",
                "source": "node.properties.outcome"
            },
            "paraphrase_pair": {
                "source": ["source_text", "target_text", "constraint"]
            }
        }),
        counts: TrainingExportCounts {
            nodes_total: selected_nodes.len(),
            edges_total: selected_edges.len(),
            objects: count_label(&selected_nodes, OBJECT_LABEL),
            reasoning_traces: count_label(&selected_nodes, REASONING_TRACE_LABEL),
            trace_steps: count_label(&selected_nodes, TRACE_STEP_LABEL),
            postmortems: count_label(&selected_nodes, POSTMORTEM_LABEL),
            artifacts: count_label(&selected_nodes, ARTIFACT_LABEL),
            training_packs: count_label(&selected_nodes, TRAINING_PACK_LABEL),
            paraphrase_pairs: count_label(&selected_nodes, PARAPHRASE_PAIR_LABEL),
            gnn_exports: count_label(&selected_nodes, GNN_EXPORT_LABEL),
            model_artifacts: count_label(&selected_nodes, MODEL_ARTIFACT_LABEL),
            evaluation_receipts: count_label(&selected_nodes, EVALUATION_RECEIPT_LABEL),
            lora_adapters: count_label(&selected_nodes, LORA_ADAPTER_LABEL),
        },
        reasoning_trace_ids,
        artifact_ids,
        paraphrase_pair_ids,
        gnn_export_ids,
    })
}

pub fn register_model_artifact<S: AdapterGraphStore>(
    store: &mut S,
    input: ModelArtifactInput,
    actor: Option<&str>,
) -> ThgResult<ModelWritebackResult> {
    let tenant_id = normalize_tenant_id(&input.tenant_id);
    let model_node_id = model_artifact_node_id(&tenant_id, &input.model_id);
    let evaluation_node_id = evaluation_receipt_node_id(&tenant_id, &input.model_id);
    let model_id = input.model_id.trim().to_string();
    let model_type = input.model_type.trim().to_string();
    let s3_uri = input.s3_uri.trim().to_string();
    let dataset_hash = input.dataset_hash.trim().to_string();
    let promotion_decision = input.promotion_decision.trim().to_string();
    let promote_to_active = promotion_decision == "active";

    if model_id.is_empty() {
        return Err(ThgError::new(
            "invalid_model_artifact",
            "model_id is required",
        ));
    }
    if model_type.is_empty() {
        return Err(ThgError::new(
            "invalid_model_artifact",
            "model_type is required",
        ));
    }
    if !s3_uri.starts_with("s3://") {
        return Err(ThgError::new(
            "invalid_model_artifact",
            "s3_uri must point at an s3:// model artifact",
        ));
    }
    if dataset_hash.is_empty() {
        return Err(ThgError::new(
            "invalid_model_artifact",
            "dataset_hash is required",
        ));
    }

    let mut mutations = vec![
        GraphMutation::NodeUpsert(NodeRecord::new(
            &model_node_id,
            [MODEL_ARTIFACT_LABEL],
            json!({
                "model_id": model_id,
                "tenant_id": tenant_id,
                "model_type": model_type,
                "s3_uri": s3_uri,
                "dataset_hash": dataset_hash,
                "source_graph_version": input.source_graph_version,
                "promotion_decision": promotion_decision,
                "manifest_version": input.manifest_version.max(1),
                "source": THG_ADAPTER_SOURCE,
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
                "source": THG_ADAPTER_SOURCE,
            }),
        )),
        GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
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
        mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            edge_id(&model_node_id, TRAINED_ON, &target),
            &model_node_id,
            TRAINED_ON,
            target,
            json!({ "tenant_id": tenant_id }),
            actor,
        )));
    }

    if promote_to_active {
        mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
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
    Ok(ModelWritebackResult {
        model_node_id,
        evaluation_node_id,
        transaction,
    })
}

pub fn reasoning_trace_node_id(tenant_id: &str, trace_id: &str) -> String {
    format!(
        "reasoning_trace:{}:{}",
        normalize_tenant_id(tenant_id),
        trace_id.trim()
    )
}

pub fn trace_step_node_id(tenant_id: &str, trace_id: &str, seq: usize) -> String {
    format!(
        "trace_step:{}:{}:{seq}",
        normalize_tenant_id(tenant_id),
        trace_id.trim()
    )
}

pub fn postmortem_node_id(tenant_id: &str, postmortem_id: &str) -> String {
    format!(
        "postmortem:{}:{}",
        normalize_tenant_id(tenant_id),
        postmortem_id.trim()
    )
}

pub fn artifact_node_id(tenant_id: &str, artifact_id: &str) -> String {
    format!(
        "artifact:{}:{}",
        normalize_tenant_id(tenant_id),
        artifact_id.trim()
    )
}

pub fn training_pack_node_id(tenant_id: &str, training_pack_id: &str) -> String {
    format!(
        "training_pack:{}:{}",
        normalize_tenant_id(tenant_id),
        training_pack_id.trim()
    )
}

pub fn paraphrase_pair_node_id(tenant_id: &str, pair_id: &str) -> String {
    format!(
        "paraphrase_pair:{}:{}",
        normalize_tenant_id(tenant_id),
        pair_id.trim()
    )
}

pub fn gnn_export_node_id(tenant_id: &str, export_id: &str) -> String {
    format!(
        "gnn_export:{}:{}",
        normalize_tenant_id(tenant_id),
        export_id.trim()
    )
}

pub fn model_artifact_node_id(tenant_id: &str, model_id: &str) -> String {
    format!(
        "model_artifact:{}:{}",
        normalize_tenant_id(tenant_id),
        model_id.trim()
    )
}

pub fn evaluation_receipt_node_id(tenant_id: &str, model_id: &str) -> String {
    format!(
        "evaluation_receipt:{}:{}",
        normalize_tenant_id(tenant_id),
        model_id.trim()
    )
}

fn ensure_node_exists<S: AdapterGraphStore>(store: &S, node_id: &str) -> ThgResult<()> {
    if store
        .get_node(node_id)
        .map_err(thg_error_from_store)?
        .is_some()
    {
        Ok(())
    } else {
        Err(ThgError::new(
            "missing_graph_endpoint",
            format!("training endpoint node {node_id} does not exist"),
        ))
    }
}

fn snapshot_training_hash(snapshot: &GraphSnapshot) -> String {
    let mut nodes = snapshot
        .nodes
        .iter()
        .filter(|node| !node.tombstone)
        .map(|node| json!({ "id": node.id, "checksum": node.checksum() }))
        .collect::<Vec<_>>();
    let mut edges = snapshot
        .edges
        .iter()
        .filter(|edge| !edge.tombstone)
        .map(|edge| json!({ "id": edge.id, "checksum": edge.checksum() }))
        .collect::<Vec<_>>();
    nodes.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    edges.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    stable_hash(json!({
        "version": snapshot.version,
        "nodes": nodes,
        "edges": edges,
    }))
}

fn count_label(nodes: &[&NodeRecord], label: &str) -> usize {
    nodes
        .iter()
        .filter(|node| node_has_label(node, label))
        .count()
}

fn node_has_label(node: &NodeRecord, label: &str) -> bool {
    node.labels.iter().any(|candidate| candidate == label)
}

fn property_str<'a>(properties: &'a Value, key: &str) -> Option<&'a str> {
    properties.get(key).and_then(Value::as_str)
}

fn edge_id(from_id: &str, edge_type: &str, to_id: &str) -> String {
    format!("edge:{from_id}:{edge_type}:{to_id}")
}
