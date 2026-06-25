use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};

use rustyred_thg_core::{ActorId, InMemoryThgExecutor, WorkingLog};
use theorem_copresence::{
    PeerConfig, ScratchpadLiveEvent, ScratchpadSession, SharedWorkingLog, SubstratePeer,
};
use theorem_harness_core::{
    ScratchpadCrdtBacking, ScratchpadDocument, ScratchpadRelationKind, ScratchpadRevision,
    ScratchpadRevisionLink, ScratchpadRevisionRelation,
};

fn session(
    actor: &str,
    crdt: ScratchpadCrdtBacking,
    client_id: u64,
    log: SharedWorkingLog,
    data_dir: &std::path::Path,
) -> ScratchpadSession {
    let peer = SubstratePeer::try_new(
        InMemoryThgExecutor::new(),
        PeerConfig::new(ActorId::from_label(actor), crdt.stream_topic.clone())
            .with_text_client_id(client_id)
            .with_working_log(log)
            .with_data_dir(data_dir),
    )
    .unwrap();
    ScratchpadSession::new(peer, crdt)
}

#[test]
fn scratchpad_live_tail_converges_multiwriter_text_graph_and_awareness() {
    let tmp = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(WorkingLog::new()));
    let crdt = ScratchpadCrdtBacking::for_document("scratchpad:theorem");
    let mut codex = session(
        "flash",
        crdt.clone(),
        1,
        log.clone(),
        &tmp.path().join("flash"),
    );
    let mut claude = session("claude", crdt.clone(), 2, log, &tmp.path().join("claude"));
    let mut scratchpad = ScratchpadDocument::new("scratchpad:theorem");

    let proposal = scratchpad.append(
        "flash",
        "fast first proposal",
        "hash:proposal",
        object_payload(json!({
            "kind": "proposal",
            "text": "Flash draft in the shared CRDT document."
        })),
        "2026-06-25T20:00:00Z",
    );
    codex
        .publish_revision(
            &proposal,
            &relations_for_revision(&scratchpad, &proposal),
            scratchpad.awareness.last().cloned(),
        )
        .unwrap();

    let critique = scratchpad.append_with_links(
        "claude",
        "critique against proposal",
        "hash:critique",
        object_payload(json!({
            "kind": "critique",
            "text": "Claude critique marks the missing verification edge."
        })),
        vec![proposal.revision_id.clone()],
        vec![ScratchpadRevisionLink::new(
            proposal.revision_id.clone(),
            ScratchpadRelationKind::Annotates,
            "critique annotates proposal",
            Map::new(),
        )],
        "2026-06-25T20:00:01Z",
    );
    claude
        .publish_revision(
            &critique,
            &relations_for_revision(&scratchpad, &critique),
            scratchpad.awareness.last().cloned(),
        )
        .unwrap();

    let codex_delta = codex.subscribe_after(0, 0).unwrap();
    let claude_delta = claude.subscribe_after(0, 0).unwrap();

    assert!(codex_delta.events.iter().any(|event| matches!(
        event,
        ScratchpadLiveEvent::TextUpdate { region_id, .. } if region_id == "critique"
    )));
    assert!(claude_delta.events.iter().any(|event| matches!(
        event,
        ScratchpadLiveEvent::TextUpdate { region_id, .. } if region_id == "proposal"
    )));
    assert!(codex_delta
        .events
        .iter()
        .any(|event| matches!(event, ScratchpadLiveEvent::Awareness { .. })));

    for session in [&codex, &claude] {
        assert!(session.peer().graph_node(&proposal.revision_id).is_some());
        assert!(session.peer().graph_node(&critique.revision_id).is_some());
        assert!(session
            .text_region_contents("proposal")
            .unwrap()
            .contains("Flash draft"));
        assert!(session
            .text_region_contents("critique")
            .unwrap()
            .contains("Claude critique"));
        let graph = session.peer().graph_snapshot().unwrap();
        assert!(graph.edges.iter().any(|edge| {
            edge.from_id == critique.revision_id
                && edge.to_id == proposal.revision_id
                && edge.edge_type == ScratchpadRelationKind::Annotates.edge_type()
        }));
    }

    let awareness = codex.awareness_snapshot().unwrap();
    assert!(awareness
        .iter()
        .any(|entry| entry.actor_head_id == "flash" && entry.region_id == "proposal"));
    assert!(awareness
        .iter()
        .any(|entry| entry.actor_head_id == "claude" && entry.region_id == "critique"));
}

fn relations_for_revision(
    scratchpad: &ScratchpadDocument,
    revision: &ScratchpadRevision,
) -> Vec<ScratchpadRevisionRelation> {
    scratchpad
        .relations
        .iter()
        .filter(|relation| relation.from_revision_id == revision.revision_id)
        .cloned()
        .collect()
}

fn object_payload(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}
