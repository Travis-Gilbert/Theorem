use rustyred_thg_core::{stable_hash, EdgeRecord, GraphStore, GraphStoreResult, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::SOURCE;

pub const PROGRAM_ANALYSIS_RUN_LABEL: &str = "ProgramAnalysisRun";
pub const BINARY_ARTIFACT_LABEL: &str = "BinaryArtifact";
pub const LOADER_FACT_LABEL: &str = "LoaderFact";
pub const INSTRUCTION_FACT_LABEL: &str = "InstructionFact";
pub const THEOREM_IR_FUNCTION_LABEL: &str = "TheoremIrFunction";
pub const PROGRAM_DATA_FLOW_FACT_LABEL: &str = "ProgramDataFlowFact";
pub const PROGRAM_SEMANTIC_HYPOTHESIS_LABEL: &str = "ProgramSemanticHypothesis";
pub const ANALYZER_PASS_RECEIPT_LABEL: &str = "AnalyzerPassReceipt";
pub const GHIDRA_ORACLE_FIXTURE_LABEL: &str = "GhidraOracleFixture";

pub const ANALYZES_ARTIFACT: &str = "ANALYZES_ARTIFACT";
pub const HAS_LOADER_FACT: &str = "HAS_LOADER_FACT";
pub const HAS_INSTRUCTION_FACT: &str = "HAS_INSTRUCTION_FACT";
pub const HAS_THIR_FUNCTION: &str = "HAS_THIR_FUNCTION";
pub const HAS_DATA_FLOW_FACT: &str = "HAS_DATA_FLOW_FACT";
pub const HAS_SEMANTIC_HYPOTHESIS: &str = "HAS_SEMANTIC_HYPOTHESIS";
pub const HAS_ANALYZER_RECEIPT: &str = "HAS_ANALYZER_RECEIPT";
pub const DERIVED_FROM_ORACLE: &str = "DERIVED_FROM_ORACLE";

const DEFAULT_TOOLCHAIN: &str = "theorem-program-analysis-v0";
const DEFAULT_PROFILE: &str = "ghidra-reference-contract-v0";
const AUTHORITY_OBSERVED_FACT: &str = "observed_fact";
const AUTHORITY_DERIVED_FACT: &str = "derived_fact";
const AUTHORITY_HYPOTHESIS: &str = "hypothesis";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ProgramAnalysisTargetKind {
    Binary,
    Repo,
    Site,
    Feature,
    Api,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ProgramAnalysisStatus {
    Pending,
    Running,
    Complete,
    Failed,
    Partial,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProgramAnalysisRun {
    pub run_id: String,
    pub tenant_id: String,
    pub artifact_id: String,
    pub target_kind: ProgramAnalysisTargetKind,
    pub toolchain: String,
    pub profile: String,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub status: ProgramAnalysisStatus,
    pub artifact_hash: String,
    pub receipt_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BinaryArtifact {
    pub artifact_id: String,
    pub sha256: String,
    pub format: String,
    pub arch: String,
    pub endian: String,
    pub entrypoints: Vec<String>,
    pub load_base: Option<String>,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BinarySection {
    pub name: String,
    pub address: String,
    pub size: u64,
    pub permissions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BinarySymbol {
    pub name: String,
    pub address: String,
    pub kind: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BinaryRelocation {
    pub address: String,
    pub target: String,
    pub kind: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BinaryImport {
    pub library: Option<String>,
    pub name: String,
    pub address: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BinaryString {
    pub address: Option<String>,
    pub value: String,
    pub encoding: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoaderFact {
    pub fact_id: String,
    pub sections: Vec<BinarySection>,
    pub symbols: Vec<BinarySymbol>,
    pub relocations: Vec<BinaryRelocation>,
    pub imports: Vec<BinaryImport>,
    pub strings: Vec<BinaryString>,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstructionFact {
    pub instruction_id: String,
    pub address: String,
    pub bytes_hash: String,
    pub mnemonic: String,
    pub operands: Vec<String>,
    pub fallthrough: Option<String>,
    pub branch_target: Option<String>,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TheoremIrFunction {
    pub function_id: String,
    pub address_range: Option<(String, String)>,
    pub basic_block_ids: Vec<String>,
    pub statement_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProgramDataFlowFact {
    pub fact_id: String,
    pub fact_kind: String,
    pub source_id: String,
    pub target_id: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProgramSemanticHypothesis {
    pub hypothesis_id: String,
    pub target_id: String,
    pub role: String,
    pub confidence: u8,
    pub model_id: Option<String>,
    pub evidence_ids: Vec<String>,
    pub unknowns: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalyzerPassReceipt {
    pub receipt_id: String,
    pub analyzer_id: String,
    pub input_labels: Vec<String>,
    pub output_labels: Vec<String>,
    pub authority_layer: String,
    pub input_hash: String,
    pub status: ProgramAnalysisStatus,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProgramAnalysisInput {
    pub tenant_id: String,
    pub artifact: BinaryArtifact,
    pub target_kind: ProgramAnalysisTargetKind,
    pub toolchain: String,
    pub profile: String,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub status: ProgramAnalysisStatus,
    pub loader_facts: Vec<LoaderFact>,
    pub instruction_facts: Vec<InstructionFact>,
    pub ir_functions: Vec<TheoremIrFunction>,
    pub data_flow_facts: Vec<ProgramDataFlowFact>,
    pub semantic_hypotheses: Vec<ProgramSemanticHypothesis>,
    pub analyzer_receipts: Vec<AnalyzerPassReceipt>,
    pub oracle_fixture: Option<GhidraOracleFixture>,
}

impl ProgramAnalysisInput {
    pub fn new(tenant_id: impl Into<String>, artifact: BinaryArtifact) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            artifact,
            target_kind: ProgramAnalysisTargetKind::Binary,
            toolchain: DEFAULT_TOOLCHAIN.to_string(),
            profile: DEFAULT_PROFILE.to_string(),
            started_at_ms: 0,
            finished_at_ms: None,
            status: ProgramAnalysisStatus::Complete,
            loader_facts: Vec::new(),
            instruction_facts: Vec::new(),
            ir_functions: Vec::new(),
            data_flow_facts: Vec::new(),
            semantic_hypotheses: Vec::new(),
            analyzer_receipts: Vec::new(),
            oracle_fixture: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProgramAnalysisOutput {
    pub run: ProgramAnalysisRun,
    pub artifact: BinaryArtifact,
    pub loader_facts: Vec<LoaderFact>,
    pub instruction_facts: Vec<InstructionFact>,
    pub ir_functions: Vec<TheoremIrFunction>,
    pub data_flow_facts: Vec<ProgramDataFlowFact>,
    pub semantic_hypotheses: Vec<ProgramSemanticHypothesis>,
    pub analyzer_receipts: Vec<AnalyzerPassReceipt>,
    pub graph_nodes: Vec<NodeRecord>,
    pub graph_edges: Vec<EdgeRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleProgramSummary {
    pub ghidra_version: String,
    pub language_id: Option<String>,
    pub compiler_spec_id: Option<String>,
    pub analysis_timeout_occurred: bool,
    pub function_count: u64,
    pub import_count: u64,
    pub string_count: u64,
    pub cfg_edge_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleFixture {
    pub fixture_id: String,
    pub source_uri: String,
    pub export_script: String,
    pub program_summary: GhidraOracleProgramSummary,
    pub evidence_ids: Vec<String>,
}

pub fn compile_program_analysis_run_in_memory(
    mut input: ProgramAnalysisInput,
) -> ProgramAnalysisOutput {
    normalize_input(&mut input);
    let artifact_hash = stable_hash(json!({
        "artifact": &input.artifact,
        "loader_facts": &input.loader_facts,
        "instruction_facts": &input.instruction_facts,
        "ir_functions": &input.ir_functions,
        "data_flow_facts": &input.data_flow_facts,
        "semantic_hypotheses": &input.semantic_hypotheses,
        "analyzer_receipts": &input.analyzer_receipts,
        "oracle_fixture": &input.oracle_fixture,
    }));
    let run_id = format!(
        "program-analysis:run:{}",
        stable_hash(json!({
            "tenant_id": &input.tenant_id,
            "artifact_id": &input.artifact.artifact_id,
            "target_kind": &input.target_kind,
            "toolchain": &input.toolchain,
            "profile": &input.profile,
            "artifact_hash": &artifact_hash,
        }))
    );
    let receipt_hash = stable_hash(json!({
        "run_id": &run_id,
        "status": &input.status,
        "started_at_ms": input.started_at_ms,
        "finished_at_ms": input.finished_at_ms,
        "artifact_hash": &artifact_hash,
    }));
    let run = ProgramAnalysisRun {
        run_id,
        tenant_id: input.tenant_id.clone(),
        artifact_id: input.artifact.artifact_id.clone(),
        target_kind: input.target_kind.clone(),
        toolchain: input.toolchain.clone(),
        profile: input.profile.clone(),
        started_at_ms: input.started_at_ms,
        finished_at_ms: input.finished_at_ms,
        status: input.status.clone(),
        artifact_hash,
        receipt_hash,
    };
    let (graph_nodes, graph_edges) = build_graph_payload(&input, &run);

    ProgramAnalysisOutput {
        run,
        artifact: input.artifact,
        loader_facts: input.loader_facts,
        instruction_facts: input.instruction_facts,
        ir_functions: input.ir_functions,
        data_flow_facts: input.data_flow_facts,
        semantic_hypotheses: input.semantic_hypotheses,
        analyzer_receipts: input.analyzer_receipts,
        graph_nodes,
        graph_edges,
    }
}

pub fn compile_program_analysis_run_in_store<S: GraphStore>(
    store: &mut S,
    input: ProgramAnalysisInput,
) -> GraphStoreResult<ProgramAnalysisOutput> {
    let output = compile_program_analysis_run_in_memory(input);
    for node in &output.graph_nodes {
        store.upsert_node(node.clone())?;
    }
    for edge in &output.graph_edges {
        store.upsert_edge(edge.clone())?;
    }
    Ok(output)
}

pub fn ghidra_oracle_fixture_to_program_analysis_input(
    tenant_id: impl Into<String>,
    artifact: BinaryArtifact,
    fixture: GhidraOracleFixture,
) -> ProgramAnalysisInput {
    let evidence_ids = normalize_strings(fixture.evidence_ids.clone());
    let mut input = ProgramAnalysisInput::new(tenant_id, artifact);
    input.toolchain = format!("ghidra-oracle:{}", fixture.program_summary.ghidra_version);
    input.profile = "ghidra-headless-oracle-v0".to_string();
    input.analyzer_receipts.push(AnalyzerPassReceipt {
        receipt_id: format!(
            "program-analysis:receipt:{}",
            stable_hash(json!(["ghidra-oracle", &fixture.fixture_id, &evidence_ids]))
        ),
        analyzer_id: "ghidra-headless-oracle".to_string(),
        input_labels: vec![BINARY_ARTIFACT_LABEL.to_string()],
        output_labels: vec![
            LOADER_FACT_LABEL.to_string(),
            INSTRUCTION_FACT_LABEL.to_string(),
            THEOREM_IR_FUNCTION_LABEL.to_string(),
        ],
        authority_layer: AUTHORITY_OBSERVED_FACT.to_string(),
        input_hash: stable_hash(json!({
            "fixture_id": &fixture.fixture_id,
            "source_uri": &fixture.source_uri,
            "summary": &fixture.program_summary,
        })),
        status: if fixture.program_summary.analysis_timeout_occurred {
            ProgramAnalysisStatus::Partial
        } else {
            ProgramAnalysisStatus::Complete
        },
        evidence_ids,
    });
    input.oracle_fixture = Some(fixture);
    input
}

fn normalize_input(input: &mut ProgramAnalysisInput) {
    input.tenant_id = input.tenant_id.trim().to_string();
    input.artifact = normalized_artifact(&input.artifact);
    input
        .loader_facts
        .sort_by(|left, right| left.fact_id.cmp(&right.fact_id));
    input
        .loader_facts
        .dedup_by(|left, right| left.fact_id == right.fact_id);
    for fact in &mut input.loader_facts {
        fact.sections.sort_by(|left, right| {
            left.address
                .cmp(&right.address)
                .then_with(|| left.name.cmp(&right.name))
        });
        fact.symbols.sort_by(|left, right| {
            left.address
                .cmp(&right.address)
                .then_with(|| left.name.cmp(&right.name))
        });
        fact.relocations.sort_by(|left, right| {
            left.address
                .cmp(&right.address)
                .then_with(|| left.target.cmp(&right.target))
        });
        fact.imports.sort_by(|left, right| {
            left.library
                .cmp(&right.library)
                .then_with(|| left.name.cmp(&right.name))
        });
        fact.strings.sort_by(|left, right| {
            left.address
                .cmp(&right.address)
                .then_with(|| left.value.cmp(&right.value))
        });
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
    }
    input
        .instruction_facts
        .sort_by(|left, right| left.address.cmp(&right.address));
    input
        .instruction_facts
        .dedup_by(|left, right| left.instruction_id == right.instruction_id);
    for fact in &mut input.instruction_facts {
        fact.operands = normalize_strings(fact.operands.clone());
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
    }
    input
        .ir_functions
        .sort_by(|left, right| left.function_id.cmp(&right.function_id));
    input
        .ir_functions
        .dedup_by(|left, right| left.function_id == right.function_id);
    for function in &mut input.ir_functions {
        function.basic_block_ids = normalize_strings(function.basic_block_ids.clone());
        function.statement_ids = normalize_strings(function.statement_ids.clone());
        function.evidence_ids = normalize_strings(function.evidence_ids.clone());
    }
    input
        .data_flow_facts
        .sort_by(|left, right| left.fact_id.cmp(&right.fact_id));
    input
        .data_flow_facts
        .dedup_by(|left, right| left.fact_id == right.fact_id);
    for fact in &mut input.data_flow_facts {
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
    }
    input
        .semantic_hypotheses
        .sort_by(|left, right| left.hypothesis_id.cmp(&right.hypothesis_id));
    input
        .semantic_hypotheses
        .dedup_by(|left, right| left.hypothesis_id == right.hypothesis_id);
    for hypothesis in &mut input.semantic_hypotheses {
        hypothesis.evidence_ids = normalize_strings(hypothesis.evidence_ids.clone());
        hypothesis.unknowns = normalize_strings(hypothesis.unknowns.clone());
    }
    input
        .analyzer_receipts
        .sort_by(|left, right| left.receipt_id.cmp(&right.receipt_id));
    input
        .analyzer_receipts
        .dedup_by(|left, right| left.receipt_id == right.receipt_id);
    for receipt in &mut input.analyzer_receipts {
        receipt.input_labels = normalize_strings(receipt.input_labels.clone());
        receipt.output_labels = normalize_strings(receipt.output_labels.clone());
        receipt.evidence_ids = normalize_strings(receipt.evidence_ids.clone());
    }
}

fn normalized_artifact(artifact: &BinaryArtifact) -> BinaryArtifact {
    let mut artifact = artifact.clone();
    artifact.artifact_id = artifact.artifact_id.trim().to_string();
    artifact.sha256 = artifact.sha256.trim().to_ascii_lowercase();
    artifact.format = artifact.format.trim().to_string();
    artifact.arch = artifact.arch.trim().to_string();
    artifact.endian = artifact.endian.trim().to_string();
    artifact.entrypoints = normalize_strings(artifact.entrypoints);
    artifact.evidence_ids = normalize_strings(artifact.evidence_ids);
    artifact
}

fn build_graph_payload(
    input: &ProgramAnalysisInput,
    run: &ProgramAnalysisRun,
) -> (Vec<NodeRecord>, Vec<EdgeRecord>) {
    let mut nodes = vec![
        NodeRecord::new(
            &run.run_id,
            [PROGRAM_ANALYSIS_RUN_LABEL],
            json!({
                "tenant_id": &run.tenant_id,
                "artifact_id": &run.artifact_id,
                "target_kind": &run.target_kind,
                "toolchain": &run.toolchain,
                "profile": &run.profile,
                "started_at_ms": run.started_at_ms,
                "finished_at_ms": run.finished_at_ms,
                "status": &run.status,
                "artifact_hash": &run.artifact_hash,
                "receipt_hash": &run.receipt_hash,
                "source": SOURCE,
            }),
        ),
        NodeRecord::new(
            &input.artifact.artifact_id,
            [BINARY_ARTIFACT_LABEL],
            json!({
                "tenant_id": &input.tenant_id,
                "sha256": &input.artifact.sha256,
                "format": &input.artifact.format,
                "arch": &input.artifact.arch,
                "endian": &input.artifact.endian,
                "entrypoints": &input.artifact.entrypoints,
                "load_base": &input.artifact.load_base,
                "evidence_ids": &input.artifact.evidence_ids,
                "authority_layer": AUTHORITY_OBSERVED_FACT,
                "source": SOURCE,
            }),
        ),
    ];
    let mut edges = vec![EdgeRecord::new(
        edge_id(&run.run_id, ANALYZES_ARTIFACT, &input.artifact.artifact_id),
        &run.run_id,
        ANALYZES_ARTIFACT,
        &input.artifact.artifact_id,
        edge_props(
            input,
            run,
            AUTHORITY_OBSERVED_FACT,
            &input.artifact.evidence_ids,
        ),
    )];

    for fact in &input.loader_facts {
        nodes.push(NodeRecord::new(
            &fact.fact_id,
            [LOADER_FACT_LABEL],
            json!({
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "sections": &fact.sections,
                "symbols": &fact.symbols,
                "relocations": &fact.relocations,
                "imports": &fact.imports,
                "strings": &fact.strings,
                "authority_layer": AUTHORITY_OBSERVED_FACT,
                "evidence_ids": &fact.evidence_ids,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(&run.run_id, HAS_LOADER_FACT, &fact.fact_id),
            &run.run_id,
            HAS_LOADER_FACT,
            &fact.fact_id,
            edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
        ));
    }

    for fact in &input.instruction_facts {
        nodes.push(NodeRecord::new(
            &fact.instruction_id,
            [INSTRUCTION_FACT_LABEL],
            json!({
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "address": &fact.address,
                "bytes_hash": &fact.bytes_hash,
                "mnemonic": &fact.mnemonic,
                "operands": &fact.operands,
                "fallthrough": &fact.fallthrough,
                "branch_target": &fact.branch_target,
                "authority_layer": AUTHORITY_OBSERVED_FACT,
                "evidence_ids": &fact.evidence_ids,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(&run.run_id, HAS_INSTRUCTION_FACT, &fact.instruction_id),
            &run.run_id,
            HAS_INSTRUCTION_FACT,
            &fact.instruction_id,
            edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
        ));
    }

    for function in &input.ir_functions {
        nodes.push(NodeRecord::new(
            &function.function_id,
            [THEOREM_IR_FUNCTION_LABEL],
            json!({
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "address_range": &function.address_range,
                "basic_block_ids": &function.basic_block_ids,
                "statement_ids": &function.statement_ids,
                "authority_layer": AUTHORITY_DERIVED_FACT,
                "evidence_ids": &function.evidence_ids,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(&run.run_id, HAS_THIR_FUNCTION, &function.function_id),
            &run.run_id,
            HAS_THIR_FUNCTION,
            &function.function_id,
            edge_props(input, run, AUTHORITY_DERIVED_FACT, &function.evidence_ids),
        ));
    }

    for fact in &input.data_flow_facts {
        nodes.push(NodeRecord::new(
            &fact.fact_id,
            [PROGRAM_DATA_FLOW_FACT_LABEL],
            json!({
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "fact_kind": &fact.fact_kind,
                "source_id": &fact.source_id,
                "target_id": &fact.target_id,
                "authority_layer": AUTHORITY_DERIVED_FACT,
                "evidence_ids": &fact.evidence_ids,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(&run.run_id, HAS_DATA_FLOW_FACT, &fact.fact_id),
            &run.run_id,
            HAS_DATA_FLOW_FACT,
            &fact.fact_id,
            edge_props(input, run, AUTHORITY_DERIVED_FACT, &fact.evidence_ids),
        ));
    }

    for hypothesis in &input.semantic_hypotheses {
        nodes.push(NodeRecord::new(
            &hypothesis.hypothesis_id,
            [PROGRAM_SEMANTIC_HYPOTHESIS_LABEL],
            json!({
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "target_id": &hypothesis.target_id,
                "role": &hypothesis.role,
                "confidence": hypothesis.confidence,
                "model_id": &hypothesis.model_id,
                "authority_layer": AUTHORITY_HYPOTHESIS,
                "evidence_ids": &hypothesis.evidence_ids,
                "unknowns": &hypothesis.unknowns,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(
                &run.run_id,
                HAS_SEMANTIC_HYPOTHESIS,
                &hypothesis.hypothesis_id,
            ),
            &run.run_id,
            HAS_SEMANTIC_HYPOTHESIS,
            &hypothesis.hypothesis_id,
            edge_props(input, run, AUTHORITY_HYPOTHESIS, &hypothesis.evidence_ids),
        ));
    }

    for receipt in &input.analyzer_receipts {
        nodes.push(NodeRecord::new(
            &receipt.receipt_id,
            [ANALYZER_PASS_RECEIPT_LABEL],
            json!({
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "analyzer_id": &receipt.analyzer_id,
                "input_labels": &receipt.input_labels,
                "output_labels": &receipt.output_labels,
                "authority_layer": &receipt.authority_layer,
                "input_hash": &receipt.input_hash,
                "status": &receipt.status,
                "evidence_ids": &receipt.evidence_ids,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(&run.run_id, HAS_ANALYZER_RECEIPT, &receipt.receipt_id),
            &run.run_id,
            HAS_ANALYZER_RECEIPT,
            &receipt.receipt_id,
            edge_props(input, run, &receipt.authority_layer, &receipt.evidence_ids),
        ));
    }

    if let Some(fixture) = &input.oracle_fixture {
        nodes.push(NodeRecord::new(
            &fixture.fixture_id,
            [GHIDRA_ORACLE_FIXTURE_LABEL],
            json!({
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "source_uri": &fixture.source_uri,
                "export_script": &fixture.export_script,
                "program_summary": &fixture.program_summary,
                "authority_layer": AUTHORITY_OBSERVED_FACT,
                "evidence_ids": &fixture.evidence_ids,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(&run.run_id, DERIVED_FROM_ORACLE, &fixture.fixture_id),
            &run.run_id,
            DERIVED_FROM_ORACLE,
            &fixture.fixture_id,
            edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fixture.evidence_ids),
        ));
    }

    (nodes, edges)
}

fn edge_props(
    input: &ProgramAnalysisInput,
    run: &ProgramAnalysisRun,
    authority_layer: &str,
    evidence_ids: &[String],
) -> Value {
    json!({
        "tenant_id": &input.tenant_id,
        "artifact_id": &input.artifact.artifact_id,
        "run_id": &run.run_id,
        "authority_layer": authority_layer,
        "evidence_ids": evidence_ids,
        "source": SOURCE,
    })
}

fn edge_id(from_id: &str, edge_kind: &str, to_id: &str) -> String {
    format!(
        "program-analysis:edge:{}",
        stable_hash(json!([from_id, edge_kind, to_id]))
    )
}

fn normalize_strings(mut values: Vec<String>) -> Vec<String> {
    values = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::{InMemoryGraphStore, NodeQuery};

    fn artifact() -> BinaryArtifact {
        BinaryArtifact {
            artifact_id: "artifact:sha256:abc".to_string(),
            sha256: "ABC".to_string(),
            format: "ELF".to_string(),
            arch: "x86_64".to_string(),
            endian: "little".to_string(),
            entrypoints: vec!["0x401000".to_string()],
            load_base: Some("0x400000".to_string()),
            evidence_ids: vec!["e:artifact".to_string()],
        }
    }

    fn fixture_input() -> ProgramAnalysisInput {
        let mut input = ProgramAnalysisInput::new("Travis-Gilbert", artifact());
        input.loader_facts.push(LoaderFact {
            fact_id: "loader:fixture".to_string(),
            sections: vec![BinarySection {
                name: ".text".to_string(),
                address: "0x401000".to_string(),
                size: 64,
                permissions: vec!["x".to_string(), "r".to_string()],
            }],
            symbols: vec![BinarySymbol {
                name: "main".to_string(),
                address: "0x401000".to_string(),
                kind: "function".to_string(),
            }],
            relocations: vec![],
            imports: vec![BinaryImport {
                library: Some("libc.so.6".to_string()),
                name: "puts".to_string(),
                address: None,
            }],
            strings: vec![BinaryString {
                address: Some("0x402000".to_string()),
                value: "hello".to_string(),
                encoding: "utf8".to_string(),
            }],
            evidence_ids: vec!["e:loader".to_string()],
        });
        input.instruction_facts.push(InstructionFact {
            instruction_id: "instr:401000".to_string(),
            address: "0x401000".to_string(),
            bytes_hash: "sha256:instr".to_string(),
            mnemonic: "call".to_string(),
            operands: vec!["puts".to_string()],
            fallthrough: Some("0x401005".to_string()),
            branch_target: Some("0x401030".to_string()),
            evidence_ids: vec!["e:instr".to_string()],
        });
        input.ir_functions.push(TheoremIrFunction {
            function_id: "thir:function:main".to_string(),
            address_range: Some(("0x401000".to_string(), "0x401040".to_string())),
            basic_block_ids: vec!["bb:main:entry".to_string()],
            statement_ids: vec!["stmt:main:call".to_string()],
            evidence_ids: vec!["e:thir".to_string()],
        });
        input.data_flow_facts.push(ProgramDataFlowFact {
            fact_id: "df:call:puts".to_string(),
            fact_kind: "call_edge".to_string(),
            source_id: "thir:function:main".to_string(),
            target_id: "import:puts".to_string(),
            evidence_ids: vec!["e:dataflow".to_string()],
        });
        input.semantic_hypotheses.push(ProgramSemanticHypothesis {
            hypothesis_id: "hypothesis:console-output".to_string(),
            target_id: "thir:function:main".to_string(),
            role: "ConsoleOutput".to_string(),
            confidence: 72,
            model_id: Some("fixture-model".to_string()),
            evidence_ids: vec!["e:hypothesis".to_string()],
            unknowns: vec!["stdout side effects not replayed".to_string()],
        });
        input.analyzer_receipts.push(AnalyzerPassReceipt {
            receipt_id: "receipt:loader".to_string(),
            analyzer_id: "loader".to_string(),
            input_labels: vec![BINARY_ARTIFACT_LABEL.to_string()],
            output_labels: vec![LOADER_FACT_LABEL.to_string()],
            authority_layer: AUTHORITY_OBSERVED_FACT.to_string(),
            input_hash: "sha256:loader-input".to_string(),
            status: ProgramAnalysisStatus::Complete,
            evidence_ids: vec!["e:loader".to_string()],
        });
        input
    }

    #[test]
    fn program_analysis_run_is_deterministic_and_scoped() {
        let first = compile_program_analysis_run_in_memory(fixture_input());
        let second = compile_program_analysis_run_in_memory(fixture_input());

        assert_eq!(first.run.run_id, second.run.run_id);
        assert_eq!(first.run.artifact_hash, second.run.artifact_hash);
        assert_eq!(first.graph_nodes, second.graph_nodes);
        assert_eq!(first.graph_edges, second.graph_edges);
        assert_eq!(first.artifact.sha256, "abc");

        for node in &first.graph_nodes {
            assert_eq!(
                node.properties.get("tenant_id"),
                Some(&json!("Travis-Gilbert"))
            );
        }
        for edge in &first.graph_edges {
            assert_eq!(
                edge.properties.get("tenant_id"),
                Some(&json!("Travis-Gilbert"))
            );
            assert_eq!(
                edge.properties.get("artifact_id"),
                Some(&json!("artifact:sha256:abc"))
            );
            assert_eq!(
                edge.properties.get("run_id"),
                Some(&json!(first.run.run_id))
            );
        }
    }

    #[test]
    fn program_analysis_run_writes_graph_payload() {
        let mut store = InMemoryGraphStore::new();
        let output = compile_program_analysis_run_in_store(&mut store, fixture_input())
            .expect("program analysis payload writes");

        assert!(store.get_node(&output.run.run_id).is_some());
        assert!(store.get_node(&output.artifact.artifact_id).is_some());
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(PROGRAM_ANALYSIS_RUN_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(INSTRUCTION_FACT_LABEL))
                .len(),
            1
        );
    }

    #[test]
    fn ghidra_oracle_fixture_lowers_to_program_analysis_input() {
        let fixture = GhidraOracleFixture {
            fixture_id: "ghidra:fixture:hello".to_string(),
            source_uri: "ghidra://headless/hello".to_string(),
            export_script: "ExportTheoremFacts.java".to_string(),
            program_summary: GhidraOracleProgramSummary {
                ghidra_version: "11.4.2".to_string(),
                language_id: Some("x86:LE:64:default".to_string()),
                compiler_spec_id: Some("gcc".to_string()),
                analysis_timeout_occurred: false,
                function_count: 1,
                import_count: 1,
                string_count: 1,
                cfg_edge_count: 1,
            },
            evidence_ids: vec!["e:ghidra".to_string()],
        };
        let input = ghidra_oracle_fixture_to_program_analysis_input(
            "Travis-Gilbert",
            artifact(),
            fixture.clone(),
        );
        let output = compile_program_analysis_run_in_memory(input);

        assert_eq!(output.run.toolchain, "ghidra-oracle:11.4.2");
        assert_eq!(output.analyzer_receipts.len(), 1);
        assert_eq!(
            output.analyzer_receipts[0].authority_layer,
            AUTHORITY_OBSERVED_FACT
        );
        assert!(output
            .graph_nodes
            .iter()
            .any(|node| node.id == fixture.fixture_id
                && node
                    .labels
                    .contains(&GHIDRA_ORACLE_FIXTURE_LABEL.to_string())));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == DERIVED_FROM_ORACLE));
    }
}
