use std::sync::{Arc, Mutex};

use rustyred_thg_core::{ActorId, InMemoryThgExecutor, VersionVector, WorkingLog};
use theorem_copresence::{
    CodeContentStrategy, CodeIntent, CodeSurfaceAdapter, CursorPos, FileRange, PeerConfig,
    PresenceKind, SubstratePeer, SurfaceAdapter, SurfaceIntent, SurfaceSnapshot,
};

fn peer(
    actor: &str,
    scope: &str,
    client_id: u64,
    log: Arc<Mutex<WorkingLog>>,
    dir: &std::path::Path,
) -> SubstratePeer {
    SubstratePeer::try_new(
        InMemoryThgExecutor::new(),
        PeerConfig::new(ActorId::from_label(actor), scope)
            .with_text_client_id(client_id)
            .with_working_log(log)
            .with_data_dir(dir),
    )
    .unwrap()
}

fn code_snapshot(adapter: &mut CodeSurfaceAdapter, peer: &SubstratePeer) -> theorem_copresence::CodeSnapshot {
    match adapter.from_peer(peer).unwrap() {
        SurfaceSnapshot::Code { snapshot } => snapshot,
        other => panic!("unexpected snapshot: {other:?}"),
    }
}

#[test]
fn two_peers_see_file_presence_and_edit_footprints_without_text_merge() {
    let tmp = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(WorkingLog::new()));
    let mut codex = peer(
        "codex",
        "code:shared",
        1,
        log.clone(),
        &tmp.path().join("codex"),
    );
    let mut claude = peer(
        "claude-code",
        "code:shared",
        2,
        log,
        &tmp.path().join("claude"),
    );
    let mut codex_code = CodeSurfaceAdapter::new("src/lib.rs");
    let mut claude_code = CodeSurfaceAdapter::new("src/lib.rs");

    codex_code
        .to_peer(
            &mut codex,
            SurfaceIntent::Code {
                intent: CodeIntent::AnnouncePresence {
                    line: 12,
                    col: 5,
                    label: "Codex".to_string(),
                    kind: PresenceKind::Agent,
                    pending_edit: Some(FileRange::new(12, 5, 14, 1)),
                },
            },
        )
        .unwrap();
    claude_code
        .to_peer(
            &mut claude,
            SurfaceIntent::Code {
                intent: CodeIntent::AnnouncePresence {
                    line: 20,
                    col: 9,
                    label: "Claude".to_string(),
                    kind: PresenceKind::Agent,
                    pending_edit: Some(FileRange::new(20, 9, 22, 1)),
                },
            },
        )
        .unwrap();

    let codex_delta = codex.delta_since(&VersionVector::default());
    let claude_delta = claude.delta_since(&VersionVector::default());
    codex.merge_delta(claude_delta).unwrap();
    claude.merge_delta(codex_delta).unwrap();

    let snapshot = code_snapshot(&mut claude_code, &claude);
    assert_eq!(snapshot.path, "src/lib.rs");
    assert_eq!(snapshot.content_strategy, CodeContentStrategy::GitMergeOnly);
    assert_eq!(snapshot.presences.len(), 2);
    assert_eq!(
        snapshot
            .presences
            .iter()
            .map(|presence| (presence.line, presence.col, presence.label.as_str()))
            .collect::<Vec<_>>(),
        vec![(12, 5, "Codex"), (20, 9, "Claude")]
    );
    assert_eq!(snapshot.edit_footprints.len(), 2);
    assert!(snapshot
        .edit_footprints
        .iter()
        .all(|footprint| footprint.path == "src/lib.rs"));

    let rejected = claude_code
        .to_peer(
            &mut claude,
            SurfaceIntent::TextPush {
                region_id: "file:src/lib.rs".to_string(),
                text: "pub fn silently_merged() {}\n".to_string(),
            },
        )
        .unwrap_err()
        .to_string();
    assert!(
        rejected.contains("does not merge file bytes"),
        "code adapter must reject text-region code writes: {rejected}"
    );
    assert_eq!(
        claude.text_region_contents("file:src/lib.rs"),
        None,
        "code bytes are not stored in a Yrs text region by the code adapter"
    );
}

#[test]
fn code_presence_ordering_is_working_log_cursor_order() {
    let tmp = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(WorkingLog::new()));
    let mut codex = peer(
        "codex",
        "code:ordering",
        1,
        log.clone(),
        &tmp.path().join("codex"),
    );
    let mut claude = peer(
        "claude-code",
        "code:ordering",
        2,
        log,
        &tmp.path().join("claude"),
    );
    let mut codex_code = CodeSurfaceAdapter::new("src/main.rs");
    let mut claude_code = CodeSurfaceAdapter::new("src/main.rs");

    claude_code
        .to_peer(
            &mut claude,
            SurfaceIntent::Code {
                intent: CodeIntent::AnnouncePresence {
                    line: 4,
                    col: 1,
                    label: "Claude first".to_string(),
                    kind: PresenceKind::Agent,
                    pending_edit: None,
                },
            },
        )
        .unwrap();
    codex_code
        .to_peer(
            &mut codex,
            SurfaceIntent::Code {
                intent: CodeIntent::AnnouncePresence {
                    line: 2,
                    col: 1,
                    label: "Codex second".to_string(),
                    kind: PresenceKind::Agent,
                    pending_edit: None,
                },
            },
        )
        .unwrap();

    let snapshot = code_snapshot(&mut codex_code, &codex);
    assert_eq!(
        snapshot
            .presences
            .iter()
            .map(|presence| presence.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Claude first", "Codex second"],
        "code presence is ordered by append cursor, which is the scoped ordering guarantee"
    );
    assert!(snapshot
        .presences
        .windows(2)
        .all(|window| window[0].cursor < window[1].cursor));
}

#[test]
fn file_position_cursor_serializes_as_file_not_text_region() {
    let value = serde_json::to_value(CursorPos::FilePosition {
        path: "src/lib.rs".to_string(),
        line: 7,
        col: 3,
    })
    .unwrap();

    assert_eq!(
        value,
        serde_json::json!({
            "kind": "file_position",
            "path": "src/lib.rs",
            "line": 7,
            "col": 3,
        })
    );
}
