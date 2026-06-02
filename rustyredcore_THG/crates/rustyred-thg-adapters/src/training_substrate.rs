//! Training-substrate records over RustyRed graph stores.
//!
//! This module keeps training-data registration and model writeback above the
//! core graph engine. RedCore remains the durable database; trainers consume
//! immutable graph snapshots and write evaluated artifacts back as graph
//! records.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;

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
pub const GNN_ENTITY_LABEL: &str = "GnnEntity";
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
pub const HAS_ENTITY: &str = "HAS_ENTITY";
pub const PRODUCED_ARTIFACT: &str = "PRODUCED_ARTIFACT";
pub const EVALUATED_BY: &str = "EVALUATED_BY";
pub const PROMOTED_TO_ACTIVE: &str = "PROMOTED_TO_ACTIVE";

const DEFAULT_GNN_IMPORT_BATCH_SIZE: usize = 10_000;

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
pub struct GnnExportImportOptions {
    pub batch_size: usize,
    pub max_entities: Option<usize>,
    pub max_triples: Option<usize>,
    pub max_temporal_triples: Option<usize>,
}

impl Default for GnnExportImportOptions {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_GNN_IMPORT_BATCH_SIZE,
            max_entities: None,
            max_triples: None,
            max_temporal_triples: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GnnExportImportResult {
    pub tenant_id: String,
    pub export_id: String,
    pub training_pack_node_id: String,
    pub gnn_export_node_id: String,
    pub imported_entity_nodes: usize,
    pub imported_sha_map_nodes: usize,
    pub imported_triple_edges: usize,
    pub imported_temporal_edges: usize,
    pub skipped_triples: usize,
    pub skipped_temporal_triples: usize,
    pub artifact_nodes: usize,
    pub transaction_count: usize,
    pub graph_version: u64,
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

pub fn register_gnn_export_dir<S: AdapterGraphStore>(
    store: &mut S,
    export_dir: impl AsRef<Path>,
    tenant_id: &str,
    export_id: &str,
    options: GnnExportImportOptions,
    actor: Option<&str>,
) -> ThgResult<GnnExportImportResult> {
    let export_dir = export_dir.as_ref();
    let tenant_id = normalize_tenant_id(tenant_id);
    let export_id = export_id.trim().to_string();
    if export_id.is_empty() {
        return Err(ThgError::new("invalid_gnn_export", "export_id is required"));
    }

    let batch_size = options.batch_size.max(1);
    let manifest =
        read_optional_json(&export_dir.join("manifest.json"))?.unwrap_or_else(|| json!({}));
    let training_metadata = read_optional_json(&export_dir.join("training_metadata.json"))?
        .unwrap_or_else(|| json!({}));
    let export_metadata =
        read_optional_json(&export_dir.join("export_metadata.json"))?.unwrap_or_else(|| json!({}));
    let mut file_manifest = manifest
        .get("files")
        .and_then(Value::as_object)
        .map(|files| {
            files
                .iter()
                .map(|(name, metadata)| (name.clone(), metadata.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    if file_manifest.is_empty() {
        file_manifest = discover_export_files(export_dir)?;
    }

    let training_pack_node_id = training_pack_node_id(&tenant_id, &export_id);
    let gnn_export_node_id = gnn_export_node_id(&tenant_id, &export_id);
    let mut transactions = 0usize;
    let mut graph_version = 0u64;

    let mut metadata_mutations = vec![
        GraphMutation::NodeUpsert(NodeRecord::new(
            tenant_node_id(&tenant_id),
            ["Tenant"],
            json!({
                "tenant_id": tenant_id,
                "source": THG_ADAPTER_SOURCE,
            }),
        )),
        GraphMutation::NodeUpsert(NodeRecord::new(
            &training_pack_node_id,
            [TRAINING_PACK_LABEL],
            json!({
                "tenant_id": tenant_id,
                "training_pack_id": export_id,
                "family": "theseus_gnn_export",
                "source": "theseus_gnn_export",
                "privacy_tier": "tier_2_structural",
            }),
        )),
        GraphMutation::NodeUpsert(NodeRecord::new(
            &gnn_export_node_id,
            [GNN_EXPORT_LABEL, TRAINING_EXPORT_LABEL],
            json!({
                "tenant_id": tenant_id,
                "export_id": export_id,
                "source": "theseus_gnn_export",
                "privacy_tier": "tier_2_structural",
                "schema_version": manifest.get("schema_version").cloned().unwrap_or(Value::Null),
                "generator": manifest.get("generator").cloned().unwrap_or(Value::Null),
                "exported_at": manifest.get("exported_at").cloned().unwrap_or(Value::Null),
                "graph_snapshot": manifest.get("graph_snapshot").cloned().unwrap_or(Value::Null),
                "training_metadata": training_metadata,
                "export_metadata": export_metadata,
            }),
        )),
        GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            edge_id(&training_pack_node_id, HAS_GNN_EXPORT, &gnn_export_node_id),
            &training_pack_node_id,
            HAS_GNN_EXPORT,
            &gnn_export_node_id,
            json!({ "tenant_id": tenant_id, "export_id": export_id }),
            actor,
        )),
    ];

    let mut artifact_nodes = 0usize;
    for (file_name, metadata) in &file_manifest {
        let artifact_node_id = gnn_export_artifact_node_id(&tenant_id, &export_id, file_name);
        metadata_mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
            &artifact_node_id,
            [ARTIFACT_LABEL],
            json!({
                "tenant_id": tenant_id,
                "artifact_id": format!("gnn-export:{export_id}:{}", slug_segment(file_name)),
                "export_id": export_id,
                "file_name": file_name,
                "local_path": export_dir.join(file_name).to_string_lossy(),
                "metadata": metadata,
                "artifact_family": "theseus_gnn_export_file",
                "privacy_tier": "tier_2_structural",
                "source": "theseus_gnn_export",
            }),
        )));
        metadata_mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            edge_id(&training_pack_node_id, PART_OF_PACK, &artifact_node_id),
            &training_pack_node_id,
            PART_OF_PACK,
            &artifact_node_id,
            json!({ "tenant_id": tenant_id, "export_id": export_id }),
            actor,
        )));
        metadata_mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            edge_id(&gnn_export_node_id, USED_ARTIFACT, &artifact_node_id),
            &gnn_export_node_id,
            USED_ARTIFACT,
            &artifact_node_id,
            json!({ "tenant_id": tenant_id, "export_id": export_id }),
            actor,
        )));
        artifact_nodes += 1;
    }
    flush_import_batch(
        store,
        &mut metadata_mutations,
        &mut transactions,
        &mut graph_version,
    )?;

    let entity_map_path = export_dir.join("entity_map.tsv");
    let triple_path = export_dir.join("triples.tsv");
    let temporal_triple_path = export_dir.join("temporal_triples.tsv");
    let sha_to_object_id_path = export_dir.join("sha_to_object_id.json");
    let sha_to_object_ids = read_sha_to_object_ids(&sha_to_object_id_path)?;
    let mut sha_to_node_id = HashMap::new();
    for (sha_hash, object_id) in &sha_to_object_ids {
        sha_to_node_id.insert(sha_hash.clone(), object_node_id(*object_id));
    }
    let mut imported_entity_nodes = 0usize;
    let mut detailed_entity_hashes = HashSet::new();
    let mut entity_mutations = Vec::with_capacity(batch_size);

    for entity in read_gnn_entities(&entity_map_path, options.max_entities)? {
        let object_id = entity
            .object_id
            .or_else(|| sha_to_object_ids.get(&entity.sha_hash).copied());
        let node_id = object_id
            .map(object_node_id)
            .unwrap_or_else(|| gnn_export_entity_node_id(&tenant_id, &export_id, &entity.sha_hash));
        sha_to_node_id.insert(entity.sha_hash.clone(), node_id.clone());
        detailed_entity_hashes.insert(entity.sha_hash.clone());
        let labels = if object_id.is_some() {
            vec![OBJECT_LABEL.to_string(), GNN_ENTITY_LABEL.to_string()]
        } else {
            vec![GNN_ENTITY_LABEL.to_string()]
        };
        let node = merge_existing_node(
            store,
            NodeRecord::new(
                &node_id,
                labels,
                json!({
                    "tenant_id": tenant_id,
                    "object_id": object_id,
                    "sha_hash": entity.sha_hash,
                    "title": entity.title,
                    "object_type": entity.object_type,
                    "export_id": export_id,
                    "privacy_tier": "tier_2_structural",
                    "source": "theseus_gnn_export",
                }),
            ),
        )?;
        entity_mutations.push(GraphMutation::NodeUpsert(node));
        imported_entity_nodes += 1;
        if entity_mutations.len() >= batch_size {
            flush_import_batch(
                store,
                &mut entity_mutations,
                &mut transactions,
                &mut graph_version,
            )?;
        }
    }
    flush_import_batch(
        store,
        &mut entity_mutations,
        &mut transactions,
        &mut graph_version,
    )?;

    let mut imported_sha_map_nodes = 0usize;
    if temporal_triple_path.exists() {
        let mut sha_map_mutations = Vec::with_capacity(batch_size);
        for (sha_hash, object_id) in &sha_to_object_ids {
            if detailed_entity_hashes.contains(sha_hash) {
                continue;
            }
            let node_id = object_node_id(*object_id);
            if store
                .get_node(&node_id)
                .map_err(thg_error_from_store)?
                .is_some()
            {
                continue;
            }
            sha_map_mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
                &node_id,
                [OBJECT_LABEL, GNN_ENTITY_LABEL],
                json!({
                    "tenant_id": tenant_id,
                    "object_id": object_id,
                    "sha_hash": sha_hash,
                    "title": Value::Null,
                    "object_type": "unknown",
                    "export_id": export_id,
                    "privacy_tier": "tier_2_structural",
                    "source": "theseus_gnn_export",
                    "materialized_from_sha_to_object_id": true,
                }),
            )));
            imported_sha_map_nodes += 1;
            if sha_map_mutations.len() >= batch_size {
                flush_import_batch(
                    store,
                    &mut sha_map_mutations,
                    &mut transactions,
                    &mut graph_version,
                )?;
            }
        }
        flush_import_batch(
            store,
            &mut sha_map_mutations,
            &mut transactions,
            &mut graph_version,
        )?;
    }

    let mut imported_triple_edges = 0usize;
    let mut skipped_triples = 0usize;
    let mut edge_mutations = Vec::with_capacity(batch_size);
    for triple in read_gnn_triples(&triple_path, options.max_triples)? {
        let Some(from_id) = sha_to_node_id.get(&triple.head).cloned() else {
            skipped_triples += 1;
            continue;
        };
        let Some(to_id) = sha_to_node_id.get(&triple.tail).cloned() else {
            skipped_triples += 1;
            continue;
        };
        let edge_type = gnn_relation_edge_type(&triple.relation);
        let edge_id = format!(
            "edge:gnn_export:{}:{}:triple:{}",
            tenant_id,
            slug_segment(&export_id),
            triple.index
        );
        edge_mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            edge_id,
            from_id,
            edge_type,
            to_id,
            json!({
                "tenant_id": tenant_id,
                "export_id": export_id,
                "relation": triple.relation,
                "triple_index": triple.index,
                "source": "theseus_gnn_export",
            }),
            actor,
        )));
        imported_triple_edges += 1;
        if edge_mutations.len() >= batch_size {
            flush_import_batch(
                store,
                &mut edge_mutations,
                &mut transactions,
                &mut graph_version,
            )?;
        }
    }
    flush_import_batch(
        store,
        &mut edge_mutations,
        &mut transactions,
        &mut graph_version,
    )?;

    let mut imported_temporal_edges = 0usize;
    let mut skipped_temporal_triples = 0usize;
    if temporal_triple_path.exists() {
        let mut temporal_edge_mutations = Vec::with_capacity(batch_size);
        for triple in
            read_gnn_temporal_triples(&temporal_triple_path, options.max_temporal_triples)?
        {
            let Some(from_id) = sha_to_node_id.get(&triple.head).cloned() else {
                skipped_temporal_triples += 1;
                continue;
            };
            let Some(to_id) = sha_to_node_id.get(&triple.tail).cloned() else {
                skipped_temporal_triples += 1;
                continue;
            };
            let edge_type = gnn_temporal_relation_edge_type(&triple.relation);
            let edge_id = format!(
                "edge:gnn_export:{}:{}:temporal:{}",
                tenant_id,
                slug_segment(&export_id),
                triple.index
            );
            temporal_edge_mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
                edge_id,
                from_id,
                edge_type,
                to_id,
                json!({
                    "tenant_id": tenant_id,
                    "export_id": export_id,
                    "relation": triple.relation,
                    "time_bucket": triple.time_bucket,
                    "weight": triple.weight,
                    "temporal_triple_index": triple.index,
                    "source": "theseus_gnn_export",
                    "source_file": "temporal_triples.tsv",
                }),
                actor,
            )));
            imported_temporal_edges += 1;
            if temporal_edge_mutations.len() >= batch_size {
                flush_import_batch(
                    store,
                    &mut temporal_edge_mutations,
                    &mut transactions,
                    &mut graph_version,
                )?;
            }
        }
        flush_import_batch(
            store,
            &mut temporal_edge_mutations,
            &mut transactions,
            &mut graph_version,
        )?;
    }

    Ok(GnnExportImportResult {
        tenant_id,
        export_id,
        training_pack_node_id,
        gnn_export_node_id,
        imported_entity_nodes,
        imported_sha_map_nodes,
        imported_triple_edges,
        imported_temporal_edges,
        skipped_triples,
        skipped_temporal_triples,
        artifact_nodes,
        transaction_count: transactions,
        graph_version,
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct GnnEntityRow {
    sha_hash: String,
    title: String,
    object_type: String,
    object_id: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GnnTripleRow {
    index: usize,
    head: String,
    relation: String,
    tail: String,
}

#[derive(Clone, Debug, PartialEq)]
struct GnnTemporalTripleRow {
    index: usize,
    head: String,
    relation: String,
    tail: String,
    time_bucket: String,
    weight: f64,
}

fn read_gnn_entities(path: &Path, max_entities: Option<usize>) -> ThgResult<Vec<GnnEntityRow>> {
    let mut reader = BufReader::new(File::open(path).map_err(io_thg("gnn_entity_map_open"))?);
    let header = read_required_line(&mut reader, path, "gnn_entity_map_header")?;
    let columns = header.split('\t').collect::<Vec<_>>();
    let sha_idx = required_column(&columns, "sha_hash", path)?;
    let title_idx = required_column(&columns, "title", path)?;
    let type_idx = required_column(&columns, "object_type", path)?;
    let object_id_idx = optional_column(&columns, "object_id");

    let mut rows = Vec::new();
    for (line_idx, line) in reader.lines().enumerate() {
        if let Some(max_entities) = max_entities {
            if rows.len() >= max_entities {
                break;
            }
        }
        let line = line.map_err(io_thg("gnn_entity_map_read"))?;
        if line.trim().is_empty() {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        let object_id = object_id_idx
            .map(|index| {
                field(&fields, index, path, line_idx + 2)?
                    .parse::<i64>()
                    .map_err(|err| {
                        ThgError::new(
                            "gnn_entity_map_parse_failed",
                            format!(
                                "{} line {} has invalid object_id: {err}",
                                path.display(),
                                line_idx + 2
                            ),
                        )
                    })
            })
            .transpose()?;
        rows.push(GnnEntityRow {
            sha_hash: field(&fields, sha_idx, path, line_idx + 2)?.to_string(),
            title: field(&fields, title_idx, path, line_idx + 2)?.to_string(),
            object_type: field(&fields, type_idx, path, line_idx + 2)?.to_string(),
            object_id,
        });
    }
    Ok(rows)
}

fn read_gnn_triples(path: &Path, max_triples: Option<usize>) -> ThgResult<Vec<GnnTripleRow>> {
    let mut reader = BufReader::new(File::open(path).map_err(io_thg("gnn_triples_open"))?);
    let header = read_required_line(&mut reader, path, "gnn_triples_header")?;
    let columns = header.split('\t').collect::<Vec<_>>();
    let head_idx = required_column(&columns, "head", path)?;
    let relation_idx = required_column(&columns, "relation", path)?;
    let tail_idx = required_column(&columns, "tail", path)?;

    let mut rows = Vec::new();
    for (line_idx, line) in reader.lines().enumerate() {
        if let Some(max_triples) = max_triples {
            if rows.len() >= max_triples {
                break;
            }
        }
        let line = line.map_err(io_thg("gnn_triples_read"))?;
        if line.trim().is_empty() {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        rows.push(GnnTripleRow {
            index: line_idx + 1,
            head: field(&fields, head_idx, path, line_idx + 2)?.to_string(),
            relation: field(&fields, relation_idx, path, line_idx + 2)?.to_string(),
            tail: field(&fields, tail_idx, path, line_idx + 2)?.to_string(),
        });
    }
    Ok(rows)
}

fn read_gnn_temporal_triples(
    path: &Path,
    max_triples: Option<usize>,
) -> ThgResult<Vec<GnnTemporalTripleRow>> {
    let mut reader = BufReader::new(File::open(path).map_err(io_thg("gnn_temporal_triples_open"))?);
    let header = read_required_line(&mut reader, path, "gnn_temporal_triples_header")?;
    let columns = header.split('\t').collect::<Vec<_>>();
    let head_idx = required_column(&columns, "head", path)?;
    let relation_idx = required_column(&columns, "relation", path)?;
    let tail_idx = required_column(&columns, "tail", path)?;
    let time_bucket_idx = required_column(&columns, "time_bucket", path)?;
    let weight_idx = required_column(&columns, "weight", path)?;

    let mut rows = Vec::new();
    for (line_idx, line) in reader.lines().enumerate() {
        if let Some(max_triples) = max_triples {
            if rows.len() >= max_triples {
                break;
            }
        }
        let line = line.map_err(io_thg("gnn_temporal_triples_read"))?;
        if line.trim().is_empty() {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        let weight = field(&fields, weight_idx, path, line_idx + 2)?
            .parse::<f64>()
            .map_err(|err| {
                ThgError::new(
                    "gnn_temporal_triples_parse_failed",
                    format!(
                        "{} line {} has invalid weight: {err}",
                        path.display(),
                        line_idx + 2
                    ),
                )
            })?;
        rows.push(GnnTemporalTripleRow {
            index: line_idx + 1,
            head: field(&fields, head_idx, path, line_idx + 2)?.to_string(),
            relation: field(&fields, relation_idx, path, line_idx + 2)?.to_string(),
            tail: field(&fields, tail_idx, path, line_idx + 2)?.to_string(),
            time_bucket: field(&fields, time_bucket_idx, path, line_idx + 2)?.to_string(),
            weight,
        });
    }
    Ok(rows)
}

fn read_required_line(
    reader: &mut BufReader<File>,
    path: &Path,
    code: &'static str,
) -> ThgResult<String> {
    let mut line = String::new();
    let bytes = reader.read_line(&mut line).map_err(io_thg(code))?;
    if bytes == 0 {
        return Err(ThgError::new(code, format!("{} is empty", path.display())));
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

fn optional_column(columns: &[&str], column: &str) -> Option<usize> {
    columns.iter().position(|candidate| *candidate == column)
}

fn required_column(columns: &[&str], column: &str, path: &Path) -> ThgResult<usize> {
    columns
        .iter()
        .position(|candidate| *candidate == column)
        .ok_or_else(|| {
            ThgError::new(
                "gnn_export_column_missing",
                format!("{} is missing required column {column}", path.display()),
            )
        })
}

fn field<'a>(fields: &'a [&str], index: usize, path: &Path, line: usize) -> ThgResult<&'a str> {
    fields.get(index).copied().ok_or_else(|| {
        ThgError::new(
            "gnn_export_field_missing",
            format!(
                "{} line {line} is missing field index {index}",
                path.display()
            ),
        )
    })
}

fn read_optional_json(path: &Path) -> ThgResult<Option<Value>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).map_err(io_thg("gnn_export_json_read"))?;
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|err| ThgError::new("gnn_export_json_parse_failed", err.to_string()))
}

fn read_sha_to_object_ids(path: &Path) -> ThgResult<HashMap<String, i64>> {
    let Some(value) = read_optional_json(path)? else {
        return Ok(HashMap::new());
    };
    let Some(entries) = value.as_object() else {
        return Err(ThgError::new(
            "gnn_export_sha_map_invalid",
            format!("{} must be a JSON object", path.display()),
        ));
    };
    let mut map = HashMap::with_capacity(entries.len());
    for (sha_hash, object_id) in entries {
        let parsed = object_id
            .as_i64()
            .or_else(|| object_id.as_str().and_then(|value| value.parse().ok()))
            .ok_or_else(|| {
                ThgError::new(
                    "gnn_export_sha_map_invalid",
                    format!(
                        "{} maps {sha_hash} to a non-integer object id",
                        path.display()
                    ),
                )
            })?;
        map.insert(sha_hash.clone(), parsed);
    }
    Ok(map)
}

fn discover_export_files(export_dir: &Path) -> ThgResult<BTreeMap<String, Value>> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(export_dir).map_err(io_thg("gnn_export_dir_read"))? {
        let entry = entry.map_err(io_thg("gnn_export_dir_read"))?;
        if !entry
            .file_type()
            .map_err(io_thg("gnn_export_dir_read"))?
            .is_file()
        {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry.metadata().map_err(io_thg("gnn_export_dir_read"))?;
        files.insert(
            file_name,
            json!({
                "size_bytes": metadata.len(),
                "discovered_from_local_export": true,
            }),
        );
    }
    Ok(files)
}

fn merge_existing_node<S: AdapterGraphStore>(
    store: &S,
    mut node: NodeRecord,
) -> ThgResult<NodeRecord> {
    let Some(existing) = store.get_node(&node.id).map_err(thg_error_from_store)? else {
        return Ok(node);
    };

    let mut labels = existing.labels;
    for label in node.labels {
        if !labels.iter().any(|candidate| candidate == &label) {
            labels.push(label);
        }
    }
    node.labels = labels;
    node.properties = merge_properties(existing.properties, node.properties);
    Ok(node)
}

fn merge_properties(existing: Value, incoming: Value) -> Value {
    let mut merged = existing.as_object().cloned().unwrap_or_default();
    let Some(incoming) = incoming.as_object() else {
        return Value::Object(merged);
    };
    for (key, value) in incoming {
        if value.is_null() && merged.contains_key(key) {
            continue;
        }
        merged.insert(key.clone(), value.clone());
    }
    Value::Object(merged)
}

fn flush_import_batch<S: AdapterGraphStore>(
    store: &mut S,
    mutations: &mut Vec<GraphMutation>,
    transactions: &mut usize,
    graph_version: &mut u64,
) -> ThgResult<()> {
    if mutations.is_empty() {
        return Ok(());
    }
    let batch = GraphMutationBatch::new(std::mem::take(mutations));
    let transaction = store.commit_batch(batch).map_err(thg_error_from_store)?;
    *transactions += 1;
    *graph_version = transaction.graph_version;
    Ok(())
}

fn gnn_export_artifact_node_id(tenant_id: &str, export_id: &str, file_name: &str) -> String {
    artifact_node_id(
        tenant_id,
        &format!(
            "gnn-export:{}:{}",
            export_id.trim(),
            slug_segment(file_name)
        ),
    )
}

fn gnn_export_entity_node_id(tenant_id: &str, export_id: &str, sha_hash: &str) -> String {
    format!(
        "gnn_entity:{}:{}:{}",
        normalize_tenant_id(tenant_id),
        slug_segment(export_id),
        slug_segment(sha_hash)
    )
}

fn gnn_relation_edge_type(relation: &str) -> String {
    let slug = relation
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if slug.is_empty() {
        "GNN_RELATED".to_string()
    } else {
        format!("GNN_{slug}")
    }
}

fn gnn_temporal_relation_edge_type(relation: &str) -> String {
    let base = gnn_relation_edge_type(relation);
    base.strip_prefix("GNN_")
        .map(|suffix| format!("GNN_TEMPORAL_{suffix}"))
        .unwrap_or_else(|| "GNN_TEMPORAL_RELATED".to_string())
}

fn slug_segment(value: &str) -> String {
    let mut slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    slug.trim_matches('-').to_string()
}

fn io_thg(code: &'static str) -> impl FnOnce(std::io::Error) -> ThgError {
    move |err| ThgError::new(code, err.to_string())
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
