use std::sync::{Arc, Mutex};

use rustyred_thg_core::{ActorId, InMemoryThgExecutor, VersionVector, WorkingLog};
use theorem_copresence::{
    NoteAdapter, NoteIntent, PeerConfig, SubstratePeer, SurfaceAdapter, SurfaceIntent,
    SurfaceSnapshot,
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

#[test]
fn two_peers_converge_on_structure_and_same_text_region() {
    let tmp = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(WorkingLog::new()));
    let mut codex = peer(
        "codex",
        "note:shared",
        1,
        log.clone(),
        &tmp.path().join("codex"),
    );
    let mut claude = peer(
        "claude-code",
        "note:shared",
        2,
        log,
        &tmp.path().join("claude"),
    );
    let mut codex_note = NoteAdapter::new("note:shared");
    let mut claude_note = NoteAdapter::new("note:shared");

    codex_note
        .to_peer(
            &mut codex,
            SurfaceIntent::Note {
                intent: NoteIntent::AddSection {
                    section_id: "intro".to_string(),
                },
            },
        )
        .unwrap();
    let setup = codex.delta_since(&VersionVector::default());
    claude.merge_delta(setup).unwrap();
    claude
        .text_region(&claude_note.body_region("intro"))
        .unwrap();

    codex_note
        .to_peer(
            &mut codex,
            SurfaceIntent::Note {
                intent: NoteIntent::SetTitle {
                    title: "Codex draft".to_string(),
                },
            },
        )
        .unwrap();
    claude_note
        .to_peer(
            &mut claude,
            SurfaceIntent::Note {
                intent: NoteIntent::SetStatus {
                    status: "reviewing".to_string(),
                },
            },
        )
        .unwrap();
    codex_note
        .to_peer(
            &mut codex,
            SurfaceIntent::Note {
                intent: NoteIntent::InsertSectionText {
                    section_id: "intro".to_string(),
                    index: 0,
                    text: "Codex ".to_string(),
                },
            },
        )
        .unwrap();
    claude_note
        .to_peer(
            &mut claude,
            SurfaceIntent::Note {
                intent: NoteIntent::InsertSectionText {
                    section_id: "intro".to_string(),
                    index: 0,
                    text: "Claude ".to_string(),
                },
            },
        )
        .unwrap();

    let codex_struct_delta = codex.delta_since(claude.seen());
    let claude_struct_delta = claude.delta_since(codex.seen());
    codex.merge_delta(claude_struct_delta).unwrap();
    claude.merge_delta(codex_struct_delta).unwrap();

    let region_id = codex_note.body_region("intro");
    let codex_text_vector = codex.text_state_vector(&region_id).unwrap();
    let claude_text_vector = claude.text_state_vector(&region_id).unwrap();
    let codex_text_update = codex
        .text_update_since(&region_id, &claude_text_vector)
        .unwrap();
    let claude_text_update = claude
        .text_update_since(&region_id, &codex_text_vector)
        .unwrap();
    codex
        .apply_text_update(&region_id, &claude_text_update)
        .unwrap();
    claude
        .apply_text_update(&region_id, &codex_text_update)
        .unwrap();

    let codex_snapshot = match codex_note.from_peer(&codex).unwrap() {
        SurfaceSnapshot::Note { snapshot } => snapshot,
        other => panic!("unexpected snapshot: {other:?}"),
    };
    let claude_snapshot = match claude_note.from_peer(&claude).unwrap() {
        SurfaceSnapshot::Note { snapshot } => snapshot,
        other => panic!("unexpected snapshot: {other:?}"),
    };

    assert_eq!(
        codex_snapshot, claude_snapshot,
        "structure CRDT deltas and yrs text updates must converge to identical note snapshots"
    );
    assert_eq!(codex_snapshot.title.as_deref(), Some("Codex draft"));
    assert_eq!(codex_snapshot.status.as_deref(), Some("reviewing"));
    let body = &codex_snapshot.sections[0].body;
    assert!(body.contains("Codex "));
    assert!(body.contains("Claude "));
    assert!(!codex
        .persisted_text_update(&region_id)
        .unwrap()
        .unwrap()
        .is_empty());
    assert!(!claude
        .persisted_text_update(&region_id)
        .unwrap()
        .unwrap()
        .is_empty());
}
