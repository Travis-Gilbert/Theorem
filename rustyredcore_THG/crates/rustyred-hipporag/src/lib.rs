//! HippoRAG 2 candidate generation with graph-native RAPTOR hubs.
//!
//! This crate owns the retrieval layer only. It indexes passages and phrases
//! into the THG graph, builds first-class hub nodes from graph communities, and
//! returns `rustyred_membrane::Candidate` values for the shared membrane gate.
//! Reranking and admission stay in `rustyred-rerank` and `rustyred-membrane`.

pub mod embedding;
pub mod indexing;
pub mod raptor;
pub mod retrieve;
pub mod schema;

pub use embedding::{HippoTextEmbedder, SEMANTIC_VECTOR_METRIC};
pub use indexing::{index_passage, index_passage_with_embedder, IndexStats};
pub use raptor::{
    build_summary_tree_for_region, build_summary_tree_with_embedder, summary_tree_hook,
    HubBuildStats, RaptorPolicy, SummaryTreeHooksPlugin,
};
pub use retrieve::{
    retrieve, retrieve_with_embedder, retrieve_with_query_vector, HippoQuery, RetrievalTrace,
};
pub use schema::{
    HippoEdge, HippoError, HippoLabel, HippoResult, HubNode, PhraseNode, EDGE_CONTAINS,
    EDGE_HUB_PARENT, EDGE_RELATES, EDGE_SUMMARIZES, EDGE_SYNONYM, LABEL_HUB, LABEL_PAGE,
    LABEL_PHRASE, NODE_SPECIFICITY_PROPERTY, SEMANTIC_VECTOR_PROPERTY,
};

#[cfg(test)]
mod tests {
    use futures_util::future::BoxFuture;
    use rustyred_membrane::{fill_to_budget, Candidate, ScoreContext, Scorer};
    use rustyred_thg_core::{EdgeRecord, InMemoryGraphStore, NeighborQuery, NodeQuery, NodeRecord};
    use serde_json::json;

    use super::*;
    use crate::schema::HippoResult;

    #[derive(Clone, Copy)]
    struct PprScorer;

    impl Scorer for PprScorer {
        fn score(&self, candidate: &Candidate, _ctx: &ScoreContext<'_>) -> f32 {
            candidate.ppr_proximity
        }
    }

    #[derive(Clone, Copy)]
    struct KeywordEmbedder;

    impl HippoTextEmbedder for KeywordEmbedder {
        fn model_id(&self) -> &str {
            "test-keyword-embedder"
        }

        fn dimension(&self) -> usize {
            3
        }

        fn embed<'a>(&'a self, inputs: &'a [String]) -> BoxFuture<'a, HippoResult<Vec<Vec<f32>>>> {
            Box::pin(async move {
                Ok(inputs
                    .iter()
                    .map(|text| {
                        let lower = text.to_ascii_lowercase();
                        vec![
                            keyword_score(&lower, &["graph", "hub", "hipporag", "retrieval"]),
                            keyword_score(&lower, &["rerank", "gate", "membrane"]),
                            keyword_score(&lower, &["browser", "crawl", "page"]),
                        ]
                    })
                    .collect())
            })
        }
    }

    fn keyword_score(text: &str, terms: &[&str]) -> f32 {
        terms.iter().filter(|term| text.contains(**term)).count() as f32
    }

    fn page(id: &str, text: &str) -> NodeRecord {
        NodeRecord::new(
            id,
            [LABEL_PAGE],
            json!({
                "url": format!("https://example.com/{id}"),
                "title": id,
                "text": text,
                SEMANTIC_VECTOR_PROPERTY: crate::schema::hash_vector(text, 16),
            }),
        )
    }

    #[test]
    fn indexing_builds_dual_node_graph_with_specificity() {
        let mut store = InMemoryGraphStore::new();
        store
            .upsert_node(page(
                "page:modernbert",
                "ModernBERT reranker model models sequence classification",
            ))
            .unwrap();

        let stats = index_passage(&mut store, "page:modernbert").unwrap();

        assert!(stats.phrases_upserted >= 4);
        assert!(stats.contains_edges >= 4);
        assert!(stats.relates_edges >= 1);
        assert!(
            stats.synonym_edges >= 1,
            "model/models produce a synonym edge"
        );
        let phrase = store
            .query_nodes(NodeQuery::label(LABEL_PHRASE).with_limit(20))
            .into_iter()
            .find(|node| node.properties.get("text").and_then(|v| v.as_str()) == Some("modernbert"))
            .expect("modernbert phrase exists");
        assert!(
            phrase
                .properties
                .get(NODE_SPECIFICITY_PROPERTY)
                .and_then(|value| value.as_f64())
                .unwrap()
                > 0.0
        );
        let contains =
            store.neighbors(NeighborQuery::out("page:modernbert").with_edge_type(EDGE_CONTAINS));
        assert!(!contains.is_empty());
    }

    #[tokio::test]
    async fn index_passage_with_embedder_vectorizes_page_and_phrases() {
        let mut store = InMemoryGraphStore::new();
        store
            .upsert_node(page(
                "page:hippo",
                "HippoRAG graph hub retrieval feeds the membrane gate",
            ))
            .unwrap();

        let stats = index_passage_with_embedder(&mut store, "page:hippo", &KeywordEmbedder)
            .await
            .unwrap();

        assert!(stats.embedded_nodes >= 2);
        assert_eq!(
            stats.embedding_model.as_deref(),
            Some("test-keyword-embedder")
        );
        let page = store.get_node("page:hippo").unwrap();
        assert_eq!(
            page.properties
                .get("semantic_vec_model")
                .and_then(|value| value.as_str()),
            Some("test-keyword-embedder")
        );
        let phrase = store
            .query_nodes(NodeQuery::label(LABEL_PHRASE).with_limit(20))
            .into_iter()
            .find(|node| node.properties.get("text").and_then(|v| v.as_str()) == Some("hipporag"))
            .expect("hipporag phrase exists");
        assert_eq!(
            phrase
                .properties
                .get("semantic_vec_dimension")
                .and_then(|value| value.as_u64()),
            Some(3)
        );
    }

    #[test]
    fn raptor_builds_hubs_once_for_dirty_region() {
        let mut store = InMemoryGraphStore::new();
        for (id, text) in [
            ("page:a", "alpha retrieval graph"),
            ("page:b", "alpha phrase links"),
            ("page:c", "beta unrelated topic"),
            ("page:d", "beta unrelated cluster"),
        ] {
            store.upsert_node(page(id, text)).unwrap();
        }
        store
            .upsert_edge(EdgeRecord::new(
                "manual:a-b",
                "page:a",
                EDGE_RELATES,
                "page:b",
                json!({}),
            ))
            .unwrap();
        store
            .upsert_edge(EdgeRecord::new(
                "manual:c-d",
                "page:c",
                EDGE_RELATES,
                "page:d",
                json!({}),
            ))
            .unwrap();
        let all = store
            .query_nodes(NodeQuery::label(LABEL_PAGE).with_limit(100))
            .len();

        let stats = build_summary_tree_for_region(
            &mut store,
            RaptorPolicy {
                region_node_threshold: 2,
                min_members: 2,
                max_level: 1,
            },
            &["page:a".to_string()],
        )
        .unwrap();

        assert!(
            stats.region_nodes_seen < all,
            "only the dirty component is clustered"
        );
        assert_eq!(stats.hook_counter, 1);
        assert!(stats.hubs_upserted >= 1);
        let hubs = store.query_nodes(NodeQuery::label(LABEL_HUB).with_limit(20));
        assert!(!hubs.is_empty());
        let summarize_edges = hubs
            .iter()
            .flat_map(|hub| {
                store.neighbors(NeighborQuery::out(&hub.id).with_edge_type(EDGE_SUMMARIZES))
            })
            .count();
        assert!(summarize_edges >= 2);
    }

    #[tokio::test]
    async fn build_summary_tree_with_embedder_vectorizes_hubs() {
        let mut store = InMemoryGraphStore::new();
        for id in ["page:one", "page:two", "page:three", "page:four"] {
            store
                .upsert_node(page(
                    id,
                    "HippoRAG graph hub retrieval connects page evidence",
                ))
                .unwrap();
            index_passage_with_embedder(&mut store, id, &KeywordEmbedder)
                .await
                .unwrap();
        }

        build_summary_tree_with_embedder(
            &mut store,
            RaptorPolicy {
                region_node_threshold: 1,
                min_members: 2,
                max_level: 1,
            },
            &["page:one".to_string()],
            &KeywordEmbedder,
        )
        .await
        .unwrap();

        let hub = store
            .query_nodes(NodeQuery::label(LABEL_HUB).with_limit(10))
            .into_iter()
            .next()
            .expect("hub created");
        assert_eq!(
            hub.properties
                .get("semantic_vec_model")
                .and_then(|value| value.as_str()),
            Some("test-keyword-embedder")
        );
    }

    #[test]
    fn retrieve_returns_hubs_and_leaf_pages_from_one_call() {
        let mut store = InMemoryGraphStore::new();
        store
            .upsert_node(page(
                "page:overview",
                "overall shape of modernbert reranker retrieval systems",
            ))
            .unwrap();
        store
            .upsert_node(page(
                "page:specific",
                "sequence classification reranker latency benchmark",
            ))
            .unwrap();
        index_passage(&mut store, "page:overview").unwrap();
        index_passage(&mut store, "page:specific").unwrap();
        build_summary_tree_for_region(
            &mut store,
            RaptorPolicy {
                region_node_threshold: 2,
                min_members: 2,
                max_level: 1,
            },
            &[],
        )
        .unwrap();

        let coarse = retrieve(
            &store,
            HippoQuery::new("overall shape modernbert retrieval", 5),
        );
        assert!(coarse.iter().any(|candidate| candidate
            .metadata
            .get("hippo_label")
            .map(String::as_str)
            == Some("hub")));

        let specific = retrieve(
            &store,
            HippoQuery {
                text: "sequence classification latency",
                top_k: 5,
                include_hubs: true,
            },
        );
        assert!(specific
            .iter()
            .any(|candidate| candidate.node_id == "page:specific"));
    }

    #[tokio::test]
    async fn retrieve_with_embedder_uses_model_query_vector_without_candidate_vector_payload() {
        let mut store = InMemoryGraphStore::new();
        store
            .upsert_node(page(
                "page:graph",
                "HippoRAG graph hub retrieval connects phrase evidence",
            ))
            .unwrap();
        store
            .upsert_node(page(
                "page:browser",
                "Browser crawl page state records links",
            ))
            .unwrap();
        index_passage_with_embedder(&mut store, "page:graph", &KeywordEmbedder)
            .await
            .unwrap();
        index_passage_with_embedder(&mut store, "page:browser", &KeywordEmbedder)
            .await
            .unwrap();

        let (candidates, trace) = retrieve_with_embedder(
            &store,
            HippoQuery {
                text: "graph hub retrieval",
                top_k: 2,
                include_hubs: false,
            },
            &KeywordEmbedder,
        )
        .await
        .unwrap();

        assert!(trace.ran_query_ppr);
        assert_eq!(candidates[0].node_id, "page:graph");
        assert_eq!(
            candidates[0]
                .metadata
                .get("semantic_vec_model")
                .map(String::as_str),
            Some("test-keyword-embedder")
        );
        assert!(!candidates[0].metadata.contains_key("semantic_vec"));
    }

    #[test]
    fn retrieval_reads_warm_hub_prior_and_candidates_feed_membrane_unchanged() {
        let mut store = InMemoryGraphStore::new();
        store
            .upsert_node(page("page:one", "graph resident candidate retrieval"))
            .unwrap();
        store
            .upsert_node(page("page:two", "graph hub summary candidate"))
            .unwrap();
        index_passage(&mut store, "page:one").unwrap();
        index_passage(&mut store, "page:two").unwrap();
        build_summary_tree_for_region(
            &mut store,
            RaptorPolicy {
                region_node_threshold: 2,
                min_members: 2,
                max_level: 1,
            },
            &[],
        )
        .unwrap();

        let (candidates, trace) = retrieve::retrieve_with_trace(
            &store,
            HippoQuery {
                text: "graph candidate",
                top_k: 8,
                include_hubs: true,
            },
        );

        assert!(trace.warm_centrality_reads > 0);
        assert!(trace.ran_query_ppr);
        assert!(!trace.ran_global_ppr);
        let active = Vec::new();
        let ctx = ScoreContext::new("graph candidate", &active).with_mmr_lambda(0.7);
        let admission = fill_to_budget(candidates.clone(), &PprScorer, &ctx, 20);
        assert_eq!(
            admission.admitted.len() + admission.deferred.len(),
            candidates.len(),
            "fill_to_budget consumes the HippoRAG candidate set without another layer"
        );
    }
}
