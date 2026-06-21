use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustyred_thg_core::{
    EdgeRecord, HookDispatcher, HookDispatcherConfig, InMemoryGraphStore, NodeRecord,
    RedCoreGraphStore,
};
use serde_json::json;

use crate::{
    admitted_edge_id, standing_pass_hook, AdvisoryCandidate, HotTemporalStandingGenerator,
    InferredEdgeCandidate, PairformerConfig, PairformerStandingGenerator, StandingGenerator,
    StandingPassConfig, StandingPassEngine, STANDING_PASS_ADMITTED_BY,
};

fn standing_config() -> StandingPassConfig {
    StandingPassConfig {
        tenant_id: "theorem".to_string(),
        max_nodes: 16,
        max_depth: 2,
        min_path_confidence: 0.0,
        confidence_threshold: 0.0,
        confidence_ceiling: 0.72,
        max_candidates: 16,
        admission_tier: "advisory_inferred".to_string(),
        allowed_edge_types: Vec::new(),
        pairformer_config: PairformerConfig {
            pair_dim: 8,
            single_dim: 8,
            blocks: 2,
            transition_hidden_dim: 16,
            max_nodes: 16,
            ..PairformerConfig::default()
        },
        auto_apply_at_confidence_ceiling: true,
    }
}

fn seed_fixture<S: crate::types::AdapterGraphStore>(store: &mut S) {
    for (node_id, properties) in [
        (
            "node:a",
            json!({ "embedding": [1.0, 0.0], "t_valid": 1_000, "t_invalid": 2_000 }),
        ),
        ("node:b", json!({ "embedding": [0.5, 0.5] })),
        ("node:c", json!({ "embedding": [0.0, 1.0] })),
        ("node:d", json!({ "t_valid": 2_100, "t_invalid": 3_000 })),
    ] {
        store
            .upsert_node(NodeRecord::new(node_id, ["Object"], properties))
            .unwrap();
    }
    store
        .upsert_edge(
            EdgeRecord::new("edge:a-b", "node:a", "RELATES_TO", "node:b", json!({}))
                .with_confidence(0.95),
        )
        .unwrap();
    store
        .upsert_edge(
            EdgeRecord::new("edge:b-c", "node:b", "RELATES_TO", "node:c", json!({}))
                .with_confidence(0.95),
        )
        .unwrap();
    store
        .upsert_edge(
            EdgeRecord::new("edge:a-d", "node:a", "OBSERVED_WITH", "node:d", json!({}))
                .with_confidence(0.95),
        )
        .unwrap();
}

#[test]
fn standing_engine_runs_pairformer_and_temporal_generators_and_admits_union() {
    let mut store = InMemoryGraphStore::new();
    seed_fixture(&mut store);
    let config = standing_config();
    let engine = StandingPassEngine::new(
        config.clone(),
        vec![
            Arc::new(PairformerStandingGenerator::new(
                config.pairformer_config.clone(),
            )),
            Arc::new(HotTemporalStandingGenerator::default()),
        ],
    )
    .unwrap();

    let result = engine.run(&mut store, vec!["node:a".to_string()]).unwrap();

    assert_eq!(
        result.generator_ids,
        vec![
            "pairformer-structural/default".to_string(),
            "hot-temporal/heuristic-v0".to_string()
        ]
    );
    assert!(result.candidates.iter().any(|candidate| {
        candidate.generator_id == "pairformer-structural/default"
            && matches!(
                &candidate.subject,
                crate::CandidateRef::EdgeProposal {
                    source_id,
                    target_id,
                    edge_type
                } if source_id == "node:a"
                    && target_id == "node:c"
                    && edge_type == "INFERRED_RELATES_TO"
            )
            && candidate.support.is_some()
    }));
    assert!(result.candidates.iter().any(|candidate| {
        candidate.generator_id == "hot-temporal/heuristic-v0"
            && matches!(
                &candidate.subject,
                crate::CandidateRef::EdgeProposal {
                    source_id,
                    target_id,
                    edge_type
                } if source_id == "node:a" && target_id == "node:d" && edge_type == "PRECEDES"
            )
            && candidate.support.is_some()
    }));
    assert!(!result.candidate_node_ids.is_empty());
    assert!(result
        .applied_edge_ids
        .iter()
        .any(|edge_id| { edge_id == "edge:node:a:INFERRED_RELATES_TO:node:c" }));
    assert!(result
        .applied_edge_ids
        .iter()
        .any(|edge_id| edge_id == "edge:node:a:PRECEDES:node:d"));

    let applied = store
        .get_edge("edge:node:a:INFERRED_RELATES_TO:node:c")
        .expect("standing pass applied precomputed structural edge");
    assert_eq!(
        applied.properties["admitted_by"],
        json!(STANDING_PASS_ADMITTED_BY)
    );
    assert!(
        (applied.confidence.unwrap_or_default() - 0.72).abs() < 1e-6,
        "stored confidence should preserve the admission ceiling"
    );
}

#[test]
fn standing_pass_hook_materializes_candidates_after_foreground_write() {
    let store = Arc::new(Mutex::new(RedCoreGraphStore::memory()));
    {
        let mut guard = store.lock().unwrap();
        seed_fixture(&mut *guard);
    }

    let dispatcher = HookDispatcher::start(
        Arc::clone(&store),
        vec![standing_pass_hook(standing_config()).unwrap()],
        HookDispatcherConfig {
            debounce: Duration::from_millis(20),
            idle_poll: Duration::from_millis(5),
            max_depth: 3,
            ..Default::default()
        },
    );
    {
        let mut guard = store.lock().unwrap();
        guard.attach_hook_emitter(dispatcher.emitter());
        guard.set_hook_tenant("theorem");
        guard
            .upsert_node(NodeRecord::new(
                "node:a",
                ["Object"],
                json!({
                    "embedding": [1.0, 0.0],
                    "t_valid": 1_000,
                    "t_invalid": 2_000,
                    "standing_pass_touch": true
                }),
            ))
            .unwrap();
    }

    assert!(dispatcher.quiesce(Duration::from_secs(5)));
    let guard = store.lock().unwrap();
    let edge = guard
        .get_edge("edge:node:a:INFERRED_RELATES_TO:node:c")
        .unwrap()
        .expect("hook materialized standing-pass edge");
    assert_eq!(
        edge.properties["admitted_by"],
        json!(STANDING_PASS_ADMITTED_BY)
    );
    let candidate_nodes = guard
        .graph_snapshot()
        .nodes
        .into_iter()
        .filter(|node| {
            node.labels
                .iter()
                .any(|label| label == crate::REFLEXIVE_EDGE_CANDIDATE_LABEL)
        })
        .count();
    assert!(candidate_nodes >= 2, "structural plus temporal candidates");
}

#[derive(Debug)]
struct CountingGenerator {
    calls: Arc<AtomicUsize>,
}

impl StandingGenerator for CountingGenerator {
    fn id(&self) -> &str {
        "counting-generator/test"
    }

    fn generate(
        &self,
        input: &crate::GeneratorInput,
    ) -> rustyred_thg_core::ThgResult<Vec<AdvisoryCandidate>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let edge = InferredEdgeCandidate {
            candidate_id: "counting-edge".to_string(),
            tenant_id: input.query.tenant_id.clone(),
            source_id: "node:a".to_string(),
            target_id: "node:b".to_string(),
            proposed_edge_type: "COUNTED_WITH".to_string(),
            confidence: input.query.confidence_ceiling,
            confidence_ceiling: input.query.confidence_ceiling,
            admission_tier: input.query.admission_tier.clone(),
            model_id: self.id().to_string(),
            support_path_edge_ids: Vec::new(),
            support_path_node_ids: vec!["node:a".to_string(), "node:b".to_string()],
        };
        Ok(vec![AdvisoryCandidate::from_edge(self.id(), edge)])
    }
}

#[test]
fn reads_use_precomputed_structure_without_calling_generators() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut store = InMemoryGraphStore::new();
    store
        .upsert_node(NodeRecord::new("node:a", ["Object"], json!({})))
        .unwrap();
    store
        .upsert_node(NodeRecord::new("node:b", ["Object"], json!({})))
        .unwrap();
    let engine = StandingPassEngine::new(
        standing_config(),
        vec![Arc::new(CountingGenerator {
            calls: Arc::clone(&calls),
        })],
    )
    .unwrap();

    let result = engine.run(&mut store, vec!["node:a".to_string()]).unwrap();
    let edge_id = admitted_edge_id(
        result.candidates[0]
            .edge_candidate()
            .expect("edge advisory payload"),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    for _ in 0..5 {
        assert!(store.get_edge(&edge_id).is_some());
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "reads did not re-enter generator"
    );
}
