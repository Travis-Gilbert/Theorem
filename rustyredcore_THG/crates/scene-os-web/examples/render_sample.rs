//! Emit a sample scene page to a file, for visual verification and as a Lane C
//! reference. Usage: `cargo run --example render_sample -- /tmp/scene.html`.
//!
//! Builds a small support tree (claim <- evidence <- source, plus a concept and
//! a tension), runs it through `render_scene` (the same call Lane C makes), and
//! writes the self-contained HTML the browser would serve.

use std::collections::BTreeMap;
use std::env;
use std::fs;

use scene_os_core::{
    AtomLifecycle, ChromeBinding, ProjectionBinding, SceneAtom, SceneRelation, ScenePackageV2,
};
use scene_os_web::render_scene;

fn atom(id: &str, kind: &str, label: &str, weight: Option<f64>) -> SceneAtom {
    SceneAtom {
        id: id.to_string(),
        kind: kind.to_string(),
        label: Some(label.to_string()),
        position: None,
        weight,
        color: None,
        opacity: None,
        glyph: None,
        scale: None,
        lifecycle: AtomLifecycle::Present,
        metadata: BTreeMap::new(),
        source_refs: Vec::new(),
    }
}

fn relation(id: &str, source: &str, target: &str) -> SceneRelation {
    SceneRelation {
        id: id.to_string(),
        source_id: source.to_string(),
        target_id: target.to_string(),
        kind: "supports".to_string(),
        weight: None,
        color: None,
        opacity: None,
        glyph: None,
        lifecycle: AtomLifecycle::Present,
        metadata: BTreeMap::new(),
        source_refs: Vec::new(),
    }
}

fn main() {
    let out = env::args().nth(1).unwrap_or_else(|| "/tmp/scene-sample.html".to_string());

    let package = ScenePackageV2 {
        version: "scene-package-v2".to_string(),
        id: "sample-1".to_string(),
        manifest_ref: "sample".to_string(),
        atoms: vec![
            atom("claim", "claim", "Cities with protected bike lanes see fewer injuries", Some(3.0)),
            atom("ev1", "evidence", "Seville network study, 2013", Some(2.0)),
            atom("ev2", "evidence", "Copenhagen injury rate decline", Some(2.0)),
            atom("src1", "source", "Transport journal A", Some(1.0)),
            atom("src2", "source", "Municipal dataset", Some(1.0)),
            atom("src3", "source", "Cohort survey", Some(1.0)),
            atom("concept", "concept", "Protected infrastructure", Some(1.5)),
            atom("person", "person", "Lead researcher", Some(1.0)),
        ],
        relations: vec![
            relation("ev1-claim", "ev1", "claim"),
            relation("ev2-claim", "ev2", "claim"),
            relation("src1-ev1", "src1", "ev1"),
            relation("src2-ev1", "src2", "ev1"),
            relation("src3-ev2", "src3", "ev2"),
            relation("concept-claim", "concept", "claim"),
            relation("person-ev1", "person", "ev1"),
        ],
        projection: ProjectionBinding {
            id: "tree_hierarchy".to_string(),
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
