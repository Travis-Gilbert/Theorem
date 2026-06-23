use serde_json::json;

use rustyred_thg_core::{EdgeRecord, InMemoryGraphStore, NodeRecord};

use crate::{
    build_hot_feature_matrices, evaluate_hot_predictions, extract_higher_order_temporal_neighbors,
    hot_input_from_snapshot, hot_temporal_link_dataset_from_snapshot, patch_align_and_concatenate,
    rank_hot_temporal_densification_candidates, run_hot, score_hot_timed_labels,
    tgat_time_encoding, train_hot_link_model, DensificationRequest, HotConfig, HotInput,
    HotLinkTrainingExample, HotNegativeSamplingScheme, HotNode, HotPairLabel,
    HotTemporalEdge, HotTemporalSplitConfig, HotTrainingConfig,
};

fn hot_config() -> HotConfig {
    HotConfig {
        aligned_dim: 4,
        time_encoding_dim: 4,
        cooccurrence_dim: 4,
        output_dim: 8,
        attention_heads: 2,
        brt_cells: 2,
        brt_block_size: 2,
        segment_size: 4,
        state_vectors: 2,
        patch_size: 2,
        s1: 2,
        s2: 2,
        max_nodes: 16,
        max_temporal_edges: 16,
        decoder_hidden_dim: 8,
        ..HotConfig::default()
    }
}

fn direct_hot_input() -> HotInput {
    HotInput {
        nodes: vec![
            HotNode {
                node_id: "node:a".to_string(),
                features: vec![1.0, 0.0],
            },
            HotNode {
                node_id: "node:b".to_string(),
                features: vec![0.5, 0.5],
            },
            HotNode {
                node_id: "node:c".to_string(),
                features: vec![0.0, 1.0],
            },
            HotNode {
                node_id: "node:x".to_string(),
                features: vec![0.25, 0.75],
            },
        ],
        temporal_edges: vec![
            HotTemporalEdge {
                source_id: "node:a".to_string(),
                target_id: "node:b".to_string(),
                timestamp: 1_000,
                edge_type: "RELATES_TO".to_string(),
                features: vec![0.8],
            },
            HotTemporalEdge {
                source_id: "node:b".to_string(),
                target_id: "node:c".to_string(),
                timestamp: 2_000,
                edge_type: "RELATES_TO".to_string(),
                features: vec![0.7],
            },
            HotTemporalEdge {
                source_id: "node:a".to_string(),
                target_id: "node:x".to_string(),
                timestamp: 2_500,
                edge_type: "MENTIONS".to_string(),
                features: vec![0.4],
            },
            HotTemporalEdge {
                source_id: "node:c".to_string(),
                target_id: "node:x".to_string(),
                timestamp: 2_600,
                edge_type: "MENTIONS".to_string(),
                features: vec![0.6],
            },
        ],
        query_pairs: vec![("node:a".to_string(), "node:c".to_string())],
        as_of: 3_000,
    }
}

#[test]
fn extractor_respects_cutoff_and_recency_caps() {
    let mut store = InMemoryGraphStore::new();
    for node_id in ["node:a", "node:b", "node:c", "node:d"] {
        store
            .upsert_node(NodeRecord::new(
                node_id,
                ["Object"],
                json!({ "embedding": [1.0, 0.0] }),
            ))
            .unwrap();
    }
    for (edge_id, source, target, timestamp) in [
        ("edge:a-b-old", "node:a", "node:b", 1_000),
        ("edge:a-c-new", "node:a", "node:c", 2_000),
        ("edge:a-d-future", "node:a", "node:d", 4_000),
        ("edge:c-d-two-hop", "node:c", "node:d", 2_500),
    ] {
        store
            .upsert_edge(EdgeRecord::new(
                edge_id,
                source,
                "RELATES_TO",
                target,
                json!({ "timestamp_ms": timestamp, "features": [0.5] }),
            ))
            .unwrap();
    }

    let input = hot_input_from_snapshot(
        &store.snapshot(),
        vec![("node:a".to_string(), "node:d".to_string())],
        3_000,
        HotConfig {
            s1: 1,
            s2: 1,
            max_temporal_edges: 4,
            ..hot_config()
        },
    )
    .unwrap();
    let interactions = extract_higher_order_temporal_neighbors(
        &input,
        "node:a",
        HotConfig {
            s1: 1,
            s2: 1,
            ..hot_config()
        },
    );

    assert!(input
        .temporal_edges
        .iter()
        .all(|edge| edge.timestamp < input.as_of));
    assert!(interactions
        .iter()
        .any(|interaction| interaction.neighbor_id == "node:c" && interaction.hop == 1));
    assert!(interactions
        .iter()
        .any(|interaction| interaction.neighbor_id == "node:d" && interaction.hop == 2));
    assert!(!interactions
        .iter()
        .any(|interaction| interaction.neighbor_id == "node:b"));
}

#[test]
fn feature_matrices_mark_hops_and_tgat_encoding_is_interleaved() {
    let input = direct_hot_input();
    let matrices =
        build_hot_feature_matrices(&input, "node:a", "node:c", hot_config()).unwrap();
    let hop_one = matrices
        .hop_marks
        .iter()
        .position(|hop| *hop == 1)
        .unwrap();
    let hop_two = matrices
        .hop_marks
        .iter()
        .position(|hop| *hop == 2)
        .unwrap();
    let hop_one_row = &matrices.node_matrix[hop_one];
    let hop_two_row = &matrices.node_matrix[hop_two];
    assert_eq!(&hop_one_row[hop_one_row.len() - 2..], &[1.0, 0.0]);
    assert_eq!(&hop_two_row[hop_two_row.len() - 2..], &[0.0, 1.0]);

    let encoding = tgat_time_encoding(0, 4);
    assert_eq!(encoding, vec![0.5, 0.0, 0.5, 0.0]);
}

#[test]
fn shared_neighbor_produces_cooccurrence_signal() {
    let input = direct_hot_input();
    let matrices =
        build_hot_feature_matrices(&input, "node:a", "node:c", hot_config()).unwrap();
    let shared_idx = matrices
        .neighbor_ids
        .iter()
        .position(|node_id| node_id == "node:x")
        .unwrap();
    assert!(matrices.cooccurrence_matrix[shared_idx]
        .iter()
        .any(|value| *value > 0.0));
}

#[test]
fn patching_is_horizontal_and_ceil_sized() {
    let input = direct_hot_input();
    let pair = patch_align_and_concatenate(&input, "node:a", "node:c", hot_config()).unwrap();
    assert_eq!(pair.source_patched_len, 2);
    assert_eq!(pair.target_patched_len, 1);
    assert_eq!(pair.patched_sequence_len, 2);
    assert_eq!(pair.rows[0].len(), hot_config().pair_sequence_width());
}

#[test]
fn hot_scores_query_pair_with_temporal_support() {
    let output = run_hot(&direct_hot_input(), hot_config()).unwrap();
    let score = output
        .link_scores
        .iter()
        .find(|score| score.source_id == "node:a" && score.target_id == "node:c")
        .unwrap();
    let support = score.support.as_ref().unwrap();
    assert!(score.score > 0.0 && score.score <= 1.0);
    assert_eq!(support.node_ids, vec!["node:a", "node:b", "node:c"]);
    assert_eq!(support.relation_hint, "TEMPORAL_INFERRED_RELATES_TO");
}

#[test]
fn hot_learned_decoder_improves_planted_link_rule() {
    let positives = (0..8).map(|idx| HotLinkTrainingExample {
        source_id: format!("node:p{idx}"),
        target_id: format!("node:q{idx}"),
        features: vec![2.0, 1.0, 0.5],
        label: true,
    });
    let negatives = (0..8).map(|idx| HotLinkTrainingExample {
        source_id: format!("node:n{idx}"),
        target_id: format!("node:m{idx}"),
        features: vec![-2.0, -1.0, 0.1],
        label: false,
    });
    let examples = positives.chain(negatives).collect::<Vec<_>>();
    let (model, report) = train_hot_link_model(
        &examples,
        HotTrainingConfig {
            learning_rate: 0.08,
            epochs: 250,
            batch_size: 4,
            negative_sampling: HotNegativeSamplingScheme::Random,
            model: HotConfig {
                decoder_hidden_dim: 6,
                ..hot_config()
            },
            ..HotTrainingConfig::default()
        },
    )
    .unwrap();

    assert!(model.score_features(&[2.0, 1.0, 0.5]) > 0.8);
    assert!(model.score_features(&[-2.0, -1.0, 0.1]) < 0.2);
    assert!(report.average_precision > 0.99);
    assert!(report.auc_roc > 0.99);

    let eval = evaluate_hot_predictions(
        &[
            (model.score_features(&[2.0, 1.0, 0.5]), true),
            (model.score_features(&[-2.0, -1.0, 0.1]), false),
        ],
        HotTrainingConfig::default(),
    );
    assert!(eval.final_loss < 0.3);
}

#[test]
fn hot_reflexive_ranker_emits_supported_candidate_and_suppresses_direct_edges() {
    let mut store = InMemoryGraphStore::new();
    for (node_id, embedding) in [
        ("node:a", vec![1.0, 0.0]),
        ("node:b", vec![0.5, 0.5]),
        ("node:c", vec![0.0, 1.0]),
    ] {
        store
            .upsert_node(NodeRecord::new(
                node_id,
                ["Object"],
                json!({ "embedding": embedding }),
            ))
            .unwrap();
    }
    store
        .upsert_edge(
            EdgeRecord::new(
                "edge:a-b",
                "node:a",
                "RELATES_TO",
                "node:b",
                json!({ "timestamp_ms": 1_000, "features": [0.7] }),
            )
            .with_confidence(0.9),
        )
        .unwrap();
    store
        .upsert_edge(
            EdgeRecord::new(
                "edge:b-c",
                "node:b",
                "RELATES_TO",
                "node:c",
                json!({ "timestamp_ms": 2_000, "features": [0.8] }),
            )
            .with_confidence(0.9),
        )
        .unwrap();

    let request = fixture_request(["node:a"]);
    let result = rank_hot_temporal_densification_candidates(
        &store.snapshot(),
        request.clone(),
        hot_config(),
    )
    .unwrap();
    let candidate = result
        .candidates
        .iter()
        .find(|candidate| candidate.source_id == "node:a" && candidate.target_id == "node:c")
        .unwrap();
    assert_eq!(candidate.proposed_edge_type, "TEMPORAL_INFERRED_RELATES_TO");
    assert_eq!(
        candidate.support_path_node_ids,
        vec!["node:a", "node:b", "node:c"]
    );

    store
        .upsert_edge(EdgeRecord::new(
            "edge:a-c-direct",
            "node:a",
            "RELATES_TO",
            "node:c",
            json!({ "timestamp_ms": 3_000 }),
        ))
        .unwrap();
    let suppressed =
        rank_hot_temporal_densification_candidates(&store.snapshot(), request, hot_config())
            .unwrap();
    assert!(suppressed
        .candidates
        .iter()
        .all(|candidate| candidate.source_id != "node:a" || candidate.target_id != "node:c"));
}

#[test]
fn training_examples_can_be_derived_from_hot_input_labels() {
    let labels = vec![HotPairLabel {
        source_id: "node:a".to_string(),
        target_id: "node:c".to_string(),
        label: true,
    }];
    let examples =
        crate::hot_training_examples_from_input(&direct_hot_input(), &labels, hot_config()).unwrap();
    assert_eq!(examples.len(), 1);
    assert!(examples[0].label);
    assert!(!examples[0].features.is_empty());
}

#[test]
fn temporal_dataset_uses_pre_label_context_and_holdout_scores() {
    let mut store = InMemoryGraphStore::new();
    for node_id in ["node:a", "node:b", "node:c", "node:d"] {
        store
            .upsert_node(NodeRecord::new(
                node_id,
                ["Object"],
                json!({ "embedding": [1.0, 0.0], "features": [0.2, 0.8] }),
            ))
            .unwrap();
    }
    for (edge_id, source, target, timestamp) in [
        ("edge:a-b", "node:a", "node:b", 1_000),
        ("edge:b-c", "node:b", "node:c", 2_000),
        ("edge:a-c", "node:a", "node:c", 3_000),
        ("edge:c-d", "node:c", "node:d", 4_000),
    ] {
        store
            .upsert_edge(EdgeRecord::new(
                edge_id,
                source,
                "RELATES_TO",
                target,
                json!({ "timestamp_ms": timestamp, "features": [0.5] }),
            ))
            .unwrap();
    }

    let snapshot = store.snapshot();
    let dataset = hot_temporal_link_dataset_from_snapshot(
        &snapshot,
        hot_config(),
        HotTemporalSplitConfig {
            holdout_fraction: 0.25,
            negatives_per_positive: 1,
            max_positive_edges: 16,
        },
    )
    .unwrap();

    assert_eq!(dataset.test_positive_edges, 1);
    assert!(dataset.train_examples.iter().any(|example| example.label));
    assert!(dataset.train_examples.iter().any(|example| !example.label));
    let positive = dataset
        .test_labels
        .iter()
        .find(|label| label.label)
        .unwrap();
    let input = hot_input_from_snapshot(
        &snapshot,
        vec![(positive.source_id.clone(), positive.target_id.clone())],
        positive.as_of,
        hot_config(),
    )
    .unwrap();
    assert!(input.temporal_edges.iter().all(|edge| {
        edge.timestamp < positive.as_of
            && (edge.source_id != positive.source_id || edge.target_id != positive.target_id)
    }));
    let predictions = score_hot_timed_labels(&snapshot, &dataset.test_labels, hot_config()).unwrap();
    assert_eq!(predictions.len(), dataset.test_labels.len());
}

fn fixture_request<const N: usize>(seed_node_ids: [&str; N]) -> DensificationRequest {
    DensificationRequest {
        tenant_id: "theorem".to_string(),
        seed_node_ids: seed_node_ids.into_iter().map(ToString::to_string).collect(),
        max_nodes: 16,
        max_depth: 2,
        min_path_confidence: 0.0,
        confidence_threshold: 0.0,
        confidence_ceiling: 0.74,
        max_candidates: 8,
        admission_tier: "advisory_inferred".to_string(),
        model_id: "hot-link-predict/test".to_string(),
        allowed_edge_types: vec!["RELATES_TO".to_string()],
    }
}
