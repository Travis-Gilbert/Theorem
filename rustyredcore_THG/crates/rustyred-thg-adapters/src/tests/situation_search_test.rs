use serde_json::json;

use rustyred_thg_core::{InMemoryGraphStore, NeighborQuery, NodeRecord};

use crate::{
    context_candidates_from_similar_situation, record_context_scoring_result,
    record_similar_situation_search, register_semantic_vector_designations, score_context_atoms,
    similar_situation_search, ContextAtomCandidate, ContextScoringPolicy,
    SimilarSituationSearchMode, SimilarSituationSearchPolicy, SimilarSituationSearchRequest,
    CODE_FILE_LABEL, CODE_OBJECT_LABEL, CODE_SYMBOL_LABEL, CONTEXT_ATOM_SELECTED,
    EMBEDDING_CODE_UNIXCODER_768, EMBEDDING_SITUATION_SBERT_384, ESCALATED_TO_SEARCH,
    MATCHED_SIMILAR_SITUATION, POSTMORTEM_LABEL, REASONING_TRACE_LABEL,
};

const TEST_EMBEDDING: &str = "embedding_test_3";

#[test]
fn semantic_designations_cover_memory_training_user_and_code_views() {
    let mut store = InMemoryGraphStore::new();

    let designations = register_semantic_vector_designations(&mut store).unwrap();

    assert!(designations
        .iter()
        .any(|designation| designation.label == POSTMORTEM_LABEL
            && designation.property == EMBEDDING_SITUATION_SBERT_384
            && designation.dimension == 384));
    assert!(designations
        .iter()
        .any(|designation| designation.label == REASONING_TRACE_LABEL
            && designation.property == EMBEDDING_SITUATION_SBERT_384
            && designation.dimension == 384));
    assert!(designations
        .iter()
        .any(|designation| designation.label == CODE_OBJECT_LABEL
            && designation.property == EMBEDDING_CODE_UNIXCODER_768
            && designation.dimension == 768));
    assert!(designations
        .iter()
        .any(|designation| designation.label == CODE_SYMBOL_LABEL
            && designation.property == EMBEDDING_CODE_UNIXCODER_768
            && designation.dimension == 768));
    assert!(designations
        .iter()
        .any(|designation| designation.label == CODE_FILE_LABEL
            && designation.property == EMBEDDING_CODE_UNIXCODER_768
            && designation.dimension == 768));
}

#[test]
fn auto_mode_biases_to_open_web_while_graph_is_still_young() {
    let mut store = seeded_situation_store();

    let result = similar_situation_search(
        &store,
        SimilarSituationSearchRequest {
            tenant_id: "theorem".to_string(),
            query_text: Some("repair a rustyweb retrieval drift".to_string()),
            query_embedding: vec![1.0, 0.0, 0.0],
            embedding_property: TEST_EMBEDDING.to_string(),
            target_labels: vec![POSTMORTEM_LABEL.to_string()],
            top_k: 5,
            mode: SimilarSituationSearchMode::Auto,
        },
        SimilarSituationSearchPolicy::default(),
    )
    .unwrap();

    assert_eq!(result.hits.len(), 3);
    assert_eq!(result.hits[0].node_id, "postmortem:theorem:retrieval-drift");
    assert_eq!(result.best_similarity, Some(1.0));
    assert!(!result.decision.local_memory_sufficient);
    assert!(!result.decision.should_search_codebase);
    assert!(result.decision.should_search_open_web);
    assert!(result
        .decision
        .reasons
        .contains(&"graph_below_maturity_threshold".to_string()));

    let receipt =
        record_similar_situation_search(&mut store, &result, Some("repair retrieval drift"), None)
            .unwrap();
    assert!(store.get_node(&receipt.search_node_id).is_some());
    assert_eq!(receipt.escalation_plan_node_ids.len(), 1);
    assert_eq!(
        store
            .neighbors(
                NeighborQuery::out(&receipt.search_node_id)
                    .with_edge_type(MATCHED_SIMILAR_SITUATION)
            )
            .len(),
        3
    );
    assert_eq!(
        store
            .neighbors(
                NeighborQuery::out(&receipt.search_node_id).with_edge_type(ESCALATED_TO_SEARCH)
            )
            .len(),
        1
    );
}

#[test]
fn auto_mode_allows_local_only_when_graph_is_mature_and_matches_are_strong() {
    let store = seeded_situation_store();

    let result = similar_situation_search(
        &store,
        SimilarSituationSearchRequest {
            tenant_id: "theorem".to_string(),
            query_text: Some("repair a rustyweb retrieval drift".to_string()),
            query_embedding: vec![1.0, 0.0, 0.0],
            embedding_property: TEST_EMBEDDING.to_string(),
            target_labels: vec![POSTMORTEM_LABEL.to_string()],
            top_k: 5,
            mode: SimilarSituationSearchMode::Auto,
        },
        SimilarSituationSearchPolicy {
            min_local_similarity_without_external: 0.90,
            min_local_hits_without_external: 2,
            graph_maturity_nodes_for_local_only: 2,
            always_search_web_until_graph_matures: true,
        },
    )
    .unwrap();

    assert!(result.decision.local_memory_sufficient);
    assert!(!result.decision.should_search_codebase);
    assert!(!result.decision.should_search_open_web);
    assert_eq!(result.decision.reasons, vec!["local_memory_sufficient"]);
}

#[test]
fn context_scorer_selects_required_and_pinned_atoms_under_budget() {
    let result = score_context_atoms(
        "theorem",
        vec![
            context_candidate("atom:expensive", 1.0, 90),
            ContextAtomCandidate {
                node_id: "atom:pinned".to_string(),
                similarity: 0.70,
                token_cost: 25,
                pinned: true,
                success_count: 3,
                ..context_candidate("atom:pinned", 0.70, 25)
            },
            ContextAtomCandidate {
                node_id: "atom:required".to_string(),
                similarity: 0.40,
                token_cost: 25,
                required: true,
                ..context_candidate("atom:required", 0.40, 25)
            },
        ],
        ContextScoringPolicy {
            token_budget: 50,
            max_atoms: 2,
            ..ContextScoringPolicy::default()
        },
    )
    .unwrap();

    assert_eq!(
        result.selected_node_ids,
        vec!["atom:required".to_string(), "atom:pinned".to_string()]
    );
    assert_eq!(result.used_tokens, 50);
    assert!(result.bounded);
    assert!(result.ranked_atoms[0]
        .reasons
        .contains(&"required".to_string()));
    assert!(result.ranked_atoms[1]
        .reasons
        .contains(&"pinned".to_string()));
    assert!(result
        .ranked_atoms
        .iter()
        .any(|atom| atom.node_id == "atom:expensive"
            && atom.reasons.contains(&"token_budget_exceeded".to_string())));
}

#[test]
fn context_scorer_uses_receipts_to_demote_failed_memory() {
    let result = score_context_atoms(
        "theorem",
        vec![
            ContextAtomCandidate {
                node_id: "atom:helped".to_string(),
                similarity: 0.90,
                use_count: 9,
                success_count: 8,
                failure_count: 1,
                ..context_candidate("atom:helped", 0.90, 20)
            },
            ContextAtomCandidate {
                node_id: "atom:hurt".to_string(),
                similarity: 0.90,
                use_count: 9,
                success_count: 1,
                failure_count: 8,
                ..context_candidate("atom:hurt", 0.90, 20)
            },
        ],
        ContextScoringPolicy::default(),
    )
    .unwrap();

    assert_eq!(result.ranked_atoms[0].node_id, "atom:helped");
    assert!(result.ranked_atoms[0]
        .reasons
        .contains(&"receipt_success".to_string()));
    assert!(result.ranked_atoms[1]
        .reasons
        .contains(&"failure_penalty".to_string()));
    assert!(result.ranked_atoms[0].score > result.ranked_atoms[1].score);
}

#[test]
fn context_candidates_from_similar_situation_can_be_recorded_as_pack() {
    let mut store = seeded_situation_store();
    let search = similar_situation_search(
        &store,
        SimilarSituationSearchRequest {
            tenant_id: "theorem".to_string(),
            query_text: Some("repair a rustyweb retrieval drift".to_string()),
            query_embedding: vec![1.0, 0.0, 0.0],
            embedding_property: TEST_EMBEDDING.to_string(),
            target_labels: vec![POSTMORTEM_LABEL.to_string()],
            top_k: 3,
            mode: SimilarSituationSearchMode::RecallOnly,
        },
        SimilarSituationSearchPolicy::default(),
    )
    .unwrap();

    let candidates = context_candidates_from_similar_situation(&search, 64);
    let pack = score_context_atoms(
        "theorem",
        candidates,
        ContextScoringPolicy {
            token_budget: 128,
            max_atoms: 2,
            ..ContextScoringPolicy::default()
        },
    )
    .unwrap();
    let receipt = record_context_scoring_result(&mut store, &pack, Some("test")).unwrap();

    assert_eq!(pack.selected_node_ids.len(), 2);
    assert_eq!(receipt.selected_edge_ids.len(), 2);
    assert!(store.get_node(&receipt.context_pack_node_id).is_some());
    assert_eq!(
        store
            .neighbors(
                NeighborQuery::out(&receipt.context_pack_node_id)
                    .with_edge_type(CONTEXT_ATOM_SELECTED)
            )
            .len(),
        2
    );
}

fn context_candidate(node_id: &str, similarity: f32, token_cost: usize) -> ContextAtomCandidate {
    ContextAtomCandidate {
        node_id: node_id.to_string(),
        label: POSTMORTEM_LABEL.to_string(),
        summary: None,
        similarity,
        token_cost,
        age_ms: None,
        use_count: 0,
        success_count: 0,
        failure_count: 0,
        pinned: false,
        required: false,
        graph_degree: 0,
    }
}

fn seeded_situation_store() -> InMemoryGraphStore {
    let mut store = InMemoryGraphStore::new();
    store
        .designate_vector_property(POSTMORTEM_LABEL, TEST_EMBEDDING, 3)
        .unwrap();
    store
        .upsert_node(NodeRecord::new(
            "postmortem:theorem:retrieval-drift",
            [POSTMORTEM_LABEL],
            json!({
                "tenant_id": "theorem",
                "failure_mode": "retrieval_drift",
                "repair_pattern": "rustyweb_refresh",
                TEST_EMBEDDING: [1.0, 0.0, 0.0],
            }),
        ))
        .unwrap();
    store
        .upsert_node(NodeRecord::new(
            "postmortem:theorem:nearby-drift",
            [POSTMORTEM_LABEL],
            json!({
                "tenant_id": "theorem",
                "failure_mode": "context_drift",
                "repair_pattern": "rerank_then_validate",
                TEST_EMBEDDING: [0.95, 0.05, 0.0],
            }),
        ))
        .unwrap();
    store
        .upsert_node(NodeRecord::new(
            "postmortem:theorem:validation-drift",
            [POSTMORTEM_LABEL],
            json!({
                "tenant_id": "theorem",
                "failure_mode": "validation_drift",
                "repair_pattern": "external_refresh_then_recheck",
                TEST_EMBEDDING: [0.90, 0.10, 0.0],
            }),
        ))
        .unwrap();
    store
        .upsert_node(NodeRecord::new(
            "postmortem:other:retrieval-drift",
            [POSTMORTEM_LABEL],
            json!({
                "tenant_id": "other",
                "failure_mode": "retrieval_drift",
                TEST_EMBEDDING: [1.0, 0.0, 0.0],
            }),
        ))
        .unwrap();
    store
}
