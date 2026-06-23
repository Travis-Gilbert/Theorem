use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use rustyred_thg_core::{
    read_manifest, GraphSnapshot, InMemoryThgExecutor, NeighborQuery, NodeQuery, RedCoreDurability,
    RedCoreGraphStore, RedCoreOptions, ThgCommand, ThgExecutor,
};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct SnapshotEnvelope {
    version: u32,
    txn_id: u64,
    graph: GraphSnapshot,
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", process::id()))
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let target_path = dst.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_recursive(&source_path, &target_path);
        } else {
            fs::copy(&source_path, &target_path).unwrap();
        }
    }
}

fn copied_redcore_fixture() -> PathBuf {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/redcore-v1");
    let dst = unique_temp_dir("redcore-snapshot-replay");
    copy_dir_recursive(&src, &dst);
    dst
}

fn thg_state_hash(snapshot: &GraphSnapshot) -> String {
    let mut executor = InMemoryThgExecutor::new();
    let response = executor
        .execute(
            ThgCommand::StateHash,
            json!({"state": serde_json::to_value(snapshot).unwrap()}),
        )
        .unwrap();
    response
        .payload
        .get("hash")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

#[test]
fn redcore_fixture_replay_matches_snapshot_hash_stats_and_query_smoke() {
    let data_dir = copied_redcore_fixture();
    let manifest = read_manifest(&data_dir).unwrap().unwrap();
    assert_eq!(manifest.format_kind, "redcore");
    assert_eq!(manifest.snapshot_txn_id, 3);

    let raw_snapshot = fs::read_to_string(data_dir.join("graph.snapshot.current")).unwrap();
    let expected: SnapshotEnvelope = serde_json::from_str(&raw_snapshot).unwrap();
    assert_eq!(expected.version, 1);

    let expected_hash = thg_state_hash(&expected.graph);

    let store = RedCoreGraphStore::open(
        &data_dir,
        RedCoreOptions {
            durability: RedCoreDurability::None,
            snapshot_interval_writes: 0,
            strict_acid: false,
        },
    )
    .unwrap();

    let replayed = store.graph_snapshot();
    assert_eq!(replayed, expected.graph);
    assert_eq!(thg_state_hash(&replayed), expected_hash);

    let status = store.status();
    assert_eq!(status.snapshot_txn_id, expected.txn_id);
    assert!(status.last_recovery_ok);

    let stats = store.stats().unwrap();
    assert_eq!(stats.version, 3);
    assert_eq!(stats.nodes_total, 2);
    assert_eq!(stats.edges_total, 1);
    assert_eq!(stats.labels_total, 1);
    assert_eq!(stats.edge_types_total, 1);

    let docs = store.query_nodes(NodeQuery::label("Doc")).unwrap();
    assert_eq!(docs.len(), 2);

    let cites = store
        .neighbors(NeighborQuery::out("doc:a").with_edge_type("CITES"))
        .unwrap();
    assert_eq!(cites.len(), 1);
    assert_eq!(cites[0].node_id, "doc:b");
    assert_eq!(cites[0].confidence, Some(0.9));

    let verify = store.verify().unwrap();
    assert!(verify.ok);
    assert_eq!(verify.stats, stats);

    drop(store);
    fs::remove_dir_all(data_dir).ok();
}
