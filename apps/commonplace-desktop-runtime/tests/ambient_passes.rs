//! Slice 3 acceptance: on a settled change set, the [`AmbientRuntime`] ingests
//! (slice 2) and then runs the registered ambient passes, writing one
//! provenance-bearing [`PassReceipt`] node per pass, linked to the
//! `CommonplaceChangeSet` lineage node (Part A deliverable 3).
//!
//! Coverage:
//! * a real (generated) ELF artifact -> ReconstructionPass is applicable and
//!   `Produced`, with a `PRODUCED_FOR` edge to the change-set node;
//! * a text-only change set -> ReconstructionPass is `NotApplicable`;
//! * the offload pass is real-wired: with only one ingested text item (the ELF
//!   is binary, so the sink skips it; `main.rs` is the only commonplace item)
//!   there is no SIMILAR_TO subgraph to compute centrality over, so it records
//!   `NotApplicable` (honest, not a fabricated result; the `Produced` path is
//!   covered by the unit test that seeds a real similar subgraph);
//! * the standing-seed pass is registered and records an `Unavailable` receipt
//!   naming its missing substrate capability (no fabrication);
//! * the canonical-git boundary still holds (writes land only in the sidecar).

use commonplace_desktop_runtime::{
    AmbientPass, AmbientRuntime, ChangeKind, ChangeSet, CommonplaceIngestSink, FileChange,
    IngestOutcome, OffloadPass, PassStatus, ReconstructionPass, SidecarCommonplace,
    StandingSeedPass, WatchConfig, PASS_RECEIPT_LABEL, PRODUCED_FOR_EDGE,
};
use rustyred_thg_core::{GraphStore, NodeQuery, RedCoreGraphStore};

/// Build a minimal but real ELF relocatable object so the reconstruction harness
/// parses it for real (a synthetic byte blob would not parse, exercising the
/// degraded path instead). Uses the same `object` version the loader parses with.
fn minimal_elf_object() -> Vec<u8> {
    use object::write::{Object, StandardSection, Symbol, SymbolSection};
    use object::{
        Architecture, BinaryFormat, Endianness, SymbolFlags, SymbolKind, SymbolScope,
    };

    let mut obj = Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(StandardSection::Text);
    // A tiny x86-64 `ret` body so there is a real executable section to lift.
    let offset = obj.append_section_data(text, &[0xc3], 1);
    obj.add_symbol(Symbol {
        name: b"_start".to_vec(),
        value: offset,
        size: 1,
        kind: SymbolKind::Text,
        scope: SymbolScope::Linkage,
        weak: false,
        section: SymbolSection::Section(text),
        flags: SymbolFlags::None,
    });
    obj.write().expect("write minimal elf object")
}

fn created(path: std::path::PathBuf) -> FileChange {
    FileChange {
        path,
        kind: ChangeKind::Created,
    }
}

#[test]
fn settled_change_set_runs_passes_and_links_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();

    // A real ELF artifact plus an ordinary source file in one settled batch.
    let elf_path = root.join("build/app.o");
    std::fs::create_dir_all(elf_path.parent().unwrap()).unwrap();
    std::fs::write(&elf_path, minimal_elf_object()).unwrap();
    std::fs::write(root.join("main.rs"), b"fn main() {}").unwrap();

    let config = WatchConfig::new(&root);
    let sink = CommonplaceIngestSink::open(&config).unwrap();
    let mut runtime = AmbientRuntime::new(sink).default_passes();

    let change_set = ChangeSet {
        changes: vec![created(elf_path.clone()), created(root.join("main.rs"))],
    };
    let report = runtime.run_cycle(&change_set).unwrap();

    // Ingest still happened (the source file landed; slices 1-2 intact).
    assert!(
        report.ingest.change_set_node_id.is_some(),
        "a change-set lineage node was written"
    );
    let change_set_node_id = report.ingest.change_set_node_id.clone().unwrap();

    // All three passes ran and each wrote a receipt node.
    assert_eq!(report.passes.len(), 3, "all three registered passes ran");

    let by_pass = |name: &str| {
        report
            .passes
            .iter()
            .find(|(receipt, _)| receipt.pass == name)
            .unwrap_or_else(|| panic!("missing receipt for {name}"))
    };

    // Reconstruction: applicable + produced, with the artifact id as evidence.
    let (recon, _) = by_pass(ReconstructionPass::NAME);
    assert_eq!(
        recon.status,
        PassStatus::Produced,
        "the real ELF artifact reconstructed: {:?}",
        recon.status
    );
    assert!(
        recon
            .applicable_inputs
            .iter()
            .any(|p| p.ends_with("app.o")),
        "the artifact is listed as an applicable input"
    );
    assert_eq!(
        recon.evidence_ids.len(),
        1,
        "one reconstructed artifact id is recorded as evidence"
    );
    assert!(
        recon.evidence_ids[0].starts_with("sha256:"),
        "evidence id is the harness artifact id"
    );

    // Offload: real-wired. Only one text item ingested (the ELF is binary and
    // skipped), so there is no SIMILAR_TO subgraph -> honest NotApplicable.
    let (offload, _) = by_pass(OffloadPass::NAME);
    assert_eq!(
        offload.status,
        PassStatus::NotApplicable,
        "one ingested text item -> no similar subgraph to offload, got {:?}",
        offload.status
    );

    // Standing-seed: registered, recorded Unavailable, gap named.
    let (standing, _) = by_pass(StandingSeedPass::NAME);
    match &standing.status {
        PassStatus::Unavailable(missing) => {
            assert!(!missing.is_empty(), "standing-seed names its missing capability");
        }
        other => panic!("standing-seed should be Unavailable, got {other:?}"),
    }

    // Provenance in the graph: one receipt node per pass, each PRODUCED_FOR the
    // change-set node.
    let receipt_nodes = GraphStore::query_nodes(
        runtime.sink().commonplace().store(),
        NodeQuery::label(PASS_RECEIPT_LABEL).with_limit(usize::MAX),
    );
    assert_eq!(receipt_nodes.len(), 3, "three receipt nodes persisted");

    for (_, receipt_node_id) in &report.passes {
        // The edge id is deterministic (see `write_pass_receipt`).
        let edge_id = format!("produced_for:{receipt_node_id}:{change_set_node_id}");
        let edge = GraphStore::get_edge_record(runtime.sink().commonplace().store(), &edge_id)
            .unwrap_or_else(|| panic!("receipt {receipt_node_id} is linked to its change set"));
        assert_eq!(edge.edge_type, PRODUCED_FOR_EDGE);
        assert_eq!(edge.from_id, *receipt_node_id);
        assert_eq!(
            edge.to_id, change_set_node_id,
            "the PRODUCED_FOR edge points at the change-set lineage node"
        );
    }
}

#[test]
fn reconstruction_not_applicable_without_an_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::write(root.join("notes.md"), b"# just text").unwrap();

    let config = WatchConfig::new(&root);
    let mut sink = CommonplaceIngestSink::open(&config).unwrap();
    let outcome = sink
        .ingest_change_set(&ChangeSet {
            changes: vec![created(root.join("notes.md"))],
        })
        .unwrap();

    // Run the reconstruction pass directly against the same store.
    let receipt = ReconstructionPass
        .run(
            &ChangeSet {
                changes: vec![created(root.join("notes.md"))],
            },
            &outcome,
            sink.commonplace_mut(),
        )
        .unwrap();

    assert_eq!(
        receipt.status,
        PassStatus::NotApplicable,
        "a text-only change set is not applicable to reconstruction"
    );
    assert!(receipt.applicable_inputs.is_empty());
    assert!(receipt.evidence_ids.is_empty());
}

#[test]
fn artifact_extension_with_unparseable_bytes_is_degraded_not_an_error() {
    // A file that LOOKS like an artifact (".bin") but is not a real object: the
    // pass must report Degraded (applicable, unreconstructable), never error.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::write(root.join("fake.bin"), b"not a real object file at all").unwrap();

    // Sidecar commonplace backed by a tempdir blob store (pinned to DiskObjectStore).
    let blob_dir = tempfile::tempdir().unwrap();
    let mut commonplace = SidecarCommonplace::new(
        RedCoreGraphStore::memory(),
        rustyred_thg_core::DiskObjectStore::open(blob_dir.path()).unwrap(),
    );

    let receipt = ReconstructionPass
        .run(
            &ChangeSet {
                changes: vec![created(root.join("fake.bin"))],
            },
            &IngestOutcome::default(),
            &mut commonplace,
        )
        .expect("an unparseable artifact is a receipt status, not a hard error");

    match receipt.status {
        PassStatus::Degraded(reason) => assert!(!reason.is_empty(), "degraded reason is named"),
        other => panic!("expected Degraded for unparseable .bin, got {other:?}"),
    }
    assert!(
        receipt.applicable_inputs.iter().any(|p| p.ends_with("fake.bin")),
        "the .bin file was considered applicable by extension"
    );
    assert!(receipt.evidence_ids.is_empty(), "nothing was reconstructed");
}

#[test]
fn ambient_runtime_apply_persists_receipts_and_keeps_tree_read_only() {
    use commonplace_desktop_runtime::ChangeSink;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::write(root.join("doc.md"), b"body").unwrap();

    let config = WatchConfig::new(&root);
    let sink = CommonplaceIngestSink::open(&config).unwrap();
    let mut runtime = AmbientRuntime::new(sink).default_passes();

    // Drive the composite through the ChangeSink seam (what the watcher calls).
    runtime.apply(ChangeSet {
        changes: vec![created(root.join("doc.md"))],
    });

    // The passes persisted receipts into the sidecar graph.
    let receipt_nodes = GraphStore::query_nodes(
        runtime.sink().commonplace().store(),
        NodeQuery::label(PASS_RECEIPT_LABEL).with_limit(usize::MAX),
    );
    assert_eq!(
        receipt_nodes.len(),
        3,
        "apply() ran all three passes and persisted their receipts"
    );

    // Canonical-git boundary: only doc.md and the sidecar exist at the top level.
    let mut top_level: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    top_level.sort();
    assert!(top_level.contains(&root.join("doc.md")));
    assert!(top_level.contains(&config.sidecar_dir));
    assert_eq!(top_level.len(), 2, "no stray writes into the tree: {top_level:?}");
}
