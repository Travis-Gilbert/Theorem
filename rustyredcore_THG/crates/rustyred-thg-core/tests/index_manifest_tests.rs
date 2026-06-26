use rustyred_thg_core::{
    IndexBackend, IndexBuildStatus, IndexCreatedBy, IndexKind, IndexManifest, IndexRegistry,
    IndexScope,
};

#[test]
fn manifest_lifecycle_updates_hash_and_retirement_reason() {
    let mut manifest = IndexManifest::new(
        "idx:artifact-card",
        "Artifact card",
        IndexKind::Covering,
        IndexBackend::RustyredCore,
        IndexScope::Project,
        "ContextArtifact",
        IndexCreatedBy::Manual,
    )
    .with_target_properties(["scope", "artifact_type", "updated_at"])
    .with_covering_fields(["id", "title", "status", "outcome"]);

    manifest.validate().unwrap();
    let proposed_hash = manifest.state_hash.clone();

    manifest.activate(7);
    assert_eq!(manifest.build_status, IndexBuildStatus::Active);
    assert_eq!(manifest.graph_version, 7);
    assert_ne!(manifest.state_hash, proposed_hash);

    manifest.record_hit(12.0, 30.0);
    assert_eq!(manifest.hit_count, 1);
    assert_eq!(manifest.avg_latency_saved_ms, 30.0);

    manifest.retire("covered by wider project artifact index");
    assert_eq!(manifest.build_status, IndexBuildStatus::Retired);
    assert_eq!(
        manifest.retirement_reason.as_deref(),
        Some("covered by wider project artifact index")
    );
}

#[test]
fn registry_registers_lists_and_retires_manifests() {
    let manifest = IndexManifest::new(
        "idx:run-list",
        "Run list",
        IndexKind::Composite,
        IndexBackend::RustyredCore,
        IndexScope::Tenant,
        "HarnessRun",
        IndexCreatedBy::Migration,
    )
    .with_target_properties(["scope", "run_status", "updated_at"]);
    let mut registry = IndexRegistry::new();

    registry.register_manifest(manifest.clone()).unwrap();
    assert_eq!(registry.list_manifests().len(), 1);
    assert_eq!(
        registry.get_manifest("idx:run-list").unwrap().name,
        "Run list"
    );

    let duplicate = registry.register_manifest(manifest);
    assert_eq!(duplicate.unwrap_err().code, "index_manifest_exists");

    registry
        .retire_manifest("idx:run-list", "replaced by covering run-card index")
        .unwrap();
    let retired = registry.get_manifest("idx:run-list").unwrap();
    assert_eq!(retired.build_status, IndexBuildStatus::Retired);
}
