use std::sync::{Arc, Mutex};

use rustyred_thg_core::{ActorId, InMemoryThgExecutor, WorkingLog};
use serde_json::json;
use theorem_copresence::{
    CodeSurfaceAdapter, CursorPos, PeerConfig, PeerEvent, Presence, PresenceKind, SharedWorkingLog,
    SubstratePeer, SurfaceAdapter, SurfaceIntent,
};

fn peer(
    actor: ActorId,
    scope: &str,
    log: SharedWorkingLog,
    dir: std::path::PathBuf,
    client: u64,
) -> SubstratePeer {
    SubstratePeer::try_new(
        InMemoryThgExecutor::new(),
        PeerConfig::new(actor, scope)
            .with_text_client_id(client)
            .with_working_log(log)
            .with_data_dir(dir),
    )
    .unwrap()
}

// W5 acceptance: two peers on one shared working tree see each other's presence
// on a code file at file:line:col, and the code adapter refuses to move file
// content through a CRDT (code is versioned by git, not character-merged).
#[test]
fn code_presence_converges_without_merging_code() {
    let tmp = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(WorkingLog::new()));
    let scope = "repo:demo";
    let actor_a = ActorId::from_label("codex");
    let actor_b = ActorId::from_label("claude-code");
    let mut a = peer(actor_a, scope, log.clone(), tmp.path().join("a"), 1);
    let mut b = peer(actor_b, scope, log.clone(), tmp.path().join("b"), 2);

    // Peer A is editing src/main.rs at line 10, col 4: announce it through the
    // code surface adapter.
    let mut adapter_a = CodeSurfaceAdapter::new("src/main.rs");
    let presence =
        Presence::at_code(actor_a, scope, "Codex", "src/main.rs", 10, 4, PresenceKind::Agent);
    adapter_a
        .to_peer(&mut a, SurfaceIntent::Presence { presence })
        .expect("announce code presence");

    // Peer B observes A's presence over the shared working log: awareness
    // converges at file:line:col.
    let observed = b.observe(0).expect("observe");
    let seen = observed
        .into_iter()
        .find_map(|event| match event {
            PeerEvent::Presence { presence, .. } => Some(presence),
            PeerEvent::WorkingLog { .. } => None,
        })
        .expect("code presence event carried by the working log");
    assert_eq!(seen.actor, actor_a);
    assert_eq!(
        seen.cursor,
        Some(CursorPos::FilePosition {
            path: "src/main.rs".to_string(),
            line: 10,
            col: 4,
        }),
        "presence addresses file:line:col"
    );
    assert_eq!(seen.focus_region.as_deref(), Some("src/main.rs"));

    // The boundary, executable: the code adapter refuses to CRDT-merge file
    // content. A text-insert intent is an error; bytes flow through git (W2).
    let mut adapter_b = CodeSurfaceAdapter::new("src/main.rs");
    let rejected = adapter_b.to_peer(
        &mut b,
        SurfaceIntent::TextInsert {
            region_id: "src/main.rs".to_string(),
            index: 0,
            text: "a character-CRDT edit to code".to_string(),
        },
    );
    assert!(
        rejected.is_err(),
        "code content must not be CRDT-merged through copresence"
    );
}

// W5.2: a file's STRUCTURAL footprint (a marker on the File node) converges
// peer-to-peer through the graph CRDT, while the file's CONTENT still never
// flows (no text region). Structure syncs; code stays in git.
#[test]
fn code_file_footprint_structure_converges_content_does_not() {
    let tmp = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(WorkingLog::new()));
    let scope = "repo:demo";
    let actor_a = ActorId::from_label("codex");
    let actor_b = ActorId::from_label("claude-code");
    let mut a = peer(actor_a, scope, log.clone(), tmp.path().join("sa"), 3);
    let mut b = peer(actor_b, scope, log.clone(), tmp.path().join("sb"), 4);

    // Peer A records an edit footprint on the File NODE (structure, not content).
    let mut adapter_a = CodeSurfaceAdapter::new("src/main.rs");
    let op = adapter_a.footprint_op("open_by", json!("codex"));
    adapter_a
        .to_peer(&mut a, SurfaceIntent::Structured { op })
        .expect("apply footprint");

    // Sync the structural delta A -> B over the graph CRDT (not the working log).
    let batch = a.delta_since(b.seen());
    b.merge_delta(batch).expect("merge structural delta");

    // Structure converged: B now sees the File node and the footprint property.
    let node = b
        .graph_node(&adapter_a.file_node_id())
        .expect("file node converged to peer B");
    assert_eq!(
        node.properties.get("open_by").and_then(|v| v.as_str()),
        Some("codex"),
        "the footprint marker synced through the graph CRDT"
    );
    assert!(
        node.labels.iter().any(|l| l == "File"),
        "the synced node carries the File label"
    );

    // Content did NOT flow: there is no text region carrying the file's bytes on
    // either peer. Code is versioned by git, never CRDT-merged as text.
    assert!(
        b.text_region_contents("src/main.rs").is_none(),
        "code content must not sync as a yrs text region"
    );
    assert!(
        a.text_region_contents("src/main.rs").is_none(),
        "the source peer never opened a text region for code either"
    );
}

// W5 acceptance #3: presence ordering is deterministic. Sequential announces
// (the working log serializes appends under its mutex) get strictly increasing
// cursors and are observed in that order, so the newest position wins.
#[test]
fn code_presence_ordering_is_deterministic_latest_wins() {
    let tmp = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(WorkingLog::new()));
    let scope = "repo:demo";
    let actor_a = ActorId::from_label("codex");
    let actor_b = ActorId::from_label("claude-code");
    let mut a = peer(actor_a, scope, log.clone(), tmp.path().join("oa"), 5);
    let mut b = peer(actor_b, scope, log.clone(), tmp.path().join("ob"), 6);

    let mut adapter = CodeSurfaceAdapter::new("src/main.rs");
    // Peer A moves its cursor down the file: line 10, then line 20.
    for line in [10u32, 20u32] {
        let presence =
            Presence::at_code(actor_a, scope, "Codex", "src/main.rs", line, 0, PresenceKind::Agent);
        adapter
            .to_peer(&mut a, SurfaceIntent::Presence { presence })
            .expect("announce");
    }

    let events: Vec<(u64, CursorPos)> = b
        .observe(0)
        .expect("observe")
        .into_iter()
        .filter_map(|event| match event {
            PeerEvent::Presence { cursor, presence } => presence.cursor.map(|c| (cursor, c)),
            PeerEvent::WorkingLog { .. } => None,
        })
        .collect();

    assert!(events.len() >= 2, "both announces observed: {events:?}");
    for window in events.windows(2) {
        assert!(
            window[0].0 < window[1].0,
            "cursors strictly increase (deterministic order): {events:?}"
        );
    }
    assert_eq!(
        events.last().unwrap().1,
        CursorPos::FilePosition {
            path: "src/main.rs".to_string(),
            line: 20,
            col: 0,
        },
        "the latest announced position wins"
    );
}
