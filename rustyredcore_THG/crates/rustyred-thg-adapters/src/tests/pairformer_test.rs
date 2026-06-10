use crate::{
    run_pairformer, PairformerConfig, PairformerEdgeInput, PairformerInput, PairformerNodeInput,
};

fn chain_input() -> PairformerInput {
    PairformerInput {
        nodes: vec![
            PairformerNodeInput {
                node_id: "node:a".to_string(),
                features: vec![1.0, 0.0, 0.2],
            },
            PairformerNodeInput {
                node_id: "node:b".to_string(),
                features: vec![0.5, 0.5, 0.1],
            },
            PairformerNodeInput {
                node_id: "node:c".to_string(),
                features: vec![0.0, 1.0, 0.3],
            },
        ],
        edges: vec![
            PairformerEdgeInput {
                edge_id: "edge:a-b".to_string(),
                source_id: "node:a".to_string(),
                target_id: "node:b".to_string(),
                edge_type: "RELATES_TO".to_string(),
                features: vec![0.8, 0.1],
                confidence: 0.91,
            },
            PairformerEdgeInput {
                edge_id: "edge:b-c".to_string(),
                source_id: "node:b".to_string(),
                target_id: "node:c".to_string(),
                edge_type: "RELATES_TO".to_string(),
                features: vec![0.7, 0.2],
                confidence: 0.88,
            },
        ],
    }
}

#[test]
fn pairformer_returns_dense_pair_and_single_representations() {
    let output = run_pairformer(
        &chain_input(),
        PairformerConfig {
            pair_dim: 8,
            single_dim: 8,
            blocks: 2,
            transition_hidden_dim: 16,
            max_nodes: 8,
            ..PairformerConfig::default()
        },
    )
    .unwrap();

    assert_eq!(output.node_ids, vec!["node:a", "node:b", "node:c"]);
    assert_eq!(output.single_representations.len(), 3);
    assert_eq!(output.single_representations[0].len(), 8);
    assert_eq!(output.pair_representations.len(), 9);
    assert_eq!(output.pair_representations[0].values.len(), 8);
    assert_eq!(output.link_scores.len(), 6);
}

#[test]
fn pairformer_is_deterministic_for_same_input() {
    let config = PairformerConfig {
        pair_dim: 8,
        single_dim: 8,
        blocks: 2,
        transition_hidden_dim: 16,
        max_nodes: 8,
        ..PairformerConfig::default()
    };
    let left = run_pairformer(&chain_input(), config.clone()).unwrap();
    let right = run_pairformer(&chain_input(), config).unwrap();

    assert_eq!(left, right);
}

#[test]
fn pairformer_scores_two_hop_supported_links() {
    let output = run_pairformer(
        &chain_input(),
        PairformerConfig {
            pair_dim: 8,
            single_dim: 8,
            blocks: 2,
            transition_hidden_dim: 16,
            max_nodes: 8,
            ..PairformerConfig::default()
        },
    )
    .unwrap();

    let a_to_c = output
        .link_scores
        .iter()
        .find(|score| score.source_id == "node:a" && score.target_id == "node:c")
        .unwrap();
    let c_to_a = output
        .link_scores
        .iter()
        .find(|score| score.source_id == "node:c" && score.target_id == "node:a")
        .unwrap();

    assert!(a_to_c.support_path.is_some());
    assert!(a_to_c.score > c_to_a.score);
}

#[test]
fn pairformer_rejects_inputs_above_bound() {
    let err = run_pairformer(
        &chain_input(),
        PairformerConfig {
            max_nodes: 2,
            ..PairformerConfig::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.code, "pairformer_bound_exceeded");
}
