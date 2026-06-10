use std::collections::BTreeSet;

use burn::backend::ndarray::NdArrayDevice;
use burn::backend::{Autodiff, NdArray};
use burn::module::AutodiffModule;
use burn::tensor::backend::Backend;
use serde_json::json;

use rustyred_thg_core::{EdgeRecord, InMemoryGraphStore, NodeRecord};

use crate::burn_pairformer::ranking_accuracy;
use crate::pairformer::{PairformerEdgeInput, PairformerInput, PairformerNodeInput};
use crate::{
    featurize_pairformer_input, load_pairformer_file, quarantine_densification_candidates,
    rank_trained_pairformer_densification_candidates, register_trained_pairformer_artifact,
    save_pairformer_file, train_pairformer, BurnPairformerConfig, DensificationRequest,
    PairformerTrainingConfig,
};

/// Burn's backend RNG is a process-global mutex; these tests seed it, so
/// they must not interleave. Each test holds this lock for its duration.
static SEED_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn seed_guard() -> std::sync::MutexGuard<'static, ()> {
    SEED_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

type Inference = NdArray<f32>;
type Training = Autodiff<Inference>;

fn small_config() -> BurnPairformerConfig {
    BurnPairformerConfig {
        node_in_dim: 12,
        edge_in_dim: 4,
        pair_dim: 16,
        single_dim: 16,
        heads: 2,
        blocks: 2,
        transition_mult: 2,
        max_nodes: 32,
    }
}

/// Planted compositional rule: for each triple i, a_i -r1-> b_i -r2-> c_i,
/// and the closure a_i -r3-> c_i. The closure edges for the last
/// `held_out_triples` are omitted entirely: the model never sees them and
/// must generalize the composition from the training triples.
fn planted_rule_input(triples: usize, held_out_triples: usize) -> PairformerInput {
    assert!(triples <= 6, "fixture supports up to six triples");
    // Role one-hot plus triple-identity one-hot: with identity features,
    // every masked-edge recovery is structurally learnable, so the
    // objective is signal rather than noise.
    let role_features = |role: usize, triple: usize| -> Vec<f32> {
        let mut features = vec![0.0_f32; 9];
        features[role] = 1.0;
        features[3 + triple] = 1.0;
        features
    };
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for triple in 0..triples {
        for (role, prefix) in ["a", "b", "c"].iter().enumerate() {
            nodes.push(PairformerNodeInput {
                node_id: format!("node:{prefix}{triple}"),
                features: role_features(role, triple),
            });
        }
        edges.push(PairformerEdgeInput {
            edge_id: format!("edge:r1-{triple}"),
            source_id: format!("node:a{triple}"),
            target_id: format!("node:b{triple}"),
            edge_type: "R1".to_string(),
            features: vec![1.0, 0.0],
            confidence: 0.95,
        });
        edges.push(PairformerEdgeInput {
            edge_id: format!("edge:r2-{triple}"),
            source_id: format!("node:b{triple}"),
            target_id: format!("node:c{triple}"),
            edge_type: "R2".to_string(),
            features: vec![0.0, 1.0],
            confidence: 0.95,
        });
        if triple < triples - held_out_triples {
            edges.push(PairformerEdgeInput {
                edge_id: format!("edge:r3-{triple}"),
                source_id: format!("node:a{triple}"),
                target_id: format!("node:c{triple}"),
                edge_type: "R3".to_string(),
                features: vec![1.0, 1.0],
                confidence: 0.9,
            });
        }
    }
    PairformerInput { nodes, edges }
}

fn flat_scores<B: Backend>(
    model: &crate::BurnPairformer<B>,
    device: &B::Device,
    input: &PairformerInput,
    config: &BurnPairformerConfig,
) -> (Vec<f32>, Vec<String>) {
    let (node_features, edge_grid, node_ids) =
        featurize_pairformer_input::<B>(device, input, config, &BTreeSet::new()).unwrap();
    let n = node_ids.len();
    let scores = model
        .forward(node_features, edge_grid, config.normalized().heads)
        .reshape([n * n])
        .into_data()
        .to_vec::<f32>()
        .unwrap();
    (scores, node_ids)
}

#[test]
fn seeded_init_is_deterministic_and_weights_round_trip_through_files() {
    let _serial = seed_guard();
    let device = NdArrayDevice::default();
    let config = small_config();
    let input = planted_rule_input(3, 0);

    // Param initialization is lazy in burn: the random draws happen at the
    // first forward, so each model must materialize directly after its seed.
    <Inference as Backend>::seed(&device, 99);
    let first = config.init::<Inference>(&device);
    let (scores_first, _) = flat_scores(&first, &device, &input, &config);
    <Inference as Backend>::seed(&device, 99);
    let second = config.init::<Inference>(&device);
    let (scores_second, _) = flat_scores(&second, &device, &input, &config);
    assert_eq!(scores_first, scores_second);
    assert!(scores_first.iter().all(|value| value.is_finite()));

    let dir = std::env::temp_dir().join("burn_pairformer_roundtrip_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pairformer-test-model");
    save_pairformer_file(first, &path).unwrap();
    let loaded = load_pairformer_file::<Inference>(&config, &path, &device).unwrap();
    let (scores_loaded, _) = flat_scores(&loaded, &device, &input, &config);
    assert_eq!(scores_first, scores_loaded);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn masked_edge_training_learns_the_planted_composition_rule() {
    let _serial = seed_guard();
    let device = NdArrayDevice::default();
    let config = small_config();
    // Six triples; the closure edges of the last two are held out entirely.
    let input = planted_rule_input(6, 2);
    let n = input.nodes.len();
    let node_index = |node_id: &str| -> usize {
        input
            .nodes
            .iter()
            .position(|node| node.node_id == node_id)
            .unwrap()
    };

    // Held-out closure pairs the model never observed as edges.
    let held_out_positives = vec![
        (node_index("node:a4"), node_index("node:c4")),
        (node_index("node:a5"), node_index("node:c5")),
    ];
    // Structural negatives: cross-triple pairs with no two-hop path.
    let negatives = vec![
        (node_index("node:a4"), node_index("node:c5")),
        (node_index("node:a5"), node_index("node:c4")),
        (node_index("node:a4"), node_index("node:b5")),
        (node_index("node:a5"), node_index("node:b4")),
        (node_index("node:c4"), node_index("node:a4")),
        (node_index("node:c5"), node_index("node:a5")),
    ];

    // Untrained baseline with a different seed: how well does random init
    // rank the held-out closures?
    <Inference as Backend>::seed(&device, 1234);
    let untrained = config.init::<Inference>(&device);
    let (untrained_scores, _) = flat_scores(&untrained, &device, &input, &config);
    let untrained_accuracy =
        ranking_accuracy(&untrained_scores, n, &held_out_positives, &negatives);

    let (trained, report) = train_pairformer::<Training>(
        &device,
        &input,
        config,
        PairformerTrainingConfig {
            epochs: 90,
            learning_rate: 4e-3,
            mask_fraction: 0.3,
            negatives_per_positive: 2,
            seed: 17,
        },
    )
    .unwrap();

    // The objective went down and the in-task ranking is strong. Each
    // epoch re-samples its mask, so single-epoch losses are noisy: compare
    // the mean of the first five epochs against the mean of the last five.
    assert!(report.final_loss.is_finite());
    let early: f32 = report.epoch_losses.iter().take(5).sum::<f32>() / 5.0;
    let late: f32 = report.epoch_losses.iter().rev().take(5).sum::<f32>() / 5.0;
    assert!(
        late < early * 0.75,
        "training should reduce smoothed loss: early {early} late {late}"
    );
    assert!(
        report.final_ranking_accuracy >= 0.8,
        "in-task ranking accuracy too low: {}",
        report.final_ranking_accuracy
    );

    // Generalization: the trained model ranks held-out closure pairs above
    // structural negatives, and beats the untrained baseline.
    let inference_model = trained.valid();
    let (trained_scores, _) = flat_scores(&inference_model, &device, &input, &config);
    let trained_accuracy = ranking_accuracy(&trained_scores, n, &held_out_positives, &negatives);
    assert!(
        trained_accuracy >= 0.75,
        "held-out composition ranking accuracy too low: {trained_accuracy}"
    );
    assert!(
        trained_accuracy > untrained_accuracy,
        "training must beat the untrained baseline: trained {trained_accuracy} untrained {untrained_accuracy}"
    );
}

#[test]
fn trained_model_drives_densification_and_artifact_registration() {
    let _serial = seed_guard();
    let device = NdArrayDevice::default();
    let config = small_config();
    let input = planted_rule_input(4, 1);
    let (trained, report) = train_pairformer::<Training>(
        &device,
        &input,
        config,
        PairformerTrainingConfig {
            epochs: 12,
            learning_rate: 4e-3,
            mask_fraction: 0.3,
            negatives_per_positive: 2,
            seed: 7,
        },
    )
    .unwrap();
    let inference_model = trained.valid();

    // Build the same planted graph in a store for the snapshot path.
    let mut store = InMemoryGraphStore::new();
    for node in &input.nodes {
        store
            .upsert_node(NodeRecord::new(
                &node.node_id,
                ["Object"],
                json!({ "features": node.features }),
            ))
            .unwrap();
    }
    for edge in &input.edges {
        store
            .upsert_edge(
                EdgeRecord::new(
                    &edge.edge_id,
                    &edge.source_id,
                    edge.edge_type.as_str(),
                    &edge.target_id,
                    json!({ "features": edge.features }),
                )
                .with_confidence(edge.confidence as f64),
            )
            .unwrap();
    }

    let result = rank_trained_pairformer_densification_candidates(
        &store.snapshot(),
        DensificationRequest {
            tenant_id: "theorem".to_string(),
            seed_node_ids: vec!["node:a3".to_string()],
            max_nodes: 32,
            max_depth: 2,
            min_path_confidence: 0.0,
            confidence_threshold: 0.05,
            confidence_ceiling: 0.72,
            max_candidates: 8,
            admission_tier: "advisory_inferred".to_string(),
            model_id: "pairformer-burn/trained-test".to_string(),
            allowed_edge_types: vec![],
        },
        &inference_model,
        &device,
        &config,
    )
    .unwrap();

    // The held-out triple's closure (a3 -> c3) surfaces as an advisory
    // candidate with two-hop provenance, capped by the ceiling.
    let candidate = result
        .candidates
        .iter()
        .find(|candidate| candidate.source_id == "node:a3" && candidate.target_id == "node:c3")
        .expect("trained model proposes the held-out closure");
    assert_eq!(candidate.support_path_edge_ids.len(), 2);
    assert!(candidate.confidence <= 0.72);

    let quarantine = quarantine_densification_candidates(
        &mut store,
        "theorem",
        "trained-run-1",
        &result.candidates,
        Some("test"),
    )
    .unwrap();
    assert_eq!(quarantine.candidate_node_ids.len(), result.candidates.len());

    // Weights persist as a file artifact registered in the graph; nodes
    // carry metrics and a pointer, never tensors.
    let dir = std::env::temp_dir().join("burn_pairformer_artifact_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pairformer-trained");
    save_pairformer_file(inference_model, &path).unwrap();
    let graph_version = store.snapshot().version;
    let writeback = register_trained_pairformer_artifact(
        &mut store,
        "theorem",
        "pairformer-burn/trained-test",
        "s3://theorem-models/pairformer/trained-test.bin",
        &config,
        &report,
        result.considered_node_ids.clone(),
        graph_version,
        Some("test"),
    )
    .unwrap();
    // Weights pointer lives on the model artifact node; metrics live on the
    // linked evaluation receipt. Neither carries tensors.
    let artifact = store.get_node(&writeback.model_node_id).unwrap();
    assert_eq!(artifact.properties["model_type"], json!("pairformer-burn"));
    let evaluation = store.get_node(&writeback.evaluation_node_id).unwrap();
    assert_eq!(
        evaluation.properties["metrics"]["final_ranking_accuracy"],
        json!(report.final_ranking_accuracy)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
