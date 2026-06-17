//! Spec 1 (Prolly incremental commit) acceptance tests.
//!
//! Handoff: docs/reference (prolly-incremental-commit-handoff). Proves the
//! commit path is O(changed), not O(graph): a fixed change set writes a flat
//! number of chunks regardless of graph size, no whole-graph object set is
//! materialized per commit, adjacent-commit diff is O(k + log n), and the
//! incremental build stays byte-identical to a full rebuild (the dual-commit
//! validation gate is ON by default in these debug-profile test builds, so
//! every `compile_graph_pack_incremental` call below additionally cross-checks
//! itself against a full recompile).

use rustyred_thg_core::{
    apply_graph_mutation_batch, build_prolly_tree, checkout_graph_version, compile_graph_pack,
    compile_graph_pack_incremental, diff_graph_snapshots, diff_graph_trees, update_graph_ref_cas,
    GraphCompileOptions, GraphMutation, GraphMutationBatch, GraphSnapshot, GraphVersionRepository,
    NodeRecord,
};
use serde_json::json;

fn node(i: usize, rev: u64) -> NodeRecord {
    NodeRecord::new(
        format!("node:{i:07}"),
        ["Doc"],
        json!({ "i": i, "rev": rev }),
    )
}

fn snapshot(version: u64, n: usize) -> GraphSnapshot {
    GraphSnapshot {
        version,
        nodes: (0..n).map(|i| node(i, 0)).collect(),
        edges: Vec::new(),
    }
}

fn opts(ts: u128) -> GraphCompileOptions {
    GraphCompileOptions {
        branch: Some("main".to_string()),
        timestamp_unix_ms: Some(ts),
        ..GraphCompileOptions::default()
    }
}

/// Acceptance #1/#2: committing one node upsert writes a flat number of chunks
/// and bytes regardless of graph size, and only the changed object is
/// materialized (no whole-graph clone on the commit path).
#[test]
fn acceptance_commit_cost_is_flat_in_graph_size() {
    let small_n = 1_000usize;
    let big_n = 40_000usize;

    let measure = |n: usize| {
        let base = snapshot(1, n);
        let base_pack = compile_graph_pack(&base, opts(1));
        let mid = n / 2;
        let batch = GraphMutationBatch::new([GraphMutation::NodeUpsert(node(mid, 1))]);
        compile_graph_pack_incremental(&base_pack, &batch, opts(2))
    };

    let small = measure(small_n);
    let big = measure(big_n);

    // Flat in graph size: the change set is identical, so the only growth is the
    // O(log n) spine. Both stay small in absolute terms.
    assert!(
        small.commit_cost.chunks_written <= 80,
        "small chunks_written={} should be small",
        small.commit_cost.chunks_written
    );
    assert!(
        big.commit_cost.chunks_written <= 80,
        "big chunks_written={} should be small",
        big.commit_cost.chunks_written
    );
    assert!(
        big.commit_cost.chunks_written <= small.commit_cost.chunks_written + 40,
        "big chunks_written={} must stay flat vs small={} (only the O(log n) spine differs)",
        big.commit_cost.chunks_written,
        small.commit_cost.chunks_written
    );
    assert!(
        big.commit_cost.bytes_written < 512_000 && small.commit_cost.bytes_written < 512_000,
        "bytes_written must be far below O(graph): small={} big={}",
        small.commit_cost.bytes_written,
        big.commit_cost.bytes_written
    );

    // No whole-graph clone: only the single changed object is carried as the
    // commit's delta payload (the parent's whole object set is never cloned).
    assert_eq!(
        big.pack.objects.len(),
        1,
        "only the delta object materializes"
    );
    assert_eq!(small.pack.objects.len(), 1);

    // Reuse dwarfs rewrite: nearly the whole tree is shared by hash.
    assert!(big.commit_cost.reused_tree_nodes > big.commit_cost.chunks_written * 10);
}

/// Acceptance #3: an adjacent-commit diff visits O(k + log n) tree nodes, not
/// O(graph), and agrees with the snapshot diff.
#[test]
fn acceptance_adjacent_commit_diff_is_o_k() {
    let n = 5_000usize;
    let base = snapshot(1, n);
    let base_tree = compile_graph_pack(&base, opts(1)).tree;

    let mut target = base.clone();
    let touched = [100usize, 2_500, 4_900];
    for &i in &touched {
        target.nodes[i] = node(i, 9);
    }
    target.version += 1;
    let target_tree = compile_graph_pack(&target, opts(2)).tree;

    let (tree_diff, visits) = diff_graph_trees(&base_tree, &target_tree);

    assert_eq!(tree_diff.modified.len(), touched.len(), "3 nodes modified");
    assert_eq!(tree_diff.added.len(), 0);
    assert_eq!(tree_diff.removed.len(), 0);
    assert!(
        visits < n / 5,
        "diff visited {visits} nodes, must be far below graph size {n} (O(k + log n))"
    );

    // Tree diff agrees with the canonical snapshot diff.
    let snap_diff = diff_graph_snapshots(&base, &target);
    assert_eq!(tree_diff.modified.len(), snap_diff.modified.len());
    assert_eq!(tree_diff.added.len(), snap_diff.added.len());
    assert_eq!(tree_diff.removed.len(), snap_diff.removed.len());
}

/// Acceptance #4: reconstructing the graph at the new root equals the
/// pre-commit state plus the applied batch, and the incremental root is
/// byte-identical to a full recompile.
#[test]
fn acceptance_incremental_commit_reconstructs_correctly() {
    let base = snapshot(7, 200);
    let base_pack = compile_graph_pack(&base, opts(1));
    let repo = update_graph_ref_cas(
        GraphVersionRepository::default(),
        base_pack.clone(),
        Some("main".to_string()),
        None,
        Some(1),
    )
    .expect("initial CAS")
    .repository;

    // One modify + one insert.
    let batch = GraphMutationBatch::new([
        GraphMutation::NodeUpsert(node(100, 3)),
        GraphMutation::NodeUpsert(node(999, 1)),
    ]);
    // Identical message so the commit hashes (which fold in the message) match;
    // the incremental and full paths only diverge on their default message.
    let meta = GraphCompileOptions {
        branch: Some("main".to_string()),
        parent_commits: vec![base_pack.commit.commit_hash.clone()],
        message: Some("reconstruct acceptance".to_string()),
        timestamp_unix_ms: Some(2),
        ..GraphCompileOptions::default()
    };
    let incremental = compile_graph_pack_incremental(&base_pack, &batch, meta.clone());

    // Root matches a full rebuild of the applied snapshot.
    let applied = apply_graph_mutation_batch(&base, &batch);
    let full = compile_graph_pack(&applied, meta);
    assert_eq!(incremental.pack.tree.root_hash, full.tree.root_hash);
    assert_eq!(incremental.pack.commit.commit_hash, full.commit.commit_hash);

    // Store the incremental pack, check out, reconstruct == applied.
    let head = base_pack.commit.commit_hash.clone();
    let repo2 = update_graph_ref_cas(
        repo,
        incremental.pack.clone(),
        Some("main".to_string()),
        Some(head),
        Some(2),
    )
    .expect("incremental CAS")
    .repository;
    let checkout = checkout_graph_version(&repo2, "main").expect("checkout");

    let mut got: Vec<String> = checkout
        .snapshot
        .nodes
        .iter()
        .map(|n| n.id.clone())
        .collect();
    let mut want: Vec<String> = applied.nodes.iter().map(|n| n.id.clone()).collect();
    got.sort();
    want.sort();
    assert_eq!(got, want, "reconstructed node set == base + batch");

    // The modified node carries its revised content after reconstruction.
    let reconstructed = checkout
        .snapshot
        .nodes
        .iter()
        .find(|n| n.id == "node:0000100")
        .expect("node:0000100 present");
    assert_eq!(reconstructed.properties.get("rev"), Some(&json!(3)));
}

/// Acceptance #4/#5 robustness: the incremental build is byte-identical to a
/// full rebuild across graph sizes (incl. chunk-boundary sizes and empty/single
/// graphs) and randomized modify+insert batches. Each call also self-validates
/// via the dual-commit gate (ON in debug).
#[test]
fn property_incremental_equals_full_over_random_batches() {
    // Deterministic xorshift so the sweep is reproducible.
    let mut state = 0x9E3779B97F4A7C15u64;
    let mut next = |bound: usize| -> usize {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        if bound == 0 {
            0
        } else {
            (state as usize) % bound
        }
    };

    for &n in &[0usize, 1, 3, 4, 5, 15, 16, 17, 63, 64, 200, 777] {
        let base = snapshot(1, n);
        let base_pack = compile_graph_pack(&base, opts(1));

        for round in 0..4u64 {
            let k = 1 + next(8);
            let mut mutations = Vec::with_capacity(k);
            for _ in 0..k {
                if n > 0 && next(2) == 0 {
                    let i = next(n);
                    mutations.push(GraphMutation::NodeUpsert(node(i, round + 1)));
                } else {
                    let i = n + next(500);
                    mutations.push(GraphMutation::NodeUpsert(node(i, 100 + round)));
                }
            }
            let batch = GraphMutationBatch::new(mutations);

            let incremental = compile_graph_pack_incremental(&base_pack, &batch, opts(2));
            let applied = apply_graph_mutation_batch(&base, &batch);
            let full = compile_graph_pack(&applied, opts(2));

            assert_eq!(
                incremental.pack.tree.root_hash, full.tree.root_hash,
                "incremental root diverged from full rebuild at n={n} round={round} k={k}"
            );
            assert_eq!(
                incremental.pack.tree.entries_total, full.tree.entries_total,
                "entry total diverged at n={n} round={round}"
            );
        }
    }
}

/// Acceptance #5: the full-rebuild reference path (snapshot/branch/diff/merge
/// foundation) is unchanged -- `build_prolly_tree` and `compile_graph_pack`
/// still produce a stable, self-consistent root over a fixed snapshot.
#[test]
fn full_rebuild_path_is_unchanged() {
    let snap = snapshot(1, 300);
    let objects = rustyred_thg_core::snapshot_content_objects(&snap, true);
    let tree_a = build_prolly_tree(&objects);
    let tree_b = build_prolly_tree(&objects);
    assert_eq!(
        tree_a.root_hash, tree_b.root_hash,
        "full build is deterministic"
    );

    let pack = compile_graph_pack(&snap, opts(1));
    assert_eq!(pack.tree.root_hash, tree_a.root_hash);
    assert_eq!(pack.tree.entries_total, 300);
}
