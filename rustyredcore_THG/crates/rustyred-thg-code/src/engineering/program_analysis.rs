use std::collections::{BTreeMap, BTreeSet};

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
pub const PROGRAM_PCODE_FACT_LABEL: &str = "ProgramPcodeFact";
pub const REFERENCE_RECOVERY_EVIDENCE_LABEL: &str = "ReferenceRecoveryEvidence";
pub const ANALYZER_PASS_RECEIPT_LABEL: &str = "AnalyzerPassReceipt";
pub const ANALYSIS_PASS_SPEC_LABEL: &str = "AnalysisPassSpec";
pub const ANALYSIS_SCHEDULER_STATE_LABEL: &str = "AnalysisSchedulerState";
pub const ANALYSIS_WORK_ITEM_LABEL: &str = "AnalysisWorkItem";
pub const ANALYSIS_DRIFT_LABEL: &str = "AnalysisDrift";
pub const GHIDRA_ORACLE_FIXTURE_LABEL: &str = "GhidraOracleFixture";
pub const GHIDRA_ORACLE_FUNCTION_FACT_LABEL: &str = "GhidraOracleFunctionFact";
pub const GHIDRA_ORACLE_PCODE_OP_FACT_LABEL: &str = "GhidraOraclePcodeOpFact";
pub const GHIDRA_ORACLE_REFERENCE_FACT_LABEL: &str = "GhidraOracleReferenceFact";
pub const GHIDRA_ORACLE_CALL_EDGE_FACT_LABEL: &str = "GhidraOracleCallEdgeFact";
pub const GHIDRA_ORACLE_SYMBOLIC_SUMMARY_FACT_LABEL: &str = "GhidraOracleSymbolicSummaryFact";
pub const GHIDRA_ORACLE_DECOMPILER_DIAGNOSTIC_FACT_LABEL: &str =
    "GhidraOracleDecompilerDiagnosticFact";
pub const GHIDRA_ORACLE_ANNOTATION_FACT_LABEL: &str = "GhidraOracleAnnotationFact";
pub const GHIDRA_ORACLE_SEMANTIC_SIGNATURE_FACT_LABEL: &str = "GhidraOracleSemanticSignatureFact";
pub const GHIDRA_ORACLE_FUNCTION_ID_FACT_LABEL: &str = "GhidraOracleFunctionIdFact";
pub const GHIDRA_ORACLE_JUMP_TABLE_FACT_LABEL: &str = "GhidraOracleJumpTableFact";
pub const GHIDRA_ORACLE_EQUATE_FACT_LABEL: &str = "GhidraOracleEquateFact";
pub const GHIDRA_ORACLE_EXTERNAL_LINKAGE_FACT_LABEL: &str = "GhidraOracleExternalLinkageFact";
pub const GHIDRA_ORACLE_FUNCTION_PROTOTYPE_FACT_LABEL: &str = "GhidraOracleFunctionPrototypeFact";
pub const GHIDRA_ORACLE_DATA_TYPE_FACT_LABEL: &str = "GhidraOracleDataTypeFact";
pub const GHIDRA_ORACLE_STRUCTURE_FIELD_ACCESS_FACT_LABEL: &str =
    "GhidraOracleStructureFieldAccessFact";
pub const GHIDRA_ORACLE_HIGH_VARIABLE_FACT_LABEL: &str = "GhidraOracleHighVariableFact";
pub const GHIDRA_ORACLE_STACK_FRAME_FACT_LABEL: &str = "GhidraOracleStackFrameFact";
pub const GHIDRA_ORACLE_PARAMETER_MEASURE_FACT_LABEL: &str = "GhidraOracleParameterMeasureFact";
pub const GHIDRA_ORACLE_CALL_STACK_EFFECT_FACT_LABEL: &str = "GhidraOracleCallStackEffectFact";
pub const RUNTIME_TRACE_LABEL: &str = "RuntimeTrace";
pub const TRACE_SNAPSHOT_LABEL: &str = "TraceSnapshot";
pub const TRACE_EVENT_LABEL: &str = "TraceEvent";
pub const TAINT_MARK_LABEL: &str = "TaintMark";

pub const ANALYZES_ARTIFACT: &str = "ANALYZES_ARTIFACT";
pub const HAS_LOADER_FACT: &str = "HAS_LOADER_FACT";
pub const HAS_INSTRUCTION_FACT: &str = "HAS_INSTRUCTION_FACT";
pub const HAS_THIR_FUNCTION: &str = "HAS_THIR_FUNCTION";
pub const HAS_DATA_FLOW_FACT: &str = "HAS_DATA_FLOW_FACT";
pub const HAS_SEMANTIC_HYPOTHESIS: &str = "HAS_SEMANTIC_HYPOTHESIS";
pub const HAS_PROGRAM_PCODE_FACT: &str = "HAS_PROGRAM_PCODE_FACT";
pub const HAS_REFERENCE_RECOVERY_EVIDENCE: &str = "HAS_REFERENCE_RECOVERY_EVIDENCE";
pub const HAS_ANALYZER_RECEIPT: &str = "HAS_ANALYZER_RECEIPT";
pub const HAS_ANALYSIS_PASS: &str = "HAS_ANALYSIS_PASS";
pub const HAS_ANALYSIS_SCHEDULER_STATE: &str = "HAS_ANALYSIS_SCHEDULER_STATE";
pub const SCHEDULER_USES_PASS: &str = "SCHEDULER_USES_PASS";
pub const HAS_ANALYSIS_WORK_ITEM: &str = "HAS_ANALYSIS_WORK_ITEM";
pub const WORK_ITEM_USES_PASS: &str = "WORK_ITEM_USES_PASS";
pub const HAS_ANALYSIS_DRIFT: &str = "HAS_ANALYSIS_DRIFT";
pub const DERIVED_FROM_ORACLE: &str = "DERIVED_FROM_ORACLE";
pub const ORACLE_FIXTURE_HAS_FUNCTION_FACT: &str = "ORACLE_FIXTURE_HAS_FUNCTION_FACT";
pub const ORACLE_FIXTURE_HAS_PCODE_OP_FACT: &str = "ORACLE_FIXTURE_HAS_PCODE_OP_FACT";
pub const ORACLE_FIXTURE_HAS_REFERENCE_FACT: &str = "ORACLE_FIXTURE_HAS_REFERENCE_FACT";
pub const ORACLE_FIXTURE_HAS_CALL_EDGE_FACT: &str = "ORACLE_FIXTURE_HAS_CALL_EDGE_FACT";
pub const ORACLE_FIXTURE_HAS_SYMBOLIC_SUMMARY_FACT: &str =
    "ORACLE_FIXTURE_HAS_SYMBOLIC_SUMMARY_FACT";
pub const ORACLE_FIXTURE_HAS_DECOMPILER_DIAGNOSTIC_FACT: &str =
    "ORACLE_FIXTURE_HAS_DECOMPILER_DIAGNOSTIC_FACT";
pub const ORACLE_FIXTURE_HAS_ANNOTATION_FACT: &str = "ORACLE_FIXTURE_HAS_ANNOTATION_FACT";
pub const ORACLE_FIXTURE_HAS_SEMANTIC_SIGNATURE_FACT: &str =
    "ORACLE_FIXTURE_HAS_SEMANTIC_SIGNATURE_FACT";
pub const ORACLE_FIXTURE_HAS_FUNCTION_ID_FACT: &str = "ORACLE_FIXTURE_HAS_FUNCTION_ID_FACT";
pub const ORACLE_FIXTURE_HAS_JUMP_TABLE_FACT: &str = "ORACLE_FIXTURE_HAS_JUMP_TABLE_FACT";
pub const ORACLE_FIXTURE_HAS_EQUATE_FACT: &str = "ORACLE_FIXTURE_HAS_EQUATE_FACT";
pub const ORACLE_FIXTURE_HAS_EXTERNAL_LINKAGE_FACT: &str =
    "ORACLE_FIXTURE_HAS_EXTERNAL_LINKAGE_FACT";
pub const ORACLE_FIXTURE_HAS_FUNCTION_PROTOTYPE_FACT: &str =
    "ORACLE_FIXTURE_HAS_FUNCTION_PROTOTYPE_FACT";
pub const ORACLE_FIXTURE_HAS_DATA_TYPE_FACT: &str = "ORACLE_FIXTURE_HAS_DATA_TYPE_FACT";
pub const ORACLE_FIXTURE_HAS_STRUCTURE_FIELD_ACCESS_FACT: &str =
    "ORACLE_FIXTURE_HAS_STRUCTURE_FIELD_ACCESS_FACT";
pub const ORACLE_FIXTURE_HAS_HIGH_VARIABLE_FACT: &str = "ORACLE_FIXTURE_HAS_HIGH_VARIABLE_FACT";
pub const ORACLE_FIXTURE_HAS_STACK_FRAME_FACT: &str = "ORACLE_FIXTURE_HAS_STACK_FRAME_FACT";
pub const ORACLE_FIXTURE_HAS_PARAMETER_MEASURE_FACT: &str =
    "ORACLE_FIXTURE_HAS_PARAMETER_MEASURE_FACT";
pub const ORACLE_FIXTURE_HAS_CALL_STACK_EFFECT_FACT: &str =
    "ORACLE_FIXTURE_HAS_CALL_STACK_EFFECT_FACT";
pub const REFERENCE_PRODUCES_RECOVERY_EVIDENCE: &str = "REFERENCE_PRODUCES_RECOVERY_EVIDENCE";
pub const HAS_RUNTIME_TRACE: &str = "HAS_RUNTIME_TRACE";
pub const TRACE_HAS_SNAPSHOT: &str = "TRACE_HAS_SNAPSHOT";
pub const SNAPSHOT_HAS_EVENT: &str = "SNAPSHOT_HAS_EVENT";
pub const SNAPSHOT_HAS_TAINT_MARK: &str = "SNAPSHOT_HAS_TAINT_MARK";
pub const EVENT_HAS_TAINT_MARK: &str = "EVENT_HAS_TAINT_MARK";

const DEFAULT_TOOLCHAIN: &str = "theorem-program-analysis-v0";
const DEFAULT_PROFILE: &str = "ghidra-reference-contract-v0";
const AUTHORITY_OBSERVED_FACT: &str = "observed_fact";
const AUTHORITY_DERIVED_FACT: &str = "derived_fact";
const AUTHORITY_HYPOTHESIS: &str = "hypothesis";

fn default_true() -> bool {
    true
}

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
pub enum ProgramAnalyzerType {
    Byte,
    Instruction,
    Function,
    FunctionModifiers,
    FunctionSignatures,
    Data,
    RuntimeTrace,
    Taint,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AnalysisWorkStatus {
    Pending,
    Scheduled,
    Running,
    Complete,
    Failed,
    Canceled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisAddressRange {
    pub start: String,
    pub end: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisChangeSet {
    pub added_ranges: Vec<AnalysisAddressRange>,
    pub removed_ranges: Vec<AnalysisAddressRange>,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisPassSpec {
    pub analyzer_id: String,
    pub analyzer_type: ProgramAnalyzerType,
    pub description: String,
    pub priority: i32,
    pub default_enabled: bool,
    pub enabled: bool,
    pub supports_one_time: bool,
    pub can_analyze: bool,
    pub input_labels: Vec<String>,
    pub output_labels: Vec<String>,
    pub option_namespace: Option<String>,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisSchedulerState {
    pub scheduler_id: String,
    pub analyzer_id: String,
    pub analyzer_type: ProgramAnalyzerType,
    pub priority: i32,
    pub default_enabled: bool,
    pub enabled: bool,
    pub scheduled: bool,
    pub supports_one_time: bool,
    pub option_namespace: Option<String>,
    pub added_ranges: Vec<AnalysisAddressRange>,
    pub removed_ranges: Vec<AnalysisAddressRange>,
    pub input_hash: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisWorkItem {
    pub work_item_id: String,
    pub analyzer_id: String,
    pub analyzer_type: ProgramAnalyzerType,
    pub priority: i32,
    pub status: AnalysisWorkStatus,
    pub added_ranges: Vec<AnalysisAddressRange>,
    pub removed_ranges: Vec<AnalysisAddressRange>,
    pub input_hash: String,
    pub evidence_ids: Vec<String>,
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
pub struct ProgramPcodeFact {
    pub pcode_id: String,
    pub address: String,
    pub sequence: u64,
    pub opcode: String,
    pub ghidra_opcode_id: u8,
    pub inputs: Vec<String>,
    pub output: Option<String>,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReferenceRecoveryEvidence {
    pub evidence_id: String,
    pub source_reference_id: String,
    pub from_address: String,
    pub to_address: String,
    pub reference_type: String,
    pub semantic_roles: Vec<String>,
    pub operand_index: i32,
    pub source_type: Option<String>,
    pub confidence: u8,
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
pub enum AnalysisDriftKind {
    CountMismatch,
    MissingNativeFact,
    MissingOracleFact,
    FieldMismatch,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisDriftFact {
    pub drift_id: String,
    pub oracle_fixture_id: String,
    pub fact_kind: String,
    pub drift_kind: AnalysisDriftKind,
    pub expected: Value,
    pub observed: Value,
    pub severity: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleFunctionFact {
    pub function_id: String,
    pub entry_point: String,
    pub name: Option<String>,
    pub body_start: String,
    pub body_end: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOraclePcodeOpFact {
    pub pcode_id: String,
    pub address: String,
    pub sequence: u64,
    pub opcode: String,
    pub ghidra_opcode_id: u8,
    pub inputs: Vec<String>,
    pub output: Option<String>,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleReferenceFact {
    pub reference_id: String,
    pub from_address: String,
    pub to_address: String,
    pub reference_type: String,
    pub operand_index: i32,
    pub is_primary: bool,
    pub source_type: Option<String>,
    #[serde(default)]
    pub semantic_roles: Vec<String>,
    #[serde(default)]
    pub is_external: bool,
    #[serde(default)]
    pub is_memory: bool,
    #[serde(default)]
    pub is_register: bool,
    #[serde(default)]
    pub is_stack: bool,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleCallEdgeFact {
    pub edge_id: String,
    pub source_entry: String,
    pub target_entry: String,
    pub callsite_address: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleJumpTableCaseFact {
    pub case_id: String,
    pub destination: String,
    #[serde(default)]
    pub label_value: Option<i64>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleJumpTableLoadTableFact {
    pub load_table_id: String,
    pub address: String,
    pub entry_byte_len: u8,
    pub entry_count: usize,
    #[serde(default)]
    pub interpreted_as_pointer_table: bool,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleJumpTableFact {
    pub jump_table_id: String,
    #[serde(default)]
    pub function_id: Option<String>,
    pub entry_point: String,
    pub switch_address: String,
    #[serde(default)]
    pub switch_statement_id: Option<String>,
    #[serde(default)]
    pub display_format: Option<String>,
    #[serde(default)]
    pub cases: Vec<GhidraOracleJumpTableCaseFact>,
    #[serde(default)]
    pub load_tables: Vec<GhidraOracleJumpTableLoadTableFact>,
    #[serde(default)]
    pub override_applied: bool,
    #[serde(default = "default_true")]
    pub references_complete: bool,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleEquateReferenceFact {
    pub reference_id: String,
    pub address: String,
    #[serde(default)]
    pub operand_index: Option<i16>,
    #[serde(default)]
    pub dynamic_hash: Option<u64>,
    #[serde(default)]
    pub instruction_id: Option<String>,
    #[serde(default)]
    pub statement_id: Option<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleEquateFact {
    pub equate_id: String,
    #[serde(default)]
    pub function_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub value: i64,
    #[serde(default)]
    pub display_value: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub enum_uuid: Option<String>,
    #[serde(default)]
    pub enum_based: bool,
    #[serde(default = "default_true")]
    pub valid_uuid: bool,
    #[serde(default)]
    pub references: Vec<GhidraOracleEquateReferenceFact>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleExternalThunkLinkFact {
    pub link_id: String,
    pub function_id: String,
    pub address: String,
    #[serde(default)]
    pub target_function_id: Option<String>,
    #[serde(default)]
    pub target_address: Option<String>,
    pub recursive_depth: usize,
    #[serde(default)]
    pub is_terminal: bool,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleExternalLinkageFact {
    pub linkage_id: String,
    #[serde(default)]
    pub function_id: Option<String>,
    pub local_address: String,
    #[serde(default)]
    pub thunked_function_id: Option<String>,
    #[serde(default)]
    pub thunked_address: Option<String>,
    #[serde(default)]
    pub recursive_target_function_id: Option<String>,
    #[serde(default)]
    pub recursive_target_address: Option<String>,
    pub external_library: String,
    #[serde(default)]
    pub external_library_path: Option<String>,
    #[serde(default)]
    pub library_ordinal: Option<i32>,
    #[serde(default)]
    pub parent_namespace: Option<String>,
    pub external_label: String,
    #[serde(default)]
    pub original_imported_name: Option<String>,
    #[serde(default)]
    pub external_address: Option<String>,
    #[serde(default)]
    pub external_space_address: Option<String>,
    #[serde(default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub is_function: bool,
    #[serde(default)]
    pub data_type: Option<String>,
    #[serde(default)]
    pub function_signature: Option<String>,
    #[serde(default)]
    pub thunk_chain: Vec<GhidraOracleExternalThunkLinkFact>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleFunctionPrototypeParameterFact {
    pub ordinal: i32,
    #[serde(default)]
    pub name: Option<String>,
    pub data_type: String,
    #[serde(default)]
    pub data_type_id: Option<String>,
    #[serde(default)]
    pub data_type_kind: Option<String>,
    #[serde(default)]
    pub data_type_byte_len: Option<u64>,
    pub storage: GhidraOracleVariableStorageFact,
    #[serde(default)]
    pub auto_parameter: bool,
    #[serde(default)]
    pub auto_parameter_type: Option<String>,
    #[serde(default)]
    pub forced_indirect: bool,
    #[serde(default)]
    pub formal_data_type: Option<String>,
    #[serde(default)]
    pub formal_data_type_id: Option<String>,
    #[serde(default)]
    pub formal_data_type_kind: Option<String>,
    #[serde(default)]
    pub formal_data_type_byte_len: Option<u64>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleFunctionPrototypeFact {
    pub prototype_id: String,
    #[serde(default)]
    pub function_id: Option<String>,
    pub entry_point: String,
    #[serde(default)]
    pub name: Option<String>,
    pub prototype: String,
    #[serde(default)]
    pub calling_convention: Option<String>,
    pub return_type: String,
    #[serde(default)]
    pub return_type_id: Option<String>,
    #[serde(default)]
    pub return_type_kind: Option<String>,
    #[serde(default)]
    pub return_type_byte_len: Option<u64>,
    pub return_storage: GhidraOracleVariableStorageFact,
    #[serde(default)]
    pub parameters: Vec<GhidraOracleFunctionPrototypeParameterFact>,
    #[serde(default)]
    pub has_varargs: bool,
    #[serde(default)]
    pub has_no_return: bool,
    #[serde(default)]
    pub is_inline: bool,
    #[serde(default)]
    pub is_thunk: bool,
    #[serde(default)]
    pub thunked_function_id: Option<String>,
    #[serde(default)]
    pub thunked_entry_point: Option<String>,
    #[serde(default)]
    pub has_custom_storage: bool,
    #[serde(default)]
    pub stack_purge_size: Option<i32>,
    #[serde(default)]
    pub signature_source: Option<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleEnumValueFact {
    pub value_id: String,
    pub name: String,
    pub value: i64,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleDataTypeFieldFact {
    pub field_id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub ordinal: i32,
    pub offset: i64,
    #[serde(default)]
    pub byte_len: Option<u64>,
    #[serde(default)]
    pub bit_offset: Option<u32>,
    #[serde(default)]
    pub bit_size: Option<u32>,
    #[serde(default)]
    pub data_type_id: Option<String>,
    pub data_type_name: String,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleDataTypeFact {
    pub type_id: String,
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub category_path: Option<String>,
    #[serde(default)]
    pub path_name: Option<String>,
    #[serde(default)]
    pub universal_id: Option<String>,
    #[serde(default)]
    pub byte_len: Option<u64>,
    #[serde(default)]
    pub aligned_byte_len: Option<u64>,
    #[serde(default)]
    pub component_count: usize,
    #[serde(default)]
    pub packing_enabled: Option<bool>,
    #[serde(default)]
    pub packing_value: Option<i32>,
    #[serde(default)]
    pub minimum_alignment: Option<i32>,
    #[serde(default)]
    pub not_yet_defined: bool,
    #[serde(default)]
    pub zero_length: bool,
    #[serde(default)]
    pub base_type_id: Option<String>,
    #[serde(default)]
    pub base_type_name: Option<String>,
    #[serde(default)]
    pub element_type_id: Option<String>,
    #[serde(default)]
    pub element_type_name: Option<String>,
    #[serde(default)]
    pub element_count: Option<u64>,
    #[serde(default)]
    pub element_byte_len: Option<u64>,
    #[serde(default)]
    pub enum_signed: Option<bool>,
    #[serde(default)]
    pub enum_values: Vec<GhidraOracleEnumValueFact>,
    #[serde(default)]
    pub fields: Vec<GhidraOracleDataTypeFieldFact>,
    #[serde(default)]
    pub hard_dependency_ids: Vec<String>,
    #[serde(default)]
    pub soft_dependency_ids: Vec<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleStructureFieldAccessFact {
    pub access_id: String,
    #[serde(default)]
    pub function_id: Option<String>,
    pub entry_point: String,
    pub address: String,
    pub access_kind: String,
    pub structure_type_id: String,
    pub structure_name: String,
    #[serde(default)]
    pub root_variable_id: Option<String>,
    #[serde(default)]
    pub root_variable_name: Option<String>,
    #[serde(default)]
    pub root_storage: Option<String>,
    #[serde(default)]
    pub field_id: Option<String>,
    #[serde(default)]
    pub field_name: Option<String>,
    pub field_offset: i64,
    #[serde(default)]
    pub field_byte_len: Option<u64>,
    #[serde(default)]
    pub field_data_type_id: Option<String>,
    pub field_data_type_name: String,
    #[serde(default)]
    pub field_data_type_kind: Option<String>,
    #[serde(default)]
    pub field_data_type_byte_len: Option<u64>,
    pub pcode_opcode: String,
    #[serde(default)]
    pub pcode_op_id: Option<String>,
    #[serde(default)]
    pub statement_id: Option<String>,
    #[serde(default)]
    pub call_target_address: Option<String>,
    #[serde(default)]
    pub call_input_slot: Option<u32>,
    #[serde(default)]
    pub pointer_relative_type: Option<String>,
    #[serde(default)]
    pub recursive_call_depth: u8,
    #[serde(default)]
    pub creates_new_structure: bool,
    #[serde(default)]
    pub extends_existing_structure: bool,
    #[serde(default)]
    pub bit_offset: Option<u32>,
    #[serde(default)]
    pub bit_size: Option<u32>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleVariableStoragePieceFact {
    pub piece_id: String,
    pub space: String,
    pub offset: i64,
    pub byte_len: u64,
    #[serde(default)]
    pub register: Option<String>,
    #[serde(default)]
    pub is_input: bool,
    #[serde(default)]
    pub is_addr_tied: bool,
    #[serde(default)]
    pub is_persistent: bool,
    #[serde(default)]
    pub is_unique: bool,
    #[serde(default)]
    pub is_constant: bool,
    #[serde(default)]
    pub is_hash: bool,
    #[serde(default)]
    pub is_stack: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleVariableStorageFact {
    pub storage: String,
    pub kind: String,
    pub byte_len: u64,
    #[serde(default)]
    pub pieces: Vec<GhidraOracleVariableStoragePieceFact>,
    #[serde(default)]
    pub dynamic_storage_required: bool,
    #[serde(default)]
    pub forced_indirect: bool,
    #[serde(default)]
    pub auto_parameter_type: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleHighVariableInstanceFact {
    pub instance_id: String,
    pub storage: GhidraOracleVariableStorageFact,
    #[serde(default)]
    pub pc_address: Option<String>,
    #[serde(default)]
    pub defining_pcode_id: Option<String>,
    #[serde(default)]
    pub merge_group: Option<i16>,
    #[serde(default)]
    pub is_representative: bool,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleHighVariableFact {
    pub variable_id: String,
    #[serde(default)]
    pub function_id: Option<String>,
    pub entry_point: String,
    #[serde(default)]
    pub symbol_id: Option<String>,
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub category_index: Option<i32>,
    pub data_type: String,
    #[serde(default)]
    pub data_type_id: Option<String>,
    pub storage: GhidraOracleVariableStorageFact,
    #[serde(default)]
    pub first_use_address: Option<String>,
    #[serde(default)]
    pub first_use_offset: Option<i64>,
    #[serde(default)]
    pub name_locked: bool,
    #[serde(default)]
    pub type_locked: bool,
    #[serde(default)]
    pub isolated: bool,
    #[serde(default)]
    pub is_this_pointer: bool,
    #[serde(default)]
    pub is_hidden_return: bool,
    #[serde(default)]
    pub mutability: String,
    #[serde(default)]
    pub high_symbol_offset: Option<i64>,
    #[serde(default)]
    pub instances: Vec<GhidraOracleHighVariableInstanceFact>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleStackFrameVariableFact {
    pub variable_id: String,
    pub name: String,
    pub kind: String,
    pub offset: i64,
    pub byte_len: u64,
    #[serde(default)]
    pub ordinal: Option<i32>,
    pub data_type: String,
    #[serde(default)]
    pub data_type_id: Option<String>,
    pub storage: GhidraOracleVariableStorageFact,
    #[serde(default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub high_variable_id: Option<String>,
    #[serde(default)]
    pub name_locked: bool,
    #[serde(default)]
    pub type_locked: bool,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleStackFrameFact {
    pub frame_id: String,
    #[serde(default)]
    pub function_id: Option<String>,
    pub entry_point: String,
    pub frame_size: u32,
    pub local_size: u32,
    pub parameter_size: u32,
    #[serde(default)]
    pub parameter_offset: Option<i32>,
    #[serde(default)]
    pub return_address_offset: Option<i32>,
    pub growth: String,
    #[serde(default)]
    pub stack_pointer_register: Option<String>,
    #[serde(default)]
    pub custom_variable_storage: bool,
    #[serde(default)]
    pub variables: Vec<GhidraOracleStackFrameVariableFact>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleParameterMeasureFact {
    pub measure_id: String,
    #[serde(default)]
    pub function_id: Option<String>,
    pub entry_point: String,
    pub io: String,
    pub rank: String,
    #[serde(default)]
    pub rank_value: Option<u8>,
    pub storage: GhidraOracleVariableStorageFact,
    pub data_type: String,
    #[serde(default)]
    pub data_type_id: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub extra_pop: Option<i32>,
    #[serde(default)]
    pub just_prototype: bool,
    #[serde(default)]
    pub base_variable_id: Option<String>,
    #[serde(default)]
    pub source_statement_id: Option<String>,
    #[serde(default)]
    pub num_calls: u32,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleCallStackEffectFact {
    pub effect_id: String,
    #[serde(default)]
    pub function_id: Option<String>,
    pub entry_point: String,
    pub callsite_address: String,
    #[serde(default)]
    pub callee_function_id: Option<String>,
    #[serde(default)]
    pub callee_name: Option<String>,
    #[serde(default)]
    pub call_opcode: Option<String>,
    #[serde(default)]
    pub prototype_model: Option<String>,
    #[serde(default)]
    pub stack_pointer_register: Option<String>,
    #[serde(default)]
    pub stack_space: Option<String>,
    #[serde(default)]
    pub stack_offset_before_call: Option<i64>,
    #[serde(default)]
    pub instruction_stack_depth_change: Option<i32>,
    #[serde(default)]
    pub stack_shift_bytes: Option<i32>,
    #[serde(default)]
    pub purge_size_bytes: Option<i32>,
    #[serde(default)]
    pub extra_pop_bytes: Option<i32>,
    #[serde(default)]
    pub effective_extra_pop_bytes: Option<i32>,
    #[serde(default)]
    pub companion_solution_bytes: Option<i32>,
    #[serde(default)]
    pub solver_variable_count: u32,
    #[serde(default)]
    pub missed_variable_count: u32,
    pub status: String,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleAnnotationFact {
    pub annotation_id: String,
    pub kind: String,
    #[serde(default)]
    pub function_id: Option<String>,
    #[serde(default)]
    pub entry_point: Option<String>,
    pub address: String,
    #[serde(default)]
    pub comment_type: Option<String>,
    #[serde(default)]
    pub bookmark_id: Option<i64>,
    #[serde(default)]
    pub bookmark_type: Option<String>,
    #[serde(default)]
    pub bookmark_category: Option<String>,
    pub message: String,
    #[serde(default)]
    pub source_api: Option<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleSymbolicModelBinding {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub value_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleSymbolicPreconditionFact {
    pub precondition_id: String,
    pub step_index: u64,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub pcode_op_id: Option<String>,
    pub branch_taken: bool,
    pub serialized_expr: String,
    #[serde(default)]
    pub display_expr: Option<String>,
    pub solver_status: String,
    #[serde(default)]
    pub model_bindings: Vec<GhidraOracleSymbolicModelBinding>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleSymbolicValueFact {
    pub value_id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub space: Option<String>,
    #[serde(default)]
    pub offset: Option<String>,
    #[serde(default)]
    pub byte_len: Option<u32>,
    pub serialized_expr: String,
    #[serde(default)]
    pub display_expr: Option<String>,
    #[serde(default)]
    pub concrete_value: Option<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleMemoryWitnessFact {
    pub witness_id: String,
    pub kind: String,
    pub address_expr: String,
    #[serde(default)]
    pub address_display: Option<String>,
    pub byte_len: u32,
    #[serde(default)]
    pub value_expr: Option<String>,
    #[serde(default)]
    pub value_display: Option<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleSymbolicSummaryFact {
    pub summary_id: String,
    #[serde(default)]
    pub function_id: Option<String>,
    pub entry_point: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub preconditions: Vec<GhidraOracleSymbolicPreconditionFact>,
    #[serde(default)]
    pub registers_read: Vec<String>,
    #[serde(default)]
    pub registers_updated: Vec<String>,
    #[serde(default)]
    pub symbolic_values: Vec<GhidraOracleSymbolicValueFact>,
    #[serde(default)]
    pub memory_witnesses: Vec<GhidraOracleMemoryWitnessFact>,
    pub solver_status: String,
    #[serde(default)]
    pub model_bindings: Vec<GhidraOracleSymbolicModelBinding>,
    #[serde(default)]
    pub valuation_hash: String,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleDecompilerDiagnosticFact {
    pub diagnostic_id: String,
    #[serde(default)]
    pub function_id: Option<String>,
    pub entry_point: String,
    #[serde(default)]
    pub address: Option<String>,
    pub category: String,
    pub severity: String,
    pub placement: String,
    pub message: String,
    #[serde(default)]
    pub source_pass: Option<String>,
    #[serde(default)]
    pub source_rule: Option<String>,
    #[serde(default)]
    pub affects_control_flow: bool,
    #[serde(default)]
    pub affects_prototype: bool,
    #[serde(default)]
    pub affects_data_flow: bool,
    #[serde(default)]
    pub remediation: Option<String>,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub cancelled: bool,
    #[serde(default)]
    pub failed_to_start: bool,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleDebugSignatureFeature {
    pub hash: String,
    pub kind: String,
    pub raw: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleSemanticSignatureFact {
    pub signature_id: String,
    #[serde(default)]
    pub function_id: Option<String>,
    pub entry_point: String,
    #[serde(default)]
    pub function_name: Option<String>,
    pub feature_version: String,
    pub signature_settings: u32,
    #[serde(default)]
    pub decompiler_major_version: Option<u16>,
    #[serde(default)]
    pub decompiler_minor_version: Option<u16>,
    pub feature_hashes: Vec<String>,
    #[serde(default)]
    pub debug_features: Vec<GhidraOracleDebugSignatureFeature>,
    #[serde(default)]
    pub call_targets: Vec<String>,
    #[serde(default)]
    pub has_unimplemented: bool,
    #[serde(default)]
    pub has_bad_data: bool,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleFunctionIdMatchFact {
    pub match_id: String,
    pub function_name: String,
    #[serde(default)]
    pub library_family_name: Option<String>,
    #[serde(default)]
    pub library_version: Option<String>,
    #[serde(default)]
    pub library_variant: Option<String>,
    #[serde(default)]
    pub library_id: Option<String>,
    #[serde(default)]
    pub function_record_id: Option<String>,
    #[serde(default)]
    pub domain_path: Option<String>,
    #[serde(default)]
    pub matched_entry_point: Option<String>,
    #[serde(default)]
    pub primary_function_match_mode: Option<String>,
    #[serde(default)]
    pub primary_function_code_unit_score: f64,
    #[serde(default)]
    pub child_function_code_unit_score: f64,
    #[serde(default)]
    pub parent_function_code_unit_score: f64,
    #[serde(default)]
    pub overall_score: f64,
    #[serde(default)]
    pub auto_pass: bool,
    #[serde(default)]
    pub auto_fail: bool,
    #[serde(default)]
    pub force_specific: bool,
    #[serde(default)]
    pub force_relation: bool,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleFunctionIdFact {
    pub fid_id: String,
    #[serde(default)]
    pub function_id: Option<String>,
    pub entry_point: String,
    #[serde(default)]
    pub function_name: Option<String>,
    pub feature_version: String,
    pub hash_algorithm: String,
    pub short_hash_code_unit_length: u8,
    pub medium_hash_code_unit_length: u8,
    pub code_unit_size: u16,
    pub full_hash: String,
    pub specific_hash_additional_size: u8,
    pub specific_hash: String,
    #[serde(default)]
    pub matches: Vec<GhidraOracleFunctionIdMatchFact>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TraceScheduleSource {
    Input,
    Record,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TraceScheduleForm {
    SnapOnly,
    SnapEventSteps,
    SnapAnySteps,
    SnapAnyStepsOps,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TraceScheduleSpec {
    pub schedule: String,
    pub snap: i64,
    pub instruction_steps: u64,
    pub pcode_steps: u64,
    pub source: TraceScheduleSource,
    pub form: TraceScheduleForm,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeTrace {
    pub trace_id: String,
    pub language_id: Option<String>,
    pub compiler_spec_id: Option<String>,
    pub emulator_cache_version: Option<String>,
    pub capture_source: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TraceSnapshotFact {
    pub snapshot_id: String,
    pub trace_id: String,
    pub snap: i64,
    pub description: String,
    pub real_time_ms: Option<u64>,
    pub event_thread_id: Option<String>,
    pub schedule: TraceScheduleSpec,
    pub version: u64,
    pub forked: bool,
    pub stale: bool,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TraceEventKind {
    ThreadStep,
    PcodeStep,
    MemoryRead,
    MemoryWrite,
    RegisterRead,
    RegisterWrite,
    Syscall,
    Network,
    BranchDecision,
    Breakpoint,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TraceEventFact {
    pub event_id: String,
    pub trace_id: String,
    pub snapshot_id: String,
    pub sequence: u64,
    pub thread_id: Option<String>,
    pub kind: TraceEventKind,
    pub address_space: Option<String>,
    pub offset: Option<String>,
    pub size: Option<u64>,
    pub register: Option<String>,
    pub value_hash: Option<String>,
    pub pcode_op: Option<String>,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaintMark {
    pub taint_id: String,
    pub trace_id: String,
    pub snapshot_id: String,
    pub event_id: Option<String>,
    pub address_space: String,
    pub offset: String,
    pub size: u64,
    pub labels: Vec<String>,
    pub originating_op: Option<String>,
    pub indirect_read: bool,
    pub indirect_write: bool,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    pub pcode_facts: Vec<ProgramPcodeFact>,
    pub reference_recovery_evidence: Vec<ReferenceRecoveryEvidence>,
    pub semantic_hypotheses: Vec<ProgramSemanticHypothesis>,
    pub analyzer_receipts: Vec<AnalyzerPassReceipt>,
    pub analysis_passes: Vec<AnalysisPassSpec>,
    pub analysis_scheduler_states: Vec<AnalysisSchedulerState>,
    pub analysis_work_items: Vec<AnalysisWorkItem>,
    pub analysis_drifts: Vec<AnalysisDriftFact>,
    pub runtime_traces: Vec<RuntimeTrace>,
    pub trace_snapshots: Vec<TraceSnapshotFact>,
    pub trace_events: Vec<TraceEventFact>,
    pub taint_marks: Vec<TaintMark>,
    pub oracle_fixture: Option<GhidraOracleFixture>,
    pub oracle_function_facts: Vec<GhidraOracleFunctionFact>,
    pub oracle_pcode_facts: Vec<GhidraOraclePcodeOpFact>,
    pub oracle_reference_facts: Vec<GhidraOracleReferenceFact>,
    pub oracle_call_edge_facts: Vec<GhidraOracleCallEdgeFact>,
    pub oracle_jump_table_facts: Vec<GhidraOracleJumpTableFact>,
    pub oracle_equate_facts: Vec<GhidraOracleEquateFact>,
    pub oracle_external_linkage_facts: Vec<GhidraOracleExternalLinkageFact>,
    pub oracle_function_prototype_facts: Vec<GhidraOracleFunctionPrototypeFact>,
    pub oracle_data_type_facts: Vec<GhidraOracleDataTypeFact>,
    pub oracle_structure_field_access_facts: Vec<GhidraOracleStructureFieldAccessFact>,
    pub oracle_high_variable_facts: Vec<GhidraOracleHighVariableFact>,
    pub oracle_stack_frame_facts: Vec<GhidraOracleStackFrameFact>,
    pub oracle_parameter_measure_facts: Vec<GhidraOracleParameterMeasureFact>,
    pub oracle_call_stack_effect_facts: Vec<GhidraOracleCallStackEffectFact>,
    pub oracle_annotation_facts: Vec<GhidraOracleAnnotationFact>,
    pub oracle_symbolic_summary_facts: Vec<GhidraOracleSymbolicSummaryFact>,
    pub oracle_decompiler_diagnostic_facts: Vec<GhidraOracleDecompilerDiagnosticFact>,
    pub oracle_semantic_signature_facts: Vec<GhidraOracleSemanticSignatureFact>,
    pub oracle_function_id_facts: Vec<GhidraOracleFunctionIdFact>,
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
            pcode_facts: Vec::new(),
            reference_recovery_evidence: Vec::new(),
            semantic_hypotheses: Vec::new(),
            analyzer_receipts: Vec::new(),
            analysis_passes: Vec::new(),
            analysis_scheduler_states: Vec::new(),
            analysis_work_items: Vec::new(),
            analysis_drifts: Vec::new(),
            runtime_traces: Vec::new(),
            trace_snapshots: Vec::new(),
            trace_events: Vec::new(),
            taint_marks: Vec::new(),
            oracle_fixture: None,
            oracle_function_facts: Vec::new(),
            oracle_pcode_facts: Vec::new(),
            oracle_reference_facts: Vec::new(),
            oracle_call_edge_facts: Vec::new(),
            oracle_jump_table_facts: Vec::new(),
            oracle_equate_facts: Vec::new(),
            oracle_external_linkage_facts: Vec::new(),
            oracle_function_prototype_facts: Vec::new(),
            oracle_data_type_facts: Vec::new(),
            oracle_structure_field_access_facts: Vec::new(),
            oracle_high_variable_facts: Vec::new(),
            oracle_stack_frame_facts: Vec::new(),
            oracle_parameter_measure_facts: Vec::new(),
            oracle_call_stack_effect_facts: Vec::new(),
            oracle_annotation_facts: Vec::new(),
            oracle_symbolic_summary_facts: Vec::new(),
            oracle_decompiler_diagnostic_facts: Vec::new(),
            oracle_semantic_signature_facts: Vec::new(),
            oracle_function_id_facts: Vec::new(),
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
    pub pcode_facts: Vec<ProgramPcodeFact>,
    pub reference_recovery_evidence: Vec<ReferenceRecoveryEvidence>,
    pub semantic_hypotheses: Vec<ProgramSemanticHypothesis>,
    pub analyzer_receipts: Vec<AnalyzerPassReceipt>,
    pub analysis_passes: Vec<AnalysisPassSpec>,
    pub analysis_scheduler_states: Vec<AnalysisSchedulerState>,
    pub analysis_work_items: Vec<AnalysisWorkItem>,
    pub analysis_drifts: Vec<AnalysisDriftFact>,
    pub runtime_traces: Vec<RuntimeTrace>,
    pub trace_snapshots: Vec<TraceSnapshotFact>,
    pub trace_events: Vec<TraceEventFact>,
    pub taint_marks: Vec<TaintMark>,
    pub oracle_function_facts: Vec<GhidraOracleFunctionFact>,
    pub oracle_pcode_facts: Vec<GhidraOraclePcodeOpFact>,
    pub oracle_reference_facts: Vec<GhidraOracleReferenceFact>,
    pub oracle_call_edge_facts: Vec<GhidraOracleCallEdgeFact>,
    pub oracle_jump_table_facts: Vec<GhidraOracleJumpTableFact>,
    pub oracle_equate_facts: Vec<GhidraOracleEquateFact>,
    pub oracle_external_linkage_facts: Vec<GhidraOracleExternalLinkageFact>,
    pub oracle_function_prototype_facts: Vec<GhidraOracleFunctionPrototypeFact>,
    pub oracle_data_type_facts: Vec<GhidraOracleDataTypeFact>,
    pub oracle_structure_field_access_facts: Vec<GhidraOracleStructureFieldAccessFact>,
    pub oracle_high_variable_facts: Vec<GhidraOracleHighVariableFact>,
    pub oracle_stack_frame_facts: Vec<GhidraOracleStackFrameFact>,
    pub oracle_parameter_measure_facts: Vec<GhidraOracleParameterMeasureFact>,
    pub oracle_call_stack_effect_facts: Vec<GhidraOracleCallStackEffectFact>,
    pub oracle_annotation_facts: Vec<GhidraOracleAnnotationFact>,
    pub oracle_symbolic_summary_facts: Vec<GhidraOracleSymbolicSummaryFact>,
    pub oracle_decompiler_diagnostic_facts: Vec<GhidraOracleDecompilerDiagnosticFact>,
    pub oracle_semantic_signature_facts: Vec<GhidraOracleSemanticSignatureFact>,
    pub oracle_function_id_facts: Vec<GhidraOracleFunctionIdFact>,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GhidraOracleExport {
    pub fixture: GhidraOracleFixture,
    #[serde(default)]
    pub functions: Vec<GhidraOracleFunctionFact>,
    #[serde(default)]
    pub pcode_ops: Vec<GhidraOraclePcodeOpFact>,
    #[serde(default)]
    pub references: Vec<GhidraOracleReferenceFact>,
    #[serde(default)]
    pub call_edges: Vec<GhidraOracleCallEdgeFact>,
    #[serde(default)]
    pub jump_tables: Vec<GhidraOracleJumpTableFact>,
    #[serde(default)]
    pub equates: Vec<GhidraOracleEquateFact>,
    #[serde(default)]
    pub external_linkages: Vec<GhidraOracleExternalLinkageFact>,
    #[serde(default)]
    pub function_prototypes: Vec<GhidraOracleFunctionPrototypeFact>,
    #[serde(default)]
    pub data_types: Vec<GhidraOracleDataTypeFact>,
    #[serde(default)]
    pub structure_field_accesses: Vec<GhidraOracleStructureFieldAccessFact>,
    #[serde(default)]
    pub high_variables: Vec<GhidraOracleHighVariableFact>,
    #[serde(default)]
    pub stack_frames: Vec<GhidraOracleStackFrameFact>,
    #[serde(default)]
    pub parameter_measures: Vec<GhidraOracleParameterMeasureFact>,
    #[serde(default)]
    pub call_stack_effects: Vec<GhidraOracleCallStackEffectFact>,
    #[serde(default)]
    pub annotations: Vec<GhidraOracleAnnotationFact>,
    #[serde(default)]
    pub symbolic_summaries: Vec<GhidraOracleSymbolicSummaryFact>,
    #[serde(default)]
    pub diagnostics: Vec<GhidraOracleDecompilerDiagnosticFact>,
    #[serde(default)]
    pub semantic_signatures: Vec<GhidraOracleSemanticSignatureFact>,
    #[serde(default)]
    pub function_id_signatures: Vec<GhidraOracleFunctionIdFact>,
}

pub fn compile_program_analysis_run_in_memory(
    mut input: ProgramAnalysisInput,
) -> ProgramAnalysisOutput {
    normalize_input(&mut input);
    input
        .reference_recovery_evidence
        .extend(derive_reference_recovery_evidence(&input));
    normalize_reference_recovery_evidence(&mut input.reference_recovery_evidence);
    input
        .analysis_drifts
        .extend(derive_ghidra_oracle_summary_drifts(&input));
    input
        .analysis_drifts
        .extend(derive_ghidra_oracle_address_drifts(&input));
    normalize_analysis_drifts(&mut input.analysis_drifts);
    let artifact_hash = stable_hash(json!({
        "artifact": &input.artifact,
        "loader_facts": &input.loader_facts,
        "instruction_facts": &input.instruction_facts,
        "ir_functions": &input.ir_functions,
        "data_flow_facts": &input.data_flow_facts,
        "pcode_facts": &input.pcode_facts,
        "reference_recovery_evidence": &input.reference_recovery_evidence,
        "semantic_hypotheses": &input.semantic_hypotheses,
        "analyzer_receipts": &input.analyzer_receipts,
        "analysis_passes": &input.analysis_passes,
        "analysis_scheduler_states": &input.analysis_scheduler_states,
        "analysis_work_items": &input.analysis_work_items,
        "analysis_drifts": &input.analysis_drifts,
        "runtime_traces": &input.runtime_traces,
        "trace_snapshots": &input.trace_snapshots,
        "trace_events": &input.trace_events,
        "taint_marks": &input.taint_marks,
        "oracle_fixture": &input.oracle_fixture,
        "oracle_function_facts": &input.oracle_function_facts,
        "oracle_pcode_facts": &input.oracle_pcode_facts,
        "oracle_reference_facts": &input.oracle_reference_facts,
        "oracle_call_edge_facts": &input.oracle_call_edge_facts,
        "oracle_jump_table_facts": &input.oracle_jump_table_facts,
        "oracle_equate_facts": &input.oracle_equate_facts,
        "oracle_external_linkage_facts": &input.oracle_external_linkage_facts,
        "oracle_function_prototype_facts": &input.oracle_function_prototype_facts,
        "oracle_data_type_facts": &input.oracle_data_type_facts,
        "oracle_structure_field_access_facts": &input.oracle_structure_field_access_facts,
        "oracle_high_variable_facts": &input.oracle_high_variable_facts,
        "oracle_stack_frame_facts": &input.oracle_stack_frame_facts,
        "oracle_parameter_measure_facts": &input.oracle_parameter_measure_facts,
        "oracle_call_stack_effect_facts": &input.oracle_call_stack_effect_facts,
        "oracle_annotation_facts": &input.oracle_annotation_facts,
        "oracle_symbolic_summary_facts": &input.oracle_symbolic_summary_facts,
        "oracle_decompiler_diagnostic_facts": &input.oracle_decompiler_diagnostic_facts,
        "oracle_semantic_signature_facts": &input.oracle_semantic_signature_facts,
        "oracle_function_id_facts": &input.oracle_function_id_facts,
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
            "started_at_ms": input.started_at_ms,
            "finished_at_ms": input.finished_at_ms,
            "status": &input.status,
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
        pcode_facts: input.pcode_facts,
        reference_recovery_evidence: input.reference_recovery_evidence,
        semantic_hypotheses: input.semantic_hypotheses,
        analyzer_receipts: input.analyzer_receipts,
        analysis_passes: input.analysis_passes,
        analysis_scheduler_states: input.analysis_scheduler_states,
        analysis_work_items: input.analysis_work_items,
        analysis_drifts: input.analysis_drifts,
        runtime_traces: input.runtime_traces,
        trace_snapshots: input.trace_snapshots,
        trace_events: input.trace_events,
        taint_marks: input.taint_marks,
        oracle_function_facts: input.oracle_function_facts,
        oracle_pcode_facts: input.oracle_pcode_facts,
        oracle_reference_facts: input.oracle_reference_facts,
        oracle_call_edge_facts: input.oracle_call_edge_facts,
        oracle_jump_table_facts: input.oracle_jump_table_facts,
        oracle_equate_facts: input.oracle_equate_facts,
        oracle_external_linkage_facts: input.oracle_external_linkage_facts,
        oracle_function_prototype_facts: input.oracle_function_prototype_facts,
        oracle_data_type_facts: input.oracle_data_type_facts,
        oracle_structure_field_access_facts: input.oracle_structure_field_access_facts,
        oracle_high_variable_facts: input.oracle_high_variable_facts,
        oracle_stack_frame_facts: input.oracle_stack_frame_facts,
        oracle_parameter_measure_facts: input.oracle_parameter_measure_facts,
        oracle_call_stack_effect_facts: input.oracle_call_stack_effect_facts,
        oracle_annotation_facts: input.oracle_annotation_facts,
        oracle_symbolic_summary_facts: input.oracle_symbolic_summary_facts,
        oracle_decompiler_diagnostic_facts: input.oracle_decompiler_diagnostic_facts,
        oracle_semantic_signature_facts: input.oracle_semantic_signature_facts,
        oracle_function_id_facts: input.oracle_function_id_facts,
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
    mut fixture: GhidraOracleFixture,
) -> ProgramAnalysisInput {
    let evidence_ids = normalize_strings(fixture.evidence_ids.clone());
    fixture.evidence_ids = evidence_ids.clone();
    let status = if fixture.program_summary.analysis_timeout_occurred {
        ProgramAnalysisStatus::Partial
    } else {
        ProgramAnalysisStatus::Complete
    };
    let mut input = ProgramAnalysisInput::new(tenant_id, artifact);
    input.toolchain = format!("ghidra-oracle:{}", fixture.program_summary.ghidra_version);
    input.profile = "ghidra-headless-oracle-v0".to_string();
    input.status = status.clone();
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
            PROGRAM_DATA_FLOW_FACT_LABEL.to_string(),
            PROGRAM_PCODE_FACT_LABEL.to_string(),
            REFERENCE_RECOVERY_EVIDENCE_LABEL.to_string(),
            GHIDRA_ORACLE_FUNCTION_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_PCODE_OP_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_REFERENCE_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_CALL_EDGE_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_JUMP_TABLE_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_EQUATE_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_EXTERNAL_LINKAGE_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_FUNCTION_PROTOTYPE_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_DATA_TYPE_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_STRUCTURE_FIELD_ACCESS_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_HIGH_VARIABLE_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_STACK_FRAME_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_PARAMETER_MEASURE_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_CALL_STACK_EFFECT_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_ANNOTATION_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_SYMBOLIC_SUMMARY_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_DECOMPILER_DIAGNOSTIC_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_SEMANTIC_SIGNATURE_FACT_LABEL.to_string(),
            GHIDRA_ORACLE_FUNCTION_ID_FACT_LABEL.to_string(),
        ],
        authority_layer: AUTHORITY_OBSERVED_FACT.to_string(),
        input_hash: stable_hash(json!({
            "fixture_id": &fixture.fixture_id,
            "source_uri": &fixture.source_uri,
            "summary": &fixture.program_summary,
        })),
        status,
        evidence_ids,
    });
    input.oracle_fixture = Some(fixture);
    input
}

pub fn ghidra_oracle_export_to_program_analysis_input(
    tenant_id: impl Into<String>,
    artifact: BinaryArtifact,
    export: GhidraOracleExport,
) -> ProgramAnalysisInput {
    let GhidraOracleExport {
        fixture,
        functions,
        pcode_ops,
        references,
        call_edges,
        jump_tables,
        equates,
        external_linkages,
        function_prototypes,
        data_types,
        structure_field_accesses,
        high_variables,
        stack_frames,
        parameter_measures,
        call_stack_effects,
        annotations,
        symbolic_summaries,
        diagnostics,
        semantic_signatures,
        function_id_signatures,
    } = export;
    let mut input = ghidra_oracle_fixture_to_program_analysis_input(tenant_id, artifact, fixture);
    input.oracle_function_facts = functions;
    input.oracle_pcode_facts = pcode_ops;
    input.oracle_reference_facts = references;
    input.oracle_call_edge_facts = call_edges;
    input.oracle_jump_table_facts = jump_tables;
    input.oracle_equate_facts = equates;
    input.oracle_external_linkage_facts = external_linkages;
    input.oracle_function_prototype_facts = function_prototypes;
    input.oracle_data_type_facts = data_types;
    input.oracle_structure_field_access_facts = structure_field_accesses;
    input.oracle_high_variable_facts = high_variables;
    input.oracle_stack_frame_facts = stack_frames;
    input.oracle_parameter_measure_facts = parameter_measures;
    input.oracle_call_stack_effect_facts = call_stack_effects;
    input.oracle_annotation_facts = annotations;
    input.oracle_symbolic_summary_facts = symbolic_summaries;
    input.oracle_decompiler_diagnostic_facts = diagnostics;
    input.oracle_semantic_signature_facts = semantic_signatures;
    input.oracle_function_id_facts = function_id_signatures;
    if let Some(receipt) = input
        .analyzer_receipts
        .iter_mut()
        .find(|receipt| receipt.analyzer_id == "ghidra-headless-oracle")
    {
        receipt.input_hash = stable_hash(json!({
            "fixture": &input.oracle_fixture,
            "functions": &input.oracle_function_facts,
            "pcode_ops": &input.oracle_pcode_facts,
            "references": &input.oracle_reference_facts,
            "call_edges": &input.oracle_call_edge_facts,
            "jump_tables": &input.oracle_jump_table_facts,
            "equates": &input.oracle_equate_facts,
            "external_linkages": &input.oracle_external_linkage_facts,
            "function_prototypes": &input.oracle_function_prototype_facts,
            "data_types": &input.oracle_data_type_facts,
            "structure_field_accesses": &input.oracle_structure_field_access_facts,
            "high_variables": &input.oracle_high_variable_facts,
            "stack_frames": &input.oracle_stack_frame_facts,
            "parameter_measures": &input.oracle_parameter_measure_facts,
            "call_stack_effects": &input.oracle_call_stack_effect_facts,
            "annotations": &input.oracle_annotation_facts,
            "symbolic_summaries": &input.oracle_symbolic_summary_facts,
            "diagnostics": &input.oracle_decompiler_diagnostic_facts,
            "semantic_signatures": &input.oracle_semantic_signature_facts,
            "function_id_signatures": &input.oracle_function_id_facts,
        }));
    }
    input
}

pub fn ghidra_oracle_export_json_to_program_analysis_input(
    tenant_id: impl Into<String>,
    artifact: BinaryArtifact,
    raw: &str,
) -> Result<ProgramAnalysisInput, serde_json::Error> {
    let export = serde_json::from_str::<GhidraOracleExport>(raw)?;
    Ok(ghidra_oracle_export_to_program_analysis_input(
        tenant_id, artifact, export,
    ))
}

pub fn derive_reference_recovery_evidence(
    input: &ProgramAnalysisInput,
) -> Vec<ReferenceRecoveryEvidence> {
    input
        .oracle_reference_facts
        .iter()
        .map(|reference| {
            let semantic_roles = normalize_reference_semantic_roles(
                &reference.reference_type,
                reference.is_primary,
                reference.semantic_roles.clone(),
                reference.is_external,
                reference.is_memory,
                reference.is_register,
                reference.is_stack,
            );
            let evidence_ids = normalize_strings(
                std::iter::once(reference.reference_id.clone())
                    .chain(reference.evidence_ids.iter().cloned())
                    .collect(),
            );
            let confidence = reference_recovery_confidence(reference, &semantic_roles);
            ReferenceRecoveryEvidence {
                evidence_id: format!(
                    "program-analysis:reference-recovery:{}",
                    stable_hash(json!([
                        &reference.reference_id,
                        normalize_address(&reference.from_address),
                        normalize_address(&reference.to_address),
                        &reference.reference_type,
                        reference.operand_index,
                        &semantic_roles,
                        &reference.source_type,
                        &evidence_ids,
                    ]))
                ),
                source_reference_id: reference.reference_id.clone(),
                from_address: normalize_address(&reference.from_address),
                to_address: normalize_address(&reference.to_address),
                reference_type: reference.reference_type.trim().to_string(),
                semantic_roles,
                operand_index: reference.operand_index,
                source_type: reference.source_type.clone(),
                confidence,
                evidence_ids,
            }
        })
        .collect()
}

fn reference_recovery_confidence(
    reference: &GhidraOracleReferenceFact,
    semantic_roles: &[String],
) -> u8 {
    let mut confidence = 70;
    if reference.is_primary {
        confidence += 10;
    }
    if reference
        .source_type
        .as_deref()
        .is_some_and(|source| source.eq_ignore_ascii_case("USER_DEFINED"))
    {
        confidence += 10;
    } else if reference
        .source_type
        .as_deref()
        .is_some_and(|source| source.eq_ignore_ascii_case("ANALYSIS"))
    {
        confidence += 5;
    }
    if semantic_roles.iter().any(|role| {
        matches!(
            role.trim(),
            "call" | "jump" | "flow" | "data" | "read" | "write"
        )
    }) {
        confidence += 5;
    }
    confidence.min(99)
}

pub fn schedule_analysis_work_items(
    passes: &[AnalysisPassSpec],
    change_set: &AnalysisChangeSet,
) -> Vec<AnalysisWorkItem> {
    let mut change_set = change_set.clone();
    normalize_change_set(&mut change_set);
    if change_set.added_ranges.is_empty() && change_set.removed_ranges.is_empty() {
        return Vec::new();
    }
    let mut work_items = passes
        .iter()
        .filter(|pass| pass.enabled && pass.can_analyze)
        .map(|pass| {
            let evidence_ids = normalize_strings(
                pass.evidence_ids
                    .iter()
                    .cloned()
                    .chain(change_set.evidence_ids.iter().cloned())
                    .collect(),
            );
            let input_hash = stable_hash(json!({
                "analyzer_id": &pass.analyzer_id,
                "analyzer_type": &pass.analyzer_type,
                "added_ranges": &change_set.added_ranges,
                "removed_ranges": &change_set.removed_ranges,
                "input_labels": &pass.input_labels,
                "output_labels": &pass.output_labels,
            }));
            AnalysisWorkItem {
                work_item_id: format!(
                    "program-analysis:work:{}",
                    stable_hash(json!([
                        &pass.analyzer_id,
                        pass.priority,
                        &input_hash,
                        &evidence_ids
                    ]))
                ),
                analyzer_id: pass.analyzer_id.clone(),
                analyzer_type: pass.analyzer_type.clone(),
                priority: pass.priority,
                status: AnalysisWorkStatus::Scheduled,
                added_ranges: change_set.added_ranges.clone(),
                removed_ranges: change_set.removed_ranges.clone(),
                input_hash,
                evidence_ids,
            }
        })
        .collect::<Vec<_>>();
    work_items.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.analyzer_id.cmp(&right.analyzer_id))
            .then_with(|| left.work_item_id.cmp(&right.work_item_id))
    });
    work_items
}

pub fn ghidra_analysis_scheduler_states(
    passes: &[AnalysisPassSpec],
    change_set: &AnalysisChangeSet,
) -> Vec<AnalysisSchedulerState> {
    let mut change_set = change_set.clone();
    normalize_change_set(&mut change_set);
    let has_changes = !change_set.added_ranges.is_empty() || !change_set.removed_ranges.is_empty();
    let mut states = passes
        .iter()
        .filter(|pass| pass.can_analyze)
        .map(|pass| {
            let queued = pass.enabled && has_changes;
            let added_ranges = if queued {
                change_set.added_ranges.clone()
            } else {
                Vec::new()
            };
            let removed_ranges = if queued {
                change_set.removed_ranges.clone()
            } else {
                Vec::new()
            };
            let evidence_ids = normalize_strings(
                pass.evidence_ids
                    .iter()
                    .cloned()
                    .chain(change_set.evidence_ids.iter().cloned())
                    .collect(),
            );
            let input_hash = stable_hash(json!({
                "analyzer_id": &pass.analyzer_id,
                "analyzer_type": &pass.analyzer_type,
                "priority": pass.priority,
                "default_enabled": pass.default_enabled,
                "enabled": pass.enabled,
                "scheduled": queued,
                "supports_one_time": pass.supports_one_time,
                "option_namespace": &pass.option_namespace,
                "added_ranges": &added_ranges,
                "removed_ranges": &removed_ranges,
            }));
            AnalysisSchedulerState {
                scheduler_id: format!(
                    "program-analysis:scheduler:{}",
                    stable_hash(json!([
                        &pass.analyzer_id,
                        pass.priority,
                        queued,
                        &input_hash,
                        &evidence_ids
                    ]))
                ),
                analyzer_id: pass.analyzer_id.clone(),
                analyzer_type: pass.analyzer_type.clone(),
                priority: pass.priority,
                default_enabled: pass.default_enabled,
                enabled: pass.enabled,
                scheduled: queued,
                supports_one_time: pass.supports_one_time,
                option_namespace: pass.option_namespace.clone(),
                added_ranges,
                removed_ranges,
                input_hash,
                evidence_ids,
            }
        })
        .collect::<Vec<_>>();
    states.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.analyzer_id.cmp(&right.analyzer_id))
            .then_with(|| left.scheduler_id.cmp(&right.scheduler_id))
    });
    states
}

pub fn derive_ghidra_oracle_summary_drifts(input: &ProgramAnalysisInput) -> Vec<AnalysisDriftFact> {
    let Some(fixture) = &input.oracle_fixture else {
        return Vec::new();
    };

    let mut drifts = Vec::new();
    push_count_drift(
        &mut drifts,
        fixture,
        "function_count",
        fixture.program_summary.function_count,
        input.ir_functions.len() as u64,
        "error",
    );
    push_count_drift(
        &mut drifts,
        fixture,
        "import_count",
        fixture.program_summary.import_count,
        native_import_count(input),
        "warning",
    );
    push_count_drift(
        &mut drifts,
        fixture,
        "string_count",
        fixture.program_summary.string_count,
        native_string_count(input),
        "warning",
    );
    push_count_drift(
        &mut drifts,
        fixture,
        "cfg_edge_count",
        fixture.program_summary.cfg_edge_count,
        native_cfg_edge_count(input),
        "warning",
    );
    drifts
}

pub fn derive_ghidra_oracle_address_drifts(input: &ProgramAnalysisInput) -> Vec<AnalysisDriftFact> {
    let Some(fixture) = &input.oracle_fixture else {
        return Vec::new();
    };

    let mut drifts = Vec::new();
    derive_function_boundary_drifts(input, fixture, &mut drifts);
    derive_pcode_op_drifts(input, fixture, &mut drifts);
    derive_reference_drifts(input, fixture, &mut drifts);
    derive_call_edge_drifts(input, fixture, &mut drifts);
    drifts
}

fn derive_function_boundary_drifts(
    input: &ProgramAnalysisInput,
    fixture: &GhidraOracleFixture,
    drifts: &mut Vec<AnalysisDriftFact>,
) {
    let native_by_entry = input
        .ir_functions
        .iter()
        .filter_map(|function| {
            let (start, end) = function.address_range.as_ref()?;
            Some((
                normalize_address(start),
                json!({
                    "function_id": &function.function_id,
                    "body_start": normalize_address(start),
                    "body_end": normalize_address(end),
                }),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let oracle_by_entry = input
        .oracle_function_facts
        .iter()
        .map(|function| (normalize_address(&function.entry_point), function))
        .collect::<BTreeMap<_, _>>();

    for (entry, oracle) in &oracle_by_entry {
        let expected = json!({
            "function_id": &oracle.function_id,
            "entry_point": normalize_address(&oracle.entry_point),
            "name": &oracle.name,
            "body_start": normalize_address(&oracle.body_start),
            "body_end": normalize_address(&oracle.body_end),
        });
        match native_by_entry.get(entry) {
            Some(observed) if function_body_matches(oracle, observed) => {}
            Some(observed) => push_exact_drift(
                drifts,
                fixture,
                "function_boundary",
                AnalysisDriftKind::FieldMismatch,
                expected,
                observed.clone(),
                "error",
                &oracle.evidence_ids,
            ),
            None => push_exact_drift(
                drifts,
                fixture,
                "function_boundary",
                AnalysisDriftKind::MissingNativeFact,
                expected,
                Value::Null,
                "error",
                &oracle.evidence_ids,
            ),
        }
    }

    if !oracle_by_entry.is_empty() {
        for (entry, observed) in native_by_entry {
            if !oracle_by_entry.contains_key(&entry) {
                push_exact_drift(
                    drifts,
                    fixture,
                    "function_boundary",
                    AnalysisDriftKind::MissingOracleFact,
                    Value::Null,
                    observed,
                    "warning",
                    &[],
                );
            }
        }
    }
}

fn derive_pcode_op_drifts(
    input: &ProgramAnalysisInput,
    fixture: &GhidraOracleFixture,
    drifts: &mut Vec<AnalysisDriftFact>,
) {
    let native_by_key = input
        .pcode_facts
        .iter()
        .map(|fact| {
            (
                pcode_key(&fact.address, fact.sequence),
                native_pcode_value(fact),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let oracle_by_key = input
        .oracle_pcode_facts
        .iter()
        .map(|fact| (pcode_key(&fact.address, fact.sequence), fact))
        .collect::<BTreeMap<_, _>>();

    for (key, oracle) in &oracle_by_key {
        let expected = oracle_pcode_value(oracle);
        match native_by_key.get(key) {
            Some(observed) if pcode_value_matches(oracle, observed) => {}
            Some(observed) => push_exact_drift(
                drifts,
                fixture,
                "pcode_op",
                AnalysisDriftKind::FieldMismatch,
                expected,
                observed.clone(),
                "error",
                &oracle.evidence_ids,
            ),
            None => push_exact_drift(
                drifts,
                fixture,
                "pcode_op",
                AnalysisDriftKind::MissingNativeFact,
                expected,
                Value::Null,
                "error",
                &oracle.evidence_ids,
            ),
        }
    }

    if !oracle_by_key.is_empty() {
        for (_key, observed) in native_by_key {
            let address = observed
                .get("address")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let sequence = observed
                .get("sequence")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            if !oracle_by_key.contains_key(&pcode_key(&address, sequence)) {
                push_exact_drift(
                    drifts,
                    fixture,
                    "pcode_op",
                    AnalysisDriftKind::MissingOracleFact,
                    Value::Null,
                    observed,
                    "warning",
                    &[],
                );
            }
        }
    }
}

fn derive_reference_drifts(
    input: &ProgramAnalysisInput,
    fixture: &GhidraOracleFixture,
    drifts: &mut Vec<AnalysisDriftFact>,
) {
    let native_refs = native_reference_keys(input);
    let oracle_refs = input
        .oracle_reference_facts
        .iter()
        .map(|fact| (reference_key(fact), fact))
        .collect::<BTreeMap<_, _>>();

    for (key, oracle) in &oracle_refs {
        if !native_refs.contains(key) {
            push_exact_drift(
                drifts,
                fixture,
                "reference",
                AnalysisDriftKind::MissingNativeFact,
                oracle_reference_value(oracle),
                Value::Null,
                "warning",
                &oracle.evidence_ids,
            );
        }
    }
    if !oracle_refs.is_empty() {
        for key in native_refs {
            if !oracle_refs.contains_key(&key) {
                push_exact_drift(
                    drifts,
                    fixture,
                    "reference",
                    AnalysisDriftKind::MissingOracleFact,
                    Value::Null,
                    json!({"key": key}),
                    "warning",
                    &[],
                );
            }
        }
    }
}

fn derive_call_edge_drifts(
    input: &ProgramAnalysisInput,
    fixture: &GhidraOracleFixture,
    drifts: &mut Vec<AnalysisDriftFact>,
) {
    let native_edges = native_call_edge_keys(input);
    let oracle_edges = input
        .oracle_call_edge_facts
        .iter()
        .map(|fact| (call_edge_key(&fact.source_entry, &fact.target_entry), fact))
        .collect::<BTreeMap<_, _>>();

    for (key, oracle) in &oracle_edges {
        if !native_edges.contains(key) {
            push_exact_drift(
                drifts,
                fixture,
                "call_edge",
                AnalysisDriftKind::MissingNativeFact,
                oracle_call_edge_value(oracle),
                Value::Null,
                "error",
                &oracle.evidence_ids,
            );
        }
    }
    if !oracle_edges.is_empty() {
        for key in native_edges {
            if !oracle_edges.contains_key(&key) {
                push_exact_drift(
                    drifts,
                    fixture,
                    "call_edge",
                    AnalysisDriftKind::MissingOracleFact,
                    Value::Null,
                    json!({"key": key}),
                    "warning",
                    &[],
                );
            }
        }
    }
}

fn push_count_drift(
    drifts: &mut Vec<AnalysisDriftFact>,
    fixture: &GhidraOracleFixture,
    fact_kind: &str,
    expected: u64,
    observed: u64,
    severity: &str,
) {
    if expected == observed {
        return;
    }
    let evidence_ids = normalize_strings(
        std::iter::once(fixture.fixture_id.clone())
            .chain(fixture.evidence_ids.iter().cloned())
            .collect(),
    );
    drifts.push(AnalysisDriftFact {
        drift_id: format!(
            "program-analysis:drift:{}",
            stable_hash(json!([
                &fixture.fixture_id,
                fact_kind,
                expected,
                observed,
                &evidence_ids
            ]))
        ),
        oracle_fixture_id: fixture.fixture_id.clone(),
        fact_kind: fact_kind.to_string(),
        drift_kind: if observed == 0 && expected > 0 {
            AnalysisDriftKind::MissingNativeFact
        } else if expected == 0 && observed > 0 {
            AnalysisDriftKind::MissingOracleFact
        } else {
            AnalysisDriftKind::CountMismatch
        },
        expected: json!(expected),
        observed: json!(observed),
        severity: severity.to_string(),
        evidence_ids,
    });
}

#[allow(clippy::too_many_arguments)]
fn push_exact_drift(
    drifts: &mut Vec<AnalysisDriftFact>,
    fixture: &GhidraOracleFixture,
    fact_kind: &str,
    drift_kind: AnalysisDriftKind,
    expected: Value,
    observed: Value,
    severity: &str,
    evidence_ids: &[String],
) {
    let evidence_ids = normalize_strings(
        std::iter::once(fixture.fixture_id.clone())
            .chain(fixture.evidence_ids.iter().cloned())
            .chain(evidence_ids.iter().cloned())
            .collect(),
    );
    drifts.push(AnalysisDriftFact {
        drift_id: format!(
            "program-analysis:drift:{}",
            stable_hash(json!([
                &fixture.fixture_id,
                fact_kind,
                &drift_kind,
                &expected,
                &observed,
                &evidence_ids
            ]))
        ),
        oracle_fixture_id: fixture.fixture_id.clone(),
        fact_kind: fact_kind.to_string(),
        drift_kind,
        expected,
        observed,
        severity: severity.to_string(),
        evidence_ids,
    });
}

fn normalize_address(address: &str) -> String {
    address.trim().to_ascii_lowercase()
}

fn function_body_matches(oracle: &GhidraOracleFunctionFact, observed: &Value) -> bool {
    let expected_start = normalize_address(&oracle.body_start);
    let expected_end = normalize_address(&oracle.body_end);
    observed.get("body_start").and_then(Value::as_str) == Some(expected_start.as_str())
        && observed.get("body_end").and_then(Value::as_str) == Some(expected_end.as_str())
}

fn pcode_key(address: &str, sequence: u64) -> String {
    format!("{}:{sequence}", normalize_address(address))
}

fn native_pcode_value(fact: &ProgramPcodeFact) -> Value {
    json!({
        "pcode_id": &fact.pcode_id,
        "address": normalize_address(&fact.address),
        "sequence": fact.sequence,
        "opcode": &fact.opcode,
        "ghidra_opcode_id": fact.ghidra_opcode_id,
        "inputs": &fact.inputs,
        "output": &fact.output,
    })
}

fn oracle_pcode_value(fact: &GhidraOraclePcodeOpFact) -> Value {
    json!({
        "pcode_id": &fact.pcode_id,
        "address": normalize_address(&fact.address),
        "sequence": fact.sequence,
        "opcode": &fact.opcode,
        "ghidra_opcode_id": fact.ghidra_opcode_id,
        "inputs": &fact.inputs,
        "output": &fact.output,
    })
}

fn pcode_value_matches(oracle: &GhidraOraclePcodeOpFact, observed: &Value) -> bool {
    observed.get("ghidra_opcode_id").and_then(Value::as_u64)
        == Some(u64::from(oracle.ghidra_opcode_id))
        && observed.get("opcode").and_then(Value::as_str) == Some(oracle.opcode.as_str())
        && observed.get("inputs") == Some(&json!(&oracle.inputs))
        && observed.get("output") == Some(&json!(&oracle.output))
}

fn native_reference_keys(input: &ProgramAnalysisInput) -> BTreeSet<String> {
    input
        .data_flow_facts
        .iter()
        .filter(|fact| {
            matches!(
                fact.fact_kind.as_str(),
                "reference" | "memory_reference" | "code_reference"
            ) && looks_like_address(&fact.source_id)
                && looks_like_address(&fact.target_id)
        })
        .map(|fact| {
            format!(
                "{}->{}:{}:-2",
                normalize_address(&fact.source_id),
                normalize_address(&fact.target_id),
                fact.fact_kind
            )
        })
        .collect()
}

fn reference_key(reference: &GhidraOracleReferenceFact) -> String {
    format!(
        "{}->{}:{}:{}",
        normalize_address(&reference.from_address),
        normalize_address(&reference.to_address),
        reference.reference_type.trim(),
        reference.operand_index,
    )
}

fn oracle_reference_value(reference: &GhidraOracleReferenceFact) -> Value {
    json!({
        "reference_id": &reference.reference_id,
        "from_address": normalize_address(&reference.from_address),
        "to_address": normalize_address(&reference.to_address),
        "reference_type": &reference.reference_type,
        "operand_index": reference.operand_index,
        "is_primary": reference.is_primary,
        "source_type": &reference.source_type,
        "semantic_roles": &reference.semantic_roles,
        "is_external": reference.is_external,
        "is_memory": reference.is_memory,
        "is_register": reference.is_register,
        "is_stack": reference.is_stack,
    })
}

fn native_call_edge_keys(input: &ProgramAnalysisInput) -> BTreeSet<String> {
    input
        .data_flow_facts
        .iter()
        .filter(|fact| {
            fact.fact_kind == "call_edge"
                && looks_like_address(&fact.source_id)
                && looks_like_address(&fact.target_id)
        })
        .map(|fact| call_edge_key(&fact.source_id, &fact.target_id))
        .collect()
}

fn call_edge_key(source_entry: &str, target_entry: &str) -> String {
    format!(
        "{}->{}",
        normalize_address(source_entry),
        normalize_address(target_entry)
    )
}

fn looks_like_address(value: &str) -> bool {
    let value = value.trim();
    value.starts_with("0x") || value.starts_with("0X")
}

fn oracle_call_edge_value(edge: &GhidraOracleCallEdgeFact) -> Value {
    json!({
        "edge_id": &edge.edge_id,
        "source_entry": normalize_address(&edge.source_entry),
        "target_entry": normalize_address(&edge.target_entry),
        "callsite_address": normalize_address(&edge.callsite_address),
    })
}

fn native_import_count(input: &ProgramAnalysisInput) -> u64 {
    let mut imports = input
        .loader_facts
        .iter()
        .flat_map(|fact| fact.imports.iter())
        .map(|import| {
            json!({
                "library": &import.library,
                "name": &import.name,
                "address": &import.address,
            })
            .to_string()
        })
        .collect::<Vec<_>>();
    imports.sort();
    imports.dedup();
    imports.len() as u64
}

fn native_string_count(input: &ProgramAnalysisInput) -> u64 {
    let mut strings = input
        .loader_facts
        .iter()
        .flat_map(|fact| fact.strings.iter())
        .map(|string| {
            json!({
                "address": &string.address,
                "value": &string.value,
                "encoding": &string.encoding,
            })
            .to_string()
        })
        .collect::<Vec<_>>();
    strings.sort();
    strings.dedup();
    strings.len() as u64
}

fn native_cfg_edge_count(input: &ProgramAnalysisInput) -> u64 {
    let mut edges = input
        .instruction_facts
        .iter()
        .flat_map(|instruction| {
            [
                instruction
                    .fallthrough
                    .as_ref()
                    .map(|target| format!("{}->{}", instruction.address, target)),
                instruction
                    .branch_target
                    .as_ref()
                    .map(|target| format!("{}->{}", instruction.address, target)),
            ]
        })
        .flatten()
        .collect::<Vec<_>>();
    edges.extend(
        input
            .data_flow_facts
            .iter()
            .filter(|fact| {
                matches!(
                    fact.fact_kind.as_str(),
                    "cfg_edge" | "control_flow_edge" | "branch_edge"
                )
            })
            .map(|fact| format!("{}->{}", fact.source_id, fact.target_id)),
    );
    edges.sort();
    edges.dedup();
    edges.len() as u64
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
    input.instruction_facts.sort_by(|left, right| {
        left.instruction_id
            .cmp(&right.instruction_id)
            .then_with(|| left.address.cmp(&right.address))
    });
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
    input.pcode_facts.sort_by(|left, right| {
        left.address
            .cmp(&right.address)
            .then_with(|| left.sequence.cmp(&right.sequence))
            .then_with(|| left.pcode_id.cmp(&right.pcode_id))
    });
    input
        .pcode_facts
        .dedup_by(|left, right| left.pcode_id == right.pcode_id);
    for fact in &mut input.pcode_facts {
        fact.pcode_id = fact.pcode_id.trim().to_string();
        fact.address = normalize_address(&fact.address);
        fact.opcode = fact.opcode.trim().to_string();
        fact.inputs = normalize_strings(fact.inputs.clone());
        fact.output = fact
            .output
            .as_ref()
            .map(|output| output.trim().to_string())
            .filter(|output| !output.is_empty());
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
    }
    normalize_reference_recovery_evidence(&mut input.reference_recovery_evidence);
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
    input
        .analysis_passes
        .sort_by(|left, right| left.analyzer_id.cmp(&right.analyzer_id));
    input
        .analysis_passes
        .dedup_by(|left, right| left.analyzer_id == right.analyzer_id);
    for pass in &mut input.analysis_passes {
        pass.analyzer_id = pass.analyzer_id.trim().to_string();
        pass.description = pass.description.trim().to_string();
        pass.input_labels = normalize_strings(pass.input_labels.clone());
        pass.output_labels = normalize_strings(pass.output_labels.clone());
        pass.option_namespace = pass
            .option_namespace
            .as_ref()
            .map(|namespace| namespace.trim().to_string())
            .filter(|namespace| !namespace.is_empty());
        pass.evidence_ids = normalize_strings(pass.evidence_ids.clone());
    }
    input.analysis_scheduler_states.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.analyzer_id.cmp(&right.analyzer_id))
            .then_with(|| left.scheduler_id.cmp(&right.scheduler_id))
    });
    input
        .analysis_scheduler_states
        .dedup_by(|left, right| left.scheduler_id == right.scheduler_id);
    for state in &mut input.analysis_scheduler_states {
        state.scheduler_id = state.scheduler_id.trim().to_string();
        state.analyzer_id = state.analyzer_id.trim().to_string();
        state.input_hash = state.input_hash.trim().to_string();
        state.option_namespace = state
            .option_namespace
            .as_ref()
            .map(|namespace| namespace.trim().to_string())
            .filter(|namespace| !namespace.is_empty());
        normalize_ranges(&mut state.added_ranges);
        normalize_ranges(&mut state.removed_ranges);
        state.evidence_ids = normalize_strings(state.evidence_ids.clone());
    }
    input.analysis_work_items.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.analyzer_id.cmp(&right.analyzer_id))
            .then_with(|| left.work_item_id.cmp(&right.work_item_id))
    });
    input
        .analysis_work_items
        .dedup_by(|left, right| left.work_item_id == right.work_item_id);
    for item in &mut input.analysis_work_items {
        item.work_item_id = item.work_item_id.trim().to_string();
        item.analyzer_id = item.analyzer_id.trim().to_string();
        item.input_hash = item.input_hash.trim().to_string();
        normalize_ranges(&mut item.added_ranges);
        normalize_ranges(&mut item.removed_ranges);
        item.evidence_ids = normalize_strings(item.evidence_ids.clone());
    }
    normalize_analysis_drifts(&mut input.analysis_drifts);
    normalize_oracle_function_facts(&mut input.oracle_function_facts);
    normalize_oracle_pcode_facts(&mut input.oracle_pcode_facts);
    normalize_oracle_reference_facts(&mut input.oracle_reference_facts);
    normalize_oracle_call_edge_facts(&mut input.oracle_call_edge_facts);
    normalize_oracle_jump_table_facts(&mut input.oracle_jump_table_facts);
    normalize_oracle_equate_facts(&mut input.oracle_equate_facts);
    normalize_oracle_external_linkage_facts(&mut input.oracle_external_linkage_facts);
    normalize_oracle_function_prototype_facts(&mut input.oracle_function_prototype_facts);
    normalize_oracle_data_type_facts(&mut input.oracle_data_type_facts);
    normalize_oracle_structure_field_access_facts(&mut input.oracle_structure_field_access_facts);
    normalize_oracle_high_variable_facts(&mut input.oracle_high_variable_facts);
    normalize_oracle_stack_frame_facts(&mut input.oracle_stack_frame_facts);
    normalize_oracle_parameter_measure_facts(&mut input.oracle_parameter_measure_facts);
    normalize_oracle_call_stack_effect_facts(&mut input.oracle_call_stack_effect_facts);
    normalize_oracle_annotation_facts(&mut input.oracle_annotation_facts);
    normalize_oracle_symbolic_summary_facts(&mut input.oracle_symbolic_summary_facts);
    normalize_oracle_decompiler_diagnostic_facts(&mut input.oracle_decompiler_diagnostic_facts);
    normalize_oracle_semantic_signature_facts(&mut input.oracle_semantic_signature_facts);
    normalize_oracle_function_id_facts(&mut input.oracle_function_id_facts);
    input
        .runtime_traces
        .sort_by(|left, right| left.trace_id.cmp(&right.trace_id));
    input
        .runtime_traces
        .dedup_by(|left, right| left.trace_id == right.trace_id);
    for trace in &mut input.runtime_traces {
        trace.trace_id = trace.trace_id.trim().to_string();
        normalize_optional_string(&mut trace.language_id);
        normalize_optional_string(&mut trace.compiler_spec_id);
        normalize_optional_string(&mut trace.emulator_cache_version);
        trace.capture_source = trace.capture_source.trim().to_string();
        trace.evidence_ids = normalize_strings(trace.evidence_ids.clone());
    }
    input.trace_snapshots.sort_by(|left, right| {
        left.trace_id
            .cmp(&right.trace_id)
            .then_with(|| left.snap.cmp(&right.snap))
            .then_with(|| left.snapshot_id.cmp(&right.snapshot_id))
    });
    input
        .trace_snapshots
        .dedup_by(|left, right| left.snapshot_id == right.snapshot_id);
    for snapshot in &mut input.trace_snapshots {
        snapshot.snapshot_id = snapshot.snapshot_id.trim().to_string();
        snapshot.trace_id = snapshot.trace_id.trim().to_string();
        snapshot.description = snapshot.description.trim().to_string();
        normalize_optional_string(&mut snapshot.event_thread_id);
        snapshot.schedule.schedule = snapshot.schedule.schedule.trim().to_string();
        snapshot.evidence_ids = normalize_strings(snapshot.evidence_ids.clone());
    }
    input.trace_events.sort_by(|left, right| {
        left.trace_id
            .cmp(&right.trace_id)
            .then_with(|| left.snapshot_id.cmp(&right.snapshot_id))
            .then_with(|| left.sequence.cmp(&right.sequence))
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    input
        .trace_events
        .dedup_by(|left, right| left.event_id == right.event_id);
    for event in &mut input.trace_events {
        event.event_id = event.event_id.trim().to_string();
        event.trace_id = event.trace_id.trim().to_string();
        event.snapshot_id = event.snapshot_id.trim().to_string();
        normalize_optional_string(&mut event.thread_id);
        normalize_optional_string(&mut event.address_space);
        normalize_optional_string(&mut event.offset);
        normalize_optional_string(&mut event.register);
        normalize_optional_string(&mut event.value_hash);
        normalize_optional_string(&mut event.pcode_op);
        event.evidence_ids = normalize_strings(event.evidence_ids.clone());
    }
    input.taint_marks.sort_by(|left, right| {
        left.trace_id
            .cmp(&right.trace_id)
            .then_with(|| left.snapshot_id.cmp(&right.snapshot_id))
            .then_with(|| left.offset.cmp(&right.offset))
            .then_with(|| left.taint_id.cmp(&right.taint_id))
    });
    input
        .taint_marks
        .dedup_by(|left, right| left.taint_id == right.taint_id);
    for mark in &mut input.taint_marks {
        mark.taint_id = mark.taint_id.trim().to_string();
        mark.trace_id = mark.trace_id.trim().to_string();
        mark.snapshot_id = mark.snapshot_id.trim().to_string();
        normalize_optional_string(&mut mark.event_id);
        mark.address_space = mark.address_space.trim().to_string();
        mark.offset = mark.offset.trim().to_ascii_lowercase();
        mark.labels = normalize_strings(mark.labels.clone());
        normalize_optional_string(&mut mark.originating_op);
        mark.evidence_ids = normalize_strings(mark.evidence_ids.clone());
    }
}

fn normalize_analysis_drifts(drifts: &mut Vec<AnalysisDriftFact>) {
    drifts.sort_by(|left, right| {
        left.oracle_fixture_id
            .cmp(&right.oracle_fixture_id)
            .then_with(|| left.fact_kind.cmp(&right.fact_kind))
            .then_with(|| left.drift_id.cmp(&right.drift_id))
    });
    drifts.dedup_by(|left, right| left.drift_id == right.drift_id);
    for drift in drifts {
        drift.drift_id = drift.drift_id.trim().to_string();
        drift.oracle_fixture_id = drift.oracle_fixture_id.trim().to_string();
        drift.fact_kind = drift.fact_kind.trim().to_string();
        drift.severity = drift.severity.trim().to_string();
        drift.evidence_ids = normalize_strings(drift.evidence_ids.clone());
    }
}

fn normalize_oracle_function_facts(facts: &mut Vec<GhidraOracleFunctionFact>) {
    facts.sort_by(|left, right| {
        left.entry_point
            .cmp(&right.entry_point)
            .then_with(|| left.function_id.cmp(&right.function_id))
    });
    facts.dedup_by(|left, right| left.function_id == right.function_id);
    for fact in &mut *facts {
        fact.function_id = fact.function_id.trim().to_string();
        fact.entry_point = normalize_address(&fact.entry_point);
        fact.name = fact
            .name
            .as_ref()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());
        fact.body_start = normalize_address(&fact.body_start);
        fact.body_end = normalize_address(&fact.body_end);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
    }
}

fn normalize_oracle_pcode_facts(facts: &mut Vec<GhidraOraclePcodeOpFact>) {
    facts.sort_by(|left, right| {
        left.address
            .cmp(&right.address)
            .then_with(|| left.sequence.cmp(&right.sequence))
            .then_with(|| left.pcode_id.cmp(&right.pcode_id))
    });
    facts.dedup_by(|left, right| left.pcode_id == right.pcode_id);
    for fact in facts {
        fact.pcode_id = fact.pcode_id.trim().to_string();
        fact.address = normalize_address(&fact.address);
        fact.opcode = fact.opcode.trim().to_string();
        fact.inputs = normalize_strings(fact.inputs.clone());
        fact.output = fact
            .output
            .as_ref()
            .map(|output| output.trim().to_string())
            .filter(|output| !output.is_empty());
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
    }
}

fn normalize_oracle_reference_facts(facts: &mut Vec<GhidraOracleReferenceFact>) {
    facts.sort_by(|left, right| {
        left.from_address
            .cmp(&right.from_address)
            .then_with(|| left.to_address.cmp(&right.to_address))
            .then_with(|| left.reference_id.cmp(&right.reference_id))
    });
    facts.dedup_by(|left, right| left.reference_id == right.reference_id);
    for fact in facts {
        fact.reference_id = fact.reference_id.trim().to_string();
        fact.from_address = normalize_address(&fact.from_address);
        fact.to_address = normalize_address(&fact.to_address);
        fact.reference_type = fact.reference_type.trim().to_string();
        fact.source_type = fact
            .source_type
            .as_ref()
            .map(|source| source.trim().to_string())
            .filter(|source| !source.is_empty());
        fact.semantic_roles = normalize_reference_semantic_roles(
            &fact.reference_type,
            fact.is_primary,
            fact.semantic_roles.clone(),
            fact.is_external,
            fact.is_memory,
            fact.is_register,
            fact.is_stack,
        );
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
    }
}

fn normalize_reference_recovery_evidence(facts: &mut Vec<ReferenceRecoveryEvidence>) {
    facts.sort_by(|left, right| {
        left.from_address
            .cmp(&right.from_address)
            .then_with(|| left.to_address.cmp(&right.to_address))
            .then_with(|| left.evidence_id.cmp(&right.evidence_id))
    });
    facts.dedup_by(|left, right| left.evidence_id == right.evidence_id);
    for fact in facts {
        fact.evidence_id = fact.evidence_id.trim().to_string();
        fact.source_reference_id = fact.source_reference_id.trim().to_string();
        fact.from_address = normalize_address(&fact.from_address);
        fact.to_address = normalize_address(&fact.to_address);
        fact.reference_type = fact.reference_type.trim().to_string();
        fact.semantic_roles = normalize_strings(fact.semantic_roles.clone())
            .into_iter()
            .map(|role| role.to_ascii_lowercase())
            .collect();
        fact.source_type = fact
            .source_type
            .as_ref()
            .map(|source| source.trim().to_string())
            .filter(|source| !source.is_empty());
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
    }
}

fn normalize_reference_semantic_roles(
    reference_type: &str,
    is_primary: bool,
    roles: Vec<String>,
    is_external: bool,
    is_memory: bool,
    is_register: bool,
    is_stack: bool,
) -> Vec<String> {
    let mut roles = roles
        .into_iter()
        .map(|role| role.trim().to_ascii_lowercase())
        .filter(|role| !role.is_empty())
        .collect::<Vec<_>>();
    let reference_type = reference_type.trim().to_ascii_lowercase();
    push_role_if(&mut roles, is_primary, "primary");
    push_role_if(&mut roles, is_external, "external");
    push_role_if(&mut roles, is_memory, "memory");
    push_role_if(&mut roles, is_register, "register");
    push_role_if(&mut roles, is_stack, "stack");
    push_role_if(&mut roles, reference_type.contains("call"), "call");
    push_role_if(
        &mut roles,
        reference_type.contains("jump") || reference_type.contains("branch"),
        "jump",
    );
    push_role_if(
        &mut roles,
        reference_type.contains("call")
            || reference_type.contains("jump")
            || reference_type.contains("branch")
            || reference_type.contains("fall"),
        "flow",
    );
    push_role_if(&mut roles, reference_type.contains("data"), "data");
    push_role_if(&mut roles, reference_type.contains("read"), "read");
    push_role_if(&mut roles, reference_type.contains("write"), "write");
    push_role_if(&mut roles, reference_type.contains("fall"), "fallthrough");
    push_role_if(
        &mut roles,
        reference_type.contains("conditional") && !reference_type.contains("unconditional"),
        "conditional",
    );
    push_role_if(&mut roles, reference_type.contains("computed"), "computed");
    push_role_if(&mut roles, reference_type.contains("terminal"), "terminal");
    push_role_if(&mut roles, reference_type.contains("override"), "override");
    roles.sort();
    roles.dedup();
    roles
}

fn push_role_if(roles: &mut Vec<String>, condition: bool, role: &str) {
    if condition {
        roles.push(role.to_string());
    }
}

fn normalize_oracle_call_edge_facts(facts: &mut Vec<GhidraOracleCallEdgeFact>) {
    facts.sort_by(|left, right| {
        left.source_entry
            .cmp(&right.source_entry)
            .then_with(|| left.target_entry.cmp(&right.target_entry))
            .then_with(|| left.callsite_address.cmp(&right.callsite_address))
            .then_with(|| left.edge_id.cmp(&right.edge_id))
    });
    facts.dedup_by(|left, right| left.edge_id == right.edge_id);
    for fact in facts {
        fact.edge_id = fact.edge_id.trim().to_string();
        fact.source_entry = normalize_address(&fact.source_entry);
        fact.target_entry = normalize_address(&fact.target_entry);
        fact.callsite_address = normalize_address(&fact.callsite_address);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
    }
}

fn normalize_oracle_jump_table_facts(facts: &mut Vec<GhidraOracleJumpTableFact>) {
    for fact in &mut *facts {
        fact.entry_point = normalize_address(&fact.entry_point);
        fact.switch_address = normalize_address(&fact.switch_address);
        normalize_optional_string(&mut fact.function_id);
        normalize_optional_string(&mut fact.switch_statement_id);
        fact.display_format = fact
            .display_format
            .as_ref()
            .map(|format| format.trim().to_ascii_lowercase())
            .filter(|format| !format.is_empty());
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        normalize_oracle_jump_table_case_facts(&mut fact.cases);
        normalize_oracle_jump_table_load_table_facts(&mut fact.load_tables);
        if fact.jump_table_id.trim().is_empty() {
            fact.jump_table_id = format!(
                "ghidra:jumptable:{}",
                stable_hash(json!([
                    &fact.function_id,
                    &fact.entry_point,
                    &fact.switch_address,
                    &fact.display_format,
                    &fact.cases,
                    &fact.load_tables,
                    fact.override_applied,
                    fact.references_complete
                ]))
            );
        } else {
            fact.jump_table_id = fact.jump_table_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.entry_point
            .cmp(&right.entry_point)
            .then_with(|| left.switch_address.cmp(&right.switch_address))
            .then_with(|| left.jump_table_id.cmp(&right.jump_table_id))
    });
    facts.dedup_by(|left, right| left.jump_table_id == right.jump_table_id);
}

fn normalize_oracle_jump_table_case_facts(facts: &mut Vec<GhidraOracleJumpTableCaseFact>) {
    facts.sort_by(|left, right| {
        left.is_default
            .cmp(&right.is_default)
            .then_with(|| left.label_value.cmp(&right.label_value))
            .then_with(|| left.destination.cmp(&right.destination))
            .then_with(|| left.case_id.cmp(&right.case_id))
    });
    facts.dedup_by(|left, right| {
        left.is_default == right.is_default
            && left.label_value == right.label_value
            && normalize_address(&left.destination) == normalize_address(&right.destination)
    });
    for fact in facts.iter_mut() {
        fact.destination = normalize_address(&fact.destination);
        fact.label = fact
            .label
            .as_ref()
            .map(|label| label.trim().to_string())
            .filter(|label| !label.is_empty())
            .or_else(|| fact.is_default.then(|| "default".to_string()));
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.case_id.trim().is_empty() {
            fact.case_id = format!(
                "ghidra:jumptable_case:{}",
                stable_hash(json!([
                    &fact.destination,
                    fact.label_value,
                    fact.is_default,
                    &fact.label
                ]))
            );
        } else {
            fact.case_id = fact.case_id.trim().to_string();
        }
    }
}

fn normalize_oracle_jump_table_load_table_facts(
    facts: &mut Vec<GhidraOracleJumpTableLoadTableFact>,
) {
    facts.sort_by(|left, right| {
        left.address
            .cmp(&right.address)
            .then_with(|| left.entry_byte_len.cmp(&right.entry_byte_len))
            .then_with(|| left.entry_count.cmp(&right.entry_count))
            .then_with(|| left.load_table_id.cmp(&right.load_table_id))
    });
    facts.dedup_by(|left, right| {
        normalize_address(&left.address) == normalize_address(&right.address)
            && left.entry_byte_len == right.entry_byte_len
            && left.entry_count == right.entry_count
    });
    for (index, fact) in facts.iter_mut().enumerate() {
        fact.address = normalize_address(&fact.address);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.load_table_id.trim().is_empty() {
            fact.load_table_id = format!(
                "ghidra:jumptable_load_table:{}",
                stable_hash(json!([
                    index,
                    &fact.address,
                    fact.entry_byte_len,
                    fact.entry_count,
                    fact.interpreted_as_pointer_table
                ]))
            );
        } else {
            fact.load_table_id = fact.load_table_id.trim().to_string();
        }
    }
}

fn normalize_oracle_equate_facts(facts: &mut Vec<GhidraOracleEquateFact>) {
    for fact in &mut *facts {
        normalize_optional_string(&mut fact.function_id);
        fact.name = fact.name.split_whitespace().collect::<Vec<_>>().join("_");
        if fact.name.is_empty() {
            fact.name = format!("const_{:x}", fact.value);
        }
        fact.display_name = fact
            .display_name
            .as_ref()
            .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|name| !name.is_empty())
            .or_else(|| Some(fact.name.clone()));
        fact.display_value = fact
            .display_value
            .as_ref()
            .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|value| !value.is_empty());
        fact.format = fact
            .format
            .as_ref()
            .map(|format| format.trim().to_ascii_lowercase())
            .filter(|format| !format.is_empty());
        normalize_optional_string(&mut fact.enum_uuid);
        fact.valid_uuid = fact.valid_uuid || !fact.enum_based;
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        normalize_oracle_equate_reference_facts(&mut fact.references);
        if fact.equate_id.trim().is_empty() {
            fact.equate_id = format!(
                "ghidra:equate:{}",
                stable_hash(json!([
                    &fact.function_id,
                    &fact.name,
                    &fact.display_name,
                    fact.value,
                    &fact.display_value,
                    &fact.format,
                    &fact.enum_uuid,
                    fact.enum_based,
                    fact.valid_uuid,
                    &fact.references
                ]))
            );
        } else {
            fact.equate_id = fact.equate_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.function_id
            .cmp(&right.function_id)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.value.cmp(&right.value))
            .then_with(|| left.equate_id.cmp(&right.equate_id))
    });
    facts.dedup_by(|left, right| left.equate_id == right.equate_id);
}

fn normalize_oracle_equate_reference_facts(facts: &mut Vec<GhidraOracleEquateReferenceFact>) {
    for (index, fact) in facts.iter_mut().enumerate() {
        fact.address = normalize_address(&fact.address);
        fact.operand_index = fact
            .operand_index
            .filter(|operand_index| *operand_index >= 0);
        fact.dynamic_hash = fact.dynamic_hash.filter(|hash| *hash != 0);
        normalize_optional_string(&mut fact.instruction_id);
        normalize_optional_string(&mut fact.statement_id);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.reference_id.trim().is_empty() {
            fact.reference_id = format!(
                "ghidra:equate_ref:{}",
                stable_hash(json!([
                    index,
                    &fact.address,
                    fact.operand_index,
                    fact.dynamic_hash,
                    &fact.instruction_id,
                    &fact.statement_id
                ]))
            );
        } else {
            fact.reference_id = fact.reference_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.address
            .cmp(&right.address)
            .then_with(|| left.operand_index.cmp(&right.operand_index))
            .then_with(|| left.dynamic_hash.cmp(&right.dynamic_hash))
            .then_with(|| left.reference_id.cmp(&right.reference_id))
    });
    facts.dedup_by(|left, right| left.reference_id == right.reference_id);
}

fn normalize_oracle_external_linkage_facts(facts: &mut Vec<GhidraOracleExternalLinkageFact>) {
    for fact in &mut *facts {
        normalize_optional_string(&mut fact.function_id);
        fact.local_address = normalize_address(&fact.local_address);
        normalize_optional_string(&mut fact.thunked_function_id);
        normalize_optional_address(&mut fact.thunked_address);
        normalize_optional_string(&mut fact.recursive_target_function_id);
        normalize_optional_address(&mut fact.recursive_target_address);
        fact.external_library = fact
            .external_library
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if fact.external_library.is_empty() {
            fact.external_library = "<UNKNOWN>".to_string();
        }
        normalize_optional_string(&mut fact.external_library_path);
        fact.library_ordinal = fact.library_ordinal.filter(|ordinal| *ordinal >= 0);
        normalize_optional_string(&mut fact.parent_namespace);
        fact.external_label = fact
            .external_label
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if fact.external_label.is_empty() {
            fact.external_label = fact
                .original_imported_name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or("external")
                .to_string();
        }
        normalize_optional_string(&mut fact.original_imported_name);
        normalize_optional_address(&mut fact.external_address);
        normalize_optional_address(&mut fact.external_space_address);
        fact.source_type = fact
            .source_type
            .as_ref()
            .map(|source| source.trim().to_ascii_uppercase())
            .filter(|source| !source.is_empty());
        normalize_optional_string(&mut fact.data_type);
        normalize_optional_string(&mut fact.function_signature);
        normalize_oracle_external_thunk_link_facts(&mut fact.thunk_chain);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.linkage_id.trim().is_empty() {
            fact.linkage_id = format!(
                "ghidra:external_linkage:{}",
                stable_hash(json!([
                    &fact.function_id,
                    &fact.local_address,
                    &fact.thunked_function_id,
                    &fact.thunked_address,
                    &fact.recursive_target_function_id,
                    &fact.recursive_target_address,
                    &fact.external_library,
                    &fact.external_library_path,
                    fact.library_ordinal,
                    &fact.parent_namespace,
                    &fact.external_label,
                    &fact.original_imported_name,
                    &fact.external_address,
                    &fact.external_space_address,
                    &fact.source_type,
                    fact.is_function,
                    &fact.data_type,
                    &fact.function_signature,
                    &fact.thunk_chain
                ]))
            );
        } else {
            fact.linkage_id = fact.linkage_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.function_id
            .cmp(&right.function_id)
            .then_with(|| left.external_library.cmp(&right.external_library))
            .then_with(|| left.external_label.cmp(&right.external_label))
            .then_with(|| left.local_address.cmp(&right.local_address))
            .then_with(|| left.linkage_id.cmp(&right.linkage_id))
    });
    facts.dedup_by(|left, right| left.linkage_id == right.linkage_id);
}

fn normalize_oracle_external_thunk_link_facts(facts: &mut Vec<GhidraOracleExternalThunkLinkFact>) {
    for (index, fact) in facts.iter_mut().enumerate() {
        fact.function_id = fact.function_id.trim().to_string();
        if fact.function_id.is_empty() {
            fact.function_id = format!("ghidra:function:thunk:{index}");
        }
        fact.address = normalize_address(&fact.address);
        normalize_optional_string(&mut fact.target_function_id);
        normalize_optional_address(&mut fact.target_address);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.link_id.trim().is_empty() {
            fact.link_id = format!(
                "ghidra:external_thunk:{}",
                stable_hash(json!([
                    index,
                    &fact.function_id,
                    &fact.address,
                    &fact.target_function_id,
                    &fact.target_address,
                    fact.recursive_depth,
                    fact.is_terminal
                ]))
            );
        } else {
            fact.link_id = fact.link_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.recursive_depth
            .cmp(&right.recursive_depth)
            .then_with(|| left.address.cmp(&right.address))
            .then_with(|| left.link_id.cmp(&right.link_id))
    });
    facts.dedup_by(|left, right| left.link_id == right.link_id);
}

fn normalize_oracle_function_prototype_facts(facts: &mut Vec<GhidraOracleFunctionPrototypeFact>) {
    for fact in &mut *facts {
        normalize_optional_string(&mut fact.function_id);
        fact.entry_point = normalize_address(&fact.entry_point);
        normalize_optional_string(&mut fact.name);
        fact.prototype = fact
            .prototype
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if fact.prototype.is_empty() {
            fact.prototype = "undefined unknown()".to_string();
        }
        normalize_optional_string(&mut fact.calling_convention);
        fact.return_type = fact
            .return_type
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if fact.return_type.is_empty() {
            fact.return_type = "undefined".to_string();
        }
        normalize_optional_string(&mut fact.return_type_id);
        fact.return_type_kind = fact
            .return_type_kind
            .as_deref()
            .map(normalize_oracle_data_type_kind)
            .filter(|kind| !kind.is_empty());
        normalize_oracle_variable_storage_fact(&mut fact.return_storage);
        normalize_optional_string(&mut fact.thunked_function_id);
        normalize_optional_address(&mut fact.thunked_entry_point);
        fact.signature_source = fact
            .signature_source
            .as_ref()
            .map(|source| source.trim().to_ascii_uppercase())
            .filter(|source| !source.is_empty());
        normalize_oracle_function_prototype_parameter_facts(&mut fact.parameters);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.prototype_id.trim().is_empty() {
            fact.prototype_id = format!(
                "ghidra:prototype:{}",
                stable_hash(json!([
                    &fact.function_id,
                    &fact.entry_point,
                    &fact.name,
                    &fact.prototype,
                    &fact.calling_convention,
                    &fact.return_type,
                    &fact.return_type_id,
                    &fact.return_type_kind,
                    fact.return_type_byte_len,
                    &fact.return_storage,
                    &fact.parameters,
                    fact.has_varargs,
                    fact.has_no_return,
                    fact.is_inline,
                    fact.is_thunk,
                    &fact.thunked_function_id,
                    &fact.thunked_entry_point,
                    fact.has_custom_storage,
                    fact.stack_purge_size,
                    &fact.signature_source
                ]))
            );
        } else {
            fact.prototype_id = fact.prototype_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.entry_point
            .cmp(&right.entry_point)
            .then_with(|| left.prototype_id.cmp(&right.prototype_id))
    });
    facts.dedup_by(|left, right| left.prototype_id == right.prototype_id);
}

fn normalize_oracle_function_prototype_parameter_facts(
    facts: &mut [GhidraOracleFunctionPrototypeParameterFact],
) {
    for (index, fact) in facts.iter_mut().enumerate() {
        if fact.ordinal < 0 {
            fact.ordinal = index as i32;
        }
        normalize_optional_string(&mut fact.name);
        fact.data_type = fact
            .data_type
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if fact.data_type.is_empty() {
            fact.data_type = "undefined".to_string();
        }
        normalize_optional_string(&mut fact.data_type_id);
        fact.data_type_kind = fact
            .data_type_kind
            .as_deref()
            .map(normalize_oracle_data_type_kind)
            .filter(|kind| !kind.is_empty());
        normalize_oracle_variable_storage_fact(&mut fact.storage);
        fact.auto_parameter_type = fact
            .auto_parameter_type
            .as_ref()
            .map(|kind| kind.trim().to_ascii_uppercase())
            .filter(|kind| !kind.is_empty());
        fact.formal_data_type = fact
            .formal_data_type
            .as_ref()
            .map(|data_type| data_type.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|data_type| !data_type.is_empty());
        normalize_optional_string(&mut fact.formal_data_type_id);
        fact.formal_data_type_kind = fact
            .formal_data_type_kind
            .as_deref()
            .map(normalize_oracle_data_type_kind)
            .filter(|kind| !kind.is_empty());
        normalize_optional_string(&mut fact.comment);
        fact.source_type = fact
            .source_type
            .as_ref()
            .map(|source| source.trim().to_ascii_uppercase())
            .filter(|source| !source.is_empty());
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
    }
    facts.sort_by(|left, right| {
        left.ordinal
            .cmp(&right.ordinal)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.storage.storage.cmp(&right.storage.storage))
    });
}

fn normalize_oracle_data_type_facts(facts: &mut Vec<GhidraOracleDataTypeFact>) {
    for fact in &mut *facts {
        fact.name = fact.name.split_whitespace().collect::<Vec<_>>().join(" ");
        if fact.name.is_empty() {
            fact.name = fact
                .display_name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .or(fact.path_name.as_deref())
                .unwrap_or("unknown")
                .to_string();
        }
        fact.display_name = fact
            .display_name
            .as_ref()
            .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|name| !name.is_empty())
            .or_else(|| Some(fact.name.clone()));
        fact.kind = normalize_oracle_data_type_kind(&fact.kind);
        normalize_optional_string(&mut fact.category_path);
        normalize_optional_string(&mut fact.path_name);
        normalize_optional_string(&mut fact.universal_id);
        let zero_length = fact.zero_length;
        fact.byte_len = fact
            .byte_len
            .filter(|byte_len| *byte_len > 0 || zero_length);
        fact.aligned_byte_len = fact
            .aligned_byte_len
            .filter(|byte_len| *byte_len > 0 || zero_length);
        if fact.type_id.trim().is_empty() {
            fact.type_id = format!(
                "ghidra:datatype:{}",
                stable_hash(json!([
                    &fact.path_name,
                    &fact.name,
                    &fact.display_name,
                    &fact.kind,
                    fact.byte_len,
                    fact.aligned_byte_len,
                    fact.component_count,
                    fact.zero_length,
                    fact.not_yet_defined
                ]))
            );
        } else {
            fact.type_id = fact.type_id.trim().to_string();
        }
        fact.packing_value = fact.packing_value.filter(|value| *value > 0);
        fact.minimum_alignment = fact.minimum_alignment.filter(|value| *value > 0);
        normalize_optional_string(&mut fact.base_type_id);
        fact.base_type_name = fact
            .base_type_name
            .as_ref()
            .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|name| !name.is_empty());
        normalize_optional_string(&mut fact.element_type_id);
        fact.element_type_name = fact
            .element_type_name
            .as_ref()
            .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|name| !name.is_empty());
        fact.element_count = fact.element_count.filter(|count| *count > 0);
        fact.element_byte_len = fact.element_byte_len.filter(|byte_len| *byte_len > 0);
        normalize_oracle_enum_value_facts(&mut fact.enum_values);
        normalize_oracle_data_type_field_facts(&mut fact.fields, &fact.type_id);
        fact.component_count = fact.component_count.max(fact.fields.len());
        fact.hard_dependency_ids = normalize_strings(fact.hard_dependency_ids.clone());
        fact.soft_dependency_ids = normalize_strings(fact.soft_dependency_ids.clone());
        if let Some(base_type_id) = fact.base_type_id.as_ref() {
            if fact.kind == "typedef" {
                fact.soft_dependency_ids.push(base_type_id.clone());
            } else {
                fact.hard_dependency_ids.push(base_type_id.clone());
            }
        }
        if let Some(element_type_id) = fact.element_type_id.as_ref() {
            fact.hard_dependency_ids.push(element_type_id.clone());
        }
        fact.hard_dependency_ids.extend(
            fact.fields
                .iter()
                .filter_map(|field| field.data_type_id.as_ref().cloned()),
        );
        fact.hard_dependency_ids = normalize_strings(fact.hard_dependency_ids.clone());
        fact.soft_dependency_ids = normalize_strings(fact.soft_dependency_ids.clone())
            .into_iter()
            .filter(|dependency_id| {
                dependency_id != &fact.type_id && !fact.hard_dependency_ids.contains(dependency_id)
            })
            .collect();
        fact.hard_dependency_ids
            .retain(|dependency_id| dependency_id != &fact.type_id);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
    }
    facts.sort_by(|left, right| {
        left.path_name
            .cmp(&right.path_name)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.type_id.cmp(&right.type_id))
    });
    facts.dedup_by(|left, right| left.type_id == right.type_id);
}

fn normalize_oracle_data_type_kind(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "struct" | "structure" => "structure".to_string(),
        "union" => "union".to_string(),
        "typedef" | "type_def" | "type-def" => "typedef".to_string(),
        "ptr" | "pointer" => "pointer".to_string(),
        "array" => "array".to_string(),
        "enum" | "enumeration" => "enum".to_string(),
        "primitive" | "builtin" | "built_in" | "built-in" => "primitive".to_string(),
        "function" | "function_definition" | "function-definition" => "function".to_string(),
        "void" => "void".to_string(),
        "opaque" => "opaque".to_string(),
        _ => "unknown".to_string(),
    }
}

fn normalize_oracle_data_type_field_facts(
    facts: &mut Vec<GhidraOracleDataTypeFieldFact>,
    owner_type_id: &str,
) {
    for (index, fact) in facts.iter_mut().enumerate() {
        fact.name = fact
            .name
            .as_ref()
            .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|name| !name.is_empty());
        fact.byte_len = fact.byte_len.filter(|byte_len| *byte_len > 0);
        normalize_optional_string(&mut fact.data_type_id);
        fact.data_type_name = fact
            .data_type_name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if fact.data_type_name.is_empty() {
            fact.data_type_name = "unknown".to_string();
        }
        normalize_optional_string(&mut fact.comment);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.field_id.trim().is_empty() {
            fact.field_id = format!(
                "ghidra:datatype_field:{}",
                stable_hash(json!([
                    owner_type_id,
                    index,
                    fact.ordinal,
                    fact.offset,
                    fact.byte_len,
                    fact.bit_offset,
                    fact.bit_size,
                    &fact.name,
                    &fact.data_type_id,
                    &fact.data_type_name
                ]))
            );
        } else {
            fact.field_id = fact.field_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.ordinal
            .cmp(&right.ordinal)
            .then_with(|| left.offset.cmp(&right.offset))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.field_id.cmp(&right.field_id))
    });
    facts.dedup_by(|left, right| left.field_id == right.field_id);
}

fn normalize_oracle_structure_field_access_facts(
    facts: &mut Vec<GhidraOracleStructureFieldAccessFact>,
) {
    for fact in &mut *facts {
        normalize_optional_string(&mut fact.function_id);
        fact.entry_point = normalize_address(&fact.entry_point);
        fact.address = normalize_address(&fact.address);
        fact.access_kind = normalize_oracle_structure_field_access_kind(&fact.access_kind);
        fact.structure_type_id = fact.structure_type_id.trim().to_string();
        fact.structure_name = fact
            .structure_name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if fact.structure_name.is_empty() {
            fact.structure_name = "anonymous_structure".to_string();
        }
        if fact.structure_type_id.is_empty() {
            fact.structure_type_id = format!(
                "ghidra:datatype:{}",
                stable_hash(json!([&fact.structure_name, fact.field_offset]))
            );
        }
        normalize_optional_string(&mut fact.root_variable_id);
        fact.root_variable_name = fact
            .root_variable_name
            .as_ref()
            .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|name| !name.is_empty());
        normalize_optional_string(&mut fact.root_storage);
        normalize_optional_string(&mut fact.field_id);
        fact.field_name = fact
            .field_name
            .as_ref()
            .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|name| !name.is_empty());
        fact.field_byte_len = fact.field_byte_len.filter(|byte_len| *byte_len > 0);
        normalize_optional_string(&mut fact.field_data_type_id);
        fact.field_data_type_name = fact
            .field_data_type_name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if fact.field_data_type_name.is_empty() {
            fact.field_data_type_name = "unknown".to_string();
        }
        fact.field_data_type_kind = fact
            .field_data_type_kind
            .as_ref()
            .map(|kind| normalize_oracle_data_type_kind(kind))
            .filter(|kind| kind != "unknown");
        fact.field_data_type_byte_len = fact
            .field_data_type_byte_len
            .filter(|byte_len| *byte_len > 0);
        fact.pcode_opcode = fact.pcode_opcode.trim().to_ascii_uppercase();
        if fact.pcode_opcode.is_empty() {
            fact.pcode_opcode = fact.access_kind.to_ascii_uppercase();
        }
        normalize_optional_string(&mut fact.pcode_op_id);
        normalize_optional_string(&mut fact.statement_id);
        normalize_optional_address(&mut fact.call_target_address);
        normalize_optional_string(&mut fact.pointer_relative_type);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.access_id.trim().is_empty() {
            fact.access_id = format!(
                "ghidra:structure_field_access:{}",
                stable_hash(json!([
                    &fact.function_id,
                    &fact.entry_point,
                    &fact.address,
                    &fact.access_kind,
                    &fact.structure_type_id,
                    &fact.field_id,
                    &fact.field_name,
                    fact.field_offset,
                    &fact.pcode_op_id,
                    &fact.statement_id,
                    fact.bit_offset,
                    fact.bit_size
                ]))
            );
        } else {
            fact.access_id = fact.access_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.function_id
            .cmp(&right.function_id)
            .then_with(|| left.structure_type_id.cmp(&right.structure_type_id))
            .then_with(|| left.field_offset.cmp(&right.field_offset))
            .then_with(|| left.address.cmp(&right.address))
            .then_with(|| left.access_id.cmp(&right.access_id))
    });
    facts.dedup_by(|left, right| left.access_id == right.access_id);
}

fn normalize_oracle_structure_field_access_kind(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "load" | "read" => "load".to_string(),
        "store" | "write" => "store".to_string(),
        "reference" | "ref" => "reference".to_string(),
        "call_input" | "callinput" | "call_arg" | "call_argument" => "call_input".to_string(),
        "pointer_add" | "ptradd" | "ptr_add" => "pointer_add".to_string(),
        "pointer_sub" | "ptrsub" | "ptr_sub" => "pointer_sub".to_string(),
        "pointer_relative" | "ptr_relative" | "relative" => "pointer_relative".to_string(),
        "bit_field_read" | "bitfield_read" | "bit_read" => "bit_field_read".to_string(),
        "bit_field_write" | "bitfield_write" | "bit_write" => "bit_field_write".to_string(),
        _ => "unknown".to_string(),
    }
}

fn normalize_oracle_enum_value_facts(facts: &mut Vec<GhidraOracleEnumValueFact>) {
    for (index, fact) in facts.iter_mut().enumerate() {
        fact.name = fact.name.split_whitespace().collect::<Vec<_>>().join("_");
        if fact.name.is_empty() {
            fact.name = format!("value_{}", fact.value);
        }
        normalize_optional_string(&mut fact.comment);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.value_id.trim().is_empty() {
            fact.value_id = format!(
                "ghidra:enum_value:{}",
                stable_hash(json!([index, &fact.name, fact.value, &fact.comment]))
            );
        } else {
            fact.value_id = fact.value_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.value
            .cmp(&right.value)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.value_id.cmp(&right.value_id))
    });
    facts.dedup_by(|left, right| left.value_id == right.value_id);
}

fn normalize_oracle_high_variable_facts(facts: &mut Vec<GhidraOracleHighVariableFact>) {
    for fact in &mut *facts {
        normalize_optional_string(&mut fact.function_id);
        fact.entry_point = normalize_address(&fact.entry_point);
        normalize_optional_string(&mut fact.symbol_id);
        fact.name = fact.name.split_whitespace().collect::<Vec<_>>().join(" ");
        if fact.name.is_empty() {
            fact.name = "unnamed".to_string();
        }
        fact.kind = normalize_oracle_high_variable_kind(&fact.kind);
        fact.data_type = fact
            .data_type
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if fact.data_type.is_empty() {
            fact.data_type = "undefined".to_string();
        }
        normalize_optional_string(&mut fact.data_type_id);
        normalize_oracle_variable_storage_fact(&mut fact.storage);
        normalize_optional_address(&mut fact.first_use_address);
        fact.mutability = normalize_oracle_mutability(&fact.mutability);
        normalize_oracle_high_variable_instance_facts(&mut fact.instances, &fact.variable_id);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.variable_id.trim().is_empty() {
            fact.variable_id = format!(
                "ghidra:highvar:{}",
                stable_hash(json!([
                    &fact.entry_point,
                    &fact.symbol_id,
                    &fact.name,
                    &fact.kind,
                    fact.category_index,
                    &fact.data_type,
                    &fact.data_type_id,
                    &fact.storage,
                    &fact.first_use_address,
                    fact.first_use_offset,
                    fact.high_symbol_offset
                ]))
            );
        } else {
            fact.variable_id = fact.variable_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.entry_point
            .cmp(&right.entry_point)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.category_index.cmp(&right.category_index))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.variable_id.cmp(&right.variable_id))
    });
    facts.dedup_by(|left, right| left.variable_id == right.variable_id);
}

fn normalize_oracle_high_variable_kind(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "parameter" | "param" => "parameter".to_string(),
        "local" | "stack" => "local".to_string(),
        "global" => "global".to_string(),
        "equate" => "equate".to_string(),
        "temporary" | "temp" => "temporary".to_string(),
        "return_storage" | "return-storage" | "returnstorage" => "return_storage".to_string(),
        _ => "unknown".to_string(),
    }
}

fn normalize_oracle_mutability(mutability: &str) -> String {
    match mutability.trim().to_ascii_lowercase().as_str() {
        "readonly" | "read_only" | "read-only" => "read_only".to_string(),
        "volatile" => "volatile".to_string(),
        "constant" | "const" => "constant".to_string(),
        "normal" | "" => "normal".to_string(),
        other => other
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .split('_')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("_"),
    }
}

fn normalize_oracle_variable_storage_fact(storage: &mut GhidraOracleVariableStorageFact) {
    storage.storage = storage
        .storage
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    storage.kind = normalize_oracle_variable_storage_kind(&storage.kind);
    normalize_oracle_variable_storage_piece_facts(&mut storage.pieces);
    let piece_len = storage
        .pieces
        .iter()
        .map(|piece| piece.byte_len)
        .sum::<u64>();
    storage.byte_len = storage.byte_len.max(piece_len);
    if storage.storage.is_empty() {
        storage.storage = storage
            .pieces
            .iter()
            .map(|piece| {
                let label = piece
                    .register
                    .as_deref()
                    .unwrap_or(piece.space.as_str())
                    .to_string();
                format!("{}:{:#x}:{}", label, piece.offset, piece.byte_len)
            })
            .collect::<Vec<_>>()
            .join(",");
    }
    if storage.storage.is_empty() {
        storage.storage = "unassigned".to_string();
    }
    normalize_optional_string(&mut storage.auto_parameter_type);
}

fn normalize_oracle_variable_storage_kind(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "register" | "reg" => "register".to_string(),
        "stack" => "stack".to_string(),
        "memory" | "mem" => "memory".to_string(),
        "constant" | "const" => "constant".to_string(),
        "unique" => "unique".to_string(),
        "hash" => "hash".to_string(),
        "join" => "join".to_string(),
        "void" => "void".to_string(),
        "unassigned" => "unassigned".to_string(),
        "bad" => "bad".to_string(),
        "compound" => "compound".to_string(),
        _ => "unknown".to_string(),
    }
}

fn normalize_oracle_variable_storage_piece_facts(
    facts: &mut Vec<GhidraOracleVariableStoragePieceFact>,
) {
    for (index, fact) in facts.iter_mut().enumerate() {
        fact.space = if fact.space.trim().is_empty() {
            if fact.is_stack {
                "stack".to_string()
            } else if fact.is_unique {
                "unique".to_string()
            } else if fact.is_constant {
                "constant".to_string()
            } else if fact.is_hash {
                "hash".to_string()
            } else if fact.register.is_some() {
                "register".to_string()
            } else {
                "unknown".to_string()
            }
        } else {
            normalize_oracle_variable_storage_kind(&fact.space)
        };
        fact.register = fact
            .register
            .as_ref()
            .map(|register| register.trim().to_string())
            .filter(|register| !register.is_empty());
        if fact.piece_id.trim().is_empty() {
            fact.piece_id = format!(
                "ghidra:storage_piece:{}",
                stable_hash(json!([
                    index,
                    &fact.space,
                    fact.offset,
                    fact.byte_len,
                    &fact.register,
                    fact.is_input,
                    fact.is_addr_tied,
                    fact.is_persistent,
                    fact.is_unique,
                    fact.is_constant,
                    fact.is_hash,
                    fact.is_stack
                ]))
            );
        } else {
            fact.piece_id = fact.piece_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.space
            .cmp(&right.space)
            .then_with(|| left.offset.cmp(&right.offset))
            .then_with(|| left.byte_len.cmp(&right.byte_len))
            .then_with(|| left.piece_id.cmp(&right.piece_id))
    });
    facts.dedup_by(|left, right| left.piece_id == right.piece_id);
}

fn normalize_oracle_high_variable_instance_facts(
    facts: &mut Vec<GhidraOracleHighVariableInstanceFact>,
    owner_variable_id: &str,
) {
    for (index, fact) in facts.iter_mut().enumerate() {
        normalize_oracle_variable_storage_fact(&mut fact.storage);
        normalize_optional_address(&mut fact.pc_address);
        normalize_optional_string(&mut fact.defining_pcode_id);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.instance_id.trim().is_empty() {
            fact.instance_id = format!(
                "ghidra:highvar_instance:{}",
                stable_hash(json!([
                    owner_variable_id,
                    index,
                    &fact.storage,
                    &fact.pc_address,
                    &fact.defining_pcode_id,
                    fact.merge_group,
                    fact.is_representative
                ]))
            );
        } else {
            fact.instance_id = fact.instance_id.trim().to_string();
        }
    }
    if !facts.iter().any(|fact| fact.is_representative) {
        if let Some(first) = facts.first_mut() {
            first.is_representative = true;
        }
    }
    facts.sort_by(|left, right| {
        right
            .is_representative
            .cmp(&left.is_representative)
            .then_with(|| left.pc_address.cmp(&right.pc_address))
            .then_with(|| left.storage.storage.cmp(&right.storage.storage))
            .then_with(|| left.instance_id.cmp(&right.instance_id))
    });
    facts.dedup_by(|left, right| left.instance_id == right.instance_id);
}

fn normalize_oracle_stack_frame_facts(facts: &mut Vec<GhidraOracleStackFrameFact>) {
    for fact in &mut *facts {
        normalize_optional_string(&mut fact.function_id);
        fact.entry_point = normalize_address(&fact.entry_point);
        fact.parameter_offset = fact.parameter_offset.filter(|offset| *offset != 128 * 1024);
        fact.return_address_offset = fact
            .return_address_offset
            .filter(|offset| *offset != 128 * 1024);
        fact.growth = normalize_oracle_stack_frame_growth(&fact.growth);
        fact.stack_pointer_register = fact
            .stack_pointer_register
            .as_ref()
            .map(|register| register.trim().to_string())
            .filter(|register| !register.is_empty());
        normalize_oracle_stack_frame_variable_facts(&mut fact.variables);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.frame_id.trim().is_empty() {
            fact.frame_id = format!(
                "ghidra:stack_frame:{}",
                stable_hash(json!([
                    &fact.function_id,
                    &fact.entry_point,
                    fact.frame_size,
                    fact.local_size,
                    fact.parameter_size,
                    fact.parameter_offset,
                    fact.return_address_offset,
                    &fact.growth,
                    &fact.stack_pointer_register,
                    fact.custom_variable_storage,
                    &fact.variables
                ]))
            );
        } else {
            fact.frame_id = fact.frame_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.entry_point
            .cmp(&right.entry_point)
            .then_with(|| left.frame_id.cmp(&right.frame_id))
    });
    facts.dedup_by(|left, right| left.frame_id == right.frame_id);
}

fn normalize_oracle_stack_frame_growth(growth: &str) -> String {
    match growth
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "negative" | "grows_negative" | "down" | "downward" => "negative".to_string(),
        "positive" | "grows_positive" | "up" | "upward" => "positive".to_string(),
        _ => "unknown".to_string(),
    }
}

fn normalize_oracle_stack_frame_variable_facts(
    facts: &mut Vec<GhidraOracleStackFrameVariableFact>,
) {
    for (index, fact) in facts.iter_mut().enumerate() {
        fact.name = fact.name.split_whitespace().collect::<Vec<_>>().join(" ");
        if fact.name.is_empty() {
            fact.name = format!("stack_var_{}", fact.offset);
        }
        fact.kind = normalize_oracle_stack_frame_variable_kind(&fact.kind);
        fact.data_type = fact
            .data_type
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if fact.data_type.is_empty() {
            fact.data_type = "undefined".to_string();
        }
        normalize_optional_string(&mut fact.data_type_id);
        normalize_oracle_variable_storage_fact(&mut fact.storage);
        fact.source_type = fact
            .source_type
            .as_ref()
            .map(|source| source.trim().to_string())
            .filter(|source| !source.is_empty());
        normalize_optional_string(&mut fact.high_variable_id);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.variable_id.trim().is_empty() {
            fact.variable_id = format!(
                "ghidra:stack_var:{}",
                stable_hash(json!([
                    index,
                    &fact.name,
                    &fact.kind,
                    fact.offset,
                    fact.byte_len,
                    fact.ordinal,
                    &fact.data_type,
                    &fact.data_type_id,
                    &fact.storage,
                    &fact.source_type,
                    &fact.high_variable_id
                ]))
            );
        } else {
            fact.variable_id = fact.variable_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.offset
            .cmp(&right.offset)
            .then_with(|| left.ordinal.cmp(&right.ordinal))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.variable_id.cmp(&right.variable_id))
    });
    facts.dedup_by(|left, right| left.variable_id == right.variable_id);
}

fn normalize_oracle_stack_frame_variable_kind(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "parameter" | "param" => "parameter".to_string(),
        "local" | "stack" => "local".to_string(),
        "save_area" | "savearea" | "saved_register" | "savedregister" => "save_area".to_string(),
        "return_address" | "returnaddress" => "return_address".to_string(),
        _ => "unknown".to_string(),
    }
}

fn normalize_oracle_parameter_measure_facts(facts: &mut Vec<GhidraOracleParameterMeasureFact>) {
    for fact in &mut *facts {
        normalize_optional_string(&mut fact.function_id);
        fact.entry_point = normalize_address(&fact.entry_point);
        fact.io = normalize_oracle_parameter_measure_io(&fact.io);
        fact.rank = normalize_oracle_parameter_measure_rank(&fact.rank);
        normalize_oracle_variable_storage_fact(&mut fact.storage);
        fact.data_type = fact
            .data_type
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if fact.data_type.is_empty() {
            fact.data_type = "undefined".to_string();
        }
        normalize_optional_string(&mut fact.data_type_id);
        fact.model_name = fact
            .model_name
            .as_ref()
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty());
        normalize_optional_string(&mut fact.base_variable_id);
        normalize_optional_string(&mut fact.source_statement_id);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.measure_id.trim().is_empty() {
            fact.measure_id = format!(
                "ghidra:param_measure:{}",
                stable_hash(json!([
                    &fact.function_id,
                    &fact.entry_point,
                    &fact.io,
                    &fact.rank,
                    fact.rank_value,
                    &fact.storage,
                    &fact.data_type,
                    &fact.data_type_id,
                    &fact.model_name,
                    fact.extra_pop,
                    fact.just_prototype,
                    &fact.base_variable_id,
                    &fact.source_statement_id,
                    fact.num_calls
                ]))
            );
        } else {
            fact.measure_id = fact.measure_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.entry_point
            .cmp(&right.entry_point)
            .then_with(|| left.io.cmp(&right.io))
            .then_with(|| left.rank.cmp(&right.rank))
            .then_with(|| left.storage.storage.cmp(&right.storage.storage))
            .then_with(|| left.measure_id.cmp(&right.measure_id))
    });
    facts.dedup_by(|left, right| left.measure_id == right.measure_id);
}

fn normalize_oracle_parameter_measure_io(io: &str) -> String {
    match io.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "input" | "in" | "parameter" | "param" => "input".to_string(),
        "output" | "out" | "return" | "retval" | "return_value" => "output".to_string(),
        _ => "input".to_string(),
    }
}

fn normalize_oracle_parameter_measure_rank(rank: &str) -> String {
    match rank.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "direct_write_without_read" | "directwritewithoutread" | "direct_write_no_read" => {
            "direct_write_without_read".to_string()
        }
        "direct_read" | "directread" => "direct_read".to_string(),
        "direct_write_with_read" | "directwritewithread" => "direct_write_with_read".to_string(),
        "direct_write_unknown_read" | "directwriteunknownread" => {
            "direct_write_unknown_read".to_string()
        }
        "sub_function_parameter" | "subfunctionparameter" | "sub_param" => {
            "sub_function_parameter".to_string()
        }
        "this_function_parameter" | "thisfunctionparameter" | "this_param" | "parameter" => {
            "this_function_parameter".to_string()
        }
        "sub_function_return" | "subfunctionreturn" | "sub_return" => {
            "sub_function_return".to_string()
        }
        "this_function_return" | "thisfunctionreturn" | "return" => {
            "this_function_return".to_string()
        }
        "indirect" => "indirect".to_string(),
        "worst_rank" | "worstrank" => "worst_rank".to_string(),
        _ => "unknown".to_string(),
    }
}

fn normalize_oracle_call_stack_effect_facts(facts: &mut Vec<GhidraOracleCallStackEffectFact>) {
    for fact in &mut *facts {
        normalize_optional_string(&mut fact.function_id);
        fact.entry_point = normalize_address(&fact.entry_point);
        fact.callsite_address = normalize_address(&fact.callsite_address);
        normalize_optional_string(&mut fact.callee_function_id);
        normalize_optional_string(&mut fact.callee_name);
        fact.call_opcode = fact
            .call_opcode
            .as_ref()
            .map(|opcode| opcode.trim().to_ascii_uppercase())
            .filter(|opcode| !opcode.is_empty());
        normalize_optional_string(&mut fact.prototype_model);
        fact.stack_pointer_register = fact
            .stack_pointer_register
            .as_ref()
            .map(|register| register.trim().to_string())
            .filter(|register| !register.is_empty());
        normalize_optional_string(&mut fact.stack_space);
        fact.status = normalize_oracle_call_stack_effect_status(&fact.status);
        fact.warnings = normalize_strings(fact.warnings.clone());
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.effect_id.trim().is_empty() {
            fact.effect_id = format!(
                "ghidra:stack_effect:{}",
                stable_hash(json!([
                    &fact.function_id,
                    &fact.entry_point,
                    &fact.callsite_address,
                    &fact.callee_function_id,
                    &fact.callee_name,
                    &fact.call_opcode,
                    &fact.prototype_model,
                    &fact.stack_pointer_register,
                    &fact.stack_space,
                    fact.stack_offset_before_call,
                    fact.instruction_stack_depth_change,
                    fact.stack_shift_bytes,
                    fact.purge_size_bytes,
                    fact.extra_pop_bytes,
                    fact.effective_extra_pop_bytes,
                    fact.companion_solution_bytes,
                    fact.solver_variable_count,
                    fact.missed_variable_count,
                    &fact.status,
                    &fact.warnings
                ]))
            );
        } else {
            fact.effect_id = fact.effect_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.entry_point
            .cmp(&right.entry_point)
            .then_with(|| left.callsite_address.cmp(&right.callsite_address))
            .then_with(|| left.effect_id.cmp(&right.effect_id))
    });
    facts.dedup_by(|left, right| left.effect_id == right.effect_id);
}

fn normalize_oracle_call_stack_effect_status(status: &str) -> String {
    match status
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "known" => "known".to_string(),
        "solved" | "computed" => "solved".to_string(),
        "guessed" | "estimated" => "guessed".to_string(),
        "invalid" => "invalid".to_string(),
        _ => "unknown".to_string(),
    }
}

fn normalize_optional_address(value: &mut Option<String>) {
    if let Some(normalized) = value.as_ref().map(|address| normalize_address(address)) {
        *value = if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        };
    }
}

fn normalize_oracle_annotation_facts(facts: &mut Vec<GhidraOracleAnnotationFact>) {
    for fact in &mut *facts {
        fact.kind = match fact.kind.trim().to_ascii_lowercase().as_str() {
            "bookmark" => "bookmark".to_string(),
            "comment" => "comment".to_string(),
            _ => "annotation".to_string(),
        };
        normalize_optional_string(&mut fact.function_id);
        normalize_optional_address(&mut fact.entry_point);
        fact.address = normalize_address(&fact.address);
        fact.comment_type = fact
            .comment_type
            .as_ref()
            .map(|comment_type| comment_type.trim().to_ascii_uppercase())
            .filter(|comment_type| !comment_type.is_empty());
        normalize_optional_string(&mut fact.bookmark_type);
        normalize_optional_string(&mut fact.bookmark_category);
        fact.message = fact
            .message
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if fact.message.is_empty() {
            fact.message = "Ghidra annotation".to_string();
        }
        normalize_optional_string(&mut fact.source_api);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.annotation_id.trim().is_empty() {
            fact.annotation_id = format!(
                "ghidra:annotation:{}",
                stable_hash(json!([
                    &fact.kind,
                    &fact.function_id,
                    &fact.entry_point,
                    &fact.address,
                    &fact.comment_type,
                    fact.bookmark_id,
                    &fact.bookmark_type,
                    &fact.bookmark_category,
                    &fact.message
                ]))
            );
        } else {
            fact.annotation_id = fact.annotation_id.trim().to_string();
        }
    }
    facts.sort_by(|left, right| {
        left.address
            .cmp(&right.address)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.comment_type.cmp(&right.comment_type))
            .then_with(|| left.bookmark_type.cmp(&right.bookmark_type))
            .then_with(|| left.annotation_id.cmp(&right.annotation_id))
    });
    facts.dedup_by(|left, right| left.annotation_id == right.annotation_id);
}

fn normalize_oracle_symbolic_summary_facts(facts: &mut Vec<GhidraOracleSymbolicSummaryFact>) {
    facts.sort_by(|left, right| {
        left.entry_point
            .cmp(&right.entry_point)
            .then_with(|| left.summary_id.cmp(&right.summary_id))
    });
    facts.dedup_by(|left, right| left.summary_id == right.summary_id);
    for fact in facts {
        fact.summary_id = fact.summary_id.trim().to_string();
        fact.function_id = fact
            .function_id
            .as_ref()
            .map(|function_id| function_id.trim().to_string())
            .filter(|function_id| !function_id.is_empty());
        fact.entry_point = normalize_address(&fact.entry_point);
        normalize_optional_string(&mut fact.thread_id);
        fact.registers_read = normalize_strings(fact.registers_read.clone());
        fact.registers_updated = normalize_strings(fact.registers_updated.clone());
        fact.solver_status = fact.solver_status.trim().to_ascii_lowercase();
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        normalize_oracle_symbolic_model_bindings(&mut fact.model_bindings);
        normalize_oracle_symbolic_preconditions(&mut fact.preconditions);
        normalize_oracle_symbolic_values(&mut fact.symbolic_values);
        normalize_oracle_memory_witnesses(&mut fact.memory_witnesses);
        if fact.valuation_hash.trim().is_empty() {
            fact.valuation_hash = stable_hash(json!([
                &fact.registers_read,
                &fact.registers_updated,
                &fact.preconditions,
                &fact.symbolic_values,
                &fact.memory_witnesses,
                &fact.model_bindings,
                &fact.solver_status
            ]));
        } else {
            fact.valuation_hash = fact.valuation_hash.trim().to_string();
        }
        if fact.summary_id.is_empty() {
            fact.summary_id = format!("ghidra:symz3_summary:{}", fact.valuation_hash);
        }
    }
}

fn normalize_oracle_symbolic_preconditions(facts: &mut Vec<GhidraOracleSymbolicPreconditionFact>) {
    facts.sort_by(|left, right| {
        left.step_index
            .cmp(&right.step_index)
            .then_with(|| left.precondition_id.cmp(&right.precondition_id))
    });
    facts.dedup_by(|left, right| left.precondition_id == right.precondition_id);
    for fact in facts {
        fact.precondition_id = fact.precondition_id.trim().to_string();
        fact.address = fact
            .address
            .as_ref()
            .map(|address| normalize_address(address))
            .filter(|address| !address.is_empty());
        normalize_optional_string(&mut fact.pcode_op_id);
        fact.serialized_expr = fact.serialized_expr.trim().to_string();
        normalize_optional_string(&mut fact.display_expr);
        fact.solver_status = fact.solver_status.trim().to_ascii_lowercase();
        normalize_oracle_symbolic_model_bindings(&mut fact.model_bindings);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.precondition_id.is_empty() {
            fact.precondition_id = format!(
                "ghidra:symz3_precondition:{}",
                stable_hash(json!([
                    fact.step_index,
                    &fact.address,
                    &fact.pcode_op_id,
                    fact.branch_taken,
                    &fact.serialized_expr,
                    &fact.solver_status
                ]))
            );
        }
    }
}

fn normalize_oracle_symbolic_values(facts: &mut Vec<GhidraOracleSymbolicValueFact>) {
    facts.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.value_id.cmp(&right.value_id))
    });
    facts.dedup_by(|left, right| left.value_id == right.value_id);
    for fact in facts {
        fact.value_id = fact.value_id.trim().to_string();
        fact.kind = fact.kind.trim().to_ascii_lowercase();
        fact.name = fact.name.trim().to_string();
        normalize_optional_string(&mut fact.space);
        fact.offset = fact
            .offset
            .as_ref()
            .map(|offset| offset.trim().to_ascii_lowercase())
            .filter(|offset| !offset.is_empty());
        fact.serialized_expr = fact.serialized_expr.trim().to_string();
        normalize_optional_string(&mut fact.display_expr);
        normalize_optional_string(&mut fact.concrete_value);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.value_id.is_empty() {
            fact.value_id = format!(
                "ghidra:symz3_value:{}",
                stable_hash(json!([
                    &fact.kind,
                    &fact.name,
                    &fact.space,
                    &fact.offset,
                    fact.byte_len,
                    &fact.serialized_expr
                ]))
            );
        }
    }
}

fn normalize_oracle_memory_witnesses(facts: &mut Vec<GhidraOracleMemoryWitnessFact>) {
    facts.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.address_expr.cmp(&right.address_expr))
            .then_with(|| left.witness_id.cmp(&right.witness_id))
    });
    facts.dedup_by(|left, right| left.witness_id == right.witness_id);
    for fact in facts {
        fact.witness_id = fact.witness_id.trim().to_string();
        fact.kind = fact.kind.trim().to_ascii_lowercase();
        fact.address_expr = fact.address_expr.trim().to_string();
        normalize_optional_string(&mut fact.address_display);
        fact.byte_len = fact.byte_len.max(1);
        fact.value_expr = fact
            .value_expr
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        normalize_optional_string(&mut fact.value_display);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.witness_id.is_empty() {
            fact.witness_id = format!(
                "ghidra:symz3_memory_witness:{}",
                stable_hash(json!([
                    &fact.kind,
                    &fact.address_expr,
                    fact.byte_len,
                    &fact.value_expr
                ]))
            );
        }
    }
}

fn normalize_oracle_symbolic_model_bindings(bindings: &mut Vec<GhidraOracleSymbolicModelBinding>) {
    bindings.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.value.cmp(&right.value))
    });
    bindings.dedup();
    for binding in bindings {
        binding.name = binding.name.trim().to_string();
        binding.value = binding.value.trim().to_string();
        if binding.value_hash.trim().is_empty() {
            binding.value_hash = stable_hash(json!([&binding.name, &binding.value]));
        } else {
            binding.value_hash = binding.value_hash.trim().to_string();
        }
    }
}

fn normalize_oracle_decompiler_diagnostic_facts(
    facts: &mut Vec<GhidraOracleDecompilerDiagnosticFact>,
) {
    facts.sort_by(|left, right| {
        left.entry_point
            .cmp(&right.entry_point)
            .then_with(|| left.address.cmp(&right.address))
            .then_with(|| left.diagnostic_id.cmp(&right.diagnostic_id))
    });
    facts.dedup_by(|left, right| left.diagnostic_id == right.diagnostic_id);
    for fact in facts {
        fact.diagnostic_id = fact.diagnostic_id.trim().to_string();
        fact.function_id = fact
            .function_id
            .as_ref()
            .map(|function_id| function_id.trim().to_string())
            .filter(|function_id| !function_id.is_empty());
        fact.entry_point = normalize_address(&fact.entry_point);
        fact.address = fact
            .address
            .as_ref()
            .map(|address| normalize_address(address))
            .filter(|address| !address.is_empty());
        fact.message = fact.message.trim().to_string();
        if fact.message.is_empty() {
            fact.message = "Unknown Ghidra decompiler diagnostic".to_string();
        }
        fact.category = normalize_oracle_diagnostic_category(&fact.category, &fact.message);
        fact.severity = normalize_oracle_diagnostic_severity(&fact.severity, &fact.message, fact);
        fact.placement = normalize_oracle_diagnostic_placement(&fact.placement);
        normalize_optional_string(&mut fact.source_pass);
        normalize_optional_string(&mut fact.source_rule);
        normalize_optional_string(&mut fact.remediation);
        infer_oracle_diagnostic_effects(fact);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.diagnostic_id.is_empty() {
            fact.diagnostic_id = format!(
                "ghidra:decompiler_diagnostic:{}",
                stable_hash(json!([
                    &fact.function_id,
                    &fact.entry_point,
                    &fact.address,
                    &fact.category,
                    &fact.severity,
                    &fact.placement,
                    &fact.message,
                    &fact.source_pass,
                    &fact.source_rule,
                    fact.completed,
                    fact.timed_out,
                    fact.cancelled,
                    fact.failed_to_start
                ]))
            );
        }
    }
}

fn normalize_oracle_semantic_signature_facts(facts: &mut Vec<GhidraOracleSemanticSignatureFact>) {
    facts.sort_by(|left, right| {
        left.entry_point
            .cmp(&right.entry_point)
            .then_with(|| left.signature_id.cmp(&right.signature_id))
    });
    facts.dedup_by(|left, right| left.signature_id == right.signature_id);
    for fact in facts {
        fact.signature_id = fact.signature_id.trim().to_string();
        fact.function_id = fact
            .function_id
            .as_ref()
            .map(|function_id| function_id.trim().to_string())
            .filter(|function_id| !function_id.is_empty());
        fact.entry_point = normalize_address(&fact.entry_point);
        fact.function_name = fact
            .function_name
            .as_ref()
            .map(|function_name| function_name.trim().to_string())
            .filter(|function_name| !function_name.is_empty());
        fact.feature_version = fact.feature_version.trim().to_string();
        if fact.feature_version.is_empty() {
            fact.feature_version = "ghidra-bsim-signature-v0".to_string();
        }
        fact.feature_hashes = normalize_strings(fact.feature_hashes.clone())
            .into_iter()
            .map(|hash| normalize_oracle_signature_hash(&hash))
            .filter(|hash| !hash.is_empty())
            .collect();
        fact.feature_hashes.sort();
        fact.feature_hashes.dedup();
        fact.debug_features.sort_by(|left, right| {
            left.hash
                .cmp(&right.hash)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.raw.cmp(&right.raw))
        });
        fact.debug_features.dedup();
        for feature in &mut fact.debug_features {
            feature.hash = normalize_oracle_signature_hash(&feature.hash);
            feature.kind = feature.kind.trim().to_ascii_lowercase();
            feature.raw = feature.raw.trim().to_string();
        }
        fact.debug_features
            .retain(|feature| !feature.hash.is_empty());
        fact.call_targets = normalize_strings(fact.call_targets.clone())
            .into_iter()
            .map(|target| normalize_address(&target))
            .filter(|target| !target.is_empty())
            .collect();
        fact.call_targets.sort();
        fact.call_targets.dedup();
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        if fact.signature_id.is_empty() {
            fact.signature_id = format!(
                "ghidra:semantic_signature:{}",
                stable_hash(json!([
                    &fact.function_id,
                    &fact.entry_point,
                    &fact.feature_version,
                    fact.signature_settings,
                    fact.decompiler_major_version,
                    fact.decompiler_minor_version,
                    &fact.feature_hashes,
                    &fact.call_targets,
                    fact.has_unimplemented,
                    fact.has_bad_data
                ]))
            );
        }
    }
}

fn normalize_oracle_signature_hash(hash: &str) -> String {
    let hash = hash.trim();
    if hash.is_empty() {
        return String::new();
    }
    if let Some(hex) = hash.strip_prefix("0x").or_else(|| hash.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16)
            .map(|value| format!("0x{value:08x}"))
            .unwrap_or_else(|_| hash.to_ascii_lowercase());
    }
    if let Ok(value) = hash.parse::<i64>() {
        return format!("0x{:08x}", value as u32);
    }
    hash.to_ascii_lowercase()
}

fn normalize_oracle_function_id_facts(facts: &mut Vec<GhidraOracleFunctionIdFact>) {
    facts.sort_by(|left, right| {
        left.entry_point
            .cmp(&right.entry_point)
            .then_with(|| left.fid_id.cmp(&right.fid_id))
    });
    facts.dedup_by(|left, right| left.fid_id == right.fid_id);
    for fact in facts {
        fact.fid_id = fact.fid_id.trim().to_string();
        fact.function_id = fact
            .function_id
            .as_ref()
            .map(|function_id| function_id.trim().to_string())
            .filter(|function_id| !function_id.is_empty());
        fact.entry_point = normalize_address(&fact.entry_point);
        fact.function_name = fact
            .function_name
            .as_ref()
            .map(|function_name| function_name.trim().to_string())
            .filter(|function_name| !function_name.is_empty());
        fact.feature_version = fact.feature_version.trim().to_string();
        if fact.feature_version.is_empty() {
            fact.feature_version = "ghidra-function-id-v0".to_string();
        }
        fact.hash_algorithm = fact.hash_algorithm.trim().to_ascii_lowercase();
        if fact.hash_algorithm.is_empty() {
            fact.hash_algorithm = "fnv1a64".to_string();
        }
        fact.full_hash = normalize_oracle_u64_hash(&fact.full_hash);
        fact.specific_hash = normalize_oracle_u64_hash(&fact.specific_hash);
        fact.evidence_ids = normalize_strings(fact.evidence_ids.clone());
        normalize_oracle_function_id_matches(&mut fact.matches);
        if fact.fid_id.is_empty() {
            fact.fid_id = format!(
                "ghidra:function_id:{}",
                stable_hash(json!([
                    &fact.function_id,
                    &fact.entry_point,
                    &fact.feature_version,
                    &fact.hash_algorithm,
                    fact.short_hash_code_unit_length,
                    fact.medium_hash_code_unit_length,
                    fact.code_unit_size,
                    &fact.full_hash,
                    fact.specific_hash_additional_size,
                    &fact.specific_hash
                ]))
            );
        }
    }
}

fn normalize_oracle_function_id_matches(matches: &mut Vec<GhidraOracleFunctionIdMatchFact>) {
    matches.sort_by(|left, right| {
        right
            .overall_score
            .partial_cmp(&left.overall_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.function_name.cmp(&right.function_name))
            .then_with(|| left.match_id.cmp(&right.match_id))
    });
    matches.dedup_by(|left, right| left.match_id == right.match_id);
    for fid_match in matches {
        fid_match.match_id = fid_match.match_id.trim().to_string();
        fid_match.function_name = fid_match.function_name.trim().to_string();
        normalize_optional_string(&mut fid_match.library_family_name);
        normalize_optional_string(&mut fid_match.library_version);
        normalize_optional_string(&mut fid_match.library_variant);
        normalize_optional_string(&mut fid_match.library_id);
        normalize_optional_string(&mut fid_match.function_record_id);
        normalize_optional_string(&mut fid_match.domain_path);
        fid_match.matched_entry_point = fid_match
            .matched_entry_point
            .as_ref()
            .map(|address| normalize_address(address))
            .filter(|address| !address.is_empty());
        fid_match.primary_function_match_mode = fid_match
            .primary_function_match_mode
            .as_ref()
            .map(|mode| mode.trim().to_ascii_lowercase())
            .filter(|mode| !mode.is_empty());
        fid_match.evidence_ids = normalize_strings(fid_match.evidence_ids.clone());
        if fid_match.match_id.is_empty() {
            fid_match.match_id = format!(
                "ghidra:function_id_match:{}",
                stable_hash(json!([
                    &fid_match.function_name,
                    &fid_match.library_family_name,
                    &fid_match.library_version,
                    &fid_match.library_variant,
                    &fid_match.function_record_id,
                    fid_match.overall_score
                ]))
            );
        }
    }
}

fn normalize_oracle_u64_hash(hash: &str) -> String {
    let hash = hash.trim();
    if hash.is_empty() {
        return String::new();
    }
    if let Some(hex) = hash.strip_prefix("0x").or_else(|| hash.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16)
            .map(|value| format!("0x{value:016x}"))
            .unwrap_or_else(|_| hash.to_ascii_lowercase());
    }
    if let Ok(value) = hash.parse::<i64>() {
        return format!("0x{:016x}", value as u64);
    }
    if let Ok(value) = hash.parse::<u64>() {
        return format!("0x{value:016x}");
    }
    hash.to_ascii_lowercase()
}

fn normalize_oracle_diagnostic_category(category: &str, message: &str) -> String {
    let category = category.trim().to_ascii_lowercase();
    if !category.is_empty() && category != "unknown" {
        return category;
    }
    let message = message.to_ascii_lowercase();
    if message.contains("jump") || message.contains("switch") {
        "jump_table".to_string()
    } else if message.contains("stack") || message.contains("spacebase") {
        "stack_pointer".to_string()
    } else if message.contains("prototype")
        || message.contains("calling convention")
        || message.contains("extrapop")
    {
        "prototype".to_string()
    } else if message.contains("paramid") || message.contains("parameter") {
        "parameter_id".to_string()
    } else if message.contains("type") || message.contains("datatype") {
        "type_recovery".to_string()
    } else if message.contains("variable") || message.contains("merge") {
        "variable_merge".to_string()
    } else if message.contains("export") || message.contains("c markup") {
        "export".to_string()
    } else if message.contains("flow")
        || message.contains("branch")
        || message.contains("instruction")
        || message.contains("unimplemented")
    {
        "flow".to_string()
    } else {
        "unknown".to_string()
    }
}

fn normalize_oracle_diagnostic_severity(
    severity: &str,
    message: &str,
    fact: &GhidraOracleDecompilerDiagnosticFact,
) -> String {
    let severity = severity.trim().to_ascii_lowercase();
    if !severity.is_empty() && severity != "unknown" {
        return severity;
    }
    let message = message.to_ascii_lowercase();
    if fact.failed_to_start || message.contains("fatal") || message.contains("crash") {
        "fatal".to_string()
    } else if fact.timed_out || !fact.completed || message.contains("error") {
        "error".to_string()
    } else if message.contains("warn") || !message.is_empty() {
        "warning".to_string()
    } else {
        "unknown".to_string()
    }
}

fn normalize_oracle_diagnostic_placement(placement: &str) -> String {
    match placement.trim().to_ascii_lowercase().as_str() {
        "header" | "body" | "bookmark" | "export" => placement.trim().to_ascii_lowercase(),
        _ => "unknown".to_string(),
    }
}

fn infer_oracle_diagnostic_effects(fact: &mut GhidraOracleDecompilerDiagnosticFact) {
    if matches!(fact.category.as_str(), "flow" | "jump_table") {
        fact.affects_control_flow = true;
    }
    if matches!(fact.category.as_str(), "prototype" | "parameter_id") {
        fact.affects_prototype = true;
    }
    if matches!(
        fact.category.as_str(),
        "stack_pointer" | "parameter_id" | "type_recovery" | "variable_merge" | "data_type"
    ) {
        fact.affects_data_flow = true;
    }
    let message = fact.message.to_ascii_lowercase();
    if message.contains("flow") || message.contains("branch") || message.contains("jump") {
        fact.affects_control_flow = true;
    }
    if message.contains("prototype") || message.contains("parameter") {
        fact.affects_prototype = true;
    }
    if message.contains("stack")
        || message.contains("data")
        || message.contains("type")
        || message.contains("variable")
    {
        fact.affects_data_flow = true;
    }
}

fn normalize_change_set(change_set: &mut AnalysisChangeSet) {
    normalize_ranges(&mut change_set.added_ranges);
    normalize_ranges(&mut change_set.removed_ranges);
    change_set.evidence_ids = normalize_strings(change_set.evidence_ids.clone());
}

fn normalize_ranges(ranges: &mut Vec<AnalysisAddressRange>) {
    for range in ranges.iter_mut() {
        range.start = range.start.trim().to_ascii_lowercase();
        range.end = range.end.trim().to_ascii_lowercase();
    }
    ranges.sort_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then_with(|| left.end.cmp(&right.end))
    });
    ranges.dedup();
}

fn normalize_optional_string(value: &mut Option<String>) {
    if let Some(normalized) = value.as_ref().map(|inner| inner.trim().to_string()) {
        *value = if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        };
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
    let run_node_id = run.run_id.clone();
    let artifact_node_id = tenant_scoped_node_id(&input.tenant_id, &input.artifact.artifact_id);
    let mut nodes = vec![
        NodeRecord::new(
            &run_node_id,
            [PROGRAM_ANALYSIS_RUN_LABEL],
            json!({
                "logical_id": &run.run_id,
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
            &artifact_node_id,
            [BINARY_ARTIFACT_LABEL],
            json!({
                "logical_id": &input.artifact.artifact_id,
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
        edge_id(&run_node_id, ANALYZES_ARTIFACT, &artifact_node_id),
        &run_node_id,
        ANALYZES_ARTIFACT,
        &artifact_node_id,
        edge_props(
            input,
            run,
            AUTHORITY_OBSERVED_FACT,
            &input.artifact.evidence_ids,
        ),
    )];

    for fact in &input.loader_facts {
        let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.fact_id);
        nodes.push(NodeRecord::new(
            &fact_node_id,
            [LOADER_FACT_LABEL],
            json!({
                "logical_id": &fact.fact_id,
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
            edge_id(&run_node_id, HAS_LOADER_FACT, &fact_node_id),
            &run_node_id,
            HAS_LOADER_FACT,
            &fact_node_id,
            edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
        ));
    }

    for fact in &input.instruction_facts {
        let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.instruction_id);
        nodes.push(NodeRecord::new(
            &fact_node_id,
            [INSTRUCTION_FACT_LABEL],
            json!({
                "logical_id": &fact.instruction_id,
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
            edge_id(&run_node_id, HAS_INSTRUCTION_FACT, &fact_node_id),
            &run_node_id,
            HAS_INSTRUCTION_FACT,
            &fact_node_id,
            edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
        ));
    }

    for function in &input.ir_functions {
        let function_node_id = tenant_scoped_node_id(&input.tenant_id, &function.function_id);
        nodes.push(NodeRecord::new(
            &function_node_id,
            [THEOREM_IR_FUNCTION_LABEL],
            json!({
                "logical_id": &function.function_id,
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
            edge_id(&run_node_id, HAS_THIR_FUNCTION, &function_node_id),
            &run_node_id,
            HAS_THIR_FUNCTION,
            &function_node_id,
            edge_props(input, run, AUTHORITY_DERIVED_FACT, &function.evidence_ids),
        ));
    }

    for fact in &input.data_flow_facts {
        let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.fact_id);
        nodes.push(NodeRecord::new(
            &fact_node_id,
            [PROGRAM_DATA_FLOW_FACT_LABEL],
            json!({
                "logical_id": &fact.fact_id,
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
            edge_id(&run_node_id, HAS_DATA_FLOW_FACT, &fact_node_id),
            &run_node_id,
            HAS_DATA_FLOW_FACT,
            &fact_node_id,
            edge_props(input, run, AUTHORITY_DERIVED_FACT, &fact.evidence_ids),
        ));
    }

    for fact in &input.pcode_facts {
        let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.pcode_id);
        nodes.push(NodeRecord::new(
            &fact_node_id,
            [PROGRAM_PCODE_FACT_LABEL],
            json!({
                "logical_id": &fact.pcode_id,
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "address": &fact.address,
                "sequence": fact.sequence,
                "opcode": &fact.opcode,
                "ghidra_opcode_id": fact.ghidra_opcode_id,
                "inputs": &fact.inputs,
                "output": &fact.output,
                "authority_layer": AUTHORITY_DERIVED_FACT,
                "evidence_ids": &fact.evidence_ids,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(&run_node_id, HAS_PROGRAM_PCODE_FACT, &fact_node_id),
            &run_node_id,
            HAS_PROGRAM_PCODE_FACT,
            &fact_node_id,
            edge_props(input, run, AUTHORITY_DERIVED_FACT, &fact.evidence_ids),
        ));
    }

    for fact in &input.reference_recovery_evidence {
        let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.evidence_id);
        nodes.push(NodeRecord::new(
            &fact_node_id,
            [REFERENCE_RECOVERY_EVIDENCE_LABEL],
            json!({
                "logical_id": &fact.evidence_id,
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "source_reference_id": &fact.source_reference_id,
                "from_address": &fact.from_address,
                "to_address": &fact.to_address,
                "reference_type": &fact.reference_type,
                "semantic_roles": &fact.semantic_roles,
                "operand_index": fact.operand_index,
                "source_type": &fact.source_type,
                "confidence": fact.confidence,
                "authority_layer": AUTHORITY_DERIVED_FACT,
                "evidence_ids": &fact.evidence_ids,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(&run_node_id, HAS_REFERENCE_RECOVERY_EVIDENCE, &fact_node_id),
            &run_node_id,
            HAS_REFERENCE_RECOVERY_EVIDENCE,
            &fact_node_id,
            edge_props(input, run, AUTHORITY_DERIVED_FACT, &fact.evidence_ids),
        ));
        let source_reference_node_id =
            tenant_scoped_node_id(&input.tenant_id, &fact.source_reference_id);
        edges.push(EdgeRecord::new(
            edge_id(
                &source_reference_node_id,
                REFERENCE_PRODUCES_RECOVERY_EVIDENCE,
                &fact_node_id,
            ),
            &source_reference_node_id,
            REFERENCE_PRODUCES_RECOVERY_EVIDENCE,
            &fact_node_id,
            edge_props(input, run, AUTHORITY_DERIVED_FACT, &fact.evidence_ids),
        ));
    }

    for hypothesis in &input.semantic_hypotheses {
        let hypothesis_node_id = tenant_scoped_node_id(&input.tenant_id, &hypothesis.hypothesis_id);
        nodes.push(NodeRecord::new(
            &hypothesis_node_id,
            [PROGRAM_SEMANTIC_HYPOTHESIS_LABEL],
            json!({
                "logical_id": &hypothesis.hypothesis_id,
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
            edge_id(&run_node_id, HAS_SEMANTIC_HYPOTHESIS, &hypothesis_node_id),
            &run_node_id,
            HAS_SEMANTIC_HYPOTHESIS,
            &hypothesis_node_id,
            edge_props(input, run, AUTHORITY_HYPOTHESIS, &hypothesis.evidence_ids),
        ));
    }

    for receipt in &input.analyzer_receipts {
        let receipt_node_id = tenant_scoped_node_id(&input.tenant_id, &receipt.receipt_id);
        nodes.push(NodeRecord::new(
            &receipt_node_id,
            [ANALYZER_PASS_RECEIPT_LABEL],
            json!({
                "logical_id": &receipt.receipt_id,
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
            edge_id(&run_node_id, HAS_ANALYZER_RECEIPT, &receipt_node_id),
            &run_node_id,
            HAS_ANALYZER_RECEIPT,
            &receipt_node_id,
            edge_props(input, run, &receipt.authority_layer, &receipt.evidence_ids),
        ));
    }

    for pass in &input.analysis_passes {
        let pass_node_id = tenant_scoped_node_id(&input.tenant_id, &pass.analyzer_id);
        nodes.push(NodeRecord::new(
            &pass_node_id,
            [ANALYSIS_PASS_SPEC_LABEL],
            json!({
                "logical_id": &pass.analyzer_id,
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "analyzer_type": &pass.analyzer_type,
                "description": &pass.description,
                "priority": pass.priority,
                "default_enabled": pass.default_enabled,
                "enabled": pass.enabled,
                "supports_one_time": pass.supports_one_time,
                "can_analyze": pass.can_analyze,
                "input_labels": &pass.input_labels,
                "output_labels": &pass.output_labels,
                "option_namespace": &pass.option_namespace,
                "authority_layer": AUTHORITY_DERIVED_FACT,
                "evidence_ids": &pass.evidence_ids,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(&run_node_id, HAS_ANALYSIS_PASS, &pass_node_id),
            &run_node_id,
            HAS_ANALYSIS_PASS,
            &pass_node_id,
            edge_props(input, run, AUTHORITY_DERIVED_FACT, &pass.evidence_ids),
        ));
    }

    for state in &input.analysis_scheduler_states {
        let state_node_id = tenant_scoped_node_id(&input.tenant_id, &state.scheduler_id);
        nodes.push(NodeRecord::new(
            &state_node_id,
            [ANALYSIS_SCHEDULER_STATE_LABEL],
            json!({
                "logical_id": &state.scheduler_id,
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "analyzer_id": &state.analyzer_id,
                "analyzer_type": &state.analyzer_type,
                "priority": state.priority,
                "default_enabled": state.default_enabled,
                "enabled": state.enabled,
                "scheduled": state.scheduled,
                "supports_one_time": state.supports_one_time,
                "option_namespace": &state.option_namespace,
                "added_ranges": &state.added_ranges,
                "removed_ranges": &state.removed_ranges,
                "input_hash": &state.input_hash,
                "authority_layer": AUTHORITY_DERIVED_FACT,
                "evidence_ids": &state.evidence_ids,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(&run_node_id, HAS_ANALYSIS_SCHEDULER_STATE, &state_node_id),
            &run_node_id,
            HAS_ANALYSIS_SCHEDULER_STATE,
            &state_node_id,
            edge_props(input, run, AUTHORITY_DERIVED_FACT, &state.evidence_ids),
        ));
        let pass_node_id = tenant_scoped_node_id(&input.tenant_id, &state.analyzer_id);
        if input
            .analysis_passes
            .iter()
            .any(|pass| pass.analyzer_id == state.analyzer_id)
        {
            edges.push(EdgeRecord::new(
                edge_id(&state_node_id, SCHEDULER_USES_PASS, &pass_node_id),
                &state_node_id,
                SCHEDULER_USES_PASS,
                &pass_node_id,
                edge_props(input, run, AUTHORITY_DERIVED_FACT, &state.evidence_ids),
            ));
        }
    }

    for item in &input.analysis_work_items {
        let item_node_id = tenant_scoped_node_id(&input.tenant_id, &item.work_item_id);
        nodes.push(NodeRecord::new(
            &item_node_id,
            [ANALYSIS_WORK_ITEM_LABEL],
            json!({
                "logical_id": &item.work_item_id,
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "analyzer_id": &item.analyzer_id,
                "analyzer_type": &item.analyzer_type,
                "priority": item.priority,
                "status": &item.status,
                "added_ranges": &item.added_ranges,
                "removed_ranges": &item.removed_ranges,
                "input_hash": &item.input_hash,
                "authority_layer": AUTHORITY_DERIVED_FACT,
                "evidence_ids": &item.evidence_ids,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(&run_node_id, HAS_ANALYSIS_WORK_ITEM, &item_node_id),
            &run_node_id,
            HAS_ANALYSIS_WORK_ITEM,
            &item_node_id,
            edge_props(input, run, AUTHORITY_DERIVED_FACT, &item.evidence_ids),
        ));
        let pass_node_id = tenant_scoped_node_id(&input.tenant_id, &item.analyzer_id);
        if input
            .analysis_passes
            .iter()
            .any(|pass| pass.analyzer_id == item.analyzer_id)
        {
            edges.push(EdgeRecord::new(
                edge_id(&item_node_id, WORK_ITEM_USES_PASS, &pass_node_id),
                &item_node_id,
                WORK_ITEM_USES_PASS,
                &pass_node_id,
                edge_props(input, run, AUTHORITY_DERIVED_FACT, &item.evidence_ids),
            ));
        }
    }

    for drift in &input.analysis_drifts {
        let drift_node_id = tenant_scoped_node_id(&input.tenant_id, &drift.drift_id);
        nodes.push(NodeRecord::new(
            &drift_node_id,
            [ANALYSIS_DRIFT_LABEL],
            json!({
                "logical_id": &drift.drift_id,
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "oracle_fixture_id": &drift.oracle_fixture_id,
                "fact_kind": &drift.fact_kind,
                "drift_kind": &drift.drift_kind,
                "expected": &drift.expected,
                "observed": &drift.observed,
                "severity": &drift.severity,
                "authority_layer": AUTHORITY_DERIVED_FACT,
                "evidence_ids": &drift.evidence_ids,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(&run_node_id, HAS_ANALYSIS_DRIFT, &drift_node_id),
            &run_node_id,
            HAS_ANALYSIS_DRIFT,
            &drift_node_id,
            edge_props(input, run, AUTHORITY_DERIVED_FACT, &drift.evidence_ids),
        ));
    }

    for trace in &input.runtime_traces {
        let trace_node_id = tenant_scoped_node_id(&input.tenant_id, &trace.trace_id);
        nodes.push(NodeRecord::new(
            &trace_node_id,
            [RUNTIME_TRACE_LABEL],
            json!({
                "logical_id": &trace.trace_id,
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "language_id": &trace.language_id,
                "compiler_spec_id": &trace.compiler_spec_id,
                "emulator_cache_version": &trace.emulator_cache_version,
                "capture_source": &trace.capture_source,
                "authority_layer": AUTHORITY_OBSERVED_FACT,
                "evidence_ids": &trace.evidence_ids,
                "source": SOURCE,
            }),
        ));
        edges.push(EdgeRecord::new(
            edge_id(&run_node_id, HAS_RUNTIME_TRACE, &trace_node_id),
            &run_node_id,
            HAS_RUNTIME_TRACE,
            &trace_node_id,
            edge_props(input, run, AUTHORITY_OBSERVED_FACT, &trace.evidence_ids),
        ));
    }

    for snapshot in &input.trace_snapshots {
        let snapshot_node_id = tenant_scoped_node_id(&input.tenant_id, &snapshot.snapshot_id);
        nodes.push(NodeRecord::new(
            &snapshot_node_id,
            [TRACE_SNAPSHOT_LABEL],
            json!({
                "logical_id": &snapshot.snapshot_id,
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "trace_id": &snapshot.trace_id,
                "snap": snapshot.snap,
                "description": &snapshot.description,
                "real_time_ms": snapshot.real_time_ms,
                "event_thread_id": &snapshot.event_thread_id,
                "schedule": &snapshot.schedule,
                "version": snapshot.version,
                "forked": snapshot.forked,
                "stale": snapshot.stale,
                "authority_layer": AUTHORITY_OBSERVED_FACT,
                "evidence_ids": &snapshot.evidence_ids,
                "source": SOURCE,
            }),
        ));
        let trace_node_id = tenant_scoped_node_id(&input.tenant_id, &snapshot.trace_id);
        edges.push(EdgeRecord::new(
            edge_id(&trace_node_id, TRACE_HAS_SNAPSHOT, &snapshot_node_id),
            &trace_node_id,
            TRACE_HAS_SNAPSHOT,
            &snapshot_node_id,
            edge_props(input, run, AUTHORITY_OBSERVED_FACT, &snapshot.evidence_ids),
        ));
    }

    for event in &input.trace_events {
        let event_node_id = tenant_scoped_node_id(&input.tenant_id, &event.event_id);
        nodes.push(NodeRecord::new(
            &event_node_id,
            [TRACE_EVENT_LABEL],
            json!({
                "logical_id": &event.event_id,
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "trace_id": &event.trace_id,
                "snapshot_id": &event.snapshot_id,
                "sequence": event.sequence,
                "thread_id": &event.thread_id,
                "kind": &event.kind,
                "address_space": &event.address_space,
                "offset": &event.offset,
                "size": event.size,
                "register": &event.register,
                "value_hash": &event.value_hash,
                "pcode_op": &event.pcode_op,
                "authority_layer": AUTHORITY_OBSERVED_FACT,
                "evidence_ids": &event.evidence_ids,
                "source": SOURCE,
            }),
        ));
        let snapshot_node_id = tenant_scoped_node_id(&input.tenant_id, &event.snapshot_id);
        edges.push(EdgeRecord::new(
            edge_id(&snapshot_node_id, SNAPSHOT_HAS_EVENT, &event_node_id),
            &snapshot_node_id,
            SNAPSHOT_HAS_EVENT,
            &event_node_id,
            edge_props(input, run, AUTHORITY_OBSERVED_FACT, &event.evidence_ids),
        ));
    }

    for mark in &input.taint_marks {
        let mark_node_id = tenant_scoped_node_id(&input.tenant_id, &mark.taint_id);
        nodes.push(NodeRecord::new(
            &mark_node_id,
            [TAINT_MARK_LABEL],
            json!({
                "logical_id": &mark.taint_id,
                "tenant_id": &input.tenant_id,
                "artifact_id": &input.artifact.artifact_id,
                "run_id": &run.run_id,
                "trace_id": &mark.trace_id,
                "snapshot_id": &mark.snapshot_id,
                "event_id": &mark.event_id,
                "address_space": &mark.address_space,
                "offset": &mark.offset,
                "size": mark.size,
                "labels": &mark.labels,
                "originating_op": &mark.originating_op,
                "indirect_read": mark.indirect_read,
                "indirect_write": mark.indirect_write,
                "authority_layer": AUTHORITY_DERIVED_FACT,
                "evidence_ids": &mark.evidence_ids,
                "source": SOURCE,
            }),
        ));
        let snapshot_node_id = tenant_scoped_node_id(&input.tenant_id, &mark.snapshot_id);
        edges.push(EdgeRecord::new(
            edge_id(&snapshot_node_id, SNAPSHOT_HAS_TAINT_MARK, &mark_node_id),
            &snapshot_node_id,
            SNAPSHOT_HAS_TAINT_MARK,
            &mark_node_id,
            edge_props(input, run, AUTHORITY_DERIVED_FACT, &mark.evidence_ids),
        ));
        if let Some(event_id) = &mark.event_id {
            let event_node_id = tenant_scoped_node_id(&input.tenant_id, event_id);
            edges.push(EdgeRecord::new(
                edge_id(&event_node_id, EVENT_HAS_TAINT_MARK, &mark_node_id),
                &event_node_id,
                EVENT_HAS_TAINT_MARK,
                &mark_node_id,
                edge_props(input, run, AUTHORITY_DERIVED_FACT, &mark.evidence_ids),
            ));
        }
    }

    if let Some(fixture) = &input.oracle_fixture {
        let fixture_node_id = tenant_scoped_node_id(&input.tenant_id, &fixture.fixture_id);
        nodes.push(NodeRecord::new(
            &fixture_node_id,
            [GHIDRA_ORACLE_FIXTURE_LABEL],
            json!({
                "logical_id": &fixture.fixture_id,
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
            edge_id(&run_node_id, DERIVED_FROM_ORACLE, &fixture_node_id),
            &run_node_id,
            DERIVED_FROM_ORACLE,
            &fixture_node_id,
            edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fixture.evidence_ids),
        ));

        for fact in &input.oracle_function_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.function_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_FUNCTION_FACT_LABEL],
                json!({
                    "logical_id": &fact.function_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "entry_point": &fact.entry_point,
                    "name": &fact.name,
                    "body_start": &fact.body_start,
                    "body_end": &fact.body_end,
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_FUNCTION_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_FUNCTION_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_pcode_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.pcode_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_PCODE_OP_FACT_LABEL],
                json!({
                    "logical_id": &fact.pcode_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "address": &fact.address,
                    "sequence": fact.sequence,
                    "opcode": &fact.opcode,
                    "ghidra_opcode_id": fact.ghidra_opcode_id,
                    "inputs": &fact.inputs,
                    "output": &fact.output,
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_PCODE_OP_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_PCODE_OP_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_reference_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.reference_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_REFERENCE_FACT_LABEL],
                json!({
                    "logical_id": &fact.reference_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "from_address": &fact.from_address,
                    "to_address": &fact.to_address,
                    "reference_type": &fact.reference_type,
                    "operand_index": fact.operand_index,
                    "is_primary": fact.is_primary,
                    "source_type": &fact.source_type,
                    "semantic_roles": &fact.semantic_roles,
                    "is_external": fact.is_external,
                    "is_memory": fact.is_memory,
                    "is_register": fact.is_register,
                    "is_stack": fact.is_stack,
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_REFERENCE_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_REFERENCE_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_call_edge_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.edge_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_CALL_EDGE_FACT_LABEL],
                json!({
                    "logical_id": &fact.edge_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "source_entry": &fact.source_entry,
                    "target_entry": &fact.target_entry,
                    "callsite_address": &fact.callsite_address,
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_CALL_EDGE_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_CALL_EDGE_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_jump_table_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.jump_table_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_JUMP_TABLE_FACT_LABEL],
                json!({
                    "logical_id": &fact.jump_table_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "function_id": &fact.function_id,
                    "entry_point": &fact.entry_point,
                    "switch_address": &fact.switch_address,
                    "switch_statement_id": &fact.switch_statement_id,
                    "display_format": &fact.display_format,
                    "cases": &fact.cases,
                    "case_count": fact.cases.len(),
                    "load_tables": &fact.load_tables,
                    "load_table_count": fact.load_tables.len(),
                    "override_applied": fact.override_applied,
                    "references_complete": fact.references_complete,
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_JUMP_TABLE_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_JUMP_TABLE_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_equate_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.equate_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_EQUATE_FACT_LABEL],
                json!({
                    "logical_id": &fact.equate_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "function_id": &fact.function_id,
                    "name": &fact.name,
                    "display_name": &fact.display_name,
                    "value": fact.value,
                    "display_value": &fact.display_value,
                    "format": &fact.format,
                    "enum_uuid": &fact.enum_uuid,
                    "enum_based": fact.enum_based,
                    "valid_uuid": fact.valid_uuid,
                    "references": &fact.references,
                    "reference_count": fact.references.len(),
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_EQUATE_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_EQUATE_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_external_linkage_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.linkage_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_EXTERNAL_LINKAGE_FACT_LABEL],
                json!({
                    "logical_id": &fact.linkage_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "function_id": &fact.function_id,
                    "local_address": &fact.local_address,
                    "thunked_function_id": &fact.thunked_function_id,
                    "thunked_address": &fact.thunked_address,
                    "recursive_target_function_id": &fact.recursive_target_function_id,
                    "recursive_target_address": &fact.recursive_target_address,
                    "external_library": &fact.external_library,
                    "external_library_path": &fact.external_library_path,
                    "library_ordinal": fact.library_ordinal,
                    "parent_namespace": &fact.parent_namespace,
                    "external_label": &fact.external_label,
                    "original_imported_name": &fact.original_imported_name,
                    "external_address": &fact.external_address,
                    "external_space_address": &fact.external_space_address,
                    "source_type": &fact.source_type,
                    "is_function": fact.is_function,
                    "data_type": &fact.data_type,
                    "function_signature": &fact.function_signature,
                    "thunk_chain": &fact.thunk_chain,
                    "thunk_chain_len": fact.thunk_chain.len(),
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_EXTERNAL_LINKAGE_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_EXTERNAL_LINKAGE_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_function_prototype_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.prototype_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_FUNCTION_PROTOTYPE_FACT_LABEL],
                json!({
                    "logical_id": &fact.prototype_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "function_id": &fact.function_id,
                    "entry_point": &fact.entry_point,
                    "name": &fact.name,
                    "prototype": &fact.prototype,
                    "calling_convention": &fact.calling_convention,
                    "return_type": &fact.return_type,
                    "return_type_id": &fact.return_type_id,
                    "return_type_kind": &fact.return_type_kind,
                    "return_type_byte_len": fact.return_type_byte_len,
                    "return_storage": &fact.return_storage,
                    "parameters": &fact.parameters,
                    "parameter_count": fact.parameters.len(),
                    "has_varargs": fact.has_varargs,
                    "has_no_return": fact.has_no_return,
                    "is_inline": fact.is_inline,
                    "is_thunk": fact.is_thunk,
                    "thunked_function_id": &fact.thunked_function_id,
                    "thunked_entry_point": &fact.thunked_entry_point,
                    "has_custom_storage": fact.has_custom_storage,
                    "stack_purge_size": fact.stack_purge_size,
                    "signature_source": &fact.signature_source,
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_FUNCTION_PROTOTYPE_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_FUNCTION_PROTOTYPE_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_data_type_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.type_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_DATA_TYPE_FACT_LABEL],
                json!({
                    "logical_id": &fact.type_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "name": &fact.name,
                    "display_name": &fact.display_name,
                    "kind": &fact.kind,
                    "category_path": &fact.category_path,
                    "path_name": &fact.path_name,
                    "universal_id": &fact.universal_id,
                    "byte_len": fact.byte_len,
                    "aligned_byte_len": fact.aligned_byte_len,
                    "component_count": fact.component_count,
                    "packing_enabled": fact.packing_enabled,
                    "packing_value": fact.packing_value,
                    "minimum_alignment": fact.minimum_alignment,
                    "not_yet_defined": fact.not_yet_defined,
                    "zero_length": fact.zero_length,
                    "base_type_id": &fact.base_type_id,
                    "base_type_name": &fact.base_type_name,
                    "element_type_id": &fact.element_type_id,
                    "element_type_name": &fact.element_type_name,
                    "element_count": fact.element_count,
                    "element_byte_len": fact.element_byte_len,
                    "enum_signed": fact.enum_signed,
                    "enum_values": &fact.enum_values,
                    "enum_value_count": fact.enum_values.len(),
                    "fields": &fact.fields,
                    "field_count": fact.fields.len(),
                    "hard_dependency_ids": &fact.hard_dependency_ids,
                    "soft_dependency_ids": &fact.soft_dependency_ids,
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_DATA_TYPE_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_DATA_TYPE_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_high_variable_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.variable_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_HIGH_VARIABLE_FACT_LABEL],
                json!({
                    "logical_id": &fact.variable_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "function_id": &fact.function_id,
                    "entry_point": &fact.entry_point,
                    "symbol_id": &fact.symbol_id,
                    "name": &fact.name,
                    "kind": &fact.kind,
                    "category_index": fact.category_index,
                    "data_type": &fact.data_type,
                    "data_type_id": &fact.data_type_id,
                    "storage": &fact.storage,
                    "first_use_address": &fact.first_use_address,
                    "first_use_offset": fact.first_use_offset,
                    "name_locked": fact.name_locked,
                    "type_locked": fact.type_locked,
                    "isolated": fact.isolated,
                    "is_this_pointer": fact.is_this_pointer,
                    "is_hidden_return": fact.is_hidden_return,
                    "mutability": &fact.mutability,
                    "high_symbol_offset": fact.high_symbol_offset,
                    "instances": &fact.instances,
                    "instance_count": fact.instances.len(),
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_HIGH_VARIABLE_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_HIGH_VARIABLE_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_stack_frame_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.frame_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_STACK_FRAME_FACT_LABEL],
                json!({
                    "logical_id": &fact.frame_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "function_id": &fact.function_id,
                    "entry_point": &fact.entry_point,
                    "frame_size": fact.frame_size,
                    "local_size": fact.local_size,
                    "parameter_size": fact.parameter_size,
                    "parameter_offset": fact.parameter_offset,
                    "return_address_offset": fact.return_address_offset,
                    "growth": &fact.growth,
                    "stack_pointer_register": &fact.stack_pointer_register,
                    "custom_variable_storage": fact.custom_variable_storage,
                    "variables": &fact.variables,
                    "variable_count": fact.variables.len(),
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_STACK_FRAME_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_STACK_FRAME_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_parameter_measure_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.measure_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_PARAMETER_MEASURE_FACT_LABEL],
                json!({
                    "logical_id": &fact.measure_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "function_id": &fact.function_id,
                    "entry_point": &fact.entry_point,
                    "io": &fact.io,
                    "rank": &fact.rank,
                    "rank_value": fact.rank_value,
                    "storage": &fact.storage,
                    "data_type": &fact.data_type,
                    "data_type_id": &fact.data_type_id,
                    "model_name": &fact.model_name,
                    "extra_pop": fact.extra_pop,
                    "just_prototype": fact.just_prototype,
                    "base_variable_id": &fact.base_variable_id,
                    "source_statement_id": &fact.source_statement_id,
                    "num_calls": fact.num_calls,
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_PARAMETER_MEASURE_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_PARAMETER_MEASURE_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_call_stack_effect_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.effect_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_CALL_STACK_EFFECT_FACT_LABEL],
                json!({
                    "logical_id": &fact.effect_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "function_id": &fact.function_id,
                    "entry_point": &fact.entry_point,
                    "callsite_address": &fact.callsite_address,
                    "callee_function_id": &fact.callee_function_id,
                    "callee_name": &fact.callee_name,
                    "call_opcode": &fact.call_opcode,
                    "prototype_model": &fact.prototype_model,
                    "stack_pointer_register": &fact.stack_pointer_register,
                    "stack_space": &fact.stack_space,
                    "stack_offset_before_call": fact.stack_offset_before_call,
                    "instruction_stack_depth_change": fact.instruction_stack_depth_change,
                    "stack_shift_bytes": fact.stack_shift_bytes,
                    "purge_size_bytes": fact.purge_size_bytes,
                    "extra_pop_bytes": fact.extra_pop_bytes,
                    "effective_extra_pop_bytes": fact.effective_extra_pop_bytes,
                    "companion_solution_bytes": fact.companion_solution_bytes,
                    "solver_variable_count": fact.solver_variable_count,
                    "missed_variable_count": fact.missed_variable_count,
                    "status": &fact.status,
                    "warnings": &fact.warnings,
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_CALL_STACK_EFFECT_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_CALL_STACK_EFFECT_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_annotation_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.annotation_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_ANNOTATION_FACT_LABEL],
                json!({
                    "logical_id": &fact.annotation_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "kind": &fact.kind,
                    "function_id": &fact.function_id,
                    "entry_point": &fact.entry_point,
                    "address": &fact.address,
                    "comment_type": &fact.comment_type,
                    "bookmark_id": fact.bookmark_id,
                    "bookmark_type": &fact.bookmark_type,
                    "bookmark_category": &fact.bookmark_category,
                    "message": &fact.message,
                    "source_api": &fact.source_api,
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_ANNOTATION_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_ANNOTATION_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_structure_field_access_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.access_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_STRUCTURE_FIELD_ACCESS_FACT_LABEL],
                json!({
                    "logical_id": &fact.access_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "function_id": &fact.function_id,
                    "entry_point": &fact.entry_point,
                    "address": &fact.address,
                    "access_kind": &fact.access_kind,
                    "structure_type_id": &fact.structure_type_id,
                    "structure_name": &fact.structure_name,
                    "root_variable_id": &fact.root_variable_id,
                    "root_variable_name": &fact.root_variable_name,
                    "root_storage": &fact.root_storage,
                    "field_id": &fact.field_id,
                    "field_name": &fact.field_name,
                    "field_offset": fact.field_offset,
                    "field_byte_len": fact.field_byte_len,
                    "field_data_type_id": &fact.field_data_type_id,
                    "field_data_type_name": &fact.field_data_type_name,
                    "field_data_type_kind": &fact.field_data_type_kind,
                    "field_data_type_byte_len": fact.field_data_type_byte_len,
                    "pcode_opcode": &fact.pcode_opcode,
                    "pcode_op_id": &fact.pcode_op_id,
                    "statement_id": &fact.statement_id,
                    "call_target_address": &fact.call_target_address,
                    "call_input_slot": fact.call_input_slot,
                    "pointer_relative_type": &fact.pointer_relative_type,
                    "recursive_call_depth": fact.recursive_call_depth,
                    "creates_new_structure": fact.creates_new_structure,
                    "extends_existing_structure": fact.extends_existing_structure,
                    "bit_offset": fact.bit_offset,
                    "bit_size": fact.bit_size,
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_STRUCTURE_FIELD_ACCESS_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_STRUCTURE_FIELD_ACCESS_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_symbolic_summary_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.summary_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_SYMBOLIC_SUMMARY_FACT_LABEL],
                json!({
                    "logical_id": &fact.summary_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "function_id": &fact.function_id,
                    "entry_point": &fact.entry_point,
                    "thread_id": &fact.thread_id,
                    "preconditions": &fact.preconditions,
                    "registers_read": &fact.registers_read,
                    "registers_updated": &fact.registers_updated,
                    "symbolic_values": &fact.symbolic_values,
                    "memory_witnesses": &fact.memory_witnesses,
                    "solver_status": &fact.solver_status,
                    "model_bindings": &fact.model_bindings,
                    "valuation_hash": &fact.valuation_hash,
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_SYMBOLIC_SUMMARY_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_SYMBOLIC_SUMMARY_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_decompiler_diagnostic_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.diagnostic_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_DECOMPILER_DIAGNOSTIC_FACT_LABEL],
                json!({
                    "logical_id": &fact.diagnostic_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "function_id": &fact.function_id,
                    "entry_point": &fact.entry_point,
                    "address": &fact.address,
                    "category": &fact.category,
                    "severity": &fact.severity,
                    "placement": &fact.placement,
                    "message": &fact.message,
                    "source_pass": &fact.source_pass,
                    "source_rule": &fact.source_rule,
                    "affects_control_flow": fact.affects_control_flow,
                    "affects_prototype": fact.affects_prototype,
                    "affects_data_flow": fact.affects_data_flow,
                    "remediation": &fact.remediation,
                    "completed": fact.completed,
                    "timed_out": fact.timed_out,
                    "cancelled": fact.cancelled,
                    "failed_to_start": fact.failed_to_start,
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_DECOMPILER_DIAGNOSTIC_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_DECOMPILER_DIAGNOSTIC_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_semantic_signature_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.signature_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_SEMANTIC_SIGNATURE_FACT_LABEL],
                json!({
                    "logical_id": &fact.signature_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "function_id": &fact.function_id,
                    "entry_point": &fact.entry_point,
                    "function_name": &fact.function_name,
                    "feature_version": &fact.feature_version,
                    "signature_settings": fact.signature_settings,
                    "decompiler_major_version": fact.decompiler_major_version,
                    "decompiler_minor_version": fact.decompiler_minor_version,
                    "feature_hashes": &fact.feature_hashes,
                    "debug_features": &fact.debug_features,
                    "call_targets": &fact.call_targets,
                    "has_unimplemented": fact.has_unimplemented,
                    "has_bad_data": fact.has_bad_data,
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_SEMANTIC_SIGNATURE_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_SEMANTIC_SIGNATURE_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }

        for fact in &input.oracle_function_id_facts {
            let fact_node_id = tenant_scoped_node_id(&input.tenant_id, &fact.fid_id);
            nodes.push(NodeRecord::new(
                &fact_node_id,
                [GHIDRA_ORACLE_FUNCTION_ID_FACT_LABEL],
                json!({
                    "logical_id": &fact.fid_id,
                    "tenant_id": &input.tenant_id,
                    "artifact_id": &input.artifact.artifact_id,
                    "run_id": &run.run_id,
                    "oracle_fixture_id": &fixture.fixture_id,
                    "function_id": &fact.function_id,
                    "entry_point": &fact.entry_point,
                    "function_name": &fact.function_name,
                    "feature_version": &fact.feature_version,
                    "hash_algorithm": &fact.hash_algorithm,
                    "short_hash_code_unit_length": fact.short_hash_code_unit_length,
                    "medium_hash_code_unit_length": fact.medium_hash_code_unit_length,
                    "code_unit_size": fact.code_unit_size,
                    "full_hash": &fact.full_hash,
                    "specific_hash_additional_size": fact.specific_hash_additional_size,
                    "specific_hash": &fact.specific_hash,
                    "matches": &fact.matches,
                    "authority_layer": AUTHORITY_OBSERVED_FACT,
                    "evidence_ids": &fact.evidence_ids,
                    "source": SOURCE,
                }),
            ));
            edges.push(EdgeRecord::new(
                edge_id(
                    &fixture_node_id,
                    ORACLE_FIXTURE_HAS_FUNCTION_ID_FACT,
                    &fact_node_id,
                ),
                &fixture_node_id,
                ORACLE_FIXTURE_HAS_FUNCTION_ID_FACT,
                &fact_node_id,
                edge_props(input, run, AUTHORITY_OBSERVED_FACT, &fact.evidence_ids),
            ));
        }
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

fn tenant_scoped_node_id(tenant_id: &str, logical_id: &str) -> String {
    format!(
        "program-analysis:tenant-node:{}",
        stable_hash(json!([tenant_id, logical_id]))
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
        input.analysis_passes = fixture_passes();
        let change_set = AnalysisChangeSet {
            added_ranges: vec![AnalysisAddressRange {
                start: "0x401000".to_string(),
                end: "0x401040".to_string(),
            }],
            removed_ranges: Vec::new(),
            evidence_ids: vec!["e:changed".to_string()],
        };
        input.analysis_scheduler_states =
            ghidra_analysis_scheduler_states(&input.analysis_passes, &change_set);
        input.analysis_work_items =
            schedule_analysis_work_items(&input.analysis_passes, &change_set);
        input
    }

    fn fixture_input_with_runtime_trace() -> ProgramAnalysisInput {
        let mut input = fixture_input();
        input.runtime_traces.push(RuntimeTrace {
            trace_id: "trace:hello-run".to_string(),
            language_id: Some(" x86:LE:64:default ".to_string()),
            compiler_spec_id: Some(" gcc ".to_string()),
            emulator_cache_version: Some("emu-cache:v1".to_string()),
            capture_source: "ghidra-trace-oracle".to_string(),
            evidence_ids: vec!["e:trace".to_string()],
        });
        input.trace_snapshots.push(TraceSnapshotFact {
            snapshot_id: "snapshot:hello-run:0".to_string(),
            trace_id: "trace:hello-run".to_string(),
            snap: 0,
            description: " entry snapshot ".to_string(),
            real_time_ms: Some(1234),
            event_thread_id: Some(" thread:main ".to_string()),
            schedule: TraceScheduleSpec {
                schedule: " 0:1.2 ".to_string(),
                snap: 0,
                instruction_steps: 1,
                pcode_steps: 2,
                source: TraceScheduleSource::Record,
                form: TraceScheduleForm::SnapAnyStepsOps,
            },
            version: 7,
            forked: true,
            stale: false,
            evidence_ids: vec!["e:snapshot".to_string()],
        });
        input.trace_events.push(TraceEventFact {
            event_id: "event:hello-run:mem-read".to_string(),
            trace_id: "trace:hello-run".to_string(),
            snapshot_id: "snapshot:hello-run:0".to_string(),
            sequence: 2,
            thread_id: Some("thread:main".to_string()),
            kind: TraceEventKind::MemoryRead,
            address_space: Some(" ram ".to_string()),
            offset: Some(" 0x7fffffffe000 ".to_string()),
            size: Some(8),
            register: None,
            value_hash: Some("sha256:value".to_string()),
            pcode_op: Some("LOAD".to_string()),
            evidence_ids: vec!["e:event".to_string()],
        });
        input.taint_marks.push(TaintMark {
            taint_id: "taint:hello-run:argv".to_string(),
            trace_id: "trace:hello-run".to_string(),
            snapshot_id: "snapshot:hello-run:0".to_string(),
            event_id: Some("event:hello-run:mem-read".to_string()),
            address_space: "ram".to_string(),
            offset: " 0x7fffffffe000 ".to_string(),
            size: 8,
            labels: vec![
                " http.body ".to_string(),
                "argv[1]".to_string(),
                "http.body".to_string(),
            ],
            originating_op: Some("unique:0x100@LOAD".to_string()),
            indirect_read: true,
            indirect_write: false,
            evidence_ids: vec!["e:taint".to_string(), " e:taint ".to_string()],
        });
        input
    }

    fn fixture_input_with_address_oracle() -> ProgramAnalysisInput {
        let mut input = fixture_input();
        input.data_flow_facts.push(ProgramDataFlowFact {
            fact_id: "df:ref:hello".to_string(),
            fact_kind: "reference".to_string(),
            source_id: "0x401000".to_string(),
            target_id: "0x402000".to_string(),
            evidence_ids: vec!["e:ref-native".to_string()],
        });
        input.data_flow_facts.push(ProgramDataFlowFact {
            fact_id: "df:call:main-helper".to_string(),
            fact_kind: "call_edge".to_string(),
            source_id: "0x401000".to_string(),
            target_id: "0x401030".to_string(),
            evidence_ids: vec!["e:call-native".to_string()],
        });
        input.pcode_facts.push(ProgramPcodeFact {
            pcode_id: "pcode:native:401000:0".to_string(),
            address: "0x401000".to_string(),
            sequence: 0,
            opcode: "CALL".to_string(),
            ghidra_opcode_id: 7,
            inputs: vec!["0x401030".to_string()],
            output: None,
            evidence_ids: vec!["e:pcode-native".to_string()],
        });
        input.oracle_fixture = Some(GhidraOracleFixture {
            fixture_id: "ghidra:fixture:address".to_string(),
            source_uri: "ghidra://headless/address".to_string(),
            export_script: "ExportTheoremAddressFacts.java".to_string(),
            program_summary: GhidraOracleProgramSummary {
                ghidra_version: "11.4.2".to_string(),
                language_id: Some("x86:LE:64:default".to_string()),
                compiler_spec_id: Some("gcc".to_string()),
                analysis_timeout_occurred: false,
                function_count: 1,
                import_count: 1,
                string_count: 1,
                cfg_edge_count: 2,
            },
            evidence_ids: vec!["e:ghidra-address".to_string()],
        });
        input.oracle_function_facts.push(GhidraOracleFunctionFact {
            function_id: "ghidra:function:401000".to_string(),
            entry_point: "0x401000".to_string(),
            name: Some("main".to_string()),
            body_start: "0x401000".to_string(),
            body_end: "0x401040".to_string(),
            evidence_ids: vec!["e:oracle-function".to_string()],
        });
        input.oracle_pcode_facts.push(GhidraOraclePcodeOpFact {
            pcode_id: "ghidra:pcode:401000:0".to_string(),
            address: "0x401000".to_string(),
            sequence: 0,
            opcode: "CALL".to_string(),
            ghidra_opcode_id: 7,
            inputs: vec!["0x401030".to_string()],
            output: None,
            evidence_ids: vec!["e:oracle-pcode".to_string()],
        });
        input
            .oracle_reference_facts
            .push(GhidraOracleReferenceFact {
                reference_id: "ghidra:ref:401000:402000".to_string(),
                from_address: "0x401000".to_string(),
                to_address: "0x402000".to_string(),
                reference_type: "reference".to_string(),
                operand_index: -2,
                is_primary: true,
                source_type: Some("ANALYSIS".to_string()),
                semantic_roles: vec![" memory ".to_string()],
                is_external: false,
                is_memory: true,
                is_register: false,
                is_stack: false,
                evidence_ids: vec!["e:oracle-reference".to_string()],
            });
        input.oracle_call_edge_facts.push(GhidraOracleCallEdgeFact {
            edge_id: "ghidra:call:401000:401030".to_string(),
            source_entry: "0x401000".to_string(),
            target_entry: "0x401030".to_string(),
            callsite_address: "0x401000".to_string(),
            evidence_ids: vec!["e:oracle-call".to_string()],
        });
        input
            .oracle_jump_table_facts
            .push(GhidraOracleJumpTableFact {
                jump_table_id: "ghidra:jumptable:401004".to_string(),
                function_id: Some("ghidra:function:401000".to_string()),
                entry_point: "0x401000".to_string(),
                switch_address: "0x401004".to_string(),
                switch_statement_id: Some("stmt:switch".to_string()),
                display_format: Some("hex".to_string()),
                cases: vec![
                    GhidraOracleJumpTableCaseFact {
                        case_id: "ghidra:jumptable_case:401004:0".to_string(),
                        destination: "0x401010".to_string(),
                        label_value: Some(0),
                        is_default: false,
                        label: Some("case_0".to_string()),
                        evidence_ids: vec!["e:oracle-jump-case-0".to_string()],
                    },
                    GhidraOracleJumpTableCaseFact {
                        case_id: "ghidra:jumptable_case:401004:default".to_string(),
                        destination: "0x401030".to_string(),
                        label_value: None,
                        is_default: true,
                        label: None,
                        evidence_ids: vec!["e:oracle-jump-case-default".to_string()],
                    },
                ],
                load_tables: vec![GhidraOracleJumpTableLoadTableFact {
                    load_table_id: "ghidra:jumptable_load:401004:0".to_string(),
                    address: "0x403000".to_string(),
                    entry_byte_len: 4,
                    entry_count: 2,
                    interpreted_as_pointer_table: true,
                    evidence_ids: vec!["e:oracle-jump-load-table".to_string()],
                }],
                override_applied: false,
                references_complete: true,
                evidence_ids: vec!["e:oracle-jump-table".to_string()],
            });
        input
    }

    fn fixture_passes() -> Vec<AnalysisPassSpec> {
        vec![
            AnalysisPassSpec {
                analyzer_id: "FunctionStartAnalyzer".to_string(),
                analyzer_type: ProgramAnalyzerType::Byte,
                description: "Recover likely function starts from byte evidence.".to_string(),
                priority: 10,
                default_enabled: true,
                enabled: true,
                supports_one_time: true,
                can_analyze: true,
                input_labels: vec![BINARY_ARTIFACT_LABEL.to_string()],
                output_labels: vec![THEOREM_IR_FUNCTION_LABEL.to_string()],
                option_namespace: Some("Analyzers.FunctionStartAnalyzer".to_string()),
                evidence_ids: vec!["e:function-start".to_string()],
            },
            AnalysisPassSpec {
                analyzer_id: "InstructionAnalyzer".to_string(),
                analyzer_type: ProgramAnalyzerType::Instruction,
                description: "Lift changed instructions into semantic facts.".to_string(),
                priority: 20,
                default_enabled: true,
                enabled: true,
                supports_one_time: true,
                can_analyze: true,
                input_labels: vec![INSTRUCTION_FACT_LABEL.to_string()],
                output_labels: vec![PROGRAM_DATA_FLOW_FACT_LABEL.to_string()],
                option_namespace: Some("Analyzers.InstructionAnalyzer".to_string()),
                evidence_ids: vec!["e:instruction".to_string()],
            },
            AnalysisPassSpec {
                analyzer_id: "DisabledPrototypeAnalyzer".to_string(),
                analyzer_type: ProgramAnalyzerType::Data,
                description: "Disabled fixture analyzer.".to_string(),
                priority: 1,
                default_enabled: false,
                enabled: false,
                supports_one_time: false,
                can_analyze: true,
                input_labels: vec![LOADER_FACT_LABEL.to_string()],
                output_labels: vec![PROGRAM_SEMANTIC_HYPOTHESIS_LABEL.to_string()],
                option_namespace: None,
                evidence_ids: vec!["e:disabled".to_string()],
            },
        ]
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
        assert!(output.graph_nodes.iter().any(|node| {
            node.labels.contains(&BINARY_ARTIFACT_LABEL.to_string())
                && node.properties.get("logical_id") == Some(&json!(output.artifact.artifact_id))
        }));
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
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(ANALYSIS_PASS_SPEC_LABEL))
                .len(),
            3
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(ANALYSIS_SCHEDULER_STATE_LABEL))
                .len(),
            3
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(ANALYSIS_WORK_ITEM_LABEL))
                .len(),
            2
        );
    }

    #[test]
    fn scheduler_coalesces_changed_ranges_into_priority_ordered_work() {
        let work = schedule_analysis_work_items(
            &fixture_passes(),
            &AnalysisChangeSet {
                added_ranges: vec![AnalysisAddressRange {
                    start: " 0x401040 ".to_string(),
                    end: "0x401080".to_string(),
                }],
                removed_ranges: vec![AnalysisAddressRange {
                    start: "0x401000".to_string(),
                    end: "0x401020".to_string(),
                }],
                evidence_ids: vec![" e:changed ".to_string(), "e:changed".to_string()],
            },
        );

        assert_eq!(work.len(), 2);
        assert_eq!(work[0].analyzer_id, "FunctionStartAnalyzer");
        assert_eq!(work[1].analyzer_id, "InstructionAnalyzer");
        assert_eq!(work[0].status, AnalysisWorkStatus::Scheduled);
        assert_eq!(work[0].added_ranges[0].start, "0x401040");
        assert_eq!(work[0].removed_ranges[0].start, "0x401000");
        assert_eq!(work[0].evidence_ids, vec!["e:changed", "e:function-start"]);
        assert!(work
            .iter()
            .all(|item| item.analyzer_id != "DisabledPrototypeAnalyzer"));
    }

    #[test]
    fn ghidra_scheduler_states_snapshot_enabled_and_disabled_queues() {
        let states = ghidra_analysis_scheduler_states(
            &fixture_passes(),
            &AnalysisChangeSet {
                added_ranges: vec![
                    AnalysisAddressRange {
                        start: " 0x401040 ".to_string(),
                        end: "0x401080".to_string(),
                    },
                    AnalysisAddressRange {
                        start: "0x401040".to_string(),
                        end: "0x401080".to_string(),
                    },
                ],
                removed_ranges: vec![AnalysisAddressRange {
                    start: "0x401000".to_string(),
                    end: "0x401020".to_string(),
                }],
                evidence_ids: vec![" e:changed ".to_string(), "e:changed".to_string()],
            },
        );

        assert_eq!(states.len(), 3);
        let disabled = states
            .iter()
            .find(|state| state.analyzer_id == "DisabledPrototypeAnalyzer")
            .expect("disabled analyzer state exists");
        assert!(!disabled.enabled);
        assert!(!disabled.scheduled);
        assert!(disabled.added_ranges.is_empty());
        assert!(disabled.removed_ranges.is_empty());

        let function_start = states
            .iter()
            .find(|state| state.analyzer_id == "FunctionStartAnalyzer")
            .expect("function analyzer state exists");
        assert!(function_start.enabled);
        assert!(function_start.scheduled);
        assert_eq!(function_start.priority, 10);
        assert_eq!(function_start.added_ranges.len(), 1);
        assert_eq!(function_start.added_ranges[0].start, "0x401040");
        assert_eq!(function_start.removed_ranges[0].start, "0x401000");
        assert_eq!(
            function_start.evidence_ids,
            vec!["e:changed", "e:function-start"]
        );
        assert!(function_start
            .scheduler_id
            .starts_with("program-analysis:scheduler:"));
    }

    #[test]
    fn empty_change_set_does_not_schedule_work() {
        let work = schedule_analysis_work_items(
            &fixture_passes(),
            &AnalysisChangeSet {
                added_ranges: Vec::new(),
                removed_ranges: Vec::new(),
                evidence_ids: vec!["e:none".to_string()],
            },
        );

        assert!(work.is_empty());
    }

    #[test]
    fn program_analysis_graph_nodes_are_tenant_scoped() {
        let first = compile_program_analysis_run_in_memory(fixture_input());
        let mut second_input = fixture_input();
        second_input.tenant_id = "Another-Tenant".to_string();
        let second = compile_program_analysis_run_in_memory(second_input);

        let first_artifact_node_id = graph_node_id_for_logical_id(
            &first,
            BINARY_ARTIFACT_LABEL,
            &first.artifact.artifact_id,
        );
        let second_artifact_node_id = graph_node_id_for_logical_id(
            &second,
            BINARY_ARTIFACT_LABEL,
            &second.artifact.artifact_id,
        );

        assert_ne!(first_artifact_node_id, second_artifact_node_id);
        assert!(first
            .graph_nodes
            .iter()
            .any(|node| node.id == first.run.run_id));
        assert!(second
            .graph_nodes
            .iter()
            .any(|node| node.id == second.run.run_id));
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
            evidence_ids: vec![" e:ghidra ".to_string(), "e:ghidra".to_string()],
        };
        let fixture_id = fixture.fixture_id.clone();
        let input =
            ghidra_oracle_fixture_to_program_analysis_input("Travis-Gilbert", artifact(), fixture);
        let output = compile_program_analysis_run_in_memory(input);

        assert_eq!(output.run.toolchain, "ghidra-oracle:11.4.2");
        assert_eq!(output.run.status, ProgramAnalysisStatus::Complete);
        assert_eq!(output.analyzer_receipts.len(), 1);
        assert_eq!(output.analyzer_receipts[0].evidence_ids, vec!["e:ghidra"]);
        assert_eq!(
            output.analyzer_receipts[0].authority_layer,
            AUTHORITY_OBSERVED_FACT
        );
        assert_eq!(
            output
                .graph_nodes
                .iter()
                .find(|node| node
                    .labels
                    .contains(&GHIDRA_ORACLE_FIXTURE_LABEL.to_string()))
                .and_then(|node| node.properties.get("evidence_ids")),
            Some(&json!(["e:ghidra"]))
        );
        assert!(output
            .graph_nodes
            .iter()
            .any(
                |node| node.properties.get("logical_id") == Some(&json!(fixture_id))
                    && node
                        .labels
                        .contains(&GHIDRA_ORACLE_FIXTURE_LABEL.to_string())
            ));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == DERIVED_FROM_ORACLE));
    }

    #[test]
    fn ghidra_oracle_export_json_lowers_address_facts() {
        let raw = r#"{
            "fixture": {
                "fixture_id": "ghidra:fixture:json-address",
                "source_uri": "ghidra://headless/json-address",
                "export_script": "ExportTheoremAddressFacts.java",
                "program_summary": {
                    "ghidra_version": "11.4.2",
                    "language_id": "x86:LE:64:default",
                    "compiler_spec_id": "gcc",
                    "analysis_timeout_occurred": false,
                    "function_count": 1,
                    "import_count": 0,
                    "string_count": 0,
                    "cfg_edge_count": 0
                },
                "evidence_ids": [" e:ghidra-export ", "e:ghidra-export"]
            },
            "functions": [{
                "function_id": "ghidra:function:401000",
                "entry_point": "0x401000",
                "name": "main",
                "body_start": "0x401000",
                "body_end": "0x401040",
                "evidence_ids": ["e:function"]
            }],
            "pcode_ops": [{
                "pcode_id": "ghidra:pcode:401000:0",
                "address": "0x401000",
                "sequence": 0,
                "opcode": "CALL",
                "ghidra_opcode_id": 7,
                "inputs": ["0x401030"],
                "output": null,
                "evidence_ids": ["e:pcode"]
            }],
            "references": [{
                "reference_id": "ghidra:ref:401000:402000",
                "from_address": "0x401000",
                "to_address": "0x402000",
                "reference_type": "reference",
                "operand_index": -2,
                "is_primary": true,
                "source_type": "ANALYSIS",
                "semantic_roles": ["memory"],
                "is_external": false,
                "is_memory": true,
                "is_register": false,
                "is_stack": false,
                "evidence_ids": ["e:reference"]
            }],
            "call_edges": [{
                "edge_id": "ghidra:call:401000:401030",
                "source_entry": "0x401000",
                "target_entry": "0x401030",
                "callsite_address": "0x401000",
                "evidence_ids": ["e:call"]
            }],
            "jump_tables": [{
                "jump_table_id": "",
                "function_id": "ghidra:function:401000",
                "entry_point": "0X401000",
                "switch_address": "0X401004",
                "switch_statement_id": " stmt:main:switch ",
                "display_format": " HEX ",
                "cases": [{
                    "case_id": "",
                    "destination": "0X401010",
                    "label_value": 0,
                    "is_default": false,
                    "label": " case_0 ",
                    "evidence_ids": ["e:jump-case-0"]
                }, {
                    "case_id": "",
                    "destination": "0X401030",
                    "label_value": null,
                    "is_default": true,
                    "label": null,
                    "evidence_ids": ["e:jump-case-default"]
                }],
                "load_tables": [{
                    "load_table_id": "",
                    "address": "0X403000",
                    "entry_byte_len": 4,
                    "entry_count": 2,
                    "interpreted_as_pointer_table": true,
                    "evidence_ids": ["e:jump-load-table"]
                }],
                "override_applied": false,
                "references_complete": true,
                "evidence_ids": ["e:jump-table"]
            }],
            "external_linkages": [{
                "linkage_id": " ghidra:external_linkage:main:sqlite_prepare ",
                "function_id": " ghidra:function:401000 ",
                "local_address": "0X401000",
                "thunked_function_id": " ghidra:function:external:sqlite3_prepare_v2 ",
                "thunked_address": "EXTERNAL:0X1000",
                "recursive_target_function_id": " ghidra:function:external:sqlite3_prepare_v2 ",
                "recursive_target_address": "EXTERNAL:0X1000",
                "external_library": " libsqlite3.so ",
                "external_library_path": " /usr/lib/libsqlite3.so ",
                "library_ordinal": 2,
                "parent_namespace": " libsqlite3.so ",
                "external_label": " sqlite3_prepare_v2 ",
                "original_imported_name": " _sqlite3_prepare_v2 ",
                "external_address": "0X8120",
                "external_space_address": "EXTERNAL:0X1000",
                "source_type": "IMPORTED",
                "is_function": true,
                "data_type": " int (*)(sqlite3*, char const*) ",
                "function_signature": " int sqlite3_prepare_v2(sqlite3*, char const*, int, sqlite3_stmt**, char const**) ",
                "thunk_chain": [{
                    "link_id": " ghidra:external_thunk:401000:0 ",
                    "function_id": " ghidra:function:401000 ",
                    "address": "0X401000",
                    "target_function_id": " ghidra:function:external:sqlite3_prepare_v2 ",
                    "target_address": "EXTERNAL:0X1000",
                    "recursive_depth": 0,
                    "is_terminal": true,
                    "evidence_ids": [" e:external-thunk ", "e:external-thunk"]
                }],
                "evidence_ids": [" e:external-linkage ", "e:external-linkage"]
            }],
            "function_prototypes": [{
                "prototype_id": " ghidra:prototype:login ",
                "function_id": " ghidra:function:401000 ",
                "entry_point": "0X401000",
                "name": " login ",
                "prototype": " int __stdcall login(char * user, int flags, ...) ",
                "calling_convention": " __stdcall ",
                "return_type": " int ",
                "return_type_id": " ghidra:datatype:/int ",
                "return_type_kind": " PRIMITIVE ",
                "return_type_byte_len": 4,
                "return_storage": {
                    "storage": " register:0x0:4 ",
                    "kind": " REGISTER ",
                    "byte_len": 4,
                    "pieces": [],
                    "dynamic_storage_required": false,
                    "forced_indirect": false,
                    "auto_parameter_type": null
                },
                "parameters": [{
                    "ordinal": 0,
                    "name": " user ",
                    "data_type": " char * ",
                    "data_type_id": " ghidra:datatype:/char * ",
                    "data_type_kind": " POINTER ",
                    "data_type_byte_len": 8,
                    "storage": {
                        "storage": " register:0x38:8 ",
                        "kind": " REGISTER ",
                        "byte_len": 8,
                        "pieces": [],
                        "dynamic_storage_required": false,
                        "forced_indirect": false,
                        "auto_parameter_type": null
                    },
                    "auto_parameter": false,
                    "auto_parameter_type": null,
                    "forced_indirect": false,
                    "formal_data_type": null,
                    "formal_data_type_id": null,
                    "formal_data_type_kind": null,
                    "formal_data_type_byte_len": null,
                    "comment": " username buffer ",
                    "source_type": " ANALYSIS ",
                    "evidence_ids": [" e:prototype-param-user ", "e:prototype-param-user"]
                }],
                "has_varargs": true,
                "has_no_return": false,
                "is_inline": false,
                "is_thunk": true,
                "thunked_function_id": " ghidra:function:external:sqlite3_prepare_v2 ",
                "thunked_entry_point": "EXTERNAL:0X1000",
                "has_custom_storage": true,
                "stack_purge_size": 8,
                "signature_source": " ANALYSIS ",
                "evidence_ids": [" e:prototype-login ", "e:prototype-login"]
            }],
            "data_types": [{
                "type_id": " ghidra:datatype:/fixture/LoginRequest:1001 ",
                "name": " LoginRequest ",
                "display_name": " struct LoginRequest ",
                "kind": " STRUCT ",
                "category_path": " /fixture ",
                "path_name": " /fixture/LoginRequest ",
                "universal_id": " 1001 ",
                "byte_len": 24,
                "aligned_byte_len": 24,
                "component_count": 2,
                "packing_enabled": true,
                "packing_value": 8,
                "minimum_alignment": 4,
                "not_yet_defined": false,
                "zero_length": false,
                "base_type_id": null,
                "base_type_name": null,
                "element_type_id": null,
                "element_type_name": null,
                "element_count": null,
                "element_byte_len": null,
                "enum_signed": null,
                "enum_values": [],
                "fields": [{
                    "field_id": " ghidra:datatype_field:LoginRequest:username ",
                    "name": " username ",
                    "ordinal": 0,
                    "offset": 0,
                    "byte_len": 16,
                    "bit_offset": null,
                    "bit_size": null,
                    "data_type_id": " ghidra:datatype:/char[16] ",
                    "data_type_name": " char[16] ",
                    "comment": " login name ",
                    "evidence_ids": [" e:datatype-field-username ", "e:datatype-field-username"]
                }, {
                    "field_id": " ghidra:datatype_field:LoginRequest:flags ",
                    "name": " flags ",
                    "ordinal": 1,
                    "offset": 16,
                    "byte_len": 4,
                    "bit_offset": 1,
                    "bit_size": 3,
                    "data_type_id": " ghidra:datatype:/uint32_t ",
                    "data_type_name": " uint32_t:3 ",
                    "comment": null,
                    "evidence_ids": ["e:datatype-field-flags"]
                }],
                "hard_dependency_ids": [" ghidra:datatype:/char[16] "],
                "soft_dependency_ids": [" ghidra:datatype:/uint32_t "],
                "evidence_ids": [" e:datatype-login-request ", "e:datatype-login-request"]
            }, {
                "type_id": " ghidra:datatype:/fixture/LoginState:1002 ",
                "name": " LoginState ",
                "display_name": " enum LoginState ",
                "kind": " ENUM ",
                "category_path": " /fixture ",
                "path_name": " /fixture/LoginState ",
                "universal_id": " 1002 ",
                "byte_len": 4,
                "aligned_byte_len": 4,
                "component_count": 0,
                "packing_enabled": null,
                "packing_value": null,
                "minimum_alignment": null,
                "not_yet_defined": false,
                "zero_length": false,
                "base_type_id": null,
                "base_type_name": null,
                "element_type_id": null,
                "element_type_name": null,
                "element_count": null,
                "element_byte_len": null,
                "enum_signed": false,
                "enum_values": [{
                    "value_id": " ghidra:enum_value:LoginState:OK ",
                    "name": " OK ",
                    "value": 1,
                    "comment": " success ",
                    "evidence_ids": ["e:enum-ok"]
                }],
                "fields": [],
                "hard_dependency_ids": [],
                "soft_dependency_ids": [],
                "evidence_ids": ["e:datatype-login-state"]
            }],
            "high_variables": [{
                "variable_id": " ghidra:highvar:401000:parameter:17:request ",
                "function_id": " ghidra:function:401000 ",
                "entry_point": "0X401000",
                "symbol_id": " ghidra:symbol:11 ",
                "name": " request ",
                "kind": " PARAM ",
                "category_index": 0,
                "data_type": " struct LoginRequest * ",
                "data_type_id": " ghidra:datatype:/fixture/LoginRequest:1001 ",
                "storage": {
                    "storage": " register:0x38:8 ",
                    "kind": " REGISTER ",
                    "byte_len": 8,
                    "pieces": [{
                        "piece_id": " ghidra:storage_piece:request:rdi ",
                        "space": " register ",
                        "offset": 56,
                        "byte_len": 8,
                        "register": " RDI ",
                        "is_input": true,
                        "is_addr_tied": true,
                        "is_persistent": false,
                        "is_unique": false,
                        "is_constant": false,
                        "is_hash": false,
                        "is_stack": false
                    }],
                    "dynamic_storage_required": false,
                    "forced_indirect": false,
                    "auto_parameter_type": " THIS "
                },
                "first_use_address": "0X401000",
                "first_use_offset": 0,
                "name_locked": true,
                "type_locked": true,
                "isolated": false,
                "is_this_pointer": true,
                "is_hidden_return": false,
                "mutability": " NORMAL ",
                "high_symbol_offset": null,
                "instances": [{
                    "instance_id": " ghidra:highvar_instance:request:rdi ",
                    "storage": {
                        "storage": " register:0x38:8 ",
                        "kind": " REGISTER ",
                        "byte_len": 8,
                        "pieces": [{
                            "piece_id": " ghidra:storage_piece:request:rdi:instance ",
                            "space": " register ",
                            "offset": 56,
                            "byte_len": 8,
                            "register": " RDI ",
                            "is_input": true,
                            "is_addr_tied": true,
                            "is_persistent": false,
                            "is_unique": false,
                            "is_constant": false,
                            "is_hash": false,
                            "is_stack": false
                        }],
                        "dynamic_storage_required": false,
                        "forced_indirect": false,
                        "auto_parameter_type": null
                    },
                    "pc_address": "0X401000",
                    "defining_pcode_id": " ghidra:pcode:401000:0 ",
                    "merge_group": 0,
                    "is_representative": true,
                    "evidence_ids": [" e:highvar-instance-request "]
                }],
                "evidence_ids": [" e:highvar-request ", "e:highvar-request"]
            }],
            "stack_frames": [{
                "frame_id": " ghidra:stack_frame:401000 ",
                "function_id": " ghidra:function:401000 ",
                "entry_point": "0X401000",
                "frame_size": 56,
                "local_size": 32,
                "parameter_size": 24,
                "parameter_offset": 8,
                "return_address_offset": 0,
                "growth": "GROWS-NEGATIVE",
                "stack_pointer_register": " RSP ",
                "custom_variable_storage": true,
                "variables": [{
                    "variable_id": " ghidra:stack_var:401000:8:request ",
                    "name": " request ",
                    "kind": "PARAM",
                    "offset": 8,
                    "byte_len": 8,
                    "ordinal": 0,
                    "data_type": " struct LoginRequest * ",
                    "data_type_id": " ghidra:datatype:/fixture/LoginRequest:1001 ",
                    "storage": {
                        "storage": " stack:0x8:8 ",
                        "kind": " STACK ",
                        "byte_len": 8,
                        "pieces": [{
                            "piece_id": " ghidra:storage_piece:request:stack ",
                            "space": " stack ",
                            "offset": 8,
                            "byte_len": 8,
                            "register": null,
                            "is_input": true,
                            "is_addr_tied": true,
                            "is_persistent": false,
                            "is_unique": false,
                            "is_constant": false,
                            "is_hash": false,
                            "is_stack": true
                        }],
                        "dynamic_storage_required": false,
                        "forced_indirect": false,
                        "auto_parameter_type": null
                    },
                    "source_type": "ANALYSIS",
                    "high_variable_id": " ghidra:highvar:401000:parameter:17:request ",
                    "name_locked": true,
                    "type_locked": true,
                    "evidence_ids": [" e:stack-var-request ", "e:stack-var-request"]
                }],
                "evidence_ids": [" e:stack-frame ", "e:stack-frame"]
            }],
            "call_stack_effects": [{
                "effect_id": " ghidra:stack_effect:main:sqlite_prepare ",
                "function_id": " ghidra:function:401000 ",
                "entry_point": "0X401000",
                "callsite_address": "0X401000",
                "callee_function_id": " ghidra:function:external:sqlite3_prepare_v2 ",
                "callee_name": " sqlite3_prepare_v2 ",
                "call_opcode": " call ",
                "prototype_model": " __stdcall ",
                "stack_pointer_register": " RSP ",
                "stack_space": " stack ",
                "stack_offset_before_call": -32,
                "instruction_stack_depth_change": 0,
                "stack_shift_bytes": 8,
                "purge_size_bytes": 16,
                "extra_pop_bytes": 24,
                "effective_extra_pop_bytes": 24,
                "companion_solution_bytes": 0,
                "solver_variable_count": 4,
                "missed_variable_count": 0,
                "status": "SOLVED",
                "warnings": [" effective_extra_pop_from_call_depth "],
                "evidence_ids": [" e:stack-effect ", "e:stack-effect"]
            }],
            "annotations": [{
                "annotation_id": " ghidra:annotation:comment:401000:plate ",
                "kind": " COMMENT ",
                "function_id": " ghidra:function:401000 ",
                "entry_point": "0X401000",
                "address": "0X401000",
                "comment_type": " plate ",
                "bookmark_id": null,
                "bookmark_type": null,
                "bookmark_category": null,
                "message": " Analyst says this login path still needs stack-flow review. ",
                "source_api": " Listing.getComment ",
                "evidence_ids": [" e:annotation-comment ", "e:annotation-comment"]
            }, {
                "annotation_id": " ghidra:annotation:bookmark:401014:error ",
                "kind": " BOOKMARK ",
                "function_id": " ghidra:function:401000 ",
                "entry_point": "0X401000",
                "address": "0X401014",
                "comment_type": null,
                "bookmark_id": 42,
                "bookmark_type": " Error ",
                "bookmark_category": " Stack ",
                "message": " Decompiler marked stack pointer recovery as suspicious. ",
                "source_api": " BookmarkManager.getBookmarksIterator ",
                "evidence_ids": [" e:annotation-bookmark ", "e:annotation-bookmark"]
            }],
            "parameter_measures": [{
                "measure_id": " ghidra:param_measure:login:request ",
                "function_id": " ghidra:function:401000 ",
                "entry_point": "0X401000",
                "io": " PARAM ",
                "rank": " THIS-FUNCTION-PARAMETER ",
                "rank_value": 4,
                "storage": {
                    "storage": " register:0x38:8 ",
                    "kind": " REGISTER ",
                    "byte_len": 8,
                    "pieces": [{
                        "piece_id": " ghidra:storage_piece:param_request:rdi ",
                        "space": " register ",
                        "offset": 56,
                        "byte_len": 8,
                        "register": " RDI ",
                        "is_input": true,
                        "is_addr_tied": true,
                        "is_persistent": false,
                        "is_unique": false,
                        "is_constant": false,
                        "is_hash": false,
                        "is_stack": false
                    }],
                    "dynamic_storage_required": false,
                    "forced_indirect": false,
                    "auto_parameter_type": null
                },
                "data_type": " struct LoginRequest * ",
                "data_type_id": " ghidra:datatype:/fixture/LoginRequest:1001 ",
                "model_name": " __stdcall ",
                "extra_pop": 16,
                "just_prototype": true,
                "base_variable_id": " ghidra:highvar:401000:parameter:17:request ",
                "source_statement_id": " ghidra:pcode:401000:0 ",
                "num_calls": 2,
                "evidence_ids": [" e:param-measure-request ", "e:param-measure-request"]
            }],
            "structure_field_accesses": [{
                "access_id": " ghidra:structure_field_access:login:request:flags ",
                "function_id": " ghidra:function:401000 ",
                "entry_point": "0X401000",
                "address": "0X40100C",
                "access_kind": "BITFIELD_READ",
                "structure_type_id": " ghidra:datatype:/fixture/LoginRequest:1001 ",
                "structure_name": " LoginRequest ",
                "root_variable_id": " ghidra:high_symbol:401000:17 ",
                "root_variable_name": " request ",
                "root_storage": " register:RDI:8 ",
                "field_id": " ghidra:datatype_field:LoginRequest:flags ",
                "field_name": " flags ",
                "field_offset": 16,
                "field_byte_len": 4,
                "field_data_type_id": " ghidra:datatype:/uint32_t ",
                "field_data_type_name": " uint32_t:3 ",
                "field_data_type_kind": " PRIMITIVE ",
                "field_data_type_byte_len": 4,
                "pcode_opcode": " load ",
                "pcode_op_id": " ghidra:pcode:40100c:2 ",
                "statement_id": null,
                "call_target_address": null,
                "call_input_slot": null,
                "pointer_relative_type": " LoginRequest_offset_16_uint32_t ",
                "recursive_call_depth": 0,
                "creates_new_structure": false,
                "extends_existing_structure": true,
                "bit_offset": 1,
                "bit_size": 3,
                "evidence_ids": [" e:structure-field-flags ", "e:structure-field-flags"]
            }],
            "symbolic_summaries": [{
                "summary_id": "ghidra:symz3_summary:main:valid",
                "function_id": "ghidra:function:401000",
                "entry_point": "0x401000",
                "thread_id": "thread:main",
                "preconditions": [{
                    "precondition_id": "ghidra:symz3_precondition:main:branch",
                    "step_index": 3,
                    "address": "0x401012",
                    "pcode_op_id": "ghidra:pcode:401012:1",
                    "branch_taken": true,
                    "serialized_expr": "B:(assert (> argc #x00000000))",
                    "display_expr": "argc > 0",
                    "solver_status": "SATISFIABLE",
                    "model_bindings": [{
                        "name": "argc",
                        "value": "1",
                        "value_hash": ""
                    }],
                    "evidence_ids": ["e:symz3-precondition"]
                }],
                "registers_read": ["RDI", "RFLAGS"],
                "registers_updated": ["RAX"],
                "symbolic_values": [{
                    "value_id": "ghidra:symz3_value:main:rdi",
                    "kind": "REGISTER",
                    "name": "RDI",
                    "space": "register",
                    "offset": "0x38",
                    "byte_len": 8,
                    "serialized_expr": "V:argc",
                    "display_expr": "argc",
                    "concrete_value": "1",
                    "evidence_ids": ["e:symz3-value"]
                }],
                "memory_witnesses": [{
                    "witness_id": "ghidra:symz3_memory_witness:main:argv",
                    "kind": "LOAD",
                    "address_expr": "V:argv",
                    "address_display": "argv",
                    "byte_len": 8,
                    "value_expr": "V:load_64(argv)",
                    "value_display": "MEM[argv]:64",
                    "evidence_ids": ["e:symz3-memory"]
                }],
                "solver_status": "SATISFIABLE",
                "model_bindings": [{
                    "name": "argc",
                    "value": "1",
                    "value_hash": ""
                }],
                "valuation_hash": "",
                "evidence_ids": ["e:symz3-summary"]
            }],
            "diagnostics": [{
                "diagnostic_id": "ghidra:decompiler_diagnostic:main:flow",
                "function_id": "ghidra:function:401000",
                "entry_point": "0x401000",
                "address": "0x401014",
                "category": "UNKNOWN",
                "severity": "UNKNOWN",
                "placement": "HEADER",
                "message": "Control flow encountered unimplemented instruction",
                "source_pass": "DecompInterface.decompileFunction",
                "source_rule": null,
                "affects_control_flow": false,
                "affects_prototype": false,
                "affects_data_flow": false,
                "remediation": "Preserve this uncertainty as a validator.",
                "completed": false,
                "timed_out": false,
                "cancelled": false,
                "failed_to_start": false,
                "evidence_ids": ["e:decompiler-diagnostic"]
            }],
            "semantic_signatures": [{
                "signature_id": "ghidra:semantic_signature:main:bsim",
                "function_id": "ghidra:function:401000",
                "entry_point": "0X401000",
                "function_name": " main ",
                "feature_version": "",
                "signature_settings": 7,
                "decompiler_major_version": 11,
                "decompiler_minor_version": 4,
                "feature_hashes": ["3735928559", "0x0000002a", "-1"],
                "debug_features": [{
                    "hash": "3735928559",
                    "kind": "VARNODE",
                    "raw": " register RAX reaches call "
                }],
                "call_targets": ["0X401030", "0x401030"],
                "has_unimplemented": true,
                "has_bad_data": false,
                "evidence_ids": ["e:bsim-signature"]
            }],
            "function_id_signatures": [{
                "fid_id": "ghidra:function_id:main",
                "function_id": "ghidra:function:401000",
                "entry_point": "0X401000",
                "function_name": " main ",
                "feature_version": "",
                "hash_algorithm": "",
                "short_hash_code_unit_length": 4,
                "medium_hash_code_unit_length": 24,
                "code_unit_size": 12,
                "full_hash": "-1",
                "specific_hash_additional_size": 3,
                "specific_hash": "3735928559",
                "matches": [{
                    "match_id": "",
                    "function_name": " known_main ",
                    "library_family_name": " fixture-lib ",
                    "library_version": " 1.0 ",
                    "library_variant": " x86_64 ",
                    "library_id": " 42 ",
                    "function_record_id": " 7 ",
                    "domain_path": " /fixture/lib.o ",
                    "matched_entry_point": "0X401000",
                    "primary_function_match_mode": " FULL_HASH ",
                    "primary_function_code_unit_score": 12.0,
                    "child_function_code_unit_score": 2.0,
                    "parent_function_code_unit_score": 1.0,
                    "overall_score": 15.0,
                    "auto_pass": false,
                    "auto_fail": false,
                    "force_specific": true,
                    "force_relation": false,
                    "evidence_ids": ["e:fid-match"]
                }],
                "evidence_ids": ["e:fid"]
            }]
        }"#;
        let mut input =
            ghidra_oracle_export_json_to_program_analysis_input("Travis-Gilbert", artifact(), raw)
                .expect("Ghidra oracle export JSON lowers");
        input.ir_functions.push(TheoremIrFunction {
            function_id: "thir:function:main".to_string(),
            address_range: Some(("0x401000".to_string(), "0x401040".to_string())),
            basic_block_ids: Vec::new(),
            statement_ids: Vec::new(),
            evidence_ids: vec!["e:native-function".to_string()],
        });
        input.pcode_facts.push(ProgramPcodeFact {
            pcode_id: "pcode:native:401000:0".to_string(),
            address: "0x401000".to_string(),
            sequence: 0,
            opcode: "CALL".to_string(),
            ghidra_opcode_id: 7,
            inputs: vec!["0x401030".to_string()],
            output: None,
            evidence_ids: vec!["e:native-pcode".to_string()],
        });
        input.data_flow_facts.push(ProgramDataFlowFact {
            fact_id: "df:ref:401000:402000".to_string(),
            fact_kind: "reference".to_string(),
            source_id: "0x401000".to_string(),
            target_id: "0x402000".to_string(),
            evidence_ids: vec!["e:native-reference".to_string()],
        });
        input.data_flow_facts.push(ProgramDataFlowFact {
            fact_id: "df:call:401000:401030".to_string(),
            fact_kind: "call_edge".to_string(),
            source_id: "0x401000".to_string(),
            target_id: "0x401030".to_string(),
            evidence_ids: vec!["e:native-call".to_string()],
        });

        let output = compile_program_analysis_run_in_memory(input);

        assert!(output.analysis_drifts.is_empty());
        assert_eq!(output.oracle_function_facts.len(), 1);
        assert_eq!(output.oracle_pcode_facts.len(), 1);
        assert_eq!(output.oracle_reference_facts.len(), 1);
        assert_eq!(output.oracle_call_edge_facts.len(), 1);
        assert_eq!(output.oracle_jump_table_facts.len(), 1);
        assert_eq!(output.oracle_external_linkage_facts.len(), 1);
        assert_eq!(output.oracle_function_prototype_facts.len(), 1);
        assert_eq!(output.oracle_data_type_facts.len(), 2);
        assert_eq!(output.oracle_high_variable_facts.len(), 1);
        assert_eq!(output.oracle_stack_frame_facts.len(), 1);
        assert_eq!(output.oracle_call_stack_effect_facts.len(), 1);
        assert_eq!(output.oracle_parameter_measure_facts.len(), 1);
        assert_eq!(output.oracle_structure_field_access_facts.len(), 1);
        assert_eq!(output.oracle_annotation_facts.len(), 2);
        assert_eq!(output.oracle_symbolic_summary_facts.len(), 1);
        assert_eq!(output.oracle_decompiler_diagnostic_facts.len(), 1);
        assert_eq!(output.oracle_semantic_signature_facts.len(), 1);
        assert_eq!(output.oracle_function_id_facts.len(), 1);
        let symbolic_summary = &output.oracle_symbolic_summary_facts[0];
        assert_eq!(symbolic_summary.solver_status, "satisfiable");
        assert_eq!(
            symbolic_summary.preconditions[0].solver_status,
            "satisfiable"
        );
        assert_eq!(symbolic_summary.symbolic_values[0].kind, "register");
        assert_eq!(symbolic_summary.memory_witnesses[0].kind, "load");
        assert!(!symbolic_summary.valuation_hash.is_empty());
        let diagnostic = &output.oracle_decompiler_diagnostic_facts[0];
        assert_eq!(diagnostic.category, "flow");
        assert_eq!(diagnostic.severity, "error");
        assert_eq!(diagnostic.placement, "header");
        assert!(diagnostic.affects_control_flow);
        assert_eq!(diagnostic.address.as_deref(), Some("0x401014"));
        let comment_annotation = output
            .oracle_annotation_facts
            .iter()
            .find(|fact| fact.kind == "comment")
            .expect("comment annotation fact");
        assert_eq!(
            comment_annotation.annotation_id,
            "ghidra:annotation:comment:401000:plate"
        );
        assert_eq!(comment_annotation.entry_point.as_deref(), Some("0x401000"));
        assert_eq!(comment_annotation.address, "0x401000");
        assert_eq!(comment_annotation.comment_type.as_deref(), Some("PLATE"));
        assert_eq!(
            comment_annotation.message,
            "Analyst says this login path still needs stack-flow review."
        );
        let bookmark_annotation = output
            .oracle_annotation_facts
            .iter()
            .find(|fact| fact.kind == "bookmark")
            .expect("bookmark annotation fact");
        assert_eq!(bookmark_annotation.address, "0x401014");
        assert_eq!(bookmark_annotation.bookmark_id, Some(42));
        assert_eq!(bookmark_annotation.bookmark_type.as_deref(), Some("Error"));
        assert_eq!(
            bookmark_annotation.bookmark_category.as_deref(),
            Some("Stack")
        );
        let semantic_signature = &output.oracle_semantic_signature_facts[0];
        assert_eq!(
            semantic_signature.feature_version,
            "ghidra-bsim-signature-v0"
        );
        assert_eq!(semantic_signature.entry_point, "0x401000");
        assert_eq!(semantic_signature.function_name.as_deref(), Some("main"));
        assert_eq!(
            semantic_signature.feature_hashes,
            vec!["0x0000002a", "0xdeadbeef", "0xffffffff"]
        );
        assert_eq!(semantic_signature.call_targets, vec!["0x401030"]);
        assert_eq!(semantic_signature.debug_features[0].kind, "varnode");
        assert_eq!(semantic_signature.debug_features[0].hash, "0xdeadbeef");
        let fid = &output.oracle_function_id_facts[0];
        assert_eq!(fid.feature_version, "ghidra-function-id-v0");
        assert_eq!(fid.hash_algorithm, "fnv1a64");
        assert_eq!(fid.entry_point, "0x401000");
        assert_eq!(fid.function_name.as_deref(), Some("main"));
        assert_eq!(fid.full_hash, "0xffffffffffffffff");
        assert_eq!(fid.specific_hash, "0x00000000deadbeef");
        assert_eq!(fid.matches.len(), 1);
        assert_eq!(fid.matches[0].function_name, "known_main");
        assert_eq!(
            fid.matches[0].primary_function_match_mode.as_deref(),
            Some("full_hash")
        );
        assert_eq!(
            fid.matches[0].matched_entry_point.as_deref(),
            Some("0x401000")
        );
        assert!(fid.matches[0].force_specific);
        let jump_table = &output.oracle_jump_table_facts[0];
        assert!(jump_table.jump_table_id.starts_with("ghidra:jumptable:"));
        assert_eq!(jump_table.entry_point, "0x401000");
        assert_eq!(jump_table.switch_address, "0x401004");
        assert_eq!(jump_table.display_format.as_deref(), Some("hex"));
        assert_eq!(jump_table.cases.len(), 2);
        assert!(jump_table.cases.iter().any(|case| case.is_default
            && case.destination == "0x401030"
            && case.label.as_deref() == Some("default")));
        assert_eq!(jump_table.load_tables.len(), 1);
        assert_eq!(jump_table.load_tables[0].address, "0x403000");
        assert!(jump_table.load_tables[0].interpreted_as_pointer_table);
        let external_linkage = &output.oracle_external_linkage_facts[0];
        assert_eq!(
            external_linkage.linkage_id,
            "ghidra:external_linkage:main:sqlite_prepare"
        );
        assert_eq!(
            external_linkage.function_id.as_deref(),
            Some("ghidra:function:401000")
        );
        assert_eq!(external_linkage.local_address, "0x401000");
        assert_eq!(
            external_linkage.thunked_address.as_deref(),
            Some("external:0x1000")
        );
        assert_eq!(external_linkage.external_library, "libsqlite3.so");
        assert_eq!(external_linkage.external_label, "sqlite3_prepare_v2");
        assert_eq!(external_linkage.source_type.as_deref(), Some("IMPORTED"));
        assert_eq!(external_linkage.thunk_chain.len(), 1);
        assert!(external_linkage.thunk_chain[0].is_terminal);
        assert_eq!(
            external_linkage.thunk_chain[0].target_address.as_deref(),
            Some("external:0x1000")
        );
        let prototype = &output.oracle_function_prototype_facts[0];
        assert_eq!(prototype.prototype_id, "ghidra:prototype:login");
        assert_eq!(
            prototype.function_id.as_deref(),
            Some("ghidra:function:401000")
        );
        assert_eq!(prototype.entry_point, "0x401000");
        assert_eq!(prototype.name.as_deref(), Some("login"));
        assert_eq!(
            prototype.prototype,
            "int __stdcall login(char * user, int flags, ...)"
        );
        assert_eq!(prototype.calling_convention.as_deref(), Some("__stdcall"));
        assert_eq!(prototype.return_type, "int");
        assert_eq!(
            prototype.return_type_id.as_deref(),
            Some("ghidra:datatype:/int")
        );
        assert_eq!(prototype.return_type_kind.as_deref(), Some("primitive"));
        assert_eq!(prototype.return_type_byte_len, Some(4));
        assert_eq!(prototype.return_storage.kind, "register");
        assert_eq!(prototype.parameters.len(), 1);
        assert_eq!(prototype.parameters[0].ordinal, 0);
        assert_eq!(prototype.parameters[0].name.as_deref(), Some("user"));
        assert_eq!(prototype.parameters[0].data_type, "char *");
        assert_eq!(
            prototype.parameters[0].data_type_kind.as_deref(),
            Some("pointer")
        );
        assert_eq!(prototype.parameters[0].storage.kind, "register");
        assert_eq!(
            prototype.parameters[0].comment.as_deref(),
            Some("username buffer")
        );
        assert_eq!(prototype.parameters[0].source_type.as_deref(), Some("ANALYSIS"));
        assert!(prototype.has_varargs);
        assert!(!prototype.has_no_return);
        assert!(prototype.is_thunk);
        assert_eq!(
            prototype.thunked_entry_point.as_deref(),
            Some("external:0x1000")
        );
        assert!(prototype.has_custom_storage);
        assert_eq!(prototype.stack_purge_size, Some(8));
        assert_eq!(prototype.signature_source.as_deref(), Some("ANALYSIS"));
        let data_type = output
            .oracle_data_type_facts
            .iter()
            .find(|fact| fact.name == "LoginRequest")
            .expect("LoginRequest data type fact");
        assert_eq!(data_type.kind, "structure");
        assert_eq!(data_type.component_count, 2);
        assert_eq!(data_type.packing_value, Some(8));
        assert_eq!(data_type.fields.len(), 2);
        assert_eq!(data_type.fields[0].name.as_deref(), Some("username"));
        assert_eq!(data_type.fields[1].bit_offset, Some(1));
        assert_eq!(data_type.fields[1].bit_size, Some(3));
        assert_eq!(
            data_type.hard_dependency_ids,
            vec![
                "ghidra:datatype:/char[16]".to_string(),
                "ghidra:datatype:/uint32_t".to_string()
            ]
        );
        let enum_type = output
            .oracle_data_type_facts
            .iter()
            .find(|fact| fact.name == "LoginState")
            .expect("LoginState enum type fact");
        assert_eq!(enum_type.kind, "enum");
        assert_eq!(enum_type.enum_signed, Some(false));
        assert_eq!(enum_type.enum_values.len(), 1);
        assert_eq!(enum_type.enum_values[0].name, "OK");
        let high_variable = &output.oracle_high_variable_facts[0];
        assert_eq!(
            high_variable.variable_id,
            "ghidra:highvar:401000:parameter:17:request"
        );
        assert_eq!(high_variable.entry_point, "0x401000");
        assert_eq!(high_variable.kind, "parameter");
        assert_eq!(high_variable.category_index, Some(0));
        assert_eq!(high_variable.data_type, "struct LoginRequest *");
        assert_eq!(
            high_variable.data_type_id.as_deref(),
            Some("ghidra:datatype:/fixture/LoginRequest:1001")
        );
        assert_eq!(high_variable.storage.kind, "register");
        assert_eq!(
            high_variable.storage.auto_parameter_type.as_deref(),
            Some("THIS")
        );
        assert_eq!(
            high_variable.storage.pieces[0].register.as_deref(),
            Some("RDI")
        );
        assert!(high_variable.name_locked);
        assert!(high_variable.type_locked);
        assert!(high_variable.is_this_pointer);
        assert_eq!(high_variable.instances.len(), 1);
        assert!(high_variable.instances[0].is_representative);
        assert_eq!(
            high_variable.instances[0].defining_pcode_id.as_deref(),
            Some("ghidra:pcode:401000:0")
        );
        let stack_frame = &output.oracle_stack_frame_facts[0];
        assert_eq!(stack_frame.frame_id, "ghidra:stack_frame:401000");
        assert_eq!(
            stack_frame.function_id.as_deref(),
            Some("ghidra:function:401000")
        );
        assert_eq!(stack_frame.entry_point, "0x401000");
        assert_eq!(stack_frame.frame_size, 56);
        assert_eq!(stack_frame.local_size, 32);
        assert_eq!(stack_frame.parameter_size, 24);
        assert_eq!(stack_frame.parameter_offset, Some(8));
        assert_eq!(stack_frame.return_address_offset, Some(0));
        assert_eq!(stack_frame.growth, "negative");
        assert_eq!(stack_frame.stack_pointer_register.as_deref(), Some("RSP"));
        assert!(stack_frame.custom_variable_storage);
        assert_eq!(stack_frame.variables.len(), 1);
        assert_eq!(stack_frame.variables[0].name, "request");
        assert_eq!(stack_frame.variables[0].kind, "parameter");
        assert_eq!(stack_frame.variables[0].offset, 8);
        assert_eq!(
            stack_frame.variables[0].data_type_id.as_deref(),
            Some("ghidra:datatype:/fixture/LoginRequest:1001")
        );
        assert_eq!(stack_frame.variables[0].storage.kind, "stack");
        assert_eq!(stack_frame.variables[0].storage.pieces[0].space, "stack");
        assert_eq!(
            stack_frame.variables[0].high_variable_id.as_deref(),
            Some("ghidra:highvar:401000:parameter:17:request")
        );
        let stack_effect = &output.oracle_call_stack_effect_facts[0];
        assert_eq!(
            stack_effect.effect_id,
            "ghidra:stack_effect:main:sqlite_prepare"
        );
        assert_eq!(
            stack_effect.function_id.as_deref(),
            Some("ghidra:function:401000")
        );
        assert_eq!(stack_effect.entry_point, "0x401000");
        assert_eq!(stack_effect.callsite_address, "0x401000");
        assert_eq!(
            stack_effect.callee_function_id.as_deref(),
            Some("ghidra:function:external:sqlite3_prepare_v2")
        );
        assert_eq!(
            stack_effect.callee_name.as_deref(),
            Some("sqlite3_prepare_v2")
        );
        assert_eq!(stack_effect.call_opcode.as_deref(), Some("CALL"));
        assert_eq!(stack_effect.prototype_model.as_deref(), Some("__stdcall"));
        assert_eq!(stack_effect.stack_pointer_register.as_deref(), Some("RSP"));
        assert_eq!(stack_effect.stack_space.as_deref(), Some("stack"));
        assert_eq!(stack_effect.stack_offset_before_call, Some(-32));
        assert_eq!(stack_effect.instruction_stack_depth_change, Some(0));
        assert_eq!(stack_effect.stack_shift_bytes, Some(8));
        assert_eq!(stack_effect.purge_size_bytes, Some(16));
        assert_eq!(stack_effect.extra_pop_bytes, Some(24));
        assert_eq!(stack_effect.effective_extra_pop_bytes, Some(24));
        assert_eq!(stack_effect.companion_solution_bytes, Some(0));
        assert_eq!(stack_effect.solver_variable_count, 4);
        assert_eq!(stack_effect.missed_variable_count, 0);
        assert_eq!(stack_effect.status, "solved");
        assert_eq!(
            stack_effect.warnings,
            vec!["effective_extra_pop_from_call_depth".to_string()]
        );
        let parameter_measure = &output.oracle_parameter_measure_facts[0];
        assert_eq!(
            parameter_measure.measure_id,
            "ghidra:param_measure:login:request"
        );
        assert_eq!(
            parameter_measure.function_id.as_deref(),
            Some("ghidra:function:401000")
        );
        assert_eq!(parameter_measure.entry_point, "0x401000");
        assert_eq!(parameter_measure.io, "input");
        assert_eq!(parameter_measure.rank, "this_function_parameter");
        assert_eq!(parameter_measure.rank_value, Some(4));
        assert_eq!(parameter_measure.storage.kind, "register");
        assert_eq!(
            parameter_measure.storage.pieces[0].register.as_deref(),
            Some("RDI")
        );
        assert_eq!(parameter_measure.data_type, "struct LoginRequest *");
        assert_eq!(
            parameter_measure.data_type_id.as_deref(),
            Some("ghidra:datatype:/fixture/LoginRequest:1001")
        );
        assert_eq!(parameter_measure.model_name.as_deref(), Some("__stdcall"));
        assert_eq!(parameter_measure.extra_pop, Some(16));
        assert!(parameter_measure.just_prototype);
        assert_eq!(
            parameter_measure.base_variable_id.as_deref(),
            Some("ghidra:highvar:401000:parameter:17:request")
        );
        assert_eq!(
            parameter_measure.source_statement_id.as_deref(),
            Some("ghidra:pcode:401000:0")
        );
        assert_eq!(parameter_measure.num_calls, 2);
        let field_access = &output.oracle_structure_field_access_facts[0];
        assert_eq!(
            field_access.access_id,
            "ghidra:structure_field_access:login:request:flags"
        );
        assert_eq!(field_access.entry_point, "0x401000");
        assert_eq!(field_access.address, "0x40100c");
        assert_eq!(field_access.access_kind, "bit_field_read");
        assert_eq!(
            field_access.structure_type_id,
            "ghidra:datatype:/fixture/LoginRequest:1001"
        );
        assert_eq!(field_access.field_name.as_deref(), Some("flags"));
        assert_eq!(field_access.pcode_opcode, "LOAD");
        assert_eq!(field_access.bit_offset, Some(1));
        assert_eq!(field_access.bit_size, Some(3));
        assert_eq!(
            output.analyzer_receipts[0].evidence_ids,
            vec!["e:ghidra-export"]
        );
        assert!(output.analyzer_receipts[0]
            .output_labels
            .contains(&GHIDRA_ORACLE_PCODE_OP_FACT_LABEL.to_string()));
        assert!(output.analyzer_receipts[0]
            .output_labels
            .contains(&GHIDRA_ORACLE_SYMBOLIC_SUMMARY_FACT_LABEL.to_string()));
        assert!(output.analyzer_receipts[0]
            .output_labels
            .contains(&GHIDRA_ORACLE_DECOMPILER_DIAGNOSTIC_FACT_LABEL.to_string()));
        assert!(output.analyzer_receipts[0]
            .output_labels
            .contains(&GHIDRA_ORACLE_PARAMETER_MEASURE_FACT_LABEL.to_string()));
        assert!(output.analyzer_receipts[0]
            .output_labels
            .contains(&GHIDRA_ORACLE_SEMANTIC_SIGNATURE_FACT_LABEL.to_string()));
        assert!(output.analyzer_receipts[0]
            .output_labels
            .contains(&GHIDRA_ORACLE_FUNCTION_ID_FACT_LABEL.to_string()));
        assert!(output.analyzer_receipts[0]
            .output_labels
            .contains(&GHIDRA_ORACLE_JUMP_TABLE_FACT_LABEL.to_string()));
        assert!(output.analyzer_receipts[0]
            .output_labels
            .contains(&GHIDRA_ORACLE_EXTERNAL_LINKAGE_FACT_LABEL.to_string()));
        assert!(output.analyzer_receipts[0]
            .output_labels
            .contains(&GHIDRA_ORACLE_FUNCTION_PROTOTYPE_FACT_LABEL.to_string()));
        assert!(output.analyzer_receipts[0]
            .output_labels
            .contains(&GHIDRA_ORACLE_DATA_TYPE_FACT_LABEL.to_string()));
        assert!(output.analyzer_receipts[0]
            .output_labels
            .contains(&GHIDRA_ORACLE_HIGH_VARIABLE_FACT_LABEL.to_string()));
        assert!(output.analyzer_receipts[0]
            .output_labels
            .contains(&GHIDRA_ORACLE_STACK_FRAME_FACT_LABEL.to_string()));
        assert!(output.analyzer_receipts[0]
            .output_labels
            .contains(&GHIDRA_ORACLE_CALL_STACK_EFFECT_FACT_LABEL.to_string()));
        assert!(output.analyzer_receipts[0]
            .output_labels
            .contains(&GHIDRA_ORACLE_ANNOTATION_FACT_LABEL.to_string()));
        assert!(output.analyzer_receipts[0]
            .output_labels
            .contains(&GHIDRA_ORACLE_STRUCTURE_FIELD_ACCESS_FACT_LABEL.to_string()));
        assert!(output.graph_nodes.iter().any(|node| node
            .labels
            .contains(&GHIDRA_ORACLE_REFERENCE_FACT_LABEL.to_string())));
        assert!(output.graph_nodes.iter().any(|node| node
            .labels
            .contains(&GHIDRA_ORACLE_JUMP_TABLE_FACT_LABEL.to_string())));
        assert!(output.graph_nodes.iter().any(|node| node
            .labels
            .contains(&GHIDRA_ORACLE_EXTERNAL_LINKAGE_FACT_LABEL.to_string())));
        assert!(output.graph_nodes.iter().any(|node| node
            .labels
            .contains(&GHIDRA_ORACLE_FUNCTION_PROTOTYPE_FACT_LABEL.to_string())));
        assert!(output.graph_nodes.iter().any(|node| node
            .labels
            .contains(&GHIDRA_ORACLE_DATA_TYPE_FACT_LABEL.to_string())));
        assert!(output.graph_nodes.iter().any(|node| node
            .labels
            .contains(&GHIDRA_ORACLE_HIGH_VARIABLE_FACT_LABEL.to_string())));
        assert!(output.graph_nodes.iter().any(|node| node
            .labels
            .contains(&GHIDRA_ORACLE_STACK_FRAME_FACT_LABEL.to_string())));
        assert!(output.graph_nodes.iter().any(|node| node
            .labels
            .contains(&GHIDRA_ORACLE_CALL_STACK_EFFECT_FACT_LABEL.to_string())));
        assert!(output.graph_nodes.iter().any(|node| node
            .labels
            .contains(&GHIDRA_ORACLE_SYMBOLIC_SUMMARY_FACT_LABEL.to_string())));
        assert!(output.graph_nodes.iter().any(|node| node
            .labels
            .contains(&GHIDRA_ORACLE_DECOMPILER_DIAGNOSTIC_FACT_LABEL.to_string())));
        assert!(output.graph_nodes.iter().any(|node| node
            .labels
            .contains(&GHIDRA_ORACLE_ANNOTATION_FACT_LABEL.to_string())));
        assert!(output.graph_nodes.iter().any(|node| node
            .labels
            .contains(&GHIDRA_ORACLE_SEMANTIC_SIGNATURE_FACT_LABEL.to_string())));
        assert!(output.graph_nodes.iter().any(|node| node
            .labels
            .contains(&GHIDRA_ORACLE_FUNCTION_ID_FACT_LABEL.to_string())));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == ORACLE_FIXTURE_HAS_SYMBOLIC_SUMMARY_FACT));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == ORACLE_FIXTURE_HAS_DECOMPILER_DIAGNOSTIC_FACT));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == ORACLE_FIXTURE_HAS_ANNOTATION_FACT));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == ORACLE_FIXTURE_HAS_SEMANTIC_SIGNATURE_FACT));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == ORACLE_FIXTURE_HAS_FUNCTION_ID_FACT));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == ORACLE_FIXTURE_HAS_JUMP_TABLE_FACT));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == ORACLE_FIXTURE_HAS_EXTERNAL_LINKAGE_FACT));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == ORACLE_FIXTURE_HAS_FUNCTION_PROTOTYPE_FACT));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == ORACLE_FIXTURE_HAS_DATA_TYPE_FACT));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == ORACLE_FIXTURE_HAS_STACK_FRAME_FACT));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == ORACLE_FIXTURE_HAS_CALL_STACK_EFFECT_FACT));
    }

    #[test]
    fn ghidra_oracle_timeout_marks_run_partial() {
        let mut fixture = GhidraOracleFixture {
            fixture_id: "ghidra:fixture:timeout".to_string(),
            source_uri: "ghidra://headless/timeout".to_string(),
            export_script: "ExportTheoremFacts.java".to_string(),
            program_summary: GhidraOracleProgramSummary {
                ghidra_version: "11.4.2".to_string(),
                language_id: Some("x86:LE:64:default".to_string()),
                compiler_spec_id: Some("gcc".to_string()),
                analysis_timeout_occurred: true,
                function_count: 1,
                import_count: 1,
                string_count: 1,
                cfg_edge_count: 1,
            },
            evidence_ids: vec!["e:timeout".to_string()],
        };
        let complete = ghidra_oracle_fixture_to_program_analysis_input(
            "Travis-Gilbert",
            artifact(),
            fixture.clone(),
        );
        fixture.program_summary.analysis_timeout_occurred = false;
        let non_timeout =
            ghidra_oracle_fixture_to_program_analysis_input("Travis-Gilbert", artifact(), fixture);

        let timed_out = compile_program_analysis_run_in_memory(complete);
        let finished = compile_program_analysis_run_in_memory(non_timeout);

        assert_eq!(timed_out.run.status, ProgramAnalysisStatus::Partial);
        assert_eq!(
            timed_out.analyzer_receipts[0].status,
            ProgramAnalysisStatus::Partial
        );
        assert_eq!(finished.run.status, ProgramAnalysisStatus::Complete);
        assert_ne!(timed_out.run.run_id, finished.run.run_id);
    }

    #[test]
    fn matching_ghidra_oracle_summary_does_not_emit_drift() {
        let mut input = fixture_input();
        input.oracle_fixture = Some(GhidraOracleFixture {
            fixture_id: "ghidra:fixture:matched".to_string(),
            source_uri: "ghidra://headless/matched".to_string(),
            export_script: "ExportTheoremFacts.java".to_string(),
            program_summary: GhidraOracleProgramSummary {
                ghidra_version: "11.4.2".to_string(),
                language_id: Some("x86:LE:64:default".to_string()),
                compiler_spec_id: Some("gcc".to_string()),
                analysis_timeout_occurred: false,
                function_count: 1,
                import_count: 1,
                string_count: 1,
                cfg_edge_count: 2,
            },
            evidence_ids: vec!["e:oracle".to_string()],
        });

        let output = compile_program_analysis_run_in_memory(input);

        assert!(output.analysis_drifts.is_empty());
        assert!(!output
            .graph_nodes
            .iter()
            .any(|node| node.labels.contains(&ANALYSIS_DRIFT_LABEL.to_string())));
    }

    #[test]
    fn ghidra_oracle_summary_mismatch_writes_analysis_drift() {
        let mut store = InMemoryGraphStore::new();
        let mut input = fixture_input();
        input.oracle_fixture = Some(GhidraOracleFixture {
            fixture_id: "ghidra:fixture:mismatch".to_string(),
            source_uri: "ghidra://headless/mismatch".to_string(),
            export_script: "ExportTheoremFacts.java".to_string(),
            program_summary: GhidraOracleProgramSummary {
                ghidra_version: "11.4.2".to_string(),
                language_id: Some("x86:LE:64:default".to_string()),
                compiler_spec_id: Some("gcc".to_string()),
                analysis_timeout_occurred: false,
                function_count: 2,
                import_count: 1,
                string_count: 1,
                cfg_edge_count: 2,
            },
            evidence_ids: vec!["e:oracle".to_string()],
        });

        let output = compile_program_analysis_run_in_store(&mut store, input)
            .expect("oracle drift payload writes");

        assert_eq!(output.analysis_drifts.len(), 1);
        let drift = &output.analysis_drifts[0];
        assert_eq!(drift.fact_kind, "function_count");
        assert_eq!(drift.drift_kind, AnalysisDriftKind::CountMismatch);
        assert_eq!(drift.expected, json!(2));
        assert_eq!(drift.observed, json!(1));
        assert_eq!(
            drift.evidence_ids,
            vec!["e:oracle", "ghidra:fixture:mismatch"]
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(ANALYSIS_DRIFT_LABEL))
                .len(),
            1
        );
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == HAS_ANALYSIS_DRIFT));
    }

    #[test]
    fn matching_ghidra_address_oracle_facts_write_without_drift() {
        let mut store = InMemoryGraphStore::new();
        let output =
            compile_program_analysis_run_in_store(&mut store, fixture_input_with_address_oracle())
                .expect("address oracle payload writes");

        assert!(output.analysis_drifts.is_empty());
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(PROGRAM_PCODE_FACT_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(GHIDRA_ORACLE_FUNCTION_FACT_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(GHIDRA_ORACLE_PCODE_OP_FACT_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(GHIDRA_ORACLE_REFERENCE_FACT_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(GHIDRA_ORACLE_CALL_EDGE_FACT_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(GHIDRA_ORACLE_JUMP_TABLE_FACT_LABEL))
                .len(),
            1
        );
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == ORACLE_FIXTURE_HAS_PCODE_OP_FACT));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == ORACLE_FIXTURE_HAS_JUMP_TABLE_FACT));
        assert_eq!(output.reference_recovery_evidence.len(), 1);
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(REFERENCE_RECOVERY_EVIDENCE_LABEL))
                .len(),
            1
        );
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == REFERENCE_PRODUCES_RECOVERY_EVIDENCE));
        let reference_node = store
            .query_nodes(NodeQuery::label(GHIDRA_ORACLE_REFERENCE_FACT_LABEL))
            .into_iter()
            .next()
            .expect("reference oracle fact writes");
        assert_eq!(
            reference_node.properties.get("semantic_roles"),
            Some(&json!(["memory", "primary"]))
        );
        assert_eq!(
            reference_node.properties.get("is_memory"),
            Some(&json!(true))
        );
        let recovery_node = store
            .query_nodes(NodeQuery::label(REFERENCE_RECOVERY_EVIDENCE_LABEL))
            .into_iter()
            .next()
            .expect("reference recovery evidence writes");
        assert_eq!(
            recovery_node.properties.get("semantic_roles"),
            Some(&json!(["memory", "primary"]))
        );
        assert_eq!(recovery_node.properties.get("confidence"), Some(&json!(85)));
    }

    #[test]
    fn ghidra_reference_semantic_roles_are_derived_from_ref_type() {
        let mut input = fixture_input_with_address_oracle();
        input.oracle_reference_facts[0].reference_type = "UNCONDITIONAL_CALL".to_string();
        input.oracle_reference_facts[0].semantic_roles = vec![" external ".to_string()];
        input.oracle_reference_facts[0].is_external = true;
        input.oracle_reference_facts[0].is_memory = true;

        let output = compile_program_analysis_run_in_memory(input);
        let reference = &output.oracle_reference_facts[0];

        assert_eq!(
            reference.semantic_roles,
            vec!["call", "external", "flow", "memory", "primary"]
        );
        assert_eq!(
            output.reference_recovery_evidence[0].semantic_roles,
            vec!["call", "external", "flow", "memory", "primary"]
        );
        assert_eq!(output.reference_recovery_evidence[0].confidence, 90);
        assert!(output
            .analysis_drifts
            .iter()
            .any(|drift| drift.fact_kind == "reference"
                && drift.expected.get("semantic_roles")
                    == Some(&json!(["call", "external", "flow", "memory", "primary"]))));
    }

    #[test]
    fn ghidra_address_oracle_mismatch_emits_exact_drifts() {
        let mut input = fixture_input_with_address_oracle();
        input.oracle_function_facts[0].body_end = "0x401050".to_string();
        input.oracle_pcode_facts[0].ghidra_opcode_id = 8;
        input.oracle_reference_facts[0].to_address = "0x403000".to_string();
        input.oracle_call_edge_facts[0].target_entry = "0x404000".to_string();

        let output = compile_program_analysis_run_in_memory(input);
        let fact_kinds = output
            .analysis_drifts
            .iter()
            .map(|drift| drift.fact_kind.as_str())
            .collect::<Vec<_>>();

        assert!(fact_kinds.contains(&"function_boundary"));
        assert!(fact_kinds.contains(&"pcode_op"));
        assert!(fact_kinds.contains(&"reference"));
        assert!(fact_kinds.contains(&"call_edge"));
        assert!(output
            .analysis_drifts
            .iter()
            .any(|drift| drift.drift_kind == AnalysisDriftKind::FieldMismatch));
        assert!(output
            .analysis_drifts
            .iter()
            .any(|drift| drift.drift_kind == AnalysisDriftKind::MissingNativeFact));
    }

    #[test]
    fn runtime_trace_contract_writes_snapshots_events_and_taint_marks() {
        let mut store = InMemoryGraphStore::new();
        let output =
            compile_program_analysis_run_in_store(&mut store, fixture_input_with_runtime_trace())
                .expect("runtime trace payload writes");

        assert_eq!(
            store
                .query_nodes(NodeQuery::label(RUNTIME_TRACE_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(TRACE_SNAPSHOT_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store.query_nodes(NodeQuery::label(TRACE_EVENT_LABEL)).len(),
            1
        );
        assert_eq!(
            store.query_nodes(NodeQuery::label(TAINT_MARK_LABEL)).len(),
            1
        );

        let trace = output
            .graph_nodes
            .iter()
            .find(|node| node.labels.contains(&RUNTIME_TRACE_LABEL.to_string()))
            .expect("trace node exists");
        assert_eq!(
            trace.properties.get("language_id"),
            Some(&json!("x86:LE:64:default"))
        );
        assert_eq!(
            trace.properties.get("compiler_spec_id"),
            Some(&json!("gcc"))
        );
        assert_eq!(
            trace.properties.get("authority_layer"),
            Some(&json!(AUTHORITY_OBSERVED_FACT))
        );

        let snapshot = output
            .graph_nodes
            .iter()
            .find(|node| node.labels.contains(&TRACE_SNAPSHOT_LABEL.to_string()))
            .expect("snapshot node exists");
        assert_eq!(
            snapshot.properties.get("description"),
            Some(&json!("entry snapshot"))
        );
        assert_eq!(
            snapshot
                .properties
                .get("schedule")
                .and_then(|value| value.get("schedule")),
            Some(&json!("0:1.2"))
        );
        assert_eq!(snapshot.properties.get("forked"), Some(&json!(true)));

        let taint = output
            .graph_nodes
            .iter()
            .find(|node| node.labels.contains(&TAINT_MARK_LABEL.to_string()))
            .expect("taint node exists");
        assert_eq!(
            taint.properties.get("labels"),
            Some(&json!(["argv[1]", "http.body"]))
        );
        assert_eq!(
            taint.properties.get("offset"),
            Some(&json!("0x7fffffffe000"))
        );
        assert_eq!(
            taint.properties.get("authority_layer"),
            Some(&json!(AUTHORITY_DERIVED_FACT))
        );

        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == HAS_RUNTIME_TRACE));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == TRACE_HAS_SNAPSHOT));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == SNAPSHOT_HAS_EVENT));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == SNAPSHOT_HAS_TAINT_MARK));
        assert!(output
            .graph_edges
            .iter()
            .any(|edge| edge.edge_type == EVENT_HAS_TAINT_MARK));
    }

    #[test]
    fn runtime_trace_facts_are_part_of_the_analysis_receipt_hash() {
        let base = compile_program_analysis_run_in_memory(fixture_input());
        let traced = compile_program_analysis_run_in_memory(fixture_input_with_runtime_trace());

        assert_ne!(base.run.artifact_hash, traced.run.artifact_hash);
        assert_ne!(base.run.run_id, traced.run.run_id);
        assert_eq!(traced.runtime_traces.len(), 1);
        assert_eq!(traced.trace_snapshots.len(), 1);
        assert_eq!(traced.trace_events.len(), 1);
        assert_eq!(traced.taint_marks.len(), 1);
    }

    fn graph_node_id_for_logical_id(
        output: &ProgramAnalysisOutput,
        label: &str,
        logical_id: &str,
    ) -> String {
        output
            .graph_nodes
            .iter()
            .find(|node| {
                node.labels.contains(&label.to_string())
                    && node.properties.get("logical_id") == Some(&json!(logical_id))
            })
            .map(|node| node.id.clone())
            .expect("graph node with logical id exists")
    }
}
