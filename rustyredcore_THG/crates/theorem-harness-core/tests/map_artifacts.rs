use serde_json::{json, Map, Value};
use theorem_harness_core::{
    compile_map_artifact, describe_map_artifact, scope_for_map_kind, stable_map_id,
    MapArtifactCompileInput, MapDeltaState,
};

#[test]
fn map_scope_and_id_are_stable() {
    assert_eq!(
        stable_map_id("CodebaseMap", "repo", "Theorem"),
        stable_map_id("CodebaseMap", "repo", "Theorem")
    );
    assert_ne!(
        stable_map_id("CodebaseMap", "repo", "Theorem"),
        stable_map_id("RuleMap", "repo", "Theorem")
    );

    let mut domain = Map::new();
    domain.insert("domain".to_string(), json!("civic-atlas"));
    assert_eq!(
        scope_for_map_kind("DomainMap", Some("Theorem"), Some(&domain)),
        ("domain".to_string(), "civic-atlas".to_string())
    );
    assert_eq!(
        scope_for_map_kind("CodebaseMap", Some("Theorem"), Some(&domain)),
        ("repo".to_string(), "Theorem".to_string())
    );
}

#[test]
fn compile_codebase_map_builds_orientation_artifact() {
    let mut preview = Map::new();
    preview.insert(
        "read_first".to_string(),
        json!([
            "AGENTS.md",
            "docs/plans/harness-rust-port/implementation-plan.md"
        ]),
    );

    let artifact = compile_map_artifact(MapArtifactCompileInput {
        map_kind: "CodebaseMap".to_string(),
        scope_kind: "repo".to_string(),
        scope_ref: "Theorem".to_string(),
        task: "port harness".to_string(),
        repo: "Theorem".to_string(),
        target: "theorem-harness-core".to_string(),
        validators: vec!["cargo test -p theorem-harness-core".to_string()],
        memory_recall_preview: preview,
        pending_delta_count: 2,
        ..MapArtifactCompileInput::default()
    });

    assert_eq!(artifact.map_kind, "CodebaseMap");
    assert_eq!(artifact.entries.len(), 5);
    assert_eq!(artifact.pending_delta_count, 2);
    assert_eq!(artifact.applied_delta_count, 0);
    assert!(artifact.state_hash.starts_with("sha256:"));
    assert!(artifact.markdown_body.contains("# CodebaseMap for Theorem"));
    assert_eq!(
        artifact.descriptor["hydration_handle"],
        Value::String(format!("map_artifact:{}", artifact.map_id))
    );

    let summary = describe_map_artifact(&artifact);
    assert_eq!(summary["entry_count"], 5);
    assert_eq!(summary["export_formats"], json!(["json", "markdown"]));
}

#[test]
fn applied_deltas_merge_and_remove_entries() {
    let upsert = MapDeltaState {
        delta_id: "delta-1".to_string(),
        map_kind: "CodebaseMap".to_string(),
        scope_kind: "repo".to_string(),
        scope_ref: "Theorem".to_string(),
        status: "applied".to_string(),
        entry_id: "custom".to_string(),
        summary: "Add custom orientation".to_string(),
        entry: Map::from_iter([
            ("kind".to_string(), json!("delta")),
            ("title".to_string(), json!("Custom")),
            ("summary".to_string(), json!("A delta supplied entry.")),
        ]),
        ..delta_defaults("delta-1")
    };
    let remove = MapDeltaState {
        delta_id: "delta-2".to_string(),
        map_kind: "CodebaseMap".to_string(),
        scope_kind: "repo".to_string(),
        scope_ref: "Theorem".to_string(),
        status: "applied".to_string(),
        action: "remove".to_string(),
        entry_id: "repo-boundary".to_string(),
        ..delta_defaults("delta-2")
    };
    let ignored = MapDeltaState {
        delta_id: "delta-3".to_string(),
        map_kind: "CodebaseMap".to_string(),
        scope_kind: "repo".to_string(),
        scope_ref: "Theorem".to_string(),
        status: "proposed".to_string(),
        entry_id: "ignored".to_string(),
        ..delta_defaults("delta-3")
    };

    let artifact = compile_map_artifact(MapArtifactCompileInput {
        map_kind: "CodebaseMap".to_string(),
        scope_kind: "repo".to_string(),
        scope_ref: "Theorem".to_string(),
        repo: "Theorem".to_string(),
        applied_deltas: vec![upsert, remove, ignored],
        ..MapArtifactCompileInput::default()
    });

    let entry_ids = artifact
        .entries
        .iter()
        .map(|entry| entry["entry_id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(entry_ids, vec!["custom"]);
    assert_eq!(artifact.source_delta_ids, vec!["delta-1", "delta-2"]);
    assert_eq!(artifact.applied_delta_count, 2);
    assert_eq!(artifact.entries[0]["metadata"]["delta_id"], "delta-1");
}

#[test]
fn tool_map_entries_preserve_tool_metadata() {
    let artifact = compile_map_artifact(MapArtifactCompileInput {
        map_kind: "ToolMap".to_string(),
        scope_kind: "repo".to_string(),
        scope_ref: "Theorem".to_string(),
        selected_tools: vec![Map::from_iter([
            ("tool_id".to_string(), json!("context_artifact_compile")),
            ("name".to_string(), json!("Context Artifact Compile")),
            (
                "reason".to_string(),
                json!("Compile a reusable context artifact."),
            ),
            ("permissions".to_string(), json!(["graph_read"])),
            ("inputs".to_string(), json!(["context_pack"])),
            ("outputs".to_string(), json!(["artifact"])),
            ("cost".to_string(), json!("low")),
        ])],
        ..MapArtifactCompileInput::default()
    });

    assert_eq!(artifact.entries.len(), 1);
    assert_eq!(artifact.entries[0]["entry_id"], "context_artifact_compile");
    assert_eq!(artifact.entries[0]["kind"], "tool");
    assert_eq!(
        artifact.entries[0]["metadata"]["permissions"],
        json!(["graph_read"])
    );
}

fn delta_defaults(delta_id: &str) -> MapDeltaState {
    MapDeltaState {
        delta_id: delta_id.to_string(),
        map_kind: "CodebaseMap".to_string(),
        scope_kind: "repo".to_string(),
        scope_ref: "Theorem".to_string(),
        workstream_id: String::new(),
        target_map_id: String::new(),
        status: "applied".to_string(),
        action: "upsert".to_string(),
        summary: String::new(),
        rationale: String::new(),
        entry_id: String::new(),
        entry: Map::new(),
        proposed_by: String::new(),
        source_run_id: String::new(),
        source_event_ids: Vec::new(),
        created_at: "created".to_string(),
        applied_at: "applied".to_string(),
    }
}
