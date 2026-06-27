use rustyred_thg_code::{
    compile_program_analysis_run_in_store, ghidra_oracle_fixture_to_program_analysis_input,
    load_native_binary, GhidraOracleFixture, GhidraOracleProgramSummary, LOADER_FACT_LABEL,
};
use rustyred_thg_core::{InMemoryGraphStore, NodeQuery};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct OracleFile {
    fixture_id: String,
    source_uri: String,
    export_script: String,
    artifact: OracleArtifact,
    loader_expectations: LoaderExpectations,
    program_summary: GhidraOracleProgramSummary,
}

#[derive(Debug, Deserialize)]
struct OracleArtifact {
    sha256: String,
    format: String,
    arch: String,
    endian: String,
}

#[derive(Debug, Deserialize)]
struct LoaderExpectations {
    section_names: Vec<String>,
    symbol_names: Vec<String>,
    import_names: Vec<String>,
    relocation_targets: Vec<String>,
}

#[test]
fn native_loader_matches_ghidra_oracle_fixture_and_writes_program_analysis_graph() {
    let bytes = decode_hex(include_str!("fixtures/ghidra_oracle/hello_tiny.elf.hex"));
    let oracle: OracleFile = serde_json::from_str(include_str!(
        "fixtures/ghidra_oracle/hello_tiny.oracle.json"
    ))
    .expect("oracle fixture is valid json");

    let native = load_native_binary(
        &bytes,
        vec![format!("fixture:{}", oracle.fixture_id.as_str())],
    )
    .expect("native loader parses fixture object");

    assert_eq!(native.artifact.sha256, oracle.artifact.sha256);
    assert_eq!(native.artifact.format, oracle.artifact.format);
    assert_eq!(native.artifact.arch, oracle.artifact.arch);
    assert_eq!(native.artifact.endian, oracle.artifact.endian);
    assert_contains_all(
        native
            .loader_fact
            .sections
            .iter()
            .map(|section| section.name.as_str()),
        oracle
            .loader_expectations
            .section_names
            .iter()
            .map(String::as_str),
    );
    assert_contains_all(
        native
            .loader_fact
            .symbols
            .iter()
            .map(|symbol| symbol.name.as_str()),
        oracle
            .loader_expectations
            .symbol_names
            .iter()
            .map(String::as_str),
    );
    assert_contains_all(
        native
            .loader_fact
            .imports
            .iter()
            .map(|import| import.name.as_str()),
        oracle
            .loader_expectations
            .import_names
            .iter()
            .map(String::as_str),
    );
    assert_contains_all(
        native
            .loader_fact
            .relocations
            .iter()
            .map(|relocation| relocation.target.as_str()),
        oracle
            .loader_expectations
            .relocation_targets
            .iter()
            .map(String::as_str),
    );
    assert!(native
        .loader_fact
        .strings
        .iter()
        .any(|string| string.value == "theorem_add"));
    assert_eq!(
        native.analyzer_receipt.status,
        rustyred_thg_code::ProgramAnalysisStatus::Complete
    );

    let ghidra_fixture = GhidraOracleFixture {
        fixture_id: oracle.fixture_id.clone(),
        source_uri: oracle.source_uri,
        export_script: oracle.export_script,
        program_summary: oracle.program_summary,
        evidence_ids: native.loader_fact.evidence_ids.clone(),
    };
    let mut input = ghidra_oracle_fixture_to_program_analysis_input(
        "Travis-Gilbert",
        native.artifact.clone(),
        ghidra_fixture,
    );
    input.loader_facts.push(native.loader_fact);
    input.analyzer_receipts.push(native.analyzer_receipt);

    let mut store = InMemoryGraphStore::new();
    let compiled = compile_program_analysis_run_in_store(&mut store, input)
        .expect("program analysis output writes to graph");

    assert!(store.get_node(&compiled.run.run_id).is_some());
    assert_eq!(
        store
            .get_node(&compiled.artifact.artifact_id)
            .and_then(|node| node.properties.get("tenant_id")),
        Some(&json!("Travis-Gilbert"))
    );
    assert_eq!(
        store.query_nodes(NodeQuery::label(LOADER_FACT_LABEL)).len(),
        1
    );
    assert!(compiled
        .graph_nodes
        .iter()
        .any(|node| node.id == oracle.fixture_id));
}

fn assert_contains_all<'a, Actual, Expected>(actual: Actual, expected: Expected)
where
    Actual: IntoIterator<Item = &'a str>,
    Expected: IntoIterator<Item = &'a str>,
{
    let actual = actual.into_iter().collect::<Vec<_>>();
    for expected_value in expected {
        assert!(
            actual.contains(&expected_value),
            "expected {expected_value:?} in {actual:?}"
        );
    }
}

fn decode_hex(input: &str) -> Vec<u8> {
    let hex = input
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert_eq!(hex.len() % 2, 0, "hex fixture must have whole bytes");

    (0..hex.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&hex[index..index + 2], 16).expect("hex fixture contains bytes")
        })
        .collect()
}
