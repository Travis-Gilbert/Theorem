//! Portable behavior IR for feature-port reconstruction.
//!
//! This crate is intentionally substrate-light. Source-specific frontends lower
//! evidence into these contracts; target emitters interpret them into patches,
//! tests, and validation receipts.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const BEHAVIOR_IR_VERSION: &str = "behavior-ir-v0";
pub const FEATURE_SLICE_VERSION: &str = "feature-slice-v0";
pub const TARGET_PLAN_VERSION: &str = "target-plan-v0";
pub const PATCH_SET_VERSION: &str = "patch-set-v0";

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourceRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DependencyRef {
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvidenceRef {
    pub evidence_id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FeatureSlice {
    pub slice_id: String,
    pub version: String,
    pub source_ref: SourceRef,
    pub tenant_id: String,
    pub repo_id: String,
    #[serde(default)]
    pub seed: Option<String>,
    pub entry_symbols: Vec<String>,
    pub files: Vec<String>,
    pub tests: Vec<String>,
    pub docs: Vec<String>,
    pub runtime_examples: Vec<String>,
    pub dependencies: Vec<DependencyRef>,
    pub evidence: Vec<EvidenceRef>,
    pub unknowns: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    PureFunction,
    StatefulObject,
    ParserSerializer,
    ModelWrapper,
    HttpBoundary,
    FileEffect,
    DatabaseEffect,
    NetworkEffect,
    AsyncBoundary,
    NativeBoundary,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApiContract {
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_body: Option<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DataModelContract {
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OperationContract {
    pub operation_id: String,
    pub name: String,
    pub kind: OperationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ControlFlowContract {
    pub flow_id: String,
    pub summary: String,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectContract {
    pub effect_id: String,
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ErrorContract {
    pub error_id: String,
    pub summary: String,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExampleContract {
    pub example_id: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TestContract {
    pub test_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InvariantContract {
    pub invariant_id: String,
    pub summary: String,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PortabilityHazard {
    pub hazard_id: String,
    pub severity: String,
    pub summary: String,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BehaviorIr {
    pub ir_id: String,
    pub version: String,
    pub feature: FeatureSlice,
    pub purpose: String,
    pub public_api: Vec<ApiContract>,
    pub data_models: Vec<DataModelContract>,
    pub operations: Vec<OperationContract>,
    pub control_flow: Vec<ControlFlowContract>,
    pub effects: Vec<EffectContract>,
    pub errors: Vec<ErrorContract>,
    pub examples: Vec<ExampleContract>,
    pub tests: Vec<TestContract>,
    pub invariants: Vec<InvariantContract>,
    pub portability_hazards: Vec<PortabilityHazard>,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetLanguage {
    Python,
    #[serde(rename = "javascript", alias = "java_script")]
    JavaScript,
    #[serde(rename = "typescript", alias = "type_script")]
    TypeScript,
    Rust,
    Java,
    Go,
    Cpp,
    C,
    Ruby,
    #[serde(rename = "csharp", alias = "c_sharp")]
    CSharp,
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdiomLevel {
    Faithful,
    Idiomatic,
    FrameworkNative,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetProjectContext {
    #[serde(
        default,
        alias = "projectRoot",
        skip_serializing_if = "Option::is_none"
    )]
    pub project_root: Option<String>,
    #[serde(
        default,
        alias = "packageName",
        skip_serializing_if = "Option::is_none"
    )]
    pub package_name: Option<String>,
    #[serde(default)]
    pub conventions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetModulePlan {
    pub module_path: String,
    pub summary: String,
    #[serde(default)]
    pub operations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DependencySubstitution {
    pub source_dependency: String,
    pub target_dependency: String,
    pub rationale: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ValidationPlan {
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub parity_tests: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetObligation {
    pub obligation_id: String,
    pub summary: String,
    pub severity: String,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetPlan {
    pub plan_id: String,
    pub version: String,
    pub target_language: TargetLanguage,
    pub target_project: TargetProjectContext,
    pub module_plan: Vec<TargetModulePlan>,
    pub dependency_substitutions: Vec<DependencySubstitution>,
    pub idiom_level: IdiomLevel,
    pub validation_plan: ValidationPlan,
    pub obligations: Vec<TargetObligation>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchFileKind {
    Source,
    Test,
    Config,
    Documentation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PatchFile {
    pub path: String,
    pub kind: PatchFileKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    pub after: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ValidationReceipt {
    pub receipt_id: String,
    pub command: String,
    pub status: String,
    #[serde(default)]
    pub output_summary: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchStatus {
    Ready,
    NeedsReview,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PatchSet {
    pub patch_id: String,
    pub version: String,
    pub status: PatchStatus,
    pub target_language: TargetLanguage,
    pub files: Vec<PatchFile>,
    pub tests: Vec<PatchFile>,
    pub receipts: Vec<ValidationReceipt>,
    pub unresolved_obligations: Vec<TargetObligation>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behavior_ir_round_trips_as_stable_json() {
        let slice = FeatureSlice {
            slice_id: "slice:demo".to_string(),
            version: FEATURE_SLICE_VERSION.to_string(),
            source_ref: SourceRef {
                repo_id: Some("repo:demo".to_string()),
                ..SourceRef::default()
            },
            tenant_id: "Travis-Gilbert".to_string(),
            repo_id: "repo:demo".to_string(),
            seed: Some("parse payload".to_string()),
            entry_symbols: vec!["sym:parse".to_string()],
            files: vec!["src/parser.py".to_string()],
            tests: vec!["tests/test_parser.py".to_string()],
            docs: Vec::new(),
            runtime_examples: Vec::new(),
            dependencies: vec![DependencyRef {
                name: "json".to_string(),
                kind: "runtime".to_string(),
                source_id: None,
            }],
            evidence: vec![EvidenceRef {
                evidence_id: "sym:parse".to_string(),
                kind: "symbol".to_string(),
                file_path: Some("src/parser.py".to_string()),
                symbol_id: Some("sym:parse".to_string()),
                summary: None,
            }],
            unknowns: Vec::new(),
        };
        let ir = BehaviorIr {
            ir_id: "behavior:demo".to_string(),
            version: BEHAVIOR_IR_VERSION.to_string(),
            feature: slice,
            purpose: "Port selected parser behavior".to_string(),
            public_api: Vec::new(),
            data_models: Vec::new(),
            operations: Vec::new(),
            control_flow: Vec::new(),
            effects: Vec::new(),
            errors: Vec::new(),
            examples: Vec::new(),
            tests: Vec::new(),
            invariants: Vec::new(),
            portability_hazards: Vec::new(),
            evidence: Vec::new(),
        };

        let value = serde_json::to_value(&ir).unwrap();
        assert_eq!(value["version"], BEHAVIOR_IR_VERSION);
        assert_eq!(value["feature"]["version"], FEATURE_SLICE_VERSION);
        let decoded: BehaviorIr = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.ir_id, "behavior:demo");
    }
}
