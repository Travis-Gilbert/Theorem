#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use rustyred_thg_core::{stable_hash, EdgeRecord, GraphStore, GraphStoreResult, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::SOURCE;

pub mod native_loader;
pub mod program_analysis;
mod trace_to_contract;
pub use native_loader::{
    load_native_binary, NativeLoaderError, NativeLoaderOutput, NATIVE_LOADER_ANALYZER_ID,
};
pub use program_analysis::{
    compile_program_analysis_run_in_memory, compile_program_analysis_run_in_store,
    derive_ghidra_oracle_address_drifts, derive_ghidra_oracle_summary_drifts,
    derive_reference_recovery_evidence, ghidra_analysis_scheduler_states,
    ghidra_oracle_export_json_to_program_analysis_input,
    ghidra_oracle_export_to_program_analysis_input,
    ghidra_oracle_fixture_to_program_analysis_input, schedule_analysis_work_items,
    AnalysisAddressRange, AnalysisChangeSet, AnalysisDriftFact, AnalysisDriftKind,
    AnalysisPassSpec, AnalysisSchedulerState, AnalysisWorkItem, AnalysisWorkStatus,
    AnalyzerPassReceipt, BinaryArtifact, BinaryImport, BinaryRelocation, BinarySection,
    BinaryString, BinarySymbol, GhidraOracleAnnotationFact, GhidraOracleCallEdgeFact,
    GhidraOracleCallStackEffectFact, GhidraOracleDataTypeFact, GhidraOracleDataTypeFieldFact,
    GhidraOracleDebugSignatureFeature, GhidraOracleDecompilerDiagnosticFact,
    GhidraOracleEnumValueFact, GhidraOracleEquateFact,
    GhidraOracleEquateReferenceFact, GhidraOracleExport, GhidraOracleExternalLinkageFact,
    GhidraOracleExternalThunkLinkFact, GhidraOracleFixture, GhidraOracleFunctionFact,
    GhidraOracleFunctionIdFact, GhidraOracleFunctionIdMatchFact, GhidraOracleFunctionPrototypeFact,
    GhidraOracleFunctionPrototypeParameterFact, GhidraOracleHighVariableFact,
    GhidraOracleHighVariableInstanceFact, GhidraOracleJumpTableCaseFact, GhidraOracleJumpTableFact,
    GhidraOracleJumpTableLoadTableFact, GhidraOracleMemoryWitnessFact,
    GhidraOracleParameterMeasureFact, GhidraOraclePcodeOpFact, GhidraOracleProgramSummary,
    GhidraOracleReferenceFact, GhidraOracleSemanticSignatureFact, GhidraOracleStackFrameFact,
    GhidraOracleStackFrameVariableFact, GhidraOracleStructureFieldAccessFact,
    GhidraOracleSymbolicModelBinding, GhidraOracleSymbolicPreconditionFact,
    GhidraOracleSymbolicSummaryFact, GhidraOracleSymbolicValueFact,
    GhidraOracleVariableStorageFact, GhidraOracleVariableStoragePieceFact, InstructionFact,
    LoaderFact, ProgramAnalysisInput, ProgramAnalysisOutput, ProgramAnalysisRun,
    ProgramAnalysisStatus, ProgramAnalysisTargetKind, ProgramAnalyzerType, ProgramDataFlowFact,
    ProgramPcodeFact, ProgramSemanticHypothesis, ReferenceRecoveryEvidence, RuntimeTrace,
    TaintMark, TheoremIrFunction, TraceEventFact, TraceEventKind, TraceScheduleForm,
    TraceScheduleSource, TraceScheduleSpec, TraceSnapshotFact, ANALYSIS_DRIFT_LABEL,
    ANALYSIS_PASS_SPEC_LABEL, ANALYSIS_SCHEDULER_STATE_LABEL, ANALYSIS_WORK_ITEM_LABEL,
    ANALYZER_PASS_RECEIPT_LABEL, ANALYZES_ARTIFACT, BINARY_ARTIFACT_LABEL, DERIVED_FROM_ORACLE,
    EVENT_HAS_TAINT_MARK, GHIDRA_ORACLE_ANNOTATION_FACT_LABEL, GHIDRA_ORACLE_CALL_EDGE_FACT_LABEL,
    GHIDRA_ORACLE_CALL_STACK_EFFECT_FACT_LABEL, GHIDRA_ORACLE_DATA_TYPE_FACT_LABEL,
    GHIDRA_ORACLE_DECOMPILER_DIAGNOSTIC_FACT_LABEL, GHIDRA_ORACLE_EQUATE_FACT_LABEL,
    GHIDRA_ORACLE_EXTERNAL_LINKAGE_FACT_LABEL, GHIDRA_ORACLE_FIXTURE_LABEL,
    GHIDRA_ORACLE_FUNCTION_FACT_LABEL, GHIDRA_ORACLE_FUNCTION_ID_FACT_LABEL,
    GHIDRA_ORACLE_FUNCTION_PROTOTYPE_FACT_LABEL, GHIDRA_ORACLE_HIGH_VARIABLE_FACT_LABEL,
    GHIDRA_ORACLE_JUMP_TABLE_FACT_LABEL, GHIDRA_ORACLE_PARAMETER_MEASURE_FACT_LABEL,
    GHIDRA_ORACLE_PCODE_OP_FACT_LABEL, GHIDRA_ORACLE_REFERENCE_FACT_LABEL,
    GHIDRA_ORACLE_SEMANTIC_SIGNATURE_FACT_LABEL, GHIDRA_ORACLE_STACK_FRAME_FACT_LABEL,
    GHIDRA_ORACLE_STRUCTURE_FIELD_ACCESS_FACT_LABEL, GHIDRA_ORACLE_SYMBOLIC_SUMMARY_FACT_LABEL,
    HAS_ANALYSIS_DRIFT, HAS_ANALYSIS_PASS, HAS_ANALYSIS_SCHEDULER_STATE, HAS_ANALYSIS_WORK_ITEM,
    HAS_ANALYZER_RECEIPT, HAS_DATA_FLOW_FACT, HAS_INSTRUCTION_FACT, HAS_LOADER_FACT,
    HAS_PROGRAM_PCODE_FACT, HAS_REFERENCE_RECOVERY_EVIDENCE, HAS_RUNTIME_TRACE,
    HAS_SEMANTIC_HYPOTHESIS, HAS_THIR_FUNCTION, INSTRUCTION_FACT_LABEL, LOADER_FACT_LABEL,
    ORACLE_FIXTURE_HAS_ANNOTATION_FACT, ORACLE_FIXTURE_HAS_CALL_EDGE_FACT,
    ORACLE_FIXTURE_HAS_CALL_STACK_EFFECT_FACT, ORACLE_FIXTURE_HAS_DATA_TYPE_FACT,
    ORACLE_FIXTURE_HAS_DECOMPILER_DIAGNOSTIC_FACT,
    ORACLE_FIXTURE_HAS_EQUATE_FACT, ORACLE_FIXTURE_HAS_EXTERNAL_LINKAGE_FACT,
    ORACLE_FIXTURE_HAS_FUNCTION_FACT, ORACLE_FIXTURE_HAS_FUNCTION_ID_FACT,
    ORACLE_FIXTURE_HAS_FUNCTION_PROTOTYPE_FACT, ORACLE_FIXTURE_HAS_HIGH_VARIABLE_FACT,
    ORACLE_FIXTURE_HAS_JUMP_TABLE_FACT, ORACLE_FIXTURE_HAS_PARAMETER_MEASURE_FACT,
    ORACLE_FIXTURE_HAS_PCODE_OP_FACT, ORACLE_FIXTURE_HAS_REFERENCE_FACT,
    ORACLE_FIXTURE_HAS_SEMANTIC_SIGNATURE_FACT, ORACLE_FIXTURE_HAS_STACK_FRAME_FACT,
    ORACLE_FIXTURE_HAS_STRUCTURE_FIELD_ACCESS_FACT, ORACLE_FIXTURE_HAS_SYMBOLIC_SUMMARY_FACT,
    PROGRAM_ANALYSIS_RUN_LABEL, PROGRAM_DATA_FLOW_FACT_LABEL, PROGRAM_PCODE_FACT_LABEL,
    PROGRAM_SEMANTIC_HYPOTHESIS_LABEL, REFERENCE_PRODUCES_RECOVERY_EVIDENCE,
    REFERENCE_RECOVERY_EVIDENCE_LABEL, RUNTIME_TRACE_LABEL, SCHEDULER_USES_PASS,
    SNAPSHOT_HAS_EVENT, SNAPSHOT_HAS_TAINT_MARK, TAINT_MARK_LABEL, THEOREM_IR_FUNCTION_LABEL,
    TRACE_EVENT_LABEL, TRACE_HAS_SNAPSHOT, TRACE_SNAPSHOT_LABEL, WORK_ITEM_USES_PASS,
};
pub use trace_to_contract::{
    compile_program_analysis_trace_engineering_in_memory,
    program_analysis_trace_to_engineering_input, TraceToEngineeringOptions,
};

pub const ENGINEERING_COMPILER_VERSION: &str = "0.1.0";
pub const ENGINEERING_COMPILE_LABEL: &str = "EngineeringCompile";
pub const ENGINEERING_ARCHITECTURE_LABEL: &str = "EngineeringArchitecture";
pub const ENGINEERING_BEHAVIOR_LABEL: &str = "EngineeringBehavior";
pub const ENGINEERING_API_LABEL: &str = "EngineeringApiContract";
pub const ENGINEERING_OBLIGATION_LABEL: &str = "EngineeringImplementationObligation";
pub const ENGINEERING_VALIDATOR_LABEL: &str = "EngineeringValidator";
pub const ENGINEERING_EVIDENCE_LABEL: &str = "EngineeringEvidence";

pub const HAS_ARCHITECTURE_MAP: &str = "HAS_ARCHITECTURE_MAP";
pub const HAS_BEHAVIOR: &str = "HAS_BEHAVIOR";
pub const HAS_API_CONTRACT: &str = "HAS_API_CONTRACT";
pub const HAS_IMPLEMENTATION_OBLIGATION: &str = "HAS_IMPLEMENTATION_OBLIGATION";
pub const HAS_VALIDATOR: &str = "HAS_VALIDATOR";
pub const HAS_EVIDENCE_SOURCE: &str = "HAS_EVIDENCE_SOURCE";

pub type EvidenceMap = BTreeMap<String, Vec<EvidenceSource>>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EngineeringCompileInput {
    pub tenant_id: String,
    pub repo_id: String,
    pub compiler_version: String,
    pub architecture_inputs: Vec<ObservedArchitectureInput>,
    pub behavior_inputs: Vec<ObservedBehaviorInput>,
    pub api_contract_inputs: Vec<ObservedApiContractInput>,
    pub implementation_obligation_inputs: Vec<ObservedImplementationObligationInput>,
    pub validator_inputs: Vec<ObservedValidatorSpecInput>,
    pub evidence_sources: Vec<EvidenceSource>,
}

impl EngineeringCompileInput {
    pub fn new(tenant_id: impl Into<String>, repo_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            compiler_version: ENGINEERING_COMPILER_VERSION.to_string(),
            architecture_inputs: Vec::new(),
            behavior_inputs: Vec::new(),
            api_contract_inputs: Vec::new(),
            implementation_obligation_inputs: Vec::new(),
            validator_inputs: Vec::new(),
            evidence_sources: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EngineeringCompileOutput {
    pub compile_id: String,
    pub tenant_id: String,
    pub repo_id: String,
    pub compile_version: String,
    pub architecture_map: ArchitectureMap,
    pub behavior_specs: Vec<BehaviorSpec>,
    pub api_contracts: Vec<ApiContract>,
    pub implementation_obligations: Vec<ImplementationObligation>,
    pub validator_specs: Vec<ValidatorSpec>,
    pub unknowns: UnknownsLedger,
    pub evidence_map: EvidenceMap,
    pub artifact_hash: String,
    pub graph_nodes: Vec<NodeRecord>,
    pub graph_edges: Vec<EdgeRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceAuthority {
    pub authority_id: String,
    pub org: String,
    pub role: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceSource {
    pub evidence_id: String,
    pub authority: EvidenceAuthority,
    pub summary: String,
    pub confidence_score: u8,
    pub targets: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UnknownsLedger {
    pub architecture: Vec<String>,
    pub behavior: Vec<String>,
    pub api_contract: Vec<String>,
    pub implementation_obligation: Vec<String>,
    pub validator: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureMap {
    pub architecture_map_id: String,
    pub scope: String,
    pub components: Vec<ArchitectureComponent>,
    pub evidence_ids: Vec<String>,
    pub validator_ids: Vec<String>,
    pub unknowns: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureComponent {
    pub component_id: String,
    pub component_name: String,
    pub description: String,
    pub evidence_ids: Vec<String>,
    pub validator_refs: Vec<String>,
    pub unknowns: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BehaviorSpec {
    pub behavior_id: String,
    pub behavior_name: String,
    pub description: String,
    pub evidence_ids: Vec<String>,
    pub validator_refs: Vec<String>,
    pub unknowns: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApiContract {
    pub contract_id: String,
    pub method: String,
    pub endpoint: String,
    pub request_schema: Value,
    pub response_schema: Value,
    pub evidence_ids: Vec<String>,
    pub validator_refs: Vec<String>,
    pub unknowns: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImplementationObligation {
    pub obligation_id: String,
    pub target: String,
    pub obligation: String,
    pub evidence_ids: Vec<String>,
    pub validator_refs: Vec<String>,
    pub unknowns: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidatorSpec {
    pub validator_id: String,
    pub target_id: String,
    pub validator_kind: String,
    pub rule: String,
    pub evidence_ids: Vec<String>,
    pub unknowns: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservedArchitectureInput {
    pub component_name: String,
    pub description: String,
    pub evidence_ids: Vec<String>,
    pub validator_refs: Vec<String>,
    pub unknowns: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservedBehaviorInput {
    pub behavior_name: String,
    pub description: String,
    pub evidence_ids: Vec<String>,
    pub validator_refs: Vec<String>,
    pub unknowns: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservedApiContractInput {
    pub method: String,
    pub endpoint: String,
    pub request_schema: Value,
    pub response_schema: Value,
    pub evidence_ids: Vec<String>,
    pub validator_refs: Vec<String>,
    pub unknowns: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservedImplementationObligationInput {
    pub target: String,
    pub obligation: String,
    pub evidence_ids: Vec<String>,
    pub validator_refs: Vec<String>,
    pub unknowns: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservedValidatorSpecInput {
    pub target_id: String,
    pub validator_kind: String,
    pub rule: String,
    pub evidence_ids: Vec<String>,
    pub unknowns: Vec<String>,
}

pub fn compile_engineering_in_memory(
    mut input: EngineeringCompileInput,
) -> EngineeringCompileOutput {
    normalize_input(&mut input);

    let compile_id = compile_id_for_input(&input);
    let unknowns = collect_unknowns(&input);
    let evidence_map = evidence_map_for_input(&input, &compile_id);

    let architecture_map = build_architecture_map(&input, &compile_id);
    let behavior_specs = build_behavior_specs(&input, &compile_id);
    let api_contracts = build_api_contracts(&input, &compile_id);
    let implementation_obligations = build_implementation_obligations(&input, &compile_id);
    let validator_specs = build_validator_specs(&input, &compile_id);
    let artifact_hash = stable_hash(json!({
        "tenant_id": &input.tenant_id,
        "repo_id": &input.repo_id,
        "compiler_version": &input.compiler_version,
        "architecture_map": &architecture_map,
        "behavior_specs": &behavior_specs,
        "api_contracts": &api_contracts,
        "implementation_obligations": &implementation_obligations,
        "validator_specs": &validator_specs,
        "unknowns": &unknowns,
        "evidence_map": &evidence_map,
    }));

    let (graph_nodes, graph_edges) = build_graph_payload(
        &input,
        &compile_id,
        &architecture_map,
        &behavior_specs,
        &api_contracts,
        &implementation_obligations,
        &validator_specs,
        &evidence_map,
        &unknowns,
        &artifact_hash,
    );

    EngineeringCompileOutput {
        compile_id,
        tenant_id: input.tenant_id,
        repo_id: input.repo_id,
        compile_version: input.compiler_version,
        architecture_map,
        behavior_specs,
        api_contracts,
        implementation_obligations,
        validator_specs,
        unknowns,
        evidence_map,
        artifact_hash,
        graph_nodes,
        graph_edges,
    }
}

pub fn compile_engineering_in_store<S: GraphStore>(
    store: &mut S,
    input: EngineeringCompileInput,
) -> GraphStoreResult<EngineeringCompileOutput> {
    let output = compile_engineering_in_memory(input);
    for node in &output.graph_nodes {
        store.upsert_node(node.clone())?;
    }
    for edge in &output.graph_edges {
        store.upsert_edge(edge.clone())?;
    }
    Ok(output)
}

fn normalize_input(input: &mut EngineeringCompileInput) {
    for item in &mut input.architecture_inputs {
        item.component_name = item.component_name.trim().to_string();
        item.description = item.description.trim().to_string();
        item.evidence_ids.sort();
        item.evidence_ids.dedup();
        item.validator_refs.sort();
        item.validator_refs.dedup();
        item.unknowns.sort();
        item.unknowns.dedup();
    }
    for item in &mut input.behavior_inputs {
        item.behavior_name = item.behavior_name.trim().to_string();
        item.description = item.description.trim().to_string();
        item.evidence_ids.sort();
        item.evidence_ids.dedup();
        item.validator_refs.sort();
        item.validator_refs.dedup();
        item.unknowns.sort();
        item.unknowns.dedup();
    }
    for item in &mut input.api_contract_inputs {
        item.endpoint = item.endpoint.trim().to_string();
        item.method = item.method.trim().to_string();
        item.evidence_ids.sort();
        item.evidence_ids.dedup();
        item.validator_refs.sort();
        item.validator_refs.dedup();
        item.unknowns.sort();
        item.unknowns.dedup();
    }
    for item in &mut input.implementation_obligation_inputs {
        item.target = item.target.trim().to_string();
        item.obligation = item.obligation.trim().to_string();
        item.evidence_ids.sort();
        item.evidence_ids.dedup();
        item.validator_refs.sort();
        item.validator_refs.dedup();
        item.unknowns.sort();
        item.unknowns.dedup();
    }
    for item in &mut input.validator_inputs {
        item.target_id = item.target_id.trim().to_string();
        item.validator_kind = item.validator_kind.trim().to_string();
        item.rule = item.rule.trim().to_string();
        item.evidence_ids.sort();
        item.evidence_ids.dedup();
        item.unknowns.sort();
        item.unknowns.dedup();
    }
    input.architecture_inputs.sort_by(|left, right| {
        left.component_name
            .cmp(&right.component_name)
            .then(left.description.cmp(&right.description))
    });
    input.behavior_inputs.sort_by(|left, right| {
        left.behavior_name
            .cmp(&right.behavior_name)
            .then(left.description.cmp(&right.description))
    });
    input.api_contract_inputs.sort_by(|left, right| {
        left.endpoint
            .cmp(&right.endpoint)
            .then(left.method.cmp(&right.method))
    });
    input
        .implementation_obligation_inputs
        .sort_by(|left, right| {
            left.target
                .cmp(&right.target)
                .then(left.obligation.cmp(&right.obligation))
        });
    input.validator_inputs.sort_by(|left, right| {
        left.target_id
            .cmp(&right.target_id)
            .then(left.rule.cmp(&right.rule))
    });
    input
        .evidence_sources
        .sort_by(|left, right| left.evidence_id.cmp(&right.evidence_id));
    for source in &mut input.evidence_sources {
        source.targets.sort();
        source.targets.dedup();
    }
}

fn build_architecture_map(
    input: &EngineeringCompileInput,
    tenant_context: &str,
) -> ArchitectureMap {
    let components = input
        .architecture_inputs
        .iter()
        .map(|input| {
            let component_id = format!(
                "engineering:architecture:{}",
                stable_hash(json!((
                    &input.component_name,
                    &input.description,
                    &input.evidence_ids,
                    &input.validator_refs,
                    &input.unknowns,
                    tenant_context,
                ))),
            );
            ArchitectureComponent {
                component_id,
                component_name: input.component_name.clone(),
                description: input.description.clone(),
                evidence_ids: input.evidence_ids.clone(),
                validator_refs: input.validator_refs.clone(),
                unknowns: input.unknowns.clone(),
            }
        })
        .collect::<Vec<_>>();

    let evidence_ids = merge_sorted_unique_strings(
        components
            .iter()
            .flat_map(|component| component.evidence_ids.iter().cloned())
            .collect(),
    );
    let validator_ids = merge_sorted_unique_strings(
        components
            .iter()
            .flat_map(|component| component.validator_refs.iter().cloned())
            .collect(),
    );
    let unknowns = merge_sorted_unique_strings(
        components
            .iter()
            .flat_map(|component| component.unknowns.iter().cloned())
            .collect(),
    );
    let architecture_map_id = format!(
        "engineering:architecture-map:{}",
        stable_hash(json!([
            &input.tenant_id,
            &input.repo_id,
            &components,
            tenant_context
        ])),
    );

    ArchitectureMap {
        architecture_map_id,
        scope: "repository".to_string(),
        components,
        evidence_ids,
        validator_ids,
        unknowns,
    }
}

fn build_behavior_specs(
    input: &EngineeringCompileInput,
    tenant_context: &str,
) -> Vec<BehaviorSpec> {
    input
        .behavior_inputs
        .iter()
        .map(|input| BehaviorSpec {
            behavior_id: format!(
                "engineering:behavior:{}",
                stable_hash(json!((
                    &input.behavior_name,
                    &input.description,
                    &input.evidence_ids,
                    &input.validator_refs,
                    &input.unknowns,
                    tenant_context,
                ))),
            ),
            behavior_name: input.behavior_name.clone(),
            description: input.description.clone(),
            evidence_ids: input.evidence_ids.clone(),
            validator_refs: input.validator_refs.clone(),
            unknowns: input.unknowns.clone(),
        })
        .collect()
}

fn build_api_contracts(input: &EngineeringCompileInput, tenant_context: &str) -> Vec<ApiContract> {
    input
        .api_contract_inputs
        .iter()
        .map(|input| ApiContract {
            contract_id: format!(
                "engineering:api:{}",
                stable_hash(json!((
                    &input.method,
                    &input.endpoint,
                    &input.request_schema,
                    &input.response_schema,
                    &input.evidence_ids,
                    &input.validator_refs,
                    &input.unknowns,
                    tenant_context,
                ))),
            ),
            method: input.method.clone(),
            endpoint: input.endpoint.clone(),
            request_schema: input.request_schema.clone(),
            response_schema: input.response_schema.clone(),
            evidence_ids: input.evidence_ids.clone(),
            validator_refs: input.validator_refs.clone(),
            unknowns: input.unknowns.clone(),
        })
        .collect()
}

fn build_implementation_obligations(
    input: &EngineeringCompileInput,
    tenant_context: &str,
) -> Vec<ImplementationObligation> {
    input
        .implementation_obligation_inputs
        .iter()
        .map(|input| ImplementationObligation {
            obligation_id: format!(
                "engineering:obligation:{}",
                stable_hash(json!((
                    &input.target,
                    &input.obligation,
                    &input.evidence_ids,
                    &input.validator_refs,
                    &input.unknowns,
                    tenant_context,
                ))),
            ),
            target: input.target.clone(),
            obligation: input.obligation.clone(),
            evidence_ids: input.evidence_ids.clone(),
            validator_refs: input.validator_refs.clone(),
            unknowns: input.unknowns.clone(),
        })
        .collect()
}

fn build_validator_specs(
    input: &EngineeringCompileInput,
    tenant_context: &str,
) -> Vec<ValidatorSpec> {
    input
        .validator_inputs
        .iter()
        .map(|input| ValidatorSpec {
            validator_id: format!(
                "engineering:validator:{}",
                stable_hash(json!((
                    &input.target_id,
                    &input.validator_kind,
                    &input.rule,
                    &input.evidence_ids,
                    &input.unknowns,
                    tenant_context,
                ))),
            ),
            target_id: input.target_id.clone(),
            validator_kind: input.validator_kind.clone(),
            rule: input.rule.clone(),
            evidence_ids: input.evidence_ids.clone(),
            unknowns: input.unknowns.clone(),
        })
        .collect()
}

fn collect_unknowns(input: &EngineeringCompileInput) -> UnknownsLedger {
    UnknownsLedger {
        architecture: merge_sorted_unique_strings(
            input
                .architecture_inputs
                .iter()
                .flat_map(|item| item.unknowns.iter().cloned())
                .collect(),
        ),
        behavior: merge_sorted_unique_strings(
            input
                .behavior_inputs
                .iter()
                .flat_map(|item| item.unknowns.iter().cloned())
                .collect(),
        ),
        api_contract: merge_sorted_unique_strings(
            input
                .api_contract_inputs
                .iter()
                .flat_map(|item| item.unknowns.iter().cloned())
                .collect(),
        ),
        implementation_obligation: merge_sorted_unique_strings(
            input
                .implementation_obligation_inputs
                .iter()
                .flat_map(|item| item.unknowns.iter().cloned())
                .collect(),
        ),
        validator: merge_sorted_unique_strings(
            input
                .validator_inputs
                .iter()
                .flat_map(|item| item.unknowns.iter().cloned())
                .collect(),
        ),
    }
}

fn evidence_map_for_input(input: &EngineeringCompileInput, compile_id: &str) -> EvidenceMap {
    let mut evidence_map: EvidenceMap = BTreeMap::new();
    for source in &input.evidence_sources {
        let targets = if source.targets.is_empty() {
            vec![compile_id.to_string()]
        } else {
            source.targets.clone()
        };
        for target in targets {
            let target_sources = evidence_map.entry(target).or_default();
            target_sources.push(source.clone());
        }
    }
    for sources in evidence_map.values_mut() {
        sources.sort_by(|left, right| left.evidence_id.cmp(&right.evidence_id));
        sources.dedup_by(|left, right| left.evidence_id == right.evidence_id);
    }
    evidence_map
}

#[allow(clippy::too_many_arguments)]
fn build_graph_payload(
    input: &EngineeringCompileInput,
    compile_id: &str,
    architecture_map: &ArchitectureMap,
    behavior_specs: &[BehaviorSpec],
    api_contracts: &[ApiContract],
    implementation_obligations: &[ImplementationObligation],
    validator_specs: &[ValidatorSpec],
    evidence_map: &EvidenceMap,
    unknowns: &UnknownsLedger,
    artifact_hash: &str,
) -> (Vec<NodeRecord>, Vec<EdgeRecord>) {
    let mut nodes = vec![NodeRecord::new(
        compile_id,
        [ENGINEERING_COMPILE_LABEL],
        json!({
            "tenant_id": &input.tenant_id,
            "repo_id": &input.repo_id,
            "artifact_hash": artifact_hash,
            "compile_version": &input.compiler_version,
            "source": SOURCE,
            "architecture_map_id": &architecture_map.architecture_map_id,
            "unknowns": json!({
                "architecture": &unknowns.architecture,
                "behavior": &unknowns.behavior,
                "api_contract": &unknowns.api_contract,
                "implementation_obligation": &unknowns.implementation_obligation,
                "validator": &unknowns.validator,
            }),
        }),
    )];
    let mut edges = Vec::new();

    let architecture_node_id = &architecture_map.architecture_map_id;
    nodes.push(NodeRecord::new(
        architecture_node_id,
        [ENGINEERING_ARCHITECTURE_LABEL],
        json!({
            "tenant_id": &input.tenant_id,
            "repo_id": &input.repo_id,
            "scope": &architecture_map.scope,
            "component_count": architecture_map.components.len(),
            "evidence_ids": &architecture_map.evidence_ids,
            "validator_ids": &architecture_map.validator_ids,
            "unknowns": &architecture_map.unknowns,
            "source": SOURCE,
        }),
    ));
    edges.push(EdgeRecord::new(
        edge_id(compile_id, "architecture", architecture_node_id),
        compile_id,
        HAS_ARCHITECTURE_MAP,
        architecture_node_id,
        json!({
            "tenant_id": &input.tenant_id,
            "repo_id": &input.repo_id,
            "source": SOURCE,
        }),
    ));
    for component in &architecture_map.components {
        let component_node_id = &component.component_id;
        nodes.push(NodeRecord::new(
            component_node_id,
            ["EngineeringArchitectureComponent"],
            json!({
                "tenant_id": &input.tenant_id,
                "repo_id": &input.repo_id,
                "component_name": &component.component_name,
                "description": &component.description,
                "evidence_ids": &component.evidence_ids,
                "validator_refs": &component.validator_refs,
                "unknowns": &component.unknowns,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(architecture_node_id, "component", component_node_id),
            architecture_node_id,
            "HAS_ARCHITECTURE_COMPONENT",
            component_node_id,
            json!({
                "tenant_id": &input.tenant_id,
                "repo_id": &input.repo_id,
                "source": SOURCE,
            }),
        ));
    }

    for behavior in behavior_specs {
        let behavior_id = &behavior.behavior_id;
        nodes.push(NodeRecord::new(
            behavior_id,
            [ENGINEERING_BEHAVIOR_LABEL],
            json!({
                "tenant_id": &input.tenant_id,
                "repo_id": &input.repo_id,
                "behavior_name": &behavior.behavior_name,
                "description": &behavior.description,
                "evidence_ids": &behavior.evidence_ids,
                "validator_refs": &behavior.validator_refs,
                "unknowns": &behavior.unknowns,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(compile_id, "behavior", behavior_id),
            compile_id,
            HAS_BEHAVIOR,
            behavior_id,
            json!({
                "tenant_id": &input.tenant_id,
                "repo_id": &input.repo_id,
                "source": SOURCE,
            }),
        ));
    }

    for contract in api_contracts {
        let contract_id = &contract.contract_id;
        nodes.push(NodeRecord::new(
            contract_id,
            [ENGINEERING_API_LABEL],
            json!({
                "tenant_id": &input.tenant_id,
                "repo_id": &input.repo_id,
                "method": &contract.method,
                "endpoint": &contract.endpoint,
                "request_schema": &contract.request_schema,
                "response_schema": &contract.response_schema,
                "evidence_ids": &contract.evidence_ids,
                "validator_refs": &contract.validator_refs,
                "unknowns": &contract.unknowns,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(compile_id, "api_contract", contract_id),
            compile_id,
            HAS_API_CONTRACT,
            contract_id,
            json!({
                "tenant_id": &input.tenant_id,
                "repo_id": &input.repo_id,
                "source": SOURCE,
            }),
        ));
    }

    for obligation in implementation_obligations {
        let obligation_id = &obligation.obligation_id;
        nodes.push(NodeRecord::new(
            obligation_id,
            [ENGINEERING_OBLIGATION_LABEL],
            json!({
                "tenant_id": &input.tenant_id,
                "repo_id": &input.repo_id,
                "target": &obligation.target,
                "obligation": &obligation.obligation,
                "evidence_ids": &obligation.evidence_ids,
                "validator_refs": &obligation.validator_refs,
                "unknowns": &obligation.unknowns,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(compile_id, "obligation", obligation_id),
            compile_id,
            HAS_IMPLEMENTATION_OBLIGATION,
            obligation_id,
            json!({
                "tenant_id": &input.tenant_id,
                "repo_id": &input.repo_id,
                "source": SOURCE,
            }),
        ));
    }

    for validator in validator_specs {
        let validator_id = &validator.validator_id;
        nodes.push(NodeRecord::new(
            validator_id,
            [ENGINEERING_VALIDATOR_LABEL],
            json!({
                "tenant_id": &input.tenant_id,
                "repo_id": &input.repo_id,
                "target_id": &validator.target_id,
                "validator_kind": &validator.validator_kind,
                "rule": &validator.rule,
                "evidence_ids": &validator.evidence_ids,
                "unknowns": &validator.unknowns,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(compile_id, "validator", validator_id),
            compile_id,
            HAS_VALIDATOR,
            validator_id,
            json!({
                "tenant_id": &input.tenant_id,
                "repo_id": &input.repo_id,
                "source": SOURCE,
            }),
        ));
    }

    let known_node_ids = nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    for (target, sources) in evidence_map {
        let edge_target = if known_node_ids.contains(target) {
            target.as_str()
        } else {
            compile_id
        };
        for source in sources {
            let evidence_node_id = evidence_node_id(target, source);
            nodes.push(NodeRecord::new(
                &evidence_node_id,
                [ENGINEERING_EVIDENCE_LABEL],
                json!({
                    "tenant_id": &input.tenant_id,
                    "repo_id": &input.repo_id,
                    "evidence_id": &source.evidence_id,
                    "authority_id": &source.authority.authority_id,
                    "authority_org": &source.authority.org,
                    "authority_role": &source.authority.role,
                    "summary": &source.summary,
                    "confidence_score": source.confidence_score,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(edge_target, "evidence", &evidence_node_id),
                edge_target,
                HAS_EVIDENCE_SOURCE,
                &evidence_node_id,
                json!({
                    "tenant_id": &input.tenant_id,
                    "repo_id": &input.repo_id,
                    "evidence_id": &source.evidence_id,
                    "declared_target": target,
                    "source": SOURCE,
                }),
            ));
        }
    }

    (nodes, edges)
}

fn compile_id_for_input(input: &EngineeringCompileInput) -> String {
    format!(
        "engineering:compile:{}",
        stable_hash(json!({
            "tenant_id": &input.tenant_id,
            "repo_id": &input.repo_id,
            "compiler_version": &input.compiler_version,
            "architecture_inputs": &input.architecture_inputs,
            "behavior_inputs": &input.behavior_inputs,
            "api_contract_inputs": &input.api_contract_inputs,
            "implementation_obligation_inputs": &input.implementation_obligation_inputs,
            "validator_inputs": &input.validator_inputs,
            "evidence_sources": &input.evidence_sources,
        })),
    )
}

fn evidence_node_id(target: &str, source: &EvidenceSource) -> String {
    format!(
        "engineering:evidence:{}",
        stable_hash(json!([target, &source.evidence_id, &source.authority])),
    )
}

fn edge_id(from_id: &str, edge_kind: &str, to_id: &str) -> String {
    format!(
        "engineering:edge:{}",
        stable_hash(json!([from_id, edge_kind, to_id]))
    )
}

fn merge_sorted_unique_strings(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture_input() -> EngineeringCompileInput {
        let mut input = EngineeringCompileInput::new("Travis-Gilbert", "repo:theorem");
        input.architecture_inputs.push(ObservedArchitectureInput {
            component_name: "router".to_string(),
            description: "HTTP and MCP ingress".to_string(),
            evidence_ids: vec!["e:router-1".to_string()],
            validator_refs: vec!["v-router".to_string()],
            unknowns: vec!["router-version".to_string()],
        });
        input.architecture_inputs.push(ObservedArchitectureInput {
            component_name: "compiler".to_string(),
            description: "Engineering compiler entrypoint".to_string(),
            evidence_ids: vec!["e:compiler-1".to_string()],
            validator_refs: vec!["v-compiler".to_string()],
            unknowns: vec!["compiler-coverage".to_string()],
        });
        input.behavior_inputs.push(ObservedBehaviorInput {
            behavior_name: "ingest".to_string(),
            description: "Observe and normalize compiler inputs".to_string(),
            evidence_ids: vec!["e:behavior-1".to_string()],
            validator_refs: vec!["v-behavior".to_string()],
            unknowns: vec!["ingest-coverage".to_string()],
        });
        input.api_contract_inputs.push(ObservedApiContractInput {
            method: "POST".to_string(),
            endpoint: "/compile".to_string(),
            request_schema: json!({"kind": "EngineeringCompileInput"}),
            response_schema: json!({"kind": "EngineeringCompileOutput"}),
            evidence_ids: vec!["e:api-1".to_string()],
            validator_refs: vec!["v-api".to_string()],
            unknowns: vec!["api-rate-limit".to_string()],
        });
        input
            .implementation_obligation_inputs
            .push(ObservedImplementationObligationInput {
                target: "api-contract".to_string(),
                obligation: "Schema compatibility preserved".to_string(),
                evidence_ids: vec!["e:obligation-1".to_string()],
                validator_refs: vec!["v-obligation".to_string()],
                unknowns: vec!["obligation-coverage".to_string()],
            });
        input.validator_inputs.push(ObservedValidatorSpecInput {
            target_id: "behavior:ingest".to_string(),
            validator_kind: "schema".to_string(),
            rule: "validate required fields are present".to_string(),
            evidence_ids: vec!["e:validator-1".to_string()],
            unknowns: vec!["validator-stability".to_string()],
        });
        input.evidence_sources.push(EvidenceSource {
            evidence_id: "e:router-1".to_string(),
            authority: EvidenceAuthority {
                authority_id: "engineer:repo-inspector".to_string(),
                org: "Travis-Gilbert".to_string(),
                role: "observer".to_string(),
            },
            summary: "Observed router component in file list".to_string(),
            confidence_score: 96,
            targets: vec!["engineering:architecture".to_string()],
        });
        input.evidence_sources.push(EvidenceSource {
            evidence_id: "e:api-1".to_string(),
            authority: EvidenceAuthority {
                authority_id: "engineer:contract-lens".to_string(),
                org: "Travis-Gilbert".to_string(),
                role: "inspector".to_string(),
            },
            summary: "Observed /compile endpoint in router input".to_string(),
            confidence_score: 93,
            targets: vec!["engineering:api".to_string()],
        });
        input
    }

    #[test]
    fn compile_engineering_output_is_deterministic() {
        let first = compile_engineering_in_memory(fixture_input());
        let second = compile_engineering_in_memory(fixture_input());

        assert_eq!(first.compile_id, second.compile_id);
        assert_eq!(first.artifact_hash, second.artifact_hash);
        assert_eq!(first.graph_nodes, second.graph_nodes);
        assert_eq!(first.graph_edges, second.graph_edges);
        assert_eq!(first.unknowns, second.unknowns);
    }

    #[test]
    fn compile_engineering_output_scopes_by_tenant_and_repo() {
        let input = fixture_input();
        let output_default = compile_engineering_in_memory(input.clone());
        let mut repo_misaligned = input;
        repo_misaligned.repo_id = "repo:other".to_string();
        let output_other_repo = compile_engineering_in_memory(repo_misaligned);

        assert_ne!(output_default.compile_id, output_other_repo.compile_id);
        for node in output_default.graph_nodes {
            assert_eq!(
                node.properties.get("tenant_id"),
                Some(&json!("Travis-Gilbert"))
            );
            assert_eq!(node.properties.get("repo_id"), Some(&json!("repo:theorem")));
        }
        for edge in output_default.graph_edges {
            assert_eq!(
                edge.properties.get("tenant_id"),
                Some(&json!("Travis-Gilbert"))
            );
            assert_eq!(edge.properties.get("repo_id"), Some(&json!("repo:theorem")));
        }
    }

    #[test]
    fn compile_engineering_output_carries_evidence_unknowns_validators() {
        let output = compile_engineering_in_memory(fixture_input());

        assert!(!output.evidence_map.is_empty());
        assert!(!output.unknowns.architecture.is_empty() && !output.unknowns.behavior.is_empty());
        assert!(!output.validator_specs.is_empty());
        assert!(!output.behavior_specs[0].evidence_ids.is_empty());
        assert!(!output.behavior_specs[0].unknowns.is_empty());
        assert!(!output.architecture_map.evidence_ids.is_empty());
    }

    #[test]
    fn compile_engineering_in_store_writes_graph_payload() {
        let mut store = rustyred_thg_core::InMemoryGraphStore::new();
        let output =
            compile_engineering_in_store(&mut store, fixture_input()).expect("compile writes");

        assert!(store.get_node(&output.compile_id).is_some());
        assert!(store
            .get_node(&output.architecture_map.architecture_map_id)
            .is_some());
        assert!(!output.graph_edges.is_empty());
    }
}
