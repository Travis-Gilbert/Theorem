//! Semantic reconstruction compiler for binary artifacts.
//!
//! This crate takes observed binary facts plus THIR and emits graph-backed
//! reconstruction obligations. It intentionally does not produce human-facing
//! decompiler text; it produces bounded tasks with evidence and validators.

use std::collections::{BTreeMap, BTreeSet};

use petgraph::graphmap::DiGraphMap;
use rustyred_thg_binformat::{BinaryLoadReport, BinaryString};
use rustyred_thg_core::{stable_hash, EdgeRecord, GraphStore, GraphStoreResult, NodeRecord};
use rustyred_thg_lift::{ThirFunction, ThirProgram, ThirStmt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const SEMANTIC_ROLE_LABEL: &str = "SemanticRole";
pub const COMPONENT_HYPOTHESIS_LABEL: &str = "ComponentHypothesis";
pub const RECONSTRUCTION_PLAN_LABEL: &str = "ReconstructionPlan";
pub const RECONSTRUCTION_INSTRUCTION_LABEL: &str = "ReconstructionInstruction";
pub const VALIDATION_RECEIPT_LABEL: &str = "ValidationReceipt";

pub const FUNCTION_HAS_SEMANTIC_ROLE: &str = "FUNCTION_HAS_SEMANTIC_ROLE";
pub const ROLE_EVIDENCED_BY: &str = "ROLE_EVIDENCED_BY";
pub const BELONGS_TO_COMPONENT: &str = "BELONGS_TO_COMPONENT";
pub const PLAN_HAS_INSTRUCTION: &str = "PLAN_HAS_INSTRUCTION";
pub const INSTRUCTION_TARGETS_COMPONENT: &str = "INSTRUCTION_TARGETS_COMPONENT";
pub const INSTRUCTION_EVIDENCED_BY: &str = "INSTRUCTION_EVIDENCED_BY";
pub const RECEIPT_VALIDATES_INSTRUCTION: &str = "RECEIPT_VALIDATES_INSTRUCTION";

pub const RECONSTRUCT_SOURCE: &str = "rustyred-thg-reconstruct";
pub const RECONSTRUCT_VERSION: &str = "rustyred-thg-reconstruct-v0";

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticRoleKind {
    Parser,
    AuthCheck,
    HttpRoute,
    DatabaseAccess,
    CryptoWrapper,
    Entrypoint,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SemanticRole {
    pub role_id: String,
    pub function_id: String,
    pub role: SemanticRoleKind,
    pub confidence: f64,
    pub evidence: Vec<String>,
    pub authority: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ComponentHypothesis {
    pub component_id: String,
    pub artifact_id: String,
    pub name: String,
    pub function_ids: Vec<String>,
    pub role_ids: Vec<String>,
    pub confidence: f64,
    pub authority: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ReconstructionPlan {
    pub plan_id: String,
    pub source_artifact: String,
    pub instructions: Vec<ReconstructionInstruction>,
    pub confidence: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ReconstructionInstruction {
    pub id: String,
    pub source_artifact: String,
    pub target: ReconstructionTarget,
    pub action: ReconstructionAction,
    pub requirements: Vec<Requirement>,
    pub validators: Vec<ValidatorSpec>,
    pub evidence: Vec<String>,
    pub confidence: f64,
    pub uncertainty: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReconstructionTarget {
    pub kind: String,
    pub id: String,
    pub language: String,
    pub runtime: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ReconstructionAction {
    ImplementComponent,
    ImplementFunction,
    PreserveProtocol,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type")]
pub enum Requirement {
    SemanticRole {
        role: SemanticRoleKind,
        summary: String,
    },
    BranchInvariant {
        condition: String,
        then_behavior: String,
        else_behavior: String,
    },
    InputOutputExample {
        input: String,
        output: String,
    },
    EvidenceTrace {
        evidence: Vec<String>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type")]
pub enum ValidatorSpec {
    GoldenFixture { input: String, expected: String },
    BranchCoverage { branch_count: usize },
    EvidencePresence { evidence: Vec<String> },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ValidationReceipt {
    pub receipt_id: String,
    pub instruction_id: String,
    pub validator_type: String,
    pub passed: bool,
    pub observed: Value,
    pub expected: Value,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ReconstructionAnalysis {
    pub roles: Vec<SemanticRole>,
    pub components: Vec<ComponentHypothesis>,
    pub plan: ReconstructionPlan,
}

pub fn compile_reconstruction_analysis(
    load: &BinaryLoadReport,
    program: &ThirProgram,
) -> ReconstructionAnalysis {
    let roles = derive_semantic_roles(load, program);
    let components = recover_components(load, program, &roles);
    let plan = compile_reconstruction_plan(load, &components, &roles, program);
    ReconstructionAnalysis {
        roles,
        components,
        plan,
    }
}

pub fn derive_semantic_roles(load: &BinaryLoadReport, program: &ThirProgram) -> Vec<SemanticRole> {
    let evidence_by_function = evidence_by_function_and_role(load, program);
    let mut roles = Vec::new();
    for function in &program.functions {
        if is_entry_function(load, function) {
            roles.push(role_for(
                function,
                SemanticRoleKind::Entrypoint,
                0.72,
                Vec::new(),
                "derived_fact",
            ));
        }
        if let Some(evidence_by_role) = evidence_by_function.get(&function.function_id) {
            for (role, evidence) in evidence_by_role {
                roles.push(role_for(
                    function,
                    role.clone(),
                    role_confidence(role, evidence.len()),
                    evidence.clone(),
                    "hypothesis",
                ));
            }
        }
    }
    roles.sort_by(|left, right| left.role_id.cmp(&right.role_id));
    roles.dedup_by(|left, right| left.role_id == right.role_id);
    roles
}

pub fn recover_components(
    load: &BinaryLoadReport,
    program: &ThirProgram,
    roles: &[SemanticRole],
) -> Vec<ComponentHypothesis> {
    let mut graph = DiGraphMap::<&str, ()>::new();
    for function in &program.functions {
        graph.add_node(function.function_id.as_str());
    }
    for function in &program.functions {
        for target in function
            .blocks
            .iter()
            .flat_map(|block| block.successors.iter())
        {
            if let Some(target_fn) = function_by_address(program, *target) {
                graph.add_edge(
                    function.function_id.as_str(),
                    target_fn.function_id.as_str(),
                    (),
                );
            }
        }
    }

    let role_by_function = roles.iter().fold(
        BTreeMap::<String, Vec<&SemanticRole>>::new(),
        |mut acc, role| {
            acc.entry(role.function_id.clone()).or_default().push(role);
            acc
        },
    );
    program
        .functions
        .iter()
        .map(|function| {
            let function_roles = role_by_function
                .get(&function.function_id)
                .cloned()
                .unwrap_or_default();
            let role_ids = function_roles
                .iter()
                .map(|role| role.role_id.clone())
                .collect::<Vec<_>>();
            let name = component_name(function, &function_roles);
            let degree = graph
                .neighbors_directed(function.function_id.as_str(), petgraph::Direction::Outgoing)
                .count()
                + graph
                    .neighbors_directed(
                        function.function_id.as_str(),
                        petgraph::Direction::Incoming,
                    )
                    .count();
            ComponentHypothesis {
                component_id: format!(
                    "recon:component:{}",
                    stable_hash(json!([
                        &load.artifact.artifact_id,
                        &function.function_id,
                        &role_ids
                    ]))
                ),
                artifact_id: load.artifact.artifact_id.clone(),
                name,
                function_ids: vec![function.function_id.clone()],
                role_ids,
                confidence: (0.55 + (degree as f64 * 0.04)).min(0.82),
                authority: "hypothesis".to_string(),
            }
        })
        .collect()
}

pub fn compile_reconstruction_plan(
    load: &BinaryLoadReport,
    components: &[ComponentHypothesis],
    roles: &[SemanticRole],
    program: &ThirProgram,
) -> ReconstructionPlan {
    let roles_by_id = roles
        .iter()
        .map(|role| (role.role_id.clone(), role))
        .collect::<BTreeMap<_, _>>();
    let instructions = components
        .iter()
        .map(|component| {
            let component_roles = component
                .role_ids
                .iter()
                .filter_map(|role_id| roles_by_id.get(role_id).copied())
                .collect::<Vec<_>>();
            instruction_for_component(load, component, &component_roles, program)
        })
        .collect::<Vec<_>>();
    let confidence = if instructions.is_empty() {
        0.0
    } else {
        instructions
            .iter()
            .map(|instruction| instruction.confidence)
            .sum::<f64>()
            / instructions.len() as f64
    };
    ReconstructionPlan {
        plan_id: format!(
            "recon:plan:{}",
            stable_hash(json!([&load.artifact.artifact_id, instructions.len()]))
        ),
        source_artifact: load.artifact.artifact_id.clone(),
        instructions,
        confidence,
    }
}

pub fn write_reconstruction_analysis_in_store<S: GraphStore>(
    store: &mut S,
    analysis: &ReconstructionAnalysis,
) -> GraphStoreResult<()> {
    write_reconstruction_analysis_in_store_scoped(store, analysis, None)
}

pub fn write_reconstruction_analysis_in_store_for_tenant<S: GraphStore>(
    store: &mut S,
    analysis: &ReconstructionAnalysis,
    tenant_id: &str,
) -> GraphStoreResult<()> {
    write_reconstruction_analysis_in_store_scoped(store, analysis, Some(tenant_id))
}

fn write_reconstruction_analysis_in_store_scoped<S: GraphStore>(
    store: &mut S,
    analysis: &ReconstructionAnalysis,
    tenant_id: Option<&str>,
) -> GraphStoreResult<()> {
    for role in &analysis.roles {
        store.upsert_node(role_node(role, tenant_id))?;
        store.upsert_edge(EdgeRecord::new(
            edge_id(&role.function_id, FUNCTION_HAS_SEMANTIC_ROLE, &role.role_id),
            &role.function_id,
            FUNCTION_HAS_SEMANTIC_ROLE,
            &role.role_id,
            provenance_props(&role.authority, tenant_id),
        ))?;
        for evidence_id in &role.evidence {
            store.upsert_edge(EdgeRecord::new(
                edge_id(&role.role_id, ROLE_EVIDENCED_BY, evidence_id),
                &role.role_id,
                ROLE_EVIDENCED_BY,
                evidence_id,
                provenance_props(&role.authority, tenant_id),
            ))?;
        }
    }
    for component in &analysis.components {
        store.upsert_node(component_node(component, tenant_id))?;
        for function_id in &component.function_ids {
            store.upsert_edge(EdgeRecord::new(
                edge_id(function_id, BELONGS_TO_COMPONENT, &component.component_id),
                function_id,
                BELONGS_TO_COMPONENT,
                &component.component_id,
                provenance_props(&component.authority, tenant_id),
            ))?;
        }
    }
    store.upsert_node(plan_node(&analysis.plan, tenant_id))?;
    for instruction in &analysis.plan.instructions {
        store.upsert_node(instruction_node(instruction, tenant_id))?;
        store.upsert_edge(EdgeRecord::new(
            edge_id(
                &analysis.plan.plan_id,
                PLAN_HAS_INSTRUCTION,
                &instruction.id,
            ),
            &analysis.plan.plan_id,
            PLAN_HAS_INSTRUCTION,
            &instruction.id,
            provenance_props("instruction", tenant_id),
        ))?;
        store.upsert_edge(EdgeRecord::new(
            edge_id(
                &instruction.id,
                INSTRUCTION_TARGETS_COMPONENT,
                &instruction.target.id,
            ),
            &instruction.id,
            INSTRUCTION_TARGETS_COMPONENT,
            &instruction.target.id,
            provenance_props("instruction", tenant_id),
        ))?;
        for evidence_id in &instruction.evidence {
            store.upsert_edge(EdgeRecord::new(
                edge_id(&instruction.id, INSTRUCTION_EVIDENCED_BY, evidence_id),
                &instruction.id,
                INSTRUCTION_EVIDENCED_BY,
                evidence_id,
                provenance_props("instruction", tenant_id),
            ))?;
        }
    }
    Ok(())
}

pub fn write_validation_receipt_in_store<S: GraphStore>(
    store: &mut S,
    receipt: &ValidationReceipt,
) -> GraphStoreResult<()> {
    write_validation_receipt_in_store_scoped(store, receipt, None)
}

pub fn write_validation_receipt_in_store_for_tenant<S: GraphStore>(
    store: &mut S,
    receipt: &ValidationReceipt,
    tenant_id: &str,
) -> GraphStoreResult<()> {
    write_validation_receipt_in_store_scoped(store, receipt, Some(tenant_id))
}

fn write_validation_receipt_in_store_scoped<S: GraphStore>(
    store: &mut S,
    receipt: &ValidationReceipt,
    tenant_id: Option<&str>,
) -> GraphStoreResult<()> {
    store.upsert_node(receipt_node(receipt, tenant_id))?;
    store.upsert_edge(EdgeRecord::new(
        edge_id(
            &receipt.receipt_id,
            RECEIPT_VALIDATES_INSTRUCTION,
            &receipt.instruction_id,
        ),
        &receipt.receipt_id,
        RECEIPT_VALIDATES_INSTRUCTION,
        &receipt.instruction_id,
        provenance_props(
            if receipt.passed {
                "validated_instruction"
            } else {
                "instruction"
            },
            tenant_id,
        ),
    ))?;
    Ok(())
}

pub fn validate_instruction(
    instruction: &ReconstructionInstruction,
    observed: Value,
) -> ValidationReceipt {
    let expected = instruction
        .validators
        .first()
        .map(|validator| match validator {
            ValidatorSpec::GoldenFixture { expected, .. } => json!(expected),
            ValidatorSpec::BranchCoverage { branch_count } => json!(branch_count),
            ValidatorSpec::EvidencePresence { evidence } => json!(evidence),
        })
        .unwrap_or(Value::Null);
    let passed = observed == expected;
    ValidationReceipt {
        receipt_id: format!(
            "recon:receipt:{}",
            stable_hash(json!([&instruction.id, &observed, &expected]))
        ),
        instruction_id: instruction.id.clone(),
        validator_type: instruction
            .validators
            .first()
            .map(|validator| match validator {
                ValidatorSpec::GoldenFixture { .. } => "GoldenFixture",
                ValidatorSpec::BranchCoverage { .. } => "BranchCoverage",
                ValidatorSpec::EvidencePresence { .. } => "EvidencePresence",
            })
            .unwrap_or("None")
            .to_string(),
        passed,
        observed,
        expected,
        notes: Vec::new(),
    }
}

fn evidence_by_function_and_role(
    load: &BinaryLoadReport,
    program: &ThirProgram,
) -> BTreeMap<String, BTreeMap<SemanticRoleKind, Vec<String>>> {
    let mut map = BTreeMap::<String, BTreeMap<SemanticRoleKind, BTreeSet<String>>>::new();
    for string in &load.strings {
        if let Some(function) = single_function_owner(program) {
            for role in roles_for_string(string) {
                map.entry(function.function_id.clone())
                    .or_default()
                    .entry(role)
                    .or_default()
                    .insert(string.string_id.clone());
            }
        }
    }
    for symbol in &load.symbols {
        let name = symbol.name.to_ascii_lowercase();
        let role = if name.contains("sqlite3_") || name.contains("postgres") {
            Some(SemanticRoleKind::DatabaseAccess)
        } else if name.contains("ssl") || name.contains("crypto") || name.contains("sha") {
            Some(SemanticRoleKind::CryptoWrapper)
        } else if name.contains("parse") {
            Some(SemanticRoleKind::Parser)
        } else {
            None
        };
        if let (Some(role), Some(function)) = (role, function_for_address(program, symbol.address))
        {
            map.entry(function.function_id.clone())
                .or_default()
                .entry(role)
                .or_default()
                .insert(symbol.symbol_id.clone());
        }
    }
    map.into_iter()
        .map(|(function_id, evidence_by_role)| {
            let evidence_by_role = evidence_by_role
                .into_iter()
                .map(|(role, evidence)| (role, evidence.into_iter().collect()))
                .collect();
            (function_id, evidence_by_role)
        })
        .collect()
}

fn roles_for_string(string: &BinaryString) -> Vec<SemanticRoleKind> {
    let value = string.value.to_ascii_lowercase();
    let mut roles = Vec::new();
    if value.contains("/api/") || value.contains("http") || value.contains("route") {
        roles.push(SemanticRoleKind::HttpRoute);
    }
    if value.contains("login") || value.contains("auth") || value.contains("password") {
        roles.push(SemanticRoleKind::AuthCheck);
    }
    if value.contains("sqlite") || value.contains("select ") || value.contains("insert ") {
        roles.push(SemanticRoleKind::DatabaseAccess);
    }
    if value.contains("json") || value.contains("parse") {
        roles.push(SemanticRoleKind::Parser);
    }
    roles
}

fn role_for(
    function: &ThirFunction,
    role: SemanticRoleKind,
    confidence: f64,
    evidence: Vec<String>,
    authority: &str,
) -> SemanticRole {
    SemanticRole {
        role_id: format!(
            "semantic:role:{}",
            stable_hash(json!([&function.function_id, &role, &evidence]))
        ),
        function_id: function.function_id.clone(),
        role,
        confidence,
        evidence,
        authority: authority.to_string(),
    }
}

fn role_confidence(role: &SemanticRoleKind, evidence_count: usize) -> f64 {
    let base = match role {
        SemanticRoleKind::HttpRoute => 0.74,
        SemanticRoleKind::DatabaseAccess => 0.78,
        SemanticRoleKind::AuthCheck => 0.7,
        SemanticRoleKind::Parser => 0.66,
        SemanticRoleKind::CryptoWrapper => 0.72,
        SemanticRoleKind::Entrypoint => 0.72,
        SemanticRoleKind::Unknown => 0.3,
    };
    (base + (evidence_count.saturating_sub(1) as f64 * 0.03)).min(0.91)
}

fn is_entry_function(load: &BinaryLoadReport, function: &ThirFunction) -> bool {
    load.entrypoints
        .iter()
        .any(|entrypoint| entrypoint.address == function.address)
}

fn function_by_address<'a>(program: &'a ThirProgram, address: u64) -> Option<&'a ThirFunction> {
    program
        .functions
        .iter()
        .find(|function| function.address == address)
}

fn single_function_owner(program: &ThirProgram) -> Option<&ThirFunction> {
    (program.functions.len() == 1).then(|| &program.functions[0])
}

fn function_for_address(program: &ThirProgram, address: u64) -> Option<&ThirFunction> {
    if address == 0 {
        return None;
    }
    let mut functions = program.functions.iter().collect::<Vec<_>>();
    functions.sort_by_key(|function| function.address);
    for (index, function) in functions.iter().enumerate() {
        let next_address = functions
            .get(index + 1)
            .map(|next_function| next_function.address)
            .unwrap_or(u64::MAX);
        if address >= function.address && address < next_address {
            return Some(*function);
        }
    }
    None
}

fn component_name(function: &ThirFunction, roles: &[&SemanticRole]) -> String {
    roles
        .iter()
        .find(|role| role.role != SemanticRoleKind::Entrypoint)
        .map(|role| format!("{:?}Component", role.role))
        .or_else(|| function.name.clone())
        .unwrap_or_else(|| format!("Function_{:x}", function.address))
}

fn instruction_for_component(
    load: &BinaryLoadReport,
    component: &ComponentHypothesis,
    roles: &[&SemanticRole],
    program: &ThirProgram,
) -> ReconstructionInstruction {
    let mut requirements = roles
        .iter()
        .map(|role| Requirement::SemanticRole {
            role: role.role.clone(),
            summary: requirement_summary(&role.role),
        })
        .collect::<Vec<_>>();
    let branch_count = component
        .function_ids
        .iter()
        .filter_map(|function_id| {
            program
                .functions
                .iter()
                .find(|function| &function.function_id == function_id)
        })
        .flat_map(|function| function.blocks.iter())
        .flat_map(|block| block.statements.iter())
        .filter(|statement| matches!(statement, ThirStmt::Branch { .. }))
        .count();
    if branch_count > 0 {
        requirements.push(Requirement::BranchInvariant {
            condition: "Recovered branch structure must be preserved.".to_string(),
            then_behavior: "Follow the lifted branch target behavior.".to_string(),
            else_behavior: "Follow the lifted fallthrough behavior.".to_string(),
        });
    }
    let evidence = roles
        .iter()
        .flat_map(|role| role.evidence.iter().cloned())
        .chain(component.function_ids.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if !evidence.is_empty() {
        requirements.push(Requirement::EvidenceTrace {
            evidence: evidence.clone(),
        });
    }
    let confidence = if roles.is_empty() {
        component.confidence
    } else {
        ((roles.iter().map(|role| role.confidence).sum::<f64>() / roles.len() as f64)
            + component.confidence)
            / 2.0
    };
    ReconstructionInstruction {
        id: format!(
            "recon:instr:{}",
            stable_hash(json!([
                &load.artifact.artifact_id,
                &component.component_id,
                &evidence
            ]))
        ),
        source_artifact: load.artifact.artifact_id.clone(),
        target: ReconstructionTarget {
            kind: "component".to_string(),
            id: component.component_id.clone(),
            language: "rust".to_string(),
            runtime: "axum".to_string(),
        },
        action: ReconstructionAction::ImplementComponent,
        requirements,
        validators: vec![
            ValidatorSpec::EvidencePresence {
                evidence: evidence.clone(),
            },
            ValidatorSpec::BranchCoverage { branch_count },
        ],
        evidence,
        confidence,
        uncertainty: uncertainty_for_roles(roles),
    }
}

fn requirement_summary(role: &SemanticRoleKind) -> String {
    match role {
        SemanticRoleKind::Parser => "Implement equivalent input parsing behavior.".to_string(),
        SemanticRoleKind::AuthCheck => {
            "Preserve authorization and credential gate behavior.".to_string()
        }
        SemanticRoleKind::HttpRoute => {
            "Expose an equivalent route boundary and request/response contract.".to_string()
        }
        SemanticRoleKind::DatabaseAccess => {
            "Preserve database access and query preparation behavior.".to_string()
        }
        SemanticRoleKind::CryptoWrapper => {
            "Preserve cryptographic wrapper call boundaries and byte semantics.".to_string()
        }
        SemanticRoleKind::Entrypoint => "Preserve entrypoint reachability.".to_string(),
        SemanticRoleKind::Unknown => {
            "Preserve observed control-flow and data-flow behavior.".to_string()
        }
    }
}

fn uncertainty_for_roles(roles: &[&SemanticRole]) -> Vec<String> {
    let mut uncertainty = Vec::new();
    if roles.is_empty() {
        uncertainty.push(
            "Component has structural evidence but no semantic role evidence yet.".to_string(),
        );
    }
    for role in roles {
        if role.confidence < 0.75 {
            uncertainty.push(format!(
                "{:?} role is hypothesis-level and needs validation evidence.",
                role.role
            ));
        }
    }
    uncertainty
}

fn role_node(role: &SemanticRole, tenant_id: Option<&str>) -> NodeRecord {
    NodeRecord::new(
        &role.role_id,
        [SEMANTIC_ROLE_LABEL],
        stamp_tenant(
            json!({
            "function_id": &role.function_id,
            "role": &role.role,
            "confidence": role.confidence,
            "evidence": &role.evidence,
            "authority": &role.authority,
            "source": RECONSTRUCT_SOURCE,
            "version": RECONSTRUCT_VERSION,
            }),
            tenant_id,
        ),
    )
}

fn component_node(component: &ComponentHypothesis, tenant_id: Option<&str>) -> NodeRecord {
    NodeRecord::new(
        &component.component_id,
        [COMPONENT_HYPOTHESIS_LABEL],
        stamp_tenant(
            json!({
            "artifact_id": &component.artifact_id,
            "name": &component.name,
            "function_ids": &component.function_ids,
            "role_ids": &component.role_ids,
            "confidence": component.confidence,
            "authority": &component.authority,
            "source": RECONSTRUCT_SOURCE,
            "version": RECONSTRUCT_VERSION,
            }),
            tenant_id,
        ),
    )
}

fn plan_node(plan: &ReconstructionPlan, tenant_id: Option<&str>) -> NodeRecord {
    NodeRecord::new(
        &plan.plan_id,
        [RECONSTRUCTION_PLAN_LABEL],
        stamp_tenant(
            json!({
            "source_artifact": &plan.source_artifact,
            "instruction_count": plan.instructions.len(),
            "confidence": plan.confidence,
            "authority": "instruction",
            "source": RECONSTRUCT_SOURCE,
            "version": RECONSTRUCT_VERSION,
            }),
            tenant_id,
        ),
    )
}

fn instruction_node(
    instruction: &ReconstructionInstruction,
    tenant_id: Option<&str>,
) -> NodeRecord {
    NodeRecord::new(
        &instruction.id,
        [RECONSTRUCTION_INSTRUCTION_LABEL],
        stamp_tenant(
            json!({
            "source_artifact": &instruction.source_artifact,
            "target": &instruction.target,
            "action": &instruction.action,
            "requirements": &instruction.requirements,
            "validators": &instruction.validators,
            "evidence": &instruction.evidence,
            "confidence": instruction.confidence,
            "uncertainty": &instruction.uncertainty,
            "authority": "instruction",
            "source": RECONSTRUCT_SOURCE,
            "version": RECONSTRUCT_VERSION,
            }),
            tenant_id,
        ),
    )
}

fn receipt_node(receipt: &ValidationReceipt, tenant_id: Option<&str>) -> NodeRecord {
    NodeRecord::new(
        &receipt.receipt_id,
        [VALIDATION_RECEIPT_LABEL],
        stamp_tenant(
            json!({
            "instruction_id": &receipt.instruction_id,
            "validator_type": &receipt.validator_type,
            "passed": receipt.passed,
            "observed": &receipt.observed,
            "expected": &receipt.expected,
            "notes": &receipt.notes,
            "authority": if receipt.passed { "validated_instruction" } else { "instruction" },
            "source": RECONSTRUCT_SOURCE,
            "version": RECONSTRUCT_VERSION,
            }),
            tenant_id,
        ),
    )
}

fn provenance_props(authority: &str, tenant_id: Option<&str>) -> Value {
    stamp_tenant(
        json!({
            "authority": authority,
            "source": RECONSTRUCT_SOURCE,
            "version": RECONSTRUCT_VERSION,
        }),
        tenant_id,
    )
}

fn stamp_tenant(mut properties: Value, tenant_id: Option<&str>) -> Value {
    if let (Some(tenant_id), Value::Object(map)) = (tenant_id, &mut properties) {
        map.insert("tenant_id".to_string(), json!(tenant_id));
    }
    properties
}

fn edge_id(from: &str, edge_type: &str, to: &str) -> String {
    format!("recon:edge:{}", stable_hash(json!([from, edge_type, to])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_binformat::{
        write_binary_facts_in_store, BinaryArtifact, BinaryEntrypoint, BinaryLoadReport,
        BinarySection, BinaryString, BinarySymbol,
    };
    use rustyred_thg_core::{InMemoryGraphStore, NodeQuery};
    use rustyred_thg_disasm::{decode_instructions, write_instruction_facts_in_store};
    use rustyred_thg_lift::{lift_to_thir, write_thir_in_store};

    fn fixture_load_report() -> BinaryLoadReport {
        BinaryLoadReport {
            artifact: BinaryArtifact {
                artifact_id: "sha256:test".to_string(),
                sha256: "test".to_string(),
                name: "fixture".to_string(),
                format: "Elf".to_string(),
                arch: "X86_64".to_string(),
                endian: "Little".to_string(),
                entrypoint: 0x1000,
                byte_len: 3,
            },
            sections: vec![BinarySection {
                section_id: "section:text".to_string(),
                artifact_id: "sha256:test".to_string(),
                index: 0,
                name: ".text".to_string(),
                address: 0x1000,
                size: 3,
                kind: "Text".to_string(),
                executable: true,
                bytes: vec![0x90, 0x90, 0xc3],
            }],
            symbols: Vec::new(),
            strings: vec![
                BinaryString {
                    string_id: "string:route".to_string(),
                    artifact_id: "sha256:test".to_string(),
                    offset: 10,
                    value: "/api/login".to_string(),
                },
                BinaryString {
                    string_id: "string:db".to_string(),
                    artifact_id: "sha256:test".to_string(),
                    offset: 40,
                    value: "sqlite3_prepare_v2".to_string(),
                },
            ],
            relocations: Vec::new(),
            entrypoints: vec![BinaryEntrypoint {
                entrypoint_id: "entry".to_string(),
                artifact_id: "sha256:test".to_string(),
                address: 0x1000,
                kind: "entry".to_string(),
            }],
        }
    }

    fn thir_function(address: u64) -> ThirFunction {
        ThirFunction {
            function_id: format!("thir:function:sha256:test:{address:x}"),
            artifact_id: "sha256:test".to_string(),
            address,
            name: None,
            confidence: 0.8,
            blocks: Vec::new(),
        }
    }

    #[test]
    fn compiles_evidence_backed_instructions() {
        let load = fixture_load_report();
        let disasm = decode_instructions(&load).unwrap();
        let program = lift_to_thir(&load, &disasm);
        let analysis = compile_reconstruction_analysis(&load, &program);
        assert!(!analysis.roles.is_empty());
        assert_eq!(analysis.components.len(), 1);
        assert_eq!(analysis.plan.instructions.len(), 1);
        assert!(analysis.plan.instructions[0]
            .requirements
            .iter()
            .any(|requirement| matches!(
                requirement,
                Requirement::SemanticRole {
                    role: SemanticRoleKind::HttpRoute,
                    ..
                }
            )));
    }

    #[test]
    fn writes_reconstruction_graph_nodes() {
        let load = fixture_load_report();
        let disasm = decode_instructions(&load).unwrap();
        let program = lift_to_thir(&load, &disasm);
        let analysis = compile_reconstruction_analysis(&load, &program);
        let mut store = InMemoryGraphStore::new();
        write_binary_facts_in_store(&mut store, &load).unwrap();
        write_instruction_facts_in_store(&mut store, &disasm).unwrap();
        write_thir_in_store(&mut store, &program).unwrap();
        write_reconstruction_analysis_in_store(&mut store, &analysis).unwrap();
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(COMPONENT_HYPOTHESIS_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(RECONSTRUCTION_INSTRUCTION_LABEL))
                .len(),
            1
        );
    }

    #[test]
    fn tenant_scoped_writer_stamps_reconstruction_instruction_nodes() {
        let load = fixture_load_report();
        let disasm = decode_instructions(&load).unwrap();
        let program = lift_to_thir(&load, &disasm);
        let analysis = compile_reconstruction_analysis(&load, &program);
        let mut store = InMemoryGraphStore::new();
        write_binary_facts_in_store(&mut store, &load).unwrap();
        write_instruction_facts_in_store(&mut store, &disasm).unwrap();
        write_thir_in_store(&mut store, &program).unwrap();

        write_reconstruction_analysis_in_store_for_tenant(&mut store, &analysis, "Travis-Gilbert")
            .unwrap();

        assert_eq!(
            store
                .query_nodes(
                    NodeQuery::label(RECONSTRUCTION_INSTRUCTION_LABEL)
                        .with_property("tenant_id", json!("Travis-Gilbert")),
                )
                .len(),
            1
        );
        assert!(store
            .query_nodes(
                NodeQuery::label(RECONSTRUCTION_INSTRUCTION_LABEL)
                    .with_property("tenant_id", json!("Other-Tenant")),
            )
            .is_empty());
    }

    #[test]
    fn global_string_evidence_does_not_attach_roles_to_every_function() {
        let mut load = fixture_load_report();
        load.symbols = vec![BinarySymbol {
            symbol_id: "symbol:db".to_string(),
            artifact_id: "sha256:test".to_string(),
            index: 0,
            name: "sqlite3_prepare_v2".to_string(),
            address: 0x2000,
            size: 12,
            kind: "Text".to_string(),
            scope: "Dynamic".to_string(),
            is_definition: false,
        }];
        let program = ThirProgram {
            artifact_id: "sha256:test".to_string(),
            functions: vec![thir_function(0x1000), thir_function(0x2000)],
        };

        let roles = derive_semantic_roles(&load, &program);

        assert!(!roles.iter().any(|role| matches!(
            role.role,
            SemanticRoleKind::HttpRoute | SemanticRoleKind::AuthCheck
        )));
        assert!(roles.iter().any(|role| {
            role.function_id == "thir:function:sha256:test:2000"
                && role.role == SemanticRoleKind::DatabaseAccess
        }));
        assert!(!roles.iter().any(|role| {
            role.function_id == "thir:function:sha256:test:1000"
                && role.role == SemanticRoleKind::DatabaseAccess
        }));
    }

    #[test]
    fn validation_receipt_records_pass_fail() {
        let load = fixture_load_report();
        let disasm = decode_instructions(&load).unwrap();
        let program = lift_to_thir(&load, &disasm);
        let analysis = compile_reconstruction_analysis(&load, &program);
        let instruction = &analysis.plan.instructions[0];
        let receipt = validate_instruction(instruction, json!(instruction.evidence));
        assert!(receipt.passed);
    }
}
