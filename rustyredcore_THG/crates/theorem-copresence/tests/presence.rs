use std::sync::{Arc, Mutex};

use rustyred_thg_core::{ActorId, InMemoryThgExecutor, WorkingLog};
use theorem_copresence::{CursorPos, PeerConfig, PeerEvent, Presence, PresenceKind, SubstratePeer};

#[test]
fn peer_observes_presence_from_working_log() {
    let tmp = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(WorkingLog::new()));
    let actor_a = ActorId::from_label("codex");
    let actor_b = ActorId::from_label("claude-code");
    let mut codex = SubstratePeer::try_new(
        InMemoryThgExecutor::new(),
        PeerConfig::new(actor_a, "note:presence")
            .with_text_client_id(1)
            .with_working_log(log.clone())
            .with_data_dir(tmp.path().join("codex")),
    )
    .unwrap();
    let claude = SubstratePeer::try_new(
        InMemoryThgExecutor::new(),
        PeerConfig::new(actor_b, "note:presence")
            .with_text_client_id(2)
            .with_working_log(log)
            .with_data_dir(tmp.path().join("claude")),
    )
    .unwrap();

    codex
        .announce(Presence {
            actor: actor_a,
            scope: "note:presence".to_string(),
            focus_region: Some("note:presence:section:intro:body".to_string()),
            cursor: Some(CursorPos::TextIndex {
                region_id: "note:presence:section:intro:body".to_string(),
                index: 7,
            }),
            label: "Codex".to_string(),
            kind: PresenceKind::Agent,
        })
        .unwrap();

    let observed = claude.observe(0).unwrap();
    let presence = observed
        .into_iter()
        .find_map(|event| match event {
            PeerEvent::Presence { presence, .. } => Some(presence),
            PeerEvent::WorkingLog { .. } => None,
        })
        .expect("presence event should be carried by the working log");

    assert_eq!(presence.actor, actor_a);
    assert_eq!(
        presence.focus_region.as_deref(),
        Some("note:presence:section:intro:body")
    );
    assert_eq!(presence.kind, PresenceKind::Agent);
}
