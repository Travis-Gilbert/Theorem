//! Emit a coordination-room scene page (graph_force projection) for visual
//! parity checking against theoremweb.com/coordination-room.
//! Usage: `cargo run -p scene-os-web --example render_coordination -- out.html`.
//!
//! Mirrors the reference's nodes (room center, Codex left, Claude right, store
//! bottom, presence top, three packets) and edges, with each atom's zone set
//! via metadata.group so the d3-force layout settles into the same
//! constellation. This is the scene the browser bundle was missing a layout for.

use std::collections::BTreeMap;
use std::env;
use std::fs;

use scene_os_core::{
    AtomLifecycle, ChromeBinding, ProjectionBinding, SceneAtom, SceneRelation, ScenePackageV2,
};
use scene_os_web::render_scene;
use serde_json::{json, Value};

fn atom(id: &str, kind: &str, label: &str, group: &str, weight: f64) -> SceneAtom {
    let mut metadata: BTreeMap<String, Value> = BTreeMap::new();
    metadata.insert("group".to_string(), json!(group));
    SceneAtom {
        id: id.to_string(),
        kind: kind.to_string(),
        label: Some(label.to_string()),
        position: None,
        weight: Some(weight),
        color: None,
        opacity: None,
        glyph: None,
        scale: None,
        lifecycle: AtomLifecycle::Present,
        metadata,
        source_refs: Vec::new(),
    }
}

fn rel(id: &str, source: &str, target: &str, label: &str, kind: &str, weight: f64) -> SceneRelation {
    SceneRelation {
        id: id.to_string(),
        source_id: source.to_string(),
        target_id: target.to_string(),
        kind: kind.to_string(),
        weight: Some(weight),
        color: None,
        opacity: None,
        glyph: None,
        lifecycle: AtomLifecycle::Present,
        metadata: BTreeMap::new(),
        source_refs: Vec::new(),
    }
}

fn main() {
    let out = env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/scene-coordination.html".to_string());

    let package = ScenePackageV2 {
        version: "scene-package-v2".to_string(),
        id: "coordination-1".to_string(),
        manifest_ref: "coordination".to_string(),
        atoms: vec![
            atom("room:index-api-main", "room", "Index-API main", "center", 1.0),
            atom("agent:codex", "agent", "Codex", "left", 0.7),
            atom("agent:claude-code", "agent", "Claude Code", "right", 0.7),
            atom("store:rustyred-thg", "store", "RustyRed-THG", "bottom", 0.75),
            atom("signal:presence", "signal", "Presence", "top", 0.45),
            atom("packet:lane-claim", "packet", "Lane claim", "center", 0.4),
            atom("packet:critique", "packet", "Critique", "center", 0.45),
            atom("packet:receipt", "packet", "Receipt", "center", 0.35),
        ],
        relations: vec![
            rel("room-codex", "room:index-api-main", "agent:codex", "joined", "membership", 1.8),
            rel("room-claude", "room:index-api-main", "agent:claude-code", "joined", "membership", 1.8),
            rel("room-rustyred", "room:index-api-main", "store:rustyred-thg", "MemoryAtom", "storage", 2.3),
            rel("room-presence", "room:index-api-main", "signal:presence", "heartbeat", "liveness", 1.4),
            rel("codex-lane", "agent:codex", "packet:lane-claim", "coordinate", "packet", 1.5),
            rel("lane-claude", "packet:lane-claim", "agent:claude-code", "mention", "receipt", 1.3),
            rel("claude-critique", "agent:claude-code", "packet:critique", "critique", "packet", 1.7),
            rel("critique-codex", "packet:critique", "agent:codex", "applied", "receipt", 1.5),
            rel("codex-receipt", "agent:codex", "packet:receipt", "receipt", "packet", 1.2),
            rel("receipt-store", "packet:receipt", "store:rustyred-thg", "queued", "storage", 1.1),
        ],
        projection: ProjectionBinding {
            id: "graph_force".to_string(),
            params: BTreeMap::new(),
        },
        chrome: ChromeBinding {
            id: "document_rail".to_string(),
            params: BTreeMap::new(),
        },
        actions: Vec::new(),
        transitions: None,
        terminal_state: None,
        provenance: BTreeMap::new(),
    };

    let html = render_scene(&package).expect("render scene");
    fs::write(&out, html).expect("write html");
    println!("wrote {out}");
}
