//! Organize acceptance: the daily triage partition.
//!
//! Proves that `organize` partitions the items that arrived in a timeframe into
//! what the engine filed confidently ("organized today") and what still needs a
//! human ("needs you"), reusing the engine's classification signal (cosine of an
//! item's stored embedding to each collection's label embedding) for both, and
//! that `daily_progress` reconciles with the partition.
//!
//! The DeterministicEmbedder is text-seeded, so items that share tokens cluster.
//! We build two confident clusters (so two collections exist with label
//! embeddings) and one deliberately token-disjoint item whose cosine to every
//! collection falls below the ceiling.

use commonplace::{
    Commonplace, DeterministicEmbedder, InMemoryBlobStore, IngestInput, IngestPipeline,
};
use commonplace_api::{organize, OrganizeConfig, Timeframe};
use rustyred_thg_core::InMemoryGraphStore;

fn store() -> Commonplace<InMemoryGraphStore, InMemoryBlobStore> {
    Commonplace::new(InMemoryGraphStore::new(), InMemoryBlobStore::new())
}

// A higher-dimensional deterministic embedder: 16-bucket hashing collapses
// token-disjoint text into similar dense vectors, so we seed at 256 dims where
// distinct tokens land in distinct buckets and cosine separates clusters
// cleanly. `classify_item` only compares the STORED vectors, so the pipeline
// `organize` is called with does not need this embedder.
fn seed_pipeline() -> IngestPipeline<DeterministicEmbedder> {
    IngestPipeline::new(DeterministicEmbedder::new(256))
}

#[test]
fn organize_partitions_confident_from_needs_you() {
    let mut cp = store();
    let pipeline = seed_pipeline();

    // Cluster 1 (rust): three docs sharing strong tokens -> one auto collection,
    // each classifies confidently against that collection's label embedding.
    pipeline
        .ingest(
            &mut cp,
            IngestInput::document(
                "Rust ownership",
                "rust ownership borrowing memory safety lifetime compiler",
            ),
        )
        .unwrap();
    pipeline
        .ingest(
            &mut cp,
            IngestInput::document(
                "Rust borrowing",
                "rust ownership borrowing memory safety lifetime rules",
            ),
        )
        .unwrap();
    pipeline
        .ingest(
            &mut cp,
            IngestInput::document(
                "Rust lifetimes",
                "rust ownership borrowing memory safety lifetime annotations",
            ),
        )
        .unwrap();

    // Cluster 2 (cooking): a second confident collection, disjoint from rust.
    pipeline
        .ingest(
            &mut cp,
            IngestInput::document(
                "Pasta recipe",
                "cooking pasta tomato garlic basil olive simmer sauce",
            ),
        )
        .unwrap();
    pipeline
        .ingest(
            &mut cp,
            IngestInput::document(
                "Risotto recipe",
                "cooking pasta tomato garlic basil olive simmer broth",
            ),
        )
        .unwrap();

    // The ambiguous item: token-disjoint from both clusters. We file it into the
    // existing collections (threshold 0) so it does NOT create its own collection
    // that would later classify against itself at cosine ~1.0. Its stored
    // embedding is still far from every collection's label embedding.
    let loose = seed_pipeline().with_collection_threshold(0.0);
    let ambiguous = loose
        .ingest(
            &mut cp,
            IngestInput::note(
                "Quarterly tax question",
                "wonder whether quarterly estimated payment deadline shifted weekend holiday",
            ),
        )
        .unwrap();
    let ambiguous_id = ambiguous.item.id.clone();

    // All six items are stored (five cluster docs plus the ambiguous one); the
    // ambiguous item joined an existing collection rather than minting a third.
    let item_count = cp.all_items().unwrap().len();
    assert_eq!(item_count, 6, "five cluster docs plus the ambiguous one");

    let config = OrganizeConfig {
        needs_you_ceiling: 0.58,
        timeframe: Timeframe::Day,
        needs_you_cap: 24,
        now_ms: now_after_all(&cp),
    };
    let snapshot = organize(&cp, &IngestPipeline::default(), &config).unwrap();

    // needs_you holds the ambiguous item, below the ceiling, with a target and
    // populated alternatives (two collections exist).
    let needs = &snapshot.needs_you;
    let ambiguous_entry = needs
        .iter()
        .find(|item| item.item.id == ambiguous_id)
        .expect("ambiguous item is surfaced as needing you");
    assert!(
        ambiguous_entry.confidence < config.needs_you_ceiling,
        "ambiguous item confidence {} must be below the ceiling {}",
        ambiguous_entry.confidence,
        config.needs_you_ceiling
    );
    assert!(
        ambiguous_entry.target_collection_id.is_some(),
        "ambiguous item still gets a best-collection target"
    );
    assert!(
        !ambiguous_entry.alternatives.is_empty(),
        "two collections exist, so the ambiguous item has a next-best alternative"
    );

    // The confident rust/cooking docs are filed (organized today), NOT in needs_you.
    let needs_titles: Vec<&str> = needs
        .iter()
        .map(|item| item.item.title.as_str())
        .collect();
    assert!(
        !needs_titles.contains(&"Rust ownership"),
        "a confidently-filed item is not in needs_you"
    );

    let today = &snapshot.organized_today;
    assert!(
        today.total_count >= 5,
        "the five confident cluster docs are organized today (got {})",
        today.total_count
    );
    let most_recent = today
        .most_recent
        .as_ref()
        .expect("organized_today surfaces a most-recent filed item");
    assert!(
        !most_recent.filed_at.is_empty() && most_recent.filed_at.ends_with('Z'),
        "filed_at is an ISO-8601 UTC string"
    );
    assert!(
        !today.groups.is_empty(),
        "filed items group under their target collections"
    );

    // daily_progress reconciles: done == organized_today.total_count, and
    // done + (below-ceiling needs_you) == total intake.
    assert_eq!(snapshot.daily_progress.timeframe, "day");
    assert_eq!(snapshot.daily_progress.done, today.total_count);
    let below_ceiling_needs = needs
        .iter()
        .filter(|item| item.confidence < config.needs_you_ceiling)
        .count();
    assert_eq!(
        snapshot.daily_progress.done + below_ceiling_needs,
        snapshot.daily_progress.total,
        "done + below-ceiling needs_you reconciles with total intake"
    );
    assert_eq!(snapshot.needs_you_ceiling, 0.58);
}

#[test]
fn needs_you_truncates_at_cap() {
    let mut cp = store();

    // Five token-disjoint notes with no collection ever created confidently:
    // each carries an embedding but there is no matching collection, so each is
    // low-confidence and lands in needs_you.
    for token in ["alpha", "bravo", "charlie", "delta", "echo"] {
        // Each note seeds its own auto collection at ingest, but we file via a
        // zero-threshold pipeline into the FIRST collection so no per-item
        // self-matching collection is created after the first.
        let loose = seed_pipeline().with_collection_threshold(0.0);
        loose
            .ingest(
                &mut cp,
                IngestInput::note(format!("{token} note"), format!("{token} {token} unique body")),
            )
            .unwrap();
    }

    let config = OrganizeConfig {
        needs_you_ceiling: 0.99, // force everything below the ceiling
        timeframe: Timeframe::Day,
        needs_you_cap: 2,
        now_ms: now_after_all(&cp),
    };
    let snapshot = organize(&cp, &IngestPipeline::default(), &config).unwrap();

    assert_eq!(
        snapshot.needs_you.len(),
        2,
        "needs_you is truncated to the cap of 2"
    );
    // The full intake still counts beyond the cap.
    assert_eq!(snapshot.daily_progress.total, 5);
}

#[test]
fn singleton_seed_surfaces_in_needs_you() {
    // The F2 ingest path files EVERY item at capture, minting a fresh singleton
    // collection for anything novel. Confidence-to-best-collection would read
    // ~1.0 for that self-seeded bucket, so the item would wrongly look "filed".
    // organize measures confidence against ESTABLISHED collections (>1 member),
    // so a lonely self-seeded item correctly surfaces in needs_you.
    let mut cp = store();
    let pipeline = seed_pipeline();

    // An established cluster: two rust docs share one collection (2 members).
    pipeline
        .ingest(
            &mut cp,
            IngestInput::document("Rust A", "rust ownership borrowing memory safety lifetime"),
        )
        .unwrap();
    pipeline
        .ingest(
            &mut cp,
            IngestInput::document("Rust B", "rust ownership borrowing memory safety compiler"),
        )
        .unwrap();

    // A novel item: token-disjoint, so the default threshold mints its OWN
    // singleton collection (cosine ~1.0 to that singleton's label embedding).
    let novel = pipeline
        .ingest(
            &mut cp,
            IngestInput::note(
                "Garden plan",
                "tomato seedlings watering schedule greenhouse compost",
            ),
        )
        .unwrap();
    let novel_id = novel.item.id.clone();

    let config = OrganizeConfig {
        needs_you_ceiling: 0.58,
        timeframe: Timeframe::Day,
        needs_you_cap: 24,
        now_ms: now_after_all(&cp),
    };
    let snapshot = organize(&cp, &IngestPipeline::default(), &config).unwrap();

    // The novel item is in needs_you despite cosine ~1.0 to its own singleton.
    let entry = snapshot
        .needs_you
        .iter()
        .find(|item| item.item.id == novel_id)
        .expect("a novel self-seeded item surfaces in needs_you, not filed");
    assert!(
        entry.confidence < config.needs_you_ceiling,
        "confidence is measured against established collections, not the item's own singleton (got {})",
        entry.confidence
    );

    // The established cluster is filed (organized today), not in needs_you.
    assert!(
        snapshot.organized_today.total_count >= 2,
        "the established rust cluster is organized today (got {})",
        snapshot.organized_today.total_count
    );
    assert!(
        !snapshot
            .needs_you
            .iter()
            .any(|item| item.item.title == "Rust A"),
        "an established-cluster doc is filed, not in needs_you"
    );
}

#[test]
fn task_kind_carries_subtasks() {
    let mut cp = store();
    let pipeline = seed_pipeline();

    // Two clustered docs establish a collection (so the surface has a real
    // classification baseline, as the daily-driver path always does).
    pipeline
        .ingest(
            &mut cp,
            IngestInput::document(
                "Rust ownership",
                "rust ownership borrowing memory safety lifetime compiler",
            ),
        )
        .unwrap();
    pipeline
        .ingest(
            &mut cp,
            IngestInput::document(
                "Rust borrowing",
                "rust ownership borrowing memory safety lifetime rules",
            ),
        )
        .unwrap();

    // A checkbox note, token-disjoint from the rust cluster so it stays in
    // needs_you (below the ceiling) and does not mint a self-matching singleton
    // (filed into the existing collection via the zero-threshold pipeline).
    // Its body has three subtasks, the first done.
    let loose = seed_pipeline().with_collection_threshold(0.0);
    let task = loose
        .ingest(
            &mut cp,
            IngestInput::note(
                "Offsite checklist",
                "- [x] book the room\n- [ ] send the agenda\n* [ ] invite finance",
            ),
        )
        .unwrap();
    let task_id = task.item.id.clone();

    let config = OrganizeConfig {
        needs_you_ceiling: 0.58,
        timeframe: Timeframe::Day,
        needs_you_cap: 24,
        now_ms: now_after_all(&cp),
    };
    let snapshot = organize(&cp, &IngestPipeline::default(), &config).unwrap();

    let entry = snapshot
        .needs_you
        .iter()
        .find(|item| item.item.id == task_id)
        .expect("the checkbox note surfaces in needs_you");

    assert_eq!(
        entry.kind, "task",
        "an item with checkbox subtasks derives kind=task (got {})",
        entry.kind
    );
    assert_eq!(
        entry.subtasks.len(),
        3,
        "three checkbox lines parse into three subtasks"
    );
    assert_eq!(
        entry.subtasks.iter().filter(|s| s.done).count(),
        1,
        "exactly one subtask is done"
    );
    // Order preserved: first done, then the two open ones in document order.
    assert!(entry.subtasks[0].done, "the first subtask is done");
    assert_eq!(entry.subtasks[0].text, "book the room");
    assert_eq!(entry.subtasks[1].text, "send the agenda");
    assert!(!entry.subtasks[1].done);
    assert_eq!(entry.subtasks[2].text, "invite finance");
    assert!(!entry.subtasks[2].done);
}

/// A "now" just after every item's `updated_at_ms` so all arrive within the day.
fn now_after_all(cp: &Commonplace<InMemoryGraphStore, InMemoryBlobStore>) -> i64 {
    cp.all_items()
        .unwrap()
        .iter()
        .map(|item| item.updated_at_ms)
        .max()
        .unwrap_or(0)
        + 1
}
