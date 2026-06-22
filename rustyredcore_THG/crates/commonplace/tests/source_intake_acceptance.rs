//! Acceptance for the commonplace-side deliverables of
//! SPEC-COMMONPLACE-SOURCE-INTAKE-AND-ROUTING: A3 (source-ref lookup), A4 (batch
//! ingest parity), B1 (routing rule + soft source prior), B2 (the two-tier
//! organize boundary), and C1 (tasks as first-class graph nodes).

use std::collections::HashMap;

use commonplace::{
    decide, route, Classification, Commonplace, DeterministicEmbedder, Embedder, IngestInput,
    IngestPipeline, InMemoryBlobStore, Item, ItemKind, NeedsYouReason, OrganizeDecision,
    OrganizePolicy, RoutingRule, SourceRef, TaskFields, COLLECTION_EMBEDDING_PROPERTY,
    DEFAULT_SOURCE_PRIOR_BOOST, ENTITY_LABEL, ITEM_EMBEDDING_PROPERTY, SIMILAR_TO_EDGE,
};
use rustyred_thg_core::{InMemoryGraphStore, NeighborQuery, NodeQuery};
use serde_json::json;

type Cp = Commonplace<InMemoryGraphStore, InMemoryBlobStore>;

fn fresh() -> Cp {
    Commonplace::new(InMemoryGraphStore::new(), InMemoryBlobStore::new())
}

/// Create a collection and stamp a label embedding so cosine classification can
/// score against it (mirrors what F2 does on auto-create).
fn seed_collection(cp: &mut Cp, name: &str, label: Vec<f32>) -> String {
    let coll = cp
        .create_collection(name, commonplace::CollectionKind::Manual)
        .unwrap();
    let mut node = cp.store().get_node(&coll.id).cloned().unwrap();
    if let Some(props) = node.properties.as_object_mut() {
        props.insert(COLLECTION_EMBEDDING_PROPERTY.to_string(), json!(label));
    }
    cp.store_mut().upsert_node(node).unwrap();
    coll.id
}

/// Put a note with a chosen embedding, source, and collection membership.
fn put_embedded(
    cp: &mut Cp,
    title: &str,
    embedding: Vec<f32>,
    source: Option<&str>,
    collections: &[String],
) -> Item {
    let mut item = Item::new(ItemKind::Note, title)
        .with_extra(ITEM_EMBEDDING_PROPERTY, json!(embedding))
        .with_collections(collections.iter().cloned());
    if let Some(source) = source {
        item = item.with_source(source);
    }
    cp.put_item(item).unwrap()
}

// ---- A3: source-ref idempotency lookup --------------------------------------

#[test]
fn a3_item_by_source_ref_finds_and_reuses_in_place() {
    let mut cp = fresh();

    let stored = cp
        .put_item(
            Item::new(ItemKind::Doc, "Gmail thread")
                .with_text("v1 body")
                .with_source_ref(SourceRef::new("gmail", "msg-123")),
        )
        .unwrap();

    // Found by its exact (source, external_id).
    let found = cp.item_by_source_ref("gmail", "msg-123").unwrap().unwrap();
    assert_eq!(found.id, stored.id);
    assert_eq!(found.source.as_deref(), Some("gmail"));

    // Re-fetching the same record (same id, changed body) updates in place: one
    // item, not two.
    cp.put_item(
        Item::new(ItemKind::Doc, "Gmail thread")
            .with_id(stored.id.clone())
            .with_text("v2 body")
            .with_source_ref(SourceRef::new("gmail", "msg-123")),
    )
    .unwrap();
    let again = cp.item_by_source_ref("gmail", "msg-123").unwrap().unwrap();
    assert_eq!(again.id, stored.id, "same node, updated in place");

    let all: Vec<_> = cp
        .all_items()
        .unwrap()
        .into_iter()
        .filter(|i| i.source_ref_key().as_deref() == Some("gmail:msg-123"))
        .collect();
    assert_eq!(all.len(), 1, "exactly one item for the source record");

    // A different external id is a different item, and a missing one is None.
    assert!(cp.item_by_source_ref("gmail", "msg-999").unwrap().is_none());
    assert!(cp.item_by_source_ref("notion", "msg-123").unwrap().is_none());
}

// ---- A4: batch ingest is identical to one-at-a-time -------------------------

/// A structural summary that ignores time/counter-based node ids: (item title ->
/// collection name), (similar-edge title -> title), and entity canonicals.
type GraphSummary = (Vec<(String, String)>, Vec<(String, String)>, Vec<String>);

fn graph_summary(cp: &Cp) -> GraphSummary {
    let items = cp.all_items().unwrap();
    let id_to_title: HashMap<String, String> =
        items.iter().map(|i| (i.id.clone(), i.title.clone())).collect();

    let mut filed: Vec<(String, String)> = items
        .iter()
        .map(|item| {
            let collection = item
                .collections
                .first()
                .and_then(|cid| cp.get_collection(cid).unwrap())
                .map(|c| c.name)
                .unwrap_or_default();
            (item.title.clone(), collection)
        })
        .collect();
    filed.sort();

    let mut sims: Vec<(String, String)> = Vec::new();
    for item in &items {
        for hit in cp
            .store()
            .neighbors(NeighborQuery::out(&item.id).with_edge_type(SIMILAR_TO_EDGE))
        {
            if let Some(title) = id_to_title.get(&hit.node_id) {
                sims.push((item.title.clone(), title.clone()));
            }
        }
    }
    sims.sort();

    let mut entities: Vec<String> = cp
        .store()
        .query_nodes(NodeQuery::label(ENTITY_LABEL).with_limit(usize::MAX))
        .iter()
        .filter_map(|n| {
            n.properties
                .get("canonical")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect();
    entities.sort();

    (filed, sims, entities)
}

fn intake_inputs() -> Vec<IngestInput> {
    vec![
        IngestInput::document(
            "Lease review",
            "Client: Acme Corp. Lease contract with indemnity language and venue clause.",
        ),
        IngestInput::document(
            "Lease follow-up",
            "Client: Acme Corp. Follow-up contract memo about indemnity and lease terms.",
        ),
        IngestInput::note("Sourdough", "Recipe: flour water salt starter, long fermentation."),
    ]
}

#[test]
fn a4_batch_ingest_graph_is_identical_to_one_at_a_time() {
    let pipeline = IngestPipeline::default();

    let mut one_at_a_time = fresh();
    for input in intake_inputs() {
        pipeline.ingest(&mut one_at_a_time, input).unwrap();
    }

    let mut batched = fresh();
    let receipts = pipeline.ingest_batch(&mut batched, intake_inputs()).unwrap();
    assert_eq!(receipts.len(), 3);

    assert_eq!(
        graph_summary(&one_at_a_time),
        graph_summary(&batched),
        "batch path produces the identical graph"
    );
    // The two legal docs share a collection and a similarity edge in both stores.
    let (filed, sims, entities) = graph_summary(&batched);
    assert!(filed.iter().any(|(_, c)| c == "Legal"));
    assert!(!sims.is_empty(), "the two legal docs link as similar");
    assert!(entities.iter().any(|c| c.contains("acme")));
}

// ---- B1: source as a routing signal -----------------------------------------

#[test]
fn b1_explicit_rule_hard_routes_regardless_of_cosine() {
    let mut cp = fresh();
    let pipeline = IngestPipeline::default();

    // A pre-existing "Legal" collection whose label is the legal content, so a
    // legal-looking capture's top cosine points at Legal.
    let legal = seed_collection(
        &mut cp,
        "Legal",
        DeterministicEmbedder::default()
            .embed_text("contract indemnity venue clause")
            .unwrap(),
    );

    let rules = vec![RoutingRule::new("gmail", Some("Receipts".into()), "Finance")];
    let matched = route(&rules, "gmail", Some("Receipts")).expect("rule matches");
    assert_eq!(matched.collection, "Finance");

    // The capture is legal-looking content arriving from gmail/Receipts.
    let receipt = pipeline
        .ingest_routed(
            &mut cp,
            IngestInput::document("Receipt", "Contract indemnity venue clause invoice")
                .with_source("gmail"),
            &matched.collection,
        )
        .unwrap();

    assert_eq!(receipt.collection.name, "Finance");
    assert!(receipt.item.collections.contains(&receipt.collection.id));
    // It did NOT land in Legal even though its content cosine points there.
    let legal_members = cp.collection_items(&legal).unwrap();
    assert!(
        legal_members.iter().all(|m| m.id != receipt.item.id),
        "hard route wins over cosine"
    );

    // No rule for a different container -> falls through to auto classification.
    assert!(route(&rules, "gmail", Some("Other")).is_none());
    assert!(route(&rules, "linear", Some("Receipts")).is_none());
}

#[test]
fn b1_soft_source_prior_breaks_ties_but_content_still_wins() {
    let mut cp = fresh();
    let pipeline = IngestPipeline::default();

    // Equal-content collections A and B (both cosine 0.8 to the item), plus C
    // which is an exact content match (cosine 1.0).
    let a = seed_collection(&mut cp, "Alpha", vec![0.8, 0.6, 0.0]);
    let b = seed_collection(&mut cp, "Beta", vec![0.8, 0.0, 0.6]);
    let _c = seed_collection(&mut cp, "Gamma", vec![1.0, 0.0, 0.0]);

    // B already holds a "linear"-source item; A does not.
    put_embedded(&mut cp, "prior", vec![0.8, 0.0, 0.6], Some("linear"), std::slice::from_ref(&b));

    let item = put_embedded(&mut cp, "incoming", vec![1.0, 0.0, 0.0], Some("linear"), &[]);

    // Pure classification: A and B tie on content; the prior is not applied.
    let pure = pipeline.classify_item(&cp, &item).unwrap();
    let rank_of = |c: &Classification, id: &str| {
        c.ranked.iter().position(|r| r.collection_id == id).unwrap()
    };
    assert!(rank_of(&pure, &a) != rank_of(&pure, &b));

    // With the prior, the shared-source collection (B) ranks above the equal A.
    let primed = pipeline
        .classify_item_with_source_prior(&cp, &item, DEFAULT_SOURCE_PRIOR_BOOST)
        .unwrap();
    assert!(
        rank_of(&primed, &b) < rank_of(&primed, &a),
        "shared-source collection ranks slightly higher"
    );
    // ... but the strong content match (Gamma, cosine 1.0) still wins overall.
    assert_eq!(
        primed.best().unwrap().collection_name,
        "Gamma",
        "a strong content match beats the prior"
    );
}

// ---- B2: the two-tier boundary ----------------------------------------------

#[test]
fn b2_decide_bands_and_zero_model_calls() {
    let policy = OrganizePolicy::default(); // auto 0.72, floor 0.58, margin 0.05

    // AutoFiled: a single established collection at cosine 1.0, unambiguous.
    let mut cp = fresh();
    let x = seed_collection(&mut cp, "Exact", vec![1.0, 0.0, 0.0]);
    let item = put_embedded(&mut cp, "auto", vec![1.0, 0.0, 0.0], None, &[]);
    match decide(&cp, &item, &policy).unwrap() {
        OrganizeDecision::AutoFiled { collection_id, confidence } => {
            assert_eq!(collection_id, x);
            assert!(confidence >= policy.auto_ceiling);
        }
        other => panic!("expected AutoFiled, got {other:?}"),
    }
    // Running decide over many items makes zero model calls (no embedder in play).
    for _ in 0..1000 {
        assert!(matches!(
            decide(&cp, &item, &policy).unwrap(),
            OrganizeDecision::AutoFiled { .. }
        ));
    }

    // FiledForReview: cosine 0.65 (in the review band), unambiguous.
    let mut cp = fresh();
    seed_collection(&mut cp, "Mid", vec![0.65, 0.76, 0.0]);
    let item = put_embedded(&mut cp, "review", vec![1.0, 0.0, 0.0], None, &[]);
    assert!(matches!(
        decide(&cp, &item, &policy).unwrap(),
        OrganizeDecision::FiledForReview { .. }
    ));

    // NeedsYou / LowConfidence: cosine 0.4, below the floor.
    let mut cp = fresh();
    seed_collection(&mut cp, "Far", vec![0.4, 0.917, 0.0]);
    let item = put_embedded(&mut cp, "low", vec![1.0, 0.0, 0.0], None, &[]);
    assert!(matches!(
        decide(&cp, &item, &policy).unwrap(),
        OrganizeDecision::NeedsYou { reason: NeedsYouReason::LowConfidence, .. }
    ));

    // NeedsYou / Ambiguous: two near-equal high scores within the margin.
    let mut cp = fresh();
    seed_collection(&mut cp, "P", vec![0.80, 0.60, 0.0]);
    seed_collection(&mut cp, "Q", vec![0.81, 0.586, 0.0]);
    let item = put_embedded(&mut cp, "amb", vec![1.0, 0.0, 0.0], None, &[]);
    assert!(matches!(
        decide(&cp, &item, &policy).unwrap(),
        OrganizeDecision::NeedsYou { reason: NeedsYouReason::Ambiguous, .. }
    ));

    // NeedsYou / NoCandidates: an item with no embedding has nothing to score.
    let mut cp = fresh();
    seed_collection(&mut cp, "Unreachable", vec![1.0, 0.0, 0.0]);
    let bare = cp.put_item(Item::new(ItemKind::Note, "bare")).unwrap();
    assert!(matches!(
        decide(&cp, &bare, &policy).unwrap(),
        OrganizeDecision::NeedsYou { reason: NeedsYouReason::NoCandidates, .. }
    ));
}

// ---- C1: tasks as first-class graph nodes -----------------------------------

#[test]
fn c1_task_answers_progress_due_and_about_by_graph() {
    let mut cp = fresh();

    // A task that came from an email it is "about".
    let email = cp
        .put_item(Item::new(ItemKind::Note, "Invoice email").with_source("gmail"))
        .unwrap();
    let parent = cp
        .put_item(
            Item::task("Pay the invoice", "wire the amount before the due date")
                .with_status("todo")
                .with_priority("high")
                .with_due_at(1_700_000_000_000),
        )
        .unwrap();
    cp.link_about(&parent.id, &email.id).unwrap();

    // Two subtasks (one done) and one dependency.
    let s1 = cp.put_item(Item::task("Find the IBAN", "").with_status("done")).unwrap();
    let s2 = cp.put_item(Item::task("Get approval", "").with_status("todo")).unwrap();
    let dep = cp.put_item(Item::task("Top up the account", "").with_status("todo")).unwrap();
    cp.add_subtask(&parent.id, &s1.id).unwrap();
    cp.add_subtask(&parent.id, &s2.id).unwrap();
    cp.add_dependency(&parent.id, &dep.id).unwrap();

    // Progress by reverse SUBTASK_OF traversal + status count.
    assert_eq!(cp.subtask_progress(&parent.id).unwrap(), (1, 2));

    // "Due today" by a scalar range query.
    let due = cp
        .tasks_due_between(1_699_000_000_000, 1_701_000_000_000)
        .unwrap();
    assert!(due.iter().any(|t| t.id == parent.id));

    // "What is this about" by an ABOUT traversal back to the email.
    assert_eq!(cp.task_about(&parent.id).unwrap(), vec![email.id.clone()]);

    // Dependencies and open-task filtering.
    assert_eq!(cp.task_dependencies(&parent.id).unwrap()[0].id, dep.id);
    let open: Vec<String> = cp.open_tasks().unwrap().into_iter().map(|t| t.id).collect();
    assert!(open.contains(&parent.id) && open.contains(&s2.id));
    assert!(!open.contains(&s1.id), "the done subtask is not open");
}

#[test]
fn c1_task_classifies_into_a_collection_like_a_note() {
    let mut cp = fresh();
    let pipeline = IngestPipeline::default();

    // A task input with scalars flows through tier-one ingest exactly like a note.
    let receipt = pipeline
        .ingest(
            &mut cp,
            IngestInput::note("Ship the release", "cut the tag and publish notes")
                .as_task(TaskFields {
                    status: Some("todo".into()),
                    priority: Some("medium".into()),
                    due_at_ms: Some(1_700_000_000_000),
                }),
        )
        .unwrap();

    assert_eq!(receipt.item.kind, ItemKind::Task);
    assert!(!receipt.collection.name.is_empty(), "filed into a collection");
    assert!(!receipt.embedding.is_empty(), "embedded like any item");
    // Scalars rode the universal capture contract onto the stored task.
    let stored = cp.get_item(&receipt.item.id).unwrap().unwrap();
    assert_eq!(stored.status.as_deref(), Some("todo"));
    assert_eq!(stored.priority.as_deref(), Some("medium"));
    assert_eq!(stored.due_at_ms, Some(1_700_000_000_000));
}
