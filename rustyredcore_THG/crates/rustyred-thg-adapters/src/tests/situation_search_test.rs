use serde_json::json;

use rustyred_thg_core::{InMemoryGraphStore, NeighborQuery, NodeRecord};

use crate::{
    record_similar_situation_search, register_semantic_vector_designations,
    similar_situation_search, SimilarSituationSearchMode, SimilarSituationSearchPolicy,
    SimilarSituationSearchRequest, CODE_FILE_LABEL, CODE_OBJECT_LABEL, CODE_SYMBOL_LABEL,
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
