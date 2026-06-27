use std::collections::BTreeSet;

use rustyred_thg_code::{
    compile_program_analysis_run_in_store, ghidra_oracle_export_to_program_analysis_input,
    load_native_binary, GhidraOracleExport, GhidraOracleFixture,
    GHIDRA_ORACLE_ANNOTATION_FACT_LABEL,
    GHIDRA_ORACLE_CALL_STACK_EFFECT_FACT_LABEL, GHIDRA_ORACLE_DATA_TYPE_FACT_LABEL,
    GHIDRA_ORACLE_EQUATE_FACT_LABEL, GHIDRA_ORACLE_EXTERNAL_LINKAGE_FACT_LABEL,
    GHIDRA_ORACLE_FUNCTION_FACT_LABEL, GHIDRA_ORACLE_FUNCTION_ID_FACT_LABEL,
    GHIDRA_ORACLE_FUNCTION_PROTOTYPE_FACT_LABEL, GHIDRA_ORACLE_HIGH_VARIABLE_FACT_LABEL,
    GHIDRA_ORACLE_JUMP_TABLE_FACT_LABEL,
    GHIDRA_ORACLE_PARAMETER_MEASURE_FACT_LABEL, GHIDRA_ORACLE_SEMANTIC_SIGNATURE_FACT_LABEL,
    GHIDRA_ORACLE_STACK_FRAME_FACT_LABEL, GHIDRA_ORACLE_STRUCTURE_FIELD_ACCESS_FACT_LABEL,
    LOADER_FACT_LABEL,
};
use rustyred_thg_core::{InMemoryGraphStore, NodeQuery};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct OracleFile {
    fixture: GhidraOracleFixture,
    artifact: OracleArtifact,
    loader_expectations: LoaderExpectations,
    functions: Vec<rustyred_thg_code::GhidraOracleFunctionFact>,
    jump_tables: Vec<rustyred_thg_code::GhidraOracleJumpTableFact>,
    #[serde(default)]
    equates: Vec<rustyred_thg_code::GhidraOracleEquateFact>,
    #[serde(default)]
    external_linkages: Vec<rustyred_thg_code::GhidraOracleExternalLinkageFact>,
    #[serde(default)]
    function_prototypes: Vec<rustyred_thg_code::GhidraOracleFunctionPrototypeFact>,
    #[serde(default)]
    data_types: Vec<rustyred_thg_code::GhidraOracleDataTypeFact>,
    #[serde(default)]
    high_variables: Vec<rustyred_thg_code::GhidraOracleHighVariableFact>,
    #[serde(default)]
    stack_frames: Vec<rustyred_thg_code::GhidraOracleStackFrameFact>,
    #[serde(default)]
    parameter_measures: Vec<rustyred_thg_code::GhidraOracleParameterMeasureFact>,
    #[serde(default)]
    call_stack_effects: Vec<rustyred_thg_code::GhidraOracleCallStackEffectFact>,
    #[serde(default)]
    annotations: Vec<rustyred_thg_code::GhidraOracleAnnotationFact>,
    #[serde(default)]
    structure_field_accesses: Vec<rustyred_thg_code::GhidraOracleStructureFieldAccessFact>,
    semantic_signatures: Vec<rustyred_thg_code::GhidraOracleSemanticSignatureFact>,
    function_id_signatures: Vec<rustyred_thg_code::GhidraOracleFunctionIdFact>,
}

#[derive(Debug, Deserialize)]
struct OracleEnvelope {
    artifact: OracleArtifact,
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
    string_values: Vec<String>,
}

#[test]
fn switch_jump_table_oracle_fixture_writes_non_empty_jump_tables() {
    let raw = include_str!("fixtures/ghidra_oracle/hello_switch.oracle.json");
    let bytes = decode_hex(include_str!("fixtures/ghidra_oracle/hello_switch.elf.hex"));
    let envelope: OracleEnvelope =
        serde_json::from_str(raw).expect("switch oracle envelope is valid json");
    let export: GhidraOracleExport =
        serde_json::from_str(raw).expect("switch oracle is a valid GhidraOracleExport");
    let native = load_native_binary(&bytes, export.fixture.evidence_ids.clone())
        .expect("native loader parses switch fixture object");

    assert_eq!(native.artifact.sha256, envelope.artifact.sha256);
    assert_eq!(native.artifact.format, envelope.artifact.format);
    assert_eq!(native.artifact.arch, envelope.artifact.arch);
    assert_eq!(native.artifact.endian, envelope.artifact.endian);
    assert_eq!(export.functions.len(), 2);
    assert_eq!(export.jump_tables.len(), 1);
    assert_eq!(export.equates.len(), 1);
    assert_eq!(export.jump_tables[0].cases.len(), 9);
    assert!(export.jump_tables[0].cases.iter().any(|case| {
        case.is_default && case.destination == "0x70" && case.label.as_deref() == Some("default")
    }));

    let mut store = InMemoryGraphStore::new();
    let compiled = compile_program_analysis_run_in_store(
        &mut store,
        ghidra_oracle_export_to_program_analysis_input("Travis-Gilbert", native.artifact, export),
    )
    .expect("switch oracle export writes to graph");

    assert_eq!(compiled.oracle_jump_table_facts.len(), 1);
    assert_eq!(compiled.oracle_equate_facts.len(), 1);
    assert_eq!(compiled.reference_recovery_evidence.len(), 1);

    let jump_table_nodes = store.query_nodes(NodeQuery::label(GHIDRA_ORACLE_JUMP_TABLE_FACT_LABEL));
    assert_eq!(jump_table_nodes.len(), 1);
    let jump_table = &jump_table_nodes[0];
    assert_eq!(
        jump_table.properties.get("tenant_id"),
        Some(&json!("Travis-Gilbert"))
    );
    assert_eq!(
        jump_table.properties.get("logical_id"),
        Some(&json!("ghidra:jumptable:0x10"))
    );
    assert_eq!(jump_table.properties.get("case_count"), Some(&json!(9)));
    assert_eq!(
        jump_table.properties.get("load_table_count"),
        Some(&json!(1))
    );
    assert_eq!(
        jump_table.properties.get("references_complete"),
        Some(&json!(true))
    );

    let cases = jump_table
        .properties
        .get("cases")
        .and_then(|value| value.as_array())
        .expect("jump-table graph node stores case metadata");
    assert!(cases.iter().any(|case| {
        case.get("is_default") == Some(&json!(true))
            && case.get("destination") == Some(&json!("0x70"))
    }));

    let equate_nodes = store.query_nodes(NodeQuery::label(GHIDRA_ORACLE_EQUATE_FACT_LABEL));
    assert_eq!(equate_nodes.len(), 1);
    let equate = &equate_nodes[0];
    assert_eq!(
        equate.properties.get("tenant_id"),
        Some(&json!("Travis-Gilbert"))
    );
    assert_eq!(
        equate.properties.get("logical_id"),
        Some(&json!("ghidra:equate:switch-case-seven"))
    );
    assert_eq!(
        equate.properties.get("name"),
        Some(&json!("SWITCH_CASE_SEVEN"))
    );
    assert_eq!(equate.properties.get("value"), Some(&json!(7)));
    assert_eq!(
        equate.properties.get("enum_uuid"),
        Some(&json!("enum:uuid:switch-selector"))
    );
    let references = equate
        .properties
        .get("references")
        .and_then(|value| value.as_array())
        .expect("equate graph node stores references");
    assert!(references.iter().any(|reference| {
        reference.get("address") == Some(&json!("0x10"))
            && reference.get("operand_index") == Some(&json!(0))
            && reference.get("dynamic_hash") == Some(&json!(4276993775_u64))
    }));
}

#[test]
fn native_loader_matches_ghidra_oracle_fixture_and_writes_program_analysis_graph() {
    let bytes = decode_hex(include_str!("fixtures/ghidra_oracle/hello_tiny.elf.hex"));
    let oracle: OracleFile = serde_json::from_str(include_str!(
        "fixtures/ghidra_oracle/hello_tiny.oracle.json"
    ))
    .expect("oracle fixture is valid json");
    let ghidra_export: GhidraOracleExport = serde_json::from_str(include_str!(
        "fixtures/ghidra_oracle/hello_tiny.oracle.json"
    ))
    .expect("oracle fixture is directly loadable as GhidraOracleExport");

    let native = load_native_binary(&bytes, oracle.fixture.evidence_ids.clone())
        .expect("native loader parses fixture object");

    assert_eq!(native.artifact.sha256, oracle.artifact.sha256);
    assert_eq!(native.artifact.format, oracle.artifact.format);
    assert_eq!(native.artifact.arch, oracle.artifact.arch);
    assert_eq!(native.artifact.endian, oracle.artifact.endian);
    assert_exact_normalized_set(
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
    assert_exact_normalized_set(
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
    assert_exact_normalized_set(
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
    assert_exact_normalized_set(
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
    assert_exact_normalized_set(
        native
            .loader_fact
            .strings
            .iter()
            .map(|string| string.value.as_str()),
        oracle
            .loader_expectations
            .string_values
            .iter()
            .map(String::as_str),
    );
    assert_eq!(
        native.analyzer_receipt.status,
        rustyred_thg_code::ProgramAnalysisStatus::Complete
    );

    assert_eq!(ghidra_export.fixture, oracle.fixture);
    assert_eq!(ghidra_export.functions, oracle.functions);
    assert_eq!(ghidra_export.jump_tables, oracle.jump_tables);
    assert_eq!(ghidra_export.equates, oracle.equates);
    assert_eq!(ghidra_export.external_linkages, oracle.external_linkages);
    assert_eq!(ghidra_export.function_prototypes, oracle.function_prototypes);
    assert_eq!(ghidra_export.data_types, oracle.data_types);
    assert_eq!(ghidra_export.high_variables, oracle.high_variables);
    assert_eq!(ghidra_export.stack_frames, oracle.stack_frames);
    assert_eq!(ghidra_export.parameter_measures, oracle.parameter_measures);
    assert_eq!(ghidra_export.call_stack_effects, oracle.call_stack_effects);
    assert_eq!(ghidra_export.annotations, oracle.annotations);
    assert_eq!(
        ghidra_export.structure_field_accesses,
        oracle.structure_field_accesses
    );
    assert_eq!(
        ghidra_export.semantic_signatures,
        oracle.semantic_signatures
    );
    assert_eq!(
        ghidra_export.function_id_signatures,
        oracle.function_id_signatures
    );
    assert_eq!(
        ghidra_export.fixture.program_summary,
        oracle.fixture.program_summary
    );

    let mut input = ghidra_oracle_export_to_program_analysis_input(
        "Travis-Gilbert",
        native.artifact.clone(),
        ghidra_export,
    );
    input.loader_facts.push(native.loader_fact);
    input.analyzer_receipts.push(native.analyzer_receipt);

    let mut store = InMemoryGraphStore::new();
    let compiled = compile_program_analysis_run_in_store(&mut store, input)
        .expect("program analysis output writes to graph");

    assert!(store.get_node(&compiled.run.run_id).is_some());
    assert!(store
        .query_nodes(NodeQuery::label(rustyred_thg_code::BINARY_ARTIFACT_LABEL))
        .iter()
        .any(
            |node| node.properties.get("tenant_id") == Some(&json!("Travis-Gilbert"))
                && node.properties.get("logical_id") == Some(&json!(compiled.artifact.artifact_id))
        ));
    assert_eq!(
        store.query_nodes(NodeQuery::label(LOADER_FACT_LABEL)).len(),
        1
    );
    assert_eq!(
        store
            .query_nodes(NodeQuery::label(GHIDRA_ORACLE_FUNCTION_FACT_LABEL))
            .len(),
        oracle.functions.len()
    );
    assert_eq!(
        store
            .query_nodes(NodeQuery::label(GHIDRA_ORACLE_JUMP_TABLE_FACT_LABEL))
            .len(),
        oracle.jump_tables.len()
    );
    assert_eq!(
        store
            .query_nodes(NodeQuery::label(GHIDRA_ORACLE_EQUATE_FACT_LABEL))
            .len(),
        oracle.equates.len()
    );
    assert_eq!(
        store
            .query_nodes(NodeQuery::label(GHIDRA_ORACLE_EXTERNAL_LINKAGE_FACT_LABEL))
            .len(),
        oracle.external_linkages.len()
    );
    assert_eq!(
        store
            .query_nodes(NodeQuery::label(GHIDRA_ORACLE_FUNCTION_PROTOTYPE_FACT_LABEL))
            .len(),
        oracle.function_prototypes.len()
    );
    assert_eq!(
        store
            .query_nodes(NodeQuery::label(GHIDRA_ORACLE_DATA_TYPE_FACT_LABEL))
            .len(),
        oracle.data_types.len()
    );
    assert_eq!(
        store
            .query_nodes(NodeQuery::label(GHIDRA_ORACLE_HIGH_VARIABLE_FACT_LABEL))
            .len(),
        oracle.high_variables.len()
    );
    assert_eq!(
        store
            .query_nodes(NodeQuery::label(GHIDRA_ORACLE_STACK_FRAME_FACT_LABEL))
            .len(),
        oracle.stack_frames.len()
    );
    assert_eq!(
        store
            .query_nodes(NodeQuery::label(GHIDRA_ORACLE_PARAMETER_MEASURE_FACT_LABEL))
            .len(),
        oracle.parameter_measures.len()
    );
    assert_eq!(
        store
            .query_nodes(NodeQuery::label(GHIDRA_ORACLE_CALL_STACK_EFFECT_FACT_LABEL))
            .len(),
        oracle.call_stack_effects.len()
    );
    assert_eq!(
        store
            .query_nodes(NodeQuery::label(GHIDRA_ORACLE_ANNOTATION_FACT_LABEL))
            .len(),
        oracle.annotations.len()
    );
    assert_eq!(
        store
            .query_nodes(NodeQuery::label(
                GHIDRA_ORACLE_STRUCTURE_FIELD_ACCESS_FACT_LABEL
            ))
            .len(),
        oracle.structure_field_accesses.len()
    );
    assert_eq!(
        store
            .query_nodes(NodeQuery::label(
                GHIDRA_ORACLE_SEMANTIC_SIGNATURE_FACT_LABEL
            ))
            .len(),
        oracle.semantic_signatures.len()
    );
    assert_eq!(
        store
            .query_nodes(NodeQuery::label(GHIDRA_ORACLE_FUNCTION_ID_FACT_LABEL))
            .len(),
        oracle.function_id_signatures.len()
    );
    assert!(compiled
        .graph_nodes
        .iter()
        .any(|node| node.properties.get("logical_id") == Some(&json!(oracle.fixture.fixture_id))));
}

#[test]
fn generic_ghidra_exporter_emits_jump_table_oracle_contract() {
    let exporter = include_str!("fixtures/ghidra_oracle/ExportTheoremFacts.java");

    assert!(exporter.contains("result.getHighFunction()"));
    assert!(exporter.contains("highFunction.getJumpTables()"));
    assert!(exporter.contains("jump_tables"));
    assert!(exporter.contains("DEFAULT_CASE_VALUE = 0xbad1abe1"));
    assert!(exporter.contains("table.getCases()"));
    assert!(exporter.contains("table.getLabelValues()"));
    assert!(exporter.contains("table.getLoadTables()"));
    assert!(exporter.contains("isPointerLoadTable(loadTable, cases)"));
    assert!(exporter.contains("currentProgram.getEquateTable()"));
    assert!(exporter.contains("equate.getDisplayName()"));
    assert!(exporter.contains("equate.getDisplayValue()"));
    assert!(exporter.contains("reference.getDynamicHashValue()"));
    assert!(exporter.contains("equates"));
    assert!(exporter.contains("external_linkages"));
    assert!(exporter.contains("getExternalLocations()"));
    assert!(exporter.contains("getExternalSpaceAddress()"));
    assert!(exporter.contains("getOriginalImportedName()"));
    assert!(exporter.contains("getThunkedFunction(false)"));
    assert!(exporter.contains("function_prototypes"));
    assert!(exporter.contains("annotations"));
    assert!(exporter.contains("collectAnnotations"));
    assert!(exporter.contains("getCommentAddressIterator"));
    assert!(exporter.contains("getBookmarksIterator"));
    assert!(exporter.contains("getPrototypeString(false, true)"));
    assert!(exporter.contains("getSignatureSource()"));
    assert!(exporter.contains("hasNoReturn()"));
    assert!(exporter.contains("hasCustomVariableStorage()"));
    assert!(exporter.contains("data_types"));
    assert!(exporter.contains("getAllDataTypes()"));
    assert!(exporter.contains("getDefinedComponents()"));
    assert!(exporter.contains("getFieldName()"));
    assert!(exporter.contains("getBitSize()"));
    assert!(exporter.contains("high_variables"));
    assert!(exporter.contains("getLocalSymbolMap()"));
    assert!(exporter.contains("getGlobalSymbolMap()"));
    assert!(exporter.contains("getHighVariable()"));
    assert!(exporter.contains("getInstances()"));
    assert!(exporter.contains("getSerializationString()"));
    assert!(exporter.contains("stack_frames"));
    assert!(exporter.contains("function.getStackFrame()"));
    assert!(exporter.contains("frame.getFrameSize()"));
    assert!(exporter.contains("frame.getStackVariables()"));
    assert!(exporter.contains("getStackPointer()"));
    assert!(exporter.contains("variable.getStackOffset()"));
    assert!(exporter.contains("parameter_measures"));
    assert!(exporter.contains("setSimplificationStyle(\"paramid\")"));
    assert!(exporter.contains("toggleParamMeasures(true)"));
    assert!(exporter.contains("getHighParamID()"));
    assert!(exporter.contains("getNumInputs()"));
    assert!(exporter.contains("getNumOutputs()"));
    assert!(exporter.contains("call_stack_effects"));
    assert!(exporter.contains("CallDepthChangeInfo"));
    assert!(exporter.contains("getCallChange"));
    assert!(exporter.contains("getStackPurgeSize"));
    assert!(exporter.contains("getExtrapop"));
    assert!(exporter.contains("getStackshift"));
    assert!(exporter.contains("structure_field_accesses"));
    assert!(exporter.contains("ClangFieldToken"));
    assert!(exporter.contains("ClangBitFieldToken"));
    assert!(exporter.contains("getCCodeMarkup()"));
    assert!(exporter.contains("componentForOffset(structureType"));
}

fn assert_exact_normalized_set<'a, Actual, Expected>(actual: Actual, expected: Expected)
where
    Actual: IntoIterator<Item = &'a str>,
    Expected: IntoIterator<Item = &'a str>,
{
    let actual = actual.into_iter().collect::<BTreeSet<_>>();
    let expected = expected.into_iter().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
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
