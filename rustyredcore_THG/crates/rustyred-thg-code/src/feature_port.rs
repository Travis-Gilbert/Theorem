use std::collections::{BTreeMap, BTreeSet};

use rustyred_thg_behavior_ir::{
    ApiContract, BehaviorIr, DataModelContract, DependencyRef, EffectContract, ErrorContract,
    EvidenceRef, FeatureSlice, IdiomLevel, InvariantContract, OperationContract, OperationKind,
    PatchFile, PatchFileKind, PatchSet, PatchStatus, PortabilityHazard,
    SourceRef as BehaviorSourceRef, TargetLanguage, TargetModulePlan, TargetObligation, TargetPlan,
    TargetProjectContext, ValidationPlan, ValidationReceipt, BEHAVIOR_IR_VERSION,
    FEATURE_SLICE_VERSION, PATCH_SET_VERSION, TARGET_PLAN_VERSION,
};
use rustyred_thg_core::stable_hash;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{CodeSymbolSnapshot, ReconstructionSpec, SourceRef};

const DEFAULT_MAX_FILES: usize = 64;
const DEFAULT_MAX_SYMBOLS: usize = 128;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FeatureSliceInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<String>,
    #[serde(default)]
    pub entry_symbols: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub max_files: Option<usize>,
    #[serde(default)]
    pub max_symbols: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetPlanInput {
    pub target_language: TargetLanguage,
    #[serde(default = "default_idiom_level")]
    pub idiom_level: IdiomLevel,
    #[serde(default)]
    pub target_project: TargetProjectContext,
}

impl Default for TargetPlanInput {
    fn default() -> Self {
        Self {
            target_language: TargetLanguage::TypeScript,
            idiom_level: default_idiom_level(),
            target_project: TargetProjectContext::default(),
        }
    }
}

fn default_idiom_level() -> IdiomLevel {
    IdiomLevel::Faithful
}

pub fn feature_slice_from_reconstruction_spec(
    spec: &ReconstructionSpec,
    input: &FeatureSliceInput,
) -> FeatureSlice {
    let tenant_id = spec
        .code_spec
        .as_ref()
        .map(|code_spec| {
            code_spec
                .files
                .first()
                .map(|_| "Travis-Gilbert".to_string())
                .unwrap_or_else(|| "Travis-Gilbert".to_string())
        })
        .unwrap_or_else(|| "Travis-Gilbert".to_string());
    let repo_id = spec.provenance.repo_id.clone();
    let source_ref = behavior_source_ref(&spec.source_ref, &repo_id, spec.provenance.sha.clone());
    let Some(code_spec) = spec.code_spec.as_ref() else {
        return empty_feature_slice(
            input,
            source_ref,
            tenant_id,
            repo_id,
            "No code specification was available to select a feature slice.",
        );
    };

    let symbol_limit = input.max_symbols.unwrap_or(DEFAULT_MAX_SYMBOLS).max(1);
    let file_limit = input.max_files.unwrap_or(DEFAULT_MAX_FILES).max(1);
    let selected_symbols = select_symbols(&code_spec.symbols, input, symbol_limit);
    let selected_symbol_ids = selected_symbols
        .iter()
        .map(|symbol| symbol.symbol_id.clone())
        .collect::<BTreeSet<_>>();
    let selected_files = select_files(&code_spec.files, input, &selected_symbols, file_limit);
    let selected_file_set = selected_files.iter().cloned().collect::<BTreeSet<_>>();
    let tests = code_spec
        .files
        .iter()
        .filter(|file| selected_file_set.contains(&file.path) || is_test_path(&file.path))
        .filter(|file| is_test_path(&file.path))
        .map(|file| file.path.clone())
        .take(file_limit)
        .collect::<Vec<_>>();
    let docs = code_spec
        .files
        .iter()
        .filter(|file| is_doc_path(&file.path))
        .map(|file| file.path.clone())
        .take(file_limit)
        .collect::<Vec<_>>();
    let dependencies = dependencies_from_symbols(&selected_symbols);
    let mut evidence = Vec::new();
    evidence.extend(selected_files.iter().map(|file_path| EvidenceRef {
        evidence_id: format!("file:{file_path}"),
        kind: "file".to_string(),
        file_path: Some(file_path.clone()),
        symbol_id: None,
        summary: None,
    }));
    evidence.extend(selected_symbols.iter().map(|symbol| {
        EvidenceRef {
            evidence_id: symbol.symbol_id.clone(),
            kind: "symbol".to_string(),
            file_path: Some(symbol.file_path.clone()),
            symbol_id: Some(symbol.symbol_id.clone()),
            summary: symbol
                .signature
                .clone()
                .or_else(|| Some(symbol.name.clone())),
        }
    }));
    evidence.extend(spec.obligations.iter().filter_map(|obligation| {
        let relevant_symbol = obligation
            .target_symbol_id
            .as_deref()
            .is_some_and(|symbol_id| selected_symbol_ids.contains(symbol_id));
        let relevant_file = obligation
            .target_file
            .as_deref()
            .is_some_and(|file_path| selected_file_set.contains(file_path));
        (relevant_symbol || relevant_file || obligation.target_symbol_id.is_none()).then(|| {
            EvidenceRef {
                evidence_id: obligation.obligation_id.clone(),
                kind: "obligation".to_string(),
                file_path: obligation.target_file.clone(),
                symbol_id: obligation.target_symbol_id.clone(),
                summary: Some(obligation.obligation.clone()),
            }
        })
    }));

    let mut unknowns = spec
        .obligations
        .iter()
        .flat_map(|obligation| obligation.unknowns.clone())
        .collect::<BTreeSet<_>>();
    if selected_symbols.is_empty() {
        unknowns.insert("No source symbols matched the requested feature seed.".to_string());
    }
    if tests.is_empty() {
        unknowns.insert("No source tests were linked to the selected feature slice.".to_string());
    }

    FeatureSlice {
        slice_id: feature_slice_id(&repo_id, input, &selected_symbols, &selected_files),
        version: FEATURE_SLICE_VERSION.to_string(),
        source_ref,
        tenant_id,
        repo_id,
        seed: input.seed.clone(),
        entry_symbols: selected_symbols
            .iter()
            .map(|symbol| symbol.symbol_id.clone())
            .collect(),
        files: selected_files,
        tests,
        docs,
        runtime_examples: Vec::new(),
        dependencies,
        evidence,
        unknowns: unknowns.into_iter().collect(),
    }
}

pub fn behavior_ir_from_reconstruction_spec(
    spec: &ReconstructionSpec,
    input: &FeatureSliceInput,
) -> BehaviorIr {
    let feature = feature_slice_from_reconstruction_spec(spec, input);
    behavior_ir_from_feature_slice(spec, feature)
}

pub fn behavior_ir_from_feature_slice(
    spec: &ReconstructionSpec,
    feature: FeatureSlice,
) -> BehaviorIr {
    let symbols_by_id = spec
        .code_spec
        .as_ref()
        .map(|code_spec| {
            code_spec
                .symbols
                .iter()
                .map(|symbol| (symbol.symbol_id.clone(), symbol.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let selected_symbols = feature
        .entry_symbols
        .iter()
        .filter_map(|symbol_id| symbols_by_id.get(symbol_id).cloned())
        .collect::<Vec<_>>();
    let public_api = selected_symbols
        .iter()
        .map(api_contract_from_symbol)
        .collect::<Vec<_>>();
    let data_models = selected_symbols
        .iter()
        .filter(|symbol| is_data_model_kind(&symbol.kind))
        .map(data_model_from_symbol)
        .collect::<Vec<_>>();
    let operations = selected_symbols
        .iter()
        .filter(|symbol| !is_data_model_kind(&symbol.kind))
        .map(operation_from_symbol)
        .collect::<Vec<_>>();
    let effects = selected_symbols
        .iter()
        .flat_map(effects_from_symbol)
        .collect::<Vec<_>>();
    let errors = spec
        .drift
        .iter()
        .map(|finding| ErrorContract {
            error_id: finding.finding_id.clone(),
            summary: finding.suggested_next_step.clone(),
            evidence_ids: finding
                .symbol_id
                .clone()
                .into_iter()
                .chain([finding.symbol_key.clone()])
                .collect(),
        })
        .collect::<Vec<_>>();
    let invariants = spec
        .obligations
        .iter()
        .filter(|obligation| obligation.obligation.contains("Validate"))
        .map(|obligation| InvariantContract {
            invariant_id: obligation.obligation_id.clone(),
            summary: obligation.rationale.clone(),
            evidence_ids: obligation.evidence_ids.clone(),
        })
        .collect::<Vec<_>>();
    let mut hazards = portability_hazards_from_feature(&feature);
    if spec.binary.is_some() {
        hazards.push(PortabilityHazard {
            hazard_id: format!(
                "hazard:{}",
                stable_hash(json!([feature.slice_id, "binary-artifact"]))
            ),
            severity: "medium".to_string(),
            summary: "Binary reconstruction facts are present; target emission must decide whether source or binary evidence is authoritative.".to_string(),
            evidence_ids: Vec::new(),
        });
    }
    BehaviorIr {
        ir_id: behavior_ir_id(&feature),
        version: BEHAVIOR_IR_VERSION.to_string(),
        purpose: feature
            .seed
            .as_deref()
            .map(|seed| format!("Port behavior for feature seed `{seed}`."))
            .unwrap_or_else(|| "Port the selected source feature behavior.".to_string()),
        evidence: feature.evidence.clone(),
        feature,
        public_api,
        data_models,
        operations,
        control_flow: Vec::new(),
        effects,
        errors,
        examples: Vec::new(),
        tests: Vec::new(),
        invariants,
        portability_hazards: hazards,
    }
}

pub fn target_plan_input_from_value(value: &Value) -> Result<TargetPlanInput, String> {
    let target = value.get("target").unwrap_or(value);
    let language = target
        .get("target_language")
        .or_else(|| target.get("targetLanguage"))
        .or_else(|| target.get("language"))
        .or_else(|| value.get("target_language"))
        .or_else(|| value.get("targetLanguage"))
        .or_else(|| value.get("language"))
        .and_then(Value::as_str)
        .unwrap_or("typescript");
    let idiom = target
        .get("idiom_level")
        .or_else(|| target.get("idiomLevel"))
        .or_else(|| value.get("idiom_level"))
        .or_else(|| value.get("idiomLevel"))
        .and_then(Value::as_str)
        .unwrap_or("faithful");
    let target_project = target
        .get("target_project")
        .or_else(|| target.get("targetProject"))
        .or_else(|| value.get("target_project"))
        .or_else(|| value.get("targetProject"))
        .cloned()
        .or_else(|| {
            let mut project = serde_json::Map::new();
            for key in ["project_root", "projectRoot", "package_name", "packageName"] {
                if let Some(field) = value.get(key).cloned() {
                    project.insert(key.to_string(), field);
                }
            }
            (!project.is_empty()).then_some(Value::Object(project))
        })
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| format!("invalid target_project: {error}"))?
        .unwrap_or_default();
    Ok(TargetPlanInput {
        target_language: parse_target_language(language)?,
        idiom_level: parse_idiom_level(idiom)?,
        target_project,
    })
}

pub fn target_plan_from_behavior_ir(ir: &BehaviorIr, input: &TargetPlanInput) -> TargetPlan {
    let module_stem = module_stem(ir);
    let module_path = target_module_path(input.target_language, &module_stem);
    let test_path = target_test_path(input.target_language, &module_stem);
    let operations = ir
        .operations
        .iter()
        .map(|operation| operation.operation_id.clone())
        .collect::<Vec<_>>();
    let mut obligations = target_obligations_from_ir(ir);
    obligations.push(TargetObligation {
        obligation_id: format!(
            "target-obligation:{}",
            stable_hash(json!([ir.ir_id, "scaffold-implementation"]))
        ),
        summary: "Generated output is a behavior scaffold; implement target-language operation bodies and bind into the destination project before treating it as a port.".to_string(),
        severity: "high".to_string(),
        evidence_ids: ir.evidence.iter().map(|evidence| evidence.evidence_id.clone()).collect(),
    });
    TargetPlan {
        plan_id: format!(
            "target-plan:{}",
            stable_hash(json!([
                TARGET_PLAN_VERSION,
                ir.ir_id,
                target_language_slug(input.target_language),
                input.idiom_level
            ]))
        ),
        version: TARGET_PLAN_VERSION.to_string(),
        target_language: input.target_language,
        target_project: input.target_project.clone(),
        module_plan: vec![TargetModulePlan {
            module_path,
            summary: format!(
                "Emit a {} scaffold for `{}`.",
                target_language_label(input.target_language),
                ir.purpose
            ),
            operations,
        }],
        dependency_substitutions: ir
            .feature
            .dependencies
            .iter()
            .map(|dependency| rustyred_thg_behavior_ir::DependencySubstitution {
                source_dependency: dependency.name.clone(),
                target_dependency: "manual-selection-required".to_string(),
                rationale:
                    "The first emitter slice records dependency pressure but does not pick packages."
                        .to_string(),
            })
            .collect(),
        idiom_level: input.idiom_level,
        validation_plan: ValidationPlan {
            commands: validation_commands(input.target_language),
            parity_tests: vec![test_path],
        },
        obligations,
    }
}

pub fn patch_set_from_behavior_ir(ir: &BehaviorIr, plan: &TargetPlan) -> PatchSet {
    let Some(module) = plan.module_plan.first() else {
        return PatchSet {
            patch_id: format!("patch-set:{}", stable_hash(json!([ir.ir_id, "empty"]))),
            version: PATCH_SET_VERSION.to_string(),
            status: PatchStatus::Failed,
            target_language: plan.target_language,
            files: Vec::new(),
            tests: Vec::new(),
            receipts: Vec::new(),
            unresolved_obligations: vec![TargetObligation {
                obligation_id: format!(
                    "target-obligation:{}",
                    stable_hash(json!([ir.ir_id, "no-module"]))
                ),
                summary: "Target plan did not include a module to emit.".to_string(),
                severity: "high".to_string(),
                evidence_ids: Vec::new(),
            }],
        };
    };
    let module_stem = module_stem(ir);
    let source = match plan.target_language {
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
            emit_typescript_scaffold(ir, plan, &module_stem)
        }
        TargetLanguage::Rust => emit_rust_scaffold(ir, plan, &module_stem),
        _ => emit_portability_note_scaffold(ir, plan, &module_stem),
    };
    let test = match plan.target_language {
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
            emit_typescript_test_scaffold(ir, &module_stem)
        }
        TargetLanguage::Rust => emit_rust_test_scaffold(ir, &module_stem),
        _ => emit_generic_test_note(ir, plan),
    };
    let tests = plan
        .validation_plan
        .parity_tests
        .first()
        .cloned()
        .map(|path| PatchFile {
            path,
            kind: PatchFileKind::Test,
            before: None,
            after: test,
        })
        .into_iter()
        .collect::<Vec<_>>();
    PatchSet {
        patch_id: format!(
            "patch-set:{}",
            stable_hash(json!([
                PATCH_SET_VERSION,
                ir.ir_id,
                plan.plan_id,
                module.module_path
            ]))
        ),
        version: PATCH_SET_VERSION.to_string(),
        status: if plan.obligations.is_empty() {
            PatchStatus::Ready
        } else {
            PatchStatus::NeedsReview
        },
        target_language: plan.target_language,
        files: vec![PatchFile {
            path: module.module_path.clone(),
            kind: PatchFileKind::Source,
            before: None,
            after: source,
        }],
        tests,
        receipts: Vec::new(),
        unresolved_obligations: plan.obligations.clone(),
    }
}

pub fn validate_patch_set(plan: &TargetPlan, patch_set: &PatchSet) -> PatchSet {
    let mut validated = patch_set.clone();
    validated.receipts = plan
        .validation_plan
        .commands
        .iter()
        .map(|command| ValidationReceipt {
            receipt_id: format!(
                "validation-receipt:{}",
                stable_hash(json!([patch_set.patch_id, command]))
            ),
            command: command.clone(),
            status: "not_run".to_string(),
            output_summary: "PatchSet was emitted as structuredContent but was not applied to a target checkout; run this command after applying the patch.".to_string(),
        })
        .collect();
    if !validated.receipts.is_empty() && matches!(validated.status, PatchStatus::Ready) {
        validated.status = PatchStatus::NeedsReview;
    }
    validated
}

pub fn port_patch_set_from_reconstruction_spec(
    spec: &ReconstructionSpec,
    slice_input: &FeatureSliceInput,
    target_input: &TargetPlanInput,
) -> PatchSet {
    let ir = behavior_ir_from_reconstruction_spec(spec, slice_input);
    let plan = target_plan_from_behavior_ir(&ir, target_input);
    validate_patch_set(&plan, &patch_set_from_behavior_ir(&ir, &plan))
}

fn empty_feature_slice(
    input: &FeatureSliceInput,
    source_ref: BehaviorSourceRef,
    tenant_id: String,
    repo_id: String,
    unknown: &str,
) -> FeatureSlice {
    FeatureSlice {
        slice_id: format!(
            "feature-slice:{}",
            stable_hash(json!([repo_id, input.seed]))
        ),
        version: FEATURE_SLICE_VERSION.to_string(),
        source_ref,
        tenant_id,
        repo_id,
        seed: input.seed.clone(),
        entry_symbols: Vec::new(),
        files: Vec::new(),
        tests: Vec::new(),
        docs: Vec::new(),
        runtime_examples: Vec::new(),
        dependencies: Vec::new(),
        evidence: Vec::new(),
        unknowns: vec![unknown.to_string()],
    }
}

fn behavior_source_ref(
    source: &SourceRef,
    repo_id: &str,
    sha: Option<String>,
) -> BehaviorSourceRef {
    BehaviorSourceRef {
        github_url: source.github_url.clone(),
        repo_url: source.repo_url.clone(),
        repo_id: source.repo_id.clone().or_else(|| Some(repo_id.to_string())),
        local_path: source.local_path.clone(),
        binary_path: source.binary_path.clone(),
        web_url: source.web_url.clone(),
        sha: source.sha.clone().or(sha),
    }
}

fn select_symbols(
    symbols: &[CodeSymbolSnapshot],
    input: &FeatureSliceInput,
    limit: usize,
) -> Vec<CodeSymbolSnapshot> {
    let requested = input
        .entry_symbols
        .iter()
        .map(|symbol| symbol.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let seed_terms = input.seed.as_deref().map(query_terms).unwrap_or_default();
    let mut selected = symbols
        .iter()
        .filter(|symbol| {
            if !requested.is_empty() {
                let haystack = format!(
                    "{} {} {} {}",
                    symbol.symbol_id, symbol.name, symbol.kind, symbol.file_path
                )
                .to_ascii_lowercase();
                return requested
                    .iter()
                    .any(|needle| haystack.contains(needle.as_str()));
            }
            if seed_terms.is_empty() {
                return true;
            }
            let haystack = format!(
                "{} {} {} {}",
                symbol.name,
                symbol.kind,
                symbol.file_path,
                symbol.signature.clone().unwrap_or_default()
            )
            .to_ascii_lowercase();
            seed_terms.iter().any(|term| haystack.contains(term))
        })
        .cloned()
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.symbol_id.cmp(&right.symbol_id))
    });
    selected.truncate(limit);
    selected
}

fn select_files(
    files: &[crate::CodeFileSnapshot],
    input: &FeatureSliceInput,
    symbols: &[CodeSymbolSnapshot],
    limit: usize,
) -> Vec<String> {
    let requested = input
        .files
        .iter()
        .map(|file| file.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut selected = BTreeSet::new();
    for symbol in symbols {
        selected.insert(symbol.file_path.clone());
    }
    if !requested.is_empty() {
        for file in files {
            let path = file.path.to_ascii_lowercase();
            if requested.iter().any(|needle| path.contains(needle)) {
                selected.insert(file.path.clone());
            }
        }
    }
    if selected.is_empty() {
        for file in files.iter().take(limit) {
            selected.insert(file.path.clone());
        }
    }
    selected.into_iter().take(limit).collect()
}

fn dependencies_from_symbols(symbols: &[CodeSymbolSnapshot]) -> Vec<DependencyRef> {
    let mut refs = BTreeMap::<String, DependencyRef>::new();
    for symbol in symbols {
        for name in &symbol.call_names {
            refs.entry(format!("call:{name}")).or_insert(DependencyRef {
                name: name.clone(),
                kind: "call".to_string(),
                source_id: Some(symbol.symbol_id.clone()),
            });
        }
        for name in &symbol.dependency_names {
            refs.entry(format!("dependency:{name}"))
                .or_insert(DependencyRef {
                    name: name.clone(),
                    kind: "symbol_dependency".to_string(),
                    source_id: Some(symbol.symbol_id.clone()),
                });
        }
    }
    refs.into_values().collect()
}

fn api_contract_from_symbol(symbol: &CodeSymbolSnapshot) -> ApiContract {
    ApiContract {
        name: symbol.name.clone(),
        kind: symbol.kind.clone(),
        file_path: Some(symbol.file_path.clone()),
        signature: symbol.signature.clone(),
        source_body: symbol.body.clone(),
        evidence_ids: vec![symbol.symbol_id.clone()],
    }
}

fn data_model_from_symbol(symbol: &CodeSymbolSnapshot) -> DataModelContract {
    DataModelContract {
        name: symbol.name.clone(),
        kind: symbol.kind.clone(),
        file_path: Some(symbol.file_path.clone()),
        fields: Vec::new(),
        evidence_ids: vec![symbol.symbol_id.clone()],
    }
}

fn operation_from_symbol(symbol: &CodeSymbolSnapshot) -> OperationContract {
    OperationContract {
        operation_id: format!(
            "operation:{}",
            stable_hash(json!([symbol.symbol_id, symbol.name, symbol.file_path]))
        ),
        name: symbol.name.clone(),
        kind: operation_kind(symbol),
        file_path: Some(symbol.file_path.clone()),
        dependencies: symbol
            .call_names
            .iter()
            .chain(symbol.dependency_names.iter())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        evidence_ids: vec![symbol.symbol_id.clone()],
    }
}

fn effects_from_symbol(symbol: &CodeSymbolSnapshot) -> Vec<EffectContract> {
    let deps = symbol
        .call_names
        .iter()
        .chain(symbol.dependency_names.iter())
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut effects = Vec::new();
    for (kind, needles) in [
        (
            "network",
            ["http", "request", "fetch", "socket", "url"].as_slice(),
        ),
        ("file", ["open", "read", "write", "path", "fs"].as_slice()),
        ("database", ["sql", "db", "database", "query"].as_slice()),
        (
            "subprocess",
            ["subprocess", "process", "spawn", "exec"].as_slice(),
        ),
    ] {
        if deps
            .iter()
            .any(|dep| needles.iter().any(|needle| dep.contains(needle)))
        {
            effects.push(EffectContract {
                effect_id: format!("effect:{}", stable_hash(json!([symbol.symbol_id, kind]))),
                kind: kind.to_string(),
                summary: format!("`{}` appears to touch a {kind} boundary.", symbol.name),
                evidence_ids: vec![symbol.symbol_id.clone()],
            });
        }
    }
    effects
}

fn portability_hazards_from_feature(feature: &FeatureSlice) -> Vec<PortabilityHazard> {
    feature
        .unknowns
        .iter()
        .map(|unknown| PortabilityHazard {
            hazard_id: format!("hazard:{}", stable_hash(json!([feature.slice_id, unknown]))),
            severity: "medium".to_string(),
            summary: unknown.clone(),
            evidence_ids: Vec::new(),
        })
        .collect()
}

fn operation_kind(symbol: &CodeSymbolSnapshot) -> OperationKind {
    let haystack = format!(
        "{} {} {}",
        symbol.name,
        symbol.kind,
        symbol.signature.clone().unwrap_or_default()
    )
    .to_ascii_lowercase();
    if haystack.contains("parse")
        || haystack.contains("serial")
        || haystack.contains("json")
        || haystack.contains("encode")
        || haystack.contains("decode")
    {
        OperationKind::ParserSerializer
    } else if haystack.contains("async") || haystack.contains("await") {
        OperationKind::AsyncBoundary
    } else if symbol.kind.eq_ignore_ascii_case("class") {
        OperationKind::StatefulObject
    } else if haystack.contains("http") || haystack.contains("request") {
        OperationKind::HttpBoundary
    } else {
        OperationKind::PureFunction
    }
}

fn is_data_model_kind(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "class" | "struct" | "enum" | "interface" | "type" | "dataclass"
    )
}

fn is_test_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path.contains("/test")
        || path.starts_with("test")
        || path.contains("_test.")
        || path.contains(".test.")
        || path.contains(".spec.")
}

fn is_doc_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path == "readme.md"
        || path.starts_with("docs/")
        || path.ends_with(".md")
        || path.ends_with(".rst")
}

fn query_terms(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .map(str::trim)
        .filter(|term| term.len() >= 3)
        .map(str::to_ascii_lowercase)
        .collect()
}

fn feature_slice_id(
    repo_id: &str,
    input: &FeatureSliceInput,
    symbols: &[CodeSymbolSnapshot],
    files: &[String],
) -> String {
    format!(
        "feature-slice:{}",
        stable_hash(json!([
            repo_id,
            input.seed,
            input.entry_symbols,
            input.files,
            symbols
                .iter()
                .map(|symbol| symbol.symbol_id.as_str())
                .collect::<Vec<_>>(),
            files
        ]))
    )
}

fn behavior_ir_id(feature: &FeatureSlice) -> String {
    format!(
        "behavior-ir:{}",
        stable_hash(json!([
            BEHAVIOR_IR_VERSION,
            feature.slice_id,
            feature.entry_symbols,
            feature.files
        ]))
    )
}

fn parse_target_language(language: &str) -> Result<TargetLanguage, String> {
    match language.trim().to_ascii_lowercase().as_str() {
        "python" | "py" => Ok(TargetLanguage::Python),
        "javascript" | "js" => Ok(TargetLanguage::JavaScript),
        "typescript" | "ts" => Ok(TargetLanguage::TypeScript),
        "rust" | "rs" => Ok(TargetLanguage::Rust),
        "java" => Ok(TargetLanguage::Java),
        "go" | "golang" => Ok(TargetLanguage::Go),
        "cpp" | "c++" => Ok(TargetLanguage::Cpp),
        "c" => Ok(TargetLanguage::C),
        "ruby" | "rb" => Ok(TargetLanguage::Ruby),
        "csharp" | "c#" | "cs" => Ok(TargetLanguage::CSharp),
        "other" => Ok(TargetLanguage::Other),
        other => Err(format!("unsupported target language `{other}`")),
    }
}

fn parse_idiom_level(level: &str) -> Result<IdiomLevel, String> {
    match level.trim().to_ascii_lowercase().as_str() {
        "faithful" | "preserve" | "preservation" => Ok(IdiomLevel::Faithful),
        "idiomatic" => Ok(IdiomLevel::Idiomatic),
        "framework_native" | "framework-native" | "native" => Ok(IdiomLevel::FrameworkNative),
        other => Err(format!("unsupported idiom level `{other}`")),
    }
}

fn target_language_slug(language: TargetLanguage) -> &'static str {
    match language {
        TargetLanguage::Python => "python",
        TargetLanguage::JavaScript => "javascript",
        TargetLanguage::TypeScript => "typescript",
        TargetLanguage::Rust => "rust",
        TargetLanguage::Java => "java",
        TargetLanguage::Go => "go",
        TargetLanguage::Cpp => "cpp",
        TargetLanguage::C => "c",
        TargetLanguage::Ruby => "ruby",
        TargetLanguage::CSharp => "csharp",
        TargetLanguage::Other => "other",
    }
}

fn target_language_label(language: TargetLanguage) -> &'static str {
    match language {
        TargetLanguage::Cpp => "C++",
        TargetLanguage::CSharp => "C#",
        _ => target_language_slug(language),
    }
}

fn module_stem(ir: &BehaviorIr) -> String {
    let raw = ir
        .feature
        .seed
        .as_deref()
        .or_else(|| ir.public_api.first().map(|api| api.name.as_str()))
        .unwrap_or("ported_feature");
    let slug = identifier_slug(raw);
    if slug.is_empty() {
        "ported_feature".to_string()
    } else {
        slug
    }
}

fn identifier_slug(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator && !slug.is_empty() {
            slug.push('_');
            last_was_separator = true;
        }
    }
    while slug.ends_with('_') {
        slug.pop();
    }
    if slug.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        slug.insert(0, '_');
    }
    slug
}

fn pascal_case(value: &str) -> String {
    let mut out = String::new();
    for segment in value.split('_').filter(|segment| !segment.is_empty()) {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.extend(chars);
        }
    }
    if out.is_empty() {
        "PortedFeature".to_string()
    } else {
        out
    }
}

fn target_module_path(language: TargetLanguage, module_stem: &str) -> String {
    match language {
        TargetLanguage::TypeScript => format!("src/{module_stem}.ts"),
        TargetLanguage::JavaScript => format!("src/{module_stem}.js"),
        TargetLanguage::Rust => format!("src/{module_stem}.rs"),
        TargetLanguage::Python => format!("{module_stem}.py"),
        TargetLanguage::Java => format!("src/main/java/{module_stem}.java"),
        TargetLanguage::Go => format!("{module_stem}.go"),
        TargetLanguage::Cpp => format!("src/{module_stem}.cpp"),
        TargetLanguage::C => format!("src/{module_stem}.c"),
        TargetLanguage::Ruby => format!("lib/{module_stem}.rb"),
        TargetLanguage::CSharp => format!("{module_stem}.cs"),
        TargetLanguage::Other => format!("{module_stem}.txt"),
    }
}

fn target_test_path(language: TargetLanguage, module_stem: &str) -> String {
    match language {
        TargetLanguage::TypeScript => format!("src/{module_stem}.test.ts"),
        TargetLanguage::JavaScript => format!("src/{module_stem}.test.js"),
        TargetLanguage::Rust => format!("tests/{module_stem}_parity.rs"),
        TargetLanguage::Python => format!("tests/test_{module_stem}.py"),
        TargetLanguage::Java => format!("src/test/java/{module_stem}Test.java"),
        TargetLanguage::Go => format!("{module_stem}_test.go"),
        TargetLanguage::Cpp => format!("tests/{module_stem}_test.cpp"),
        TargetLanguage::C => format!("tests/{module_stem}_test.c"),
        TargetLanguage::Ruby => format!("test/test_{module_stem}.rb"),
        TargetLanguage::CSharp => format!("{module_stem}Tests.cs"),
        TargetLanguage::Other => format!("{module_stem}_validation.txt"),
    }
}

fn validation_commands(language: TargetLanguage) -> Vec<String> {
    match language {
        TargetLanguage::TypeScript => vec!["npx tsc --noEmit".to_string()],
        TargetLanguage::JavaScript => vec!["node --test".to_string()],
        TargetLanguage::Rust => vec!["cargo test".to_string()],
        TargetLanguage::Python => vec!["python -m pytest".to_string()],
        TargetLanguage::Java => vec!["./gradlew test".to_string()],
        TargetLanguage::Go => vec!["go test ./...".to_string()],
        TargetLanguage::Cpp | TargetLanguage::C => vec!["ctest".to_string()],
        TargetLanguage::Ruby => vec!["bundle exec ruby -Itest".to_string()],
        TargetLanguage::CSharp => vec!["dotnet test".to_string()],
        TargetLanguage::Other => Vec::new(),
    }
}

fn target_obligations_from_ir(ir: &BehaviorIr) -> Vec<TargetObligation> {
    let unknowns = ir.feature.unknowns.iter().map(|unknown| TargetObligation {
        obligation_id: format!(
            "target-obligation:{}",
            stable_hash(json!([ir.ir_id, unknown]))
        ),
        summary: unknown.clone(),
        severity: "medium".to_string(),
        evidence_ids: Vec::new(),
    });
    let hazards = ir
        .portability_hazards
        .iter()
        .map(|hazard| TargetObligation {
            obligation_id: format!(
                "target-obligation:{}",
                stable_hash(json!([ir.ir_id, hazard.hazard_id]))
            ),
            summary: hazard.summary.clone(),
            severity: hazard.severity.clone(),
            evidence_ids: hazard.evidence_ids.clone(),
        });
    unknowns.chain(hazards).collect()
}

fn emit_typescript_scaffold(ir: &BehaviorIr, plan: &TargetPlan, module_stem: &str) -> String {
    let export_name = format!("{module_stem}PortMetadata");
    let api_stubs = emit_typescript_api_stubs(ir);
    format!(
        "export type PortStatus = \"ready\" | \"needs_review\" | \"failed\";\n\
\n\
export interface PortOperationStub {{\n\
  name: string;\n\
  kind: string;\n\
  sourceSignature: string | null;\n\
  evidenceIds: string[];\n\
}}\n\
\n\
export interface PortMetadata {{\n\
  irId: string;\n\
  purpose: string;\n\
  sourceFiles: string[];\n\
  publicApi: string[];\n\
  operations: string[];\n\
  unresolvedObligations: string[];\n\
}}\n\
\n\
export const {export_name}: PortMetadata = {{\n\
  irId: {ir_id},\n\
  purpose: {purpose},\n\
  sourceFiles: {source_files},\n\
  publicApi: {public_api},\n\
  operations: {operations},\n\
  unresolvedObligations: {obligations},\n\
}};\n\
\n\
export const operationStubs: PortOperationStub[] = {operation_stubs};\n\
\n\
export function describe{pascal}Port(): PortMetadata {{\n\
  return {export_name};\n\
}}\n\
\n\
{api_stubs}",
        ir_id = json_string(&ir.ir_id),
        purpose = json_string(&ir.purpose),
        source_files = json_string(&ir.feature.files),
        public_api = json_string(
            &ir.public_api
                .iter()
                .map(|api| api.name.clone())
                .collect::<Vec<_>>()
        ),
        operations = json_string(
            &ir.operations
                .iter()
                .map(|operation| operation.name.clone())
                .collect::<Vec<_>>()
        ),
        obligations = json_string(
            &plan
                .obligations
                .iter()
                .map(|obligation| obligation.summary.clone())
                .collect::<Vec<_>>()
        ),
        operation_stubs = typescript_operation_stubs(ir),
        pascal = pascal_case(module_stem),
        api_stubs = api_stubs,
    )
}

fn emit_typescript_test_scaffold(ir: &BehaviorIr, module_stem: &str) -> String {
    let examples = semantic_examples_for_ir(ir, typescript_identifier, "portedOperation", false);
    let mut imports = BTreeSet::new();
    imports.insert(format!("describe{}Port", pascal_case(module_stem)));
    imports.extend(examples.iter().map(|example| example.function_name.clone()));
    let import_names = imports.into_iter().collect::<Vec<_>>().join(", ");
    let example_tests = render_typescript_example_tests(&examples);
    format!(
        "import {{ strict as assert }} from \"node:assert\";\n\
import {{ describe, it }} from \"node:test\";\n\
import {{ {import_names} }} from \"./{module_stem}\";\n\
\n\
describe({name}, () => {{\n\
  it(\"preserves the selected source evidence boundary\", () => {{\n\
    const metadata = describe{pascal}Port();\n\
    assert.equal(metadata.irId, {ir_id});\n\
    assert.ok(metadata.sourceFiles.length >= 0);\n\
    assert.ok(metadata.unresolvedObligations.length >= 0);\n\
  }});\n\
{example_tests}\
}});\n",
        import_names = import_names,
        pascal = pascal_case(module_stem),
        name = json_string(&format!("{module_stem} port metadata")),
        ir_id = json_string(&ir.ir_id),
        example_tests = example_tests,
    )
}

fn emit_rust_scaffold(ir: &BehaviorIr, plan: &TargetPlan, module_stem: &str) -> String {
    let function_name = format!("{module_stem}_port_metadata");
    let api_stubs = emit_rust_api_stubs(ir);
    format!(
        "#[derive(Debug, Clone, PartialEq, Eq)]\n\
pub struct PortMetadata {{\n\
    pub ir_id: &'static str,\n\
    pub purpose: &'static str,\n\
    pub source_files: &'static [&'static str],\n\
    pub public_api: &'static [&'static str],\n\
    pub operations: &'static [&'static str],\n\
    pub unresolved_obligations: &'static [&'static str],\n\
}}\n\
\n\
#[derive(Debug, Clone, PartialEq, Eq)]\n\
pub struct PortOperationStub {{\n\
    pub name: &'static str,\n\
    pub kind: &'static str,\n\
    pub source_signature: Option<&'static str>,\n\
    pub evidence_ids: &'static [&'static str],\n\
}}\n\
\n\
#[derive(Debug, Clone, PartialEq, Eq)]\n\
pub struct PortError {{\n\
    pub operation: &'static str,\n\
    pub message: &'static str,\n\
}}\n\
\n\
impl PortError {{\n\
    pub const fn unimplemented(operation: &'static str) -> Self {{\n\
        Self {{\n\
            operation,\n\
            message: \"generated port operation requires behavior implementation\",\n\
        }}\n\
    }}\n\
}}\n\
\n\
impl std::fmt::Display for PortError {{\n\
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n\
        write!(formatter, \"{{}}: {{}}\", self.operation, self.message)\n\
    }}\n\
}}\n\
\n\
impl std::error::Error for PortError {{}}\n\
\n\
pub const OPERATION_STUBS: &[PortOperationStub] = {operation_stubs};\n\
\n\
pub fn {function_name}() -> PortMetadata {{\n\
    PortMetadata {{\n\
        ir_id: {ir_id},\n\
        purpose: {purpose},\n\
        source_files: {source_files},\n\
        public_api: {public_api},\n\
        operations: {operations},\n\
        unresolved_obligations: {obligations},\n\
    }}\n\
}}\n\
\n\
{api_stubs}\n\
\n\
#[cfg(test)]\n\
mod tests {{\n\
    use super::*;\n\
\n\
    #[test]\n\
    fn metadata_preserves_evidence_boundary() {{\n\
        let metadata = {function_name}();\n\
        assert_eq!(metadata.ir_id, {ir_id});\n\
    }}\n\
}}\n",
        ir_id = rust_string(&ir.ir_id),
        purpose = rust_string(&ir.purpose),
        source_files = rust_string_slice(&ir.feature.files),
        public_api = rust_string_slice(
            &ir.public_api
                .iter()
                .map(|api| api.name.clone())
                .collect::<Vec<_>>()
        ),
        operations = rust_string_slice(
            &ir.operations
                .iter()
                .map(|operation| operation.name.clone())
                .collect::<Vec<_>>()
        ),
        obligations = rust_string_slice(
            &plan
                .obligations
                .iter()
                .map(|obligation| obligation.summary.clone())
                .collect::<Vec<_>>()
        ),
        operation_stubs = rust_operation_stubs(ir),
        api_stubs = api_stubs,
    )
}

fn emit_rust_test_scaffold(ir: &BehaviorIr, module_stem: &str) -> String {
    let examples = semantic_examples_for_ir(ir, rust_identifier, "ported_operation", true);
    let example_tests = render_rust_example_tests(&examples);
    let metadata_fn = format!("{module_stem}_port_metadata");
    format!(
        "#[path = \"../src/{module_stem}.rs\"]\n\
mod generated_port;\n\
\n\
#[test]\n\
fn generated_port_patch_records_review_obligations() {{\n\
    let metadata = generated_port::{metadata_fn}();\n\
    assert_eq!(metadata.ir_id, {ir_id});\n\
}}\n",
        module_stem = module_stem,
        metadata_fn = metadata_fn,
        ir_id = rust_string(&ir.ir_id),
    ) + &example_tests
}

fn render_typescript_example_tests(examples: &[SemanticExample]) -> String {
    if examples.is_empty() {
        return String::new();
    }
    examples
        .iter()
        .enumerate()
        .map(|(index, example)| {
            format!(
                "\n  it({name}, () => {{\n    assert.equal({function_name}({args}), {expected});\n  }});\n",
                name = json_string(&format!(
                    "translates {} semantic example {}",
                    example.source_name,
                    index + 1
                )),
                function_name = example.function_name,
                args = example
                    .args
                    .iter()
                    .map(|value| format_number_literal(*value, false))
                    .collect::<Vec<_>>()
                    .join(", "),
                expected = format_number_literal(example.expected, false),
            )
        })
        .collect::<String>()
}

fn render_rust_example_tests(examples: &[SemanticExample]) -> String {
    if examples.is_empty() {
        return String::new();
    }
    let asserts = examples
        .iter()
        .map(|example| {
            format!(
                "    assert_eq!(generated_port::{function_name}({args}).unwrap(), {expected});\n",
                function_name = example.function_name,
                args = example
                    .args
                    .iter()
                    .map(|value| format_number_literal(*value, true))
                    .collect::<Vec<_>>()
                    .join(", "),
                expected = format_number_literal(example.expected, true),
            )
        })
        .collect::<String>();
    format!(
        "\n#[test]\nfn translated_numeric_examples_match_source_semantics() {{\n{asserts}}}\n",
        asserts = asserts
    )
}

#[derive(Clone, Debug, PartialEq)]
struct SemanticExample {
    source_name: String,
    function_name: String,
    args: Vec<f64>,
    expected: f64,
}

fn semantic_examples_for_ir(
    ir: &BehaviorIr,
    mapper: fn(&str, &str) -> String,
    function_fallback: &str,
    rust_numeric_literals: bool,
) -> Vec<SemanticExample> {
    ir.public_api
        .iter()
        .filter(|api| !api_is_data_model(api))
        .flat_map(|api| {
            semantic_examples_for_api(api, mapper, function_fallback, rust_numeric_literals)
        })
        .collect()
}

fn semantic_examples_for_api(
    api: &ApiContract,
    mapper: fn(&str, &str) -> String,
    function_fallback: &str,
    rust_numeric_literals: bool,
) -> Vec<SemanticExample> {
    let body = match translate_simple_numeric_body(api, mapper, rust_numeric_literals) {
        Some(body) => body,
        None => return Vec::new(),
    };
    let param_names = unique_target_params(api, mapper);
    let target_paths = semantic_path_count(&body);
    let mut examples = BTreeMap::<usize, SemanticExample>::new();
    for args in candidate_arg_sets(param_names.len()) {
        let env = param_names
            .iter()
            .cloned()
            .zip(args.iter().copied())
            .collect::<BTreeMap<_, _>>();
        let Some((path, expected)) = evaluate_numeric_body(&body, &env) else {
            continue;
        };
        examples.entry(path).or_insert_with(|| SemanticExample {
            source_name: api.name.clone(),
            function_name: mapper(&api.name, function_fallback),
            args,
            expected,
        });
        if examples.len() >= target_paths {
            break;
        }
    }
    examples.into_values().collect()
}

fn semantic_path_count(body: &SimpleNumericBody) -> usize {
    match body {
        SimpleNumericBody::GuardedReturns { guards, .. } => guards.len() + 1,
        SimpleNumericBody::Return(_) | SimpleNumericBody::LocalAssignments { .. } => 1,
    }
}

fn candidate_arg_sets(count: usize) -> Vec<Vec<f64>> {
    if count == 0 {
        return vec![Vec::new()];
    }
    let mut sets = vec![
        vec![0.0; count],
        vec![1.0; count],
        vec![2.0; count],
        vec![-1.0; count],
    ];
    for index in 0..count {
        for value in [-2.0, -1.0, 0.0, 1.0, 2.0, 5.0, 10.0] {
            let mut args = vec![1.0; count];
            args[index] = value;
            sets.push(args);
        }
    }
    let mut seen = BTreeSet::new();
    sets.into_iter()
        .filter(|args| {
            let key = args
                .iter()
                .map(|value| format_number_literal(*value, true))
                .collect::<Vec<_>>()
                .join(",");
            seen.insert(key)
        })
        .collect()
}

fn evaluate_numeric_body(
    body: &SimpleNumericBody,
    env: &BTreeMap<String, f64>,
) -> Option<(usize, f64)> {
    match body {
        SimpleNumericBody::Return(expr) => eval_numeric_expr(expr, env).map(|value| (0, value)),
        SimpleNumericBody::LocalAssignments {
            assignments,
            result,
        } => {
            let mut env = env.clone();
            for assignment in assignments {
                let value = eval_numeric_expr(&assignment.expression, &env)?;
                env.insert(assignment.target.clone(), value);
            }
            eval_numeric_expr(result, &env).map(|value| (0, value))
        }
        SimpleNumericBody::GuardedReturns { guards, fallback } => {
            for (index, guard) in guards.iter().enumerate() {
                if eval_numeric_condition(&guard.condition, env)? {
                    return eval_numeric_expr(&guard.expression, env).map(|value| (index, value));
                }
            }
            eval_numeric_expr(fallback, env).map(|value| (guards.len(), value))
        }
    }
}

fn eval_numeric_condition(condition: &str, env: &BTreeMap<String, f64>) -> Option<bool> {
    let condition = strip_balanced_outer_parens(condition.trim());
    if let Some(parts) = split_top_level_bool(condition, " || ") {
        let mut any = false;
        for part in parts {
            any |= eval_numeric_condition(part, env)?;
        }
        return Some(any);
    }
    if let Some(parts) = split_top_level_bool(condition, " or ") {
        let mut any = false;
        for part in parts {
            any |= eval_numeric_condition(part, env)?;
        }
        return Some(any);
    }
    if let Some(parts) = split_top_level_bool(condition, " && ") {
        let mut all = true;
        for part in parts {
            all &= eval_numeric_condition(part, env)?;
        }
        return Some(all);
    }
    if let Some(parts) = split_top_level_bool(condition, " and ") {
        let mut all = true;
        for part in parts {
            all &= eval_numeric_condition(part, env)?;
        }
        return Some(all);
    }
    let (left, operator, right) = split_numeric_comparison(condition)?;
    let left = eval_numeric_expr(left, env)?;
    let right = eval_numeric_expr(right, env)?;
    Some(match operator {
        "<" => left < right,
        "<=" => left <= right,
        ">" => left > right,
        ">=" => left >= right,
        "==" => (left - right).abs() < f64::EPSILON,
        "!=" => (left - right).abs() >= f64::EPSILON,
        _ => return None,
    })
}

fn eval_numeric_expr(expr: &str, env: &BTreeMap<String, f64>) -> Option<f64> {
    let mut parser = NumericExprParser {
        input: expr,
        offset: 0,
        env,
    };
    let value = parser.parse_expr()?;
    parser.skip_ws();
    (parser.offset == expr.len()).then_some(value)
}

struct NumericExprParser<'a> {
    input: &'a str,
    offset: usize,
    env: &'a BTreeMap<String, f64>,
}

impl<'a> NumericExprParser<'a> {
    fn parse_expr(&mut self) -> Option<f64> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_ws();
            if self.consume('+') {
                value += self.parse_term()?;
            } else if self.consume('-') {
                value -= self.parse_term()?;
            } else {
                return Some(value);
            }
        }
    }

    fn parse_term(&mut self) -> Option<f64> {
        let mut value = self.parse_factor()?;
        loop {
            self.skip_ws();
            if self.consume('*') {
                value *= self.parse_factor()?;
            } else if self.consume('/') {
                value /= self.parse_factor()?;
            } else if self.consume('%') {
                value %= self.parse_factor()?;
            } else {
                return Some(value);
            }
        }
    }

    fn parse_factor(&mut self) -> Option<f64> {
        self.skip_ws();
        if self.consume('+') {
            return self.parse_factor();
        }
        if self.consume('-') {
            return self.parse_factor().map(|value| -value);
        }
        if self.consume('(') {
            let value = self.parse_expr()?;
            self.skip_ws();
            self.consume(')').then_some(value)
        } else if self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            self.parse_number()
        } else if self.peek().is_some_and(is_identifier_start) {
            self.parse_identifier()
                .and_then(|identifier| self.env.get(identifier).copied())
        } else {
            None
        }
    }

    fn parse_number(&mut self) -> Option<f64> {
        let start = self.offset;
        let mut has_dot = false;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                self.offset += ch.len_utf8();
            } else if ch == '.' && !has_dot {
                has_dot = true;
                self.offset += ch.len_utf8();
            } else {
                break;
            }
        }
        self.input[start..self.offset].parse::<f64>().ok()
    }

    fn parse_identifier(&mut self) -> Option<&'a str> {
        let start = self.offset;
        let first = self.peek()?;
        if !is_identifier_start(first) {
            return None;
        }
        self.offset += first.len_utf8();
        while let Some(ch) = self.peek() {
            if is_identifier_continue(ch) {
                self.offset += ch.len_utf8();
            } else {
                break;
            }
        }
        Some(&self.input[start..self.offset])
    }

    fn skip_ws(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_ascii_whitespace() {
                self.offset += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    fn consume(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.offset += expected.len_utf8();
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.offset..].chars().next()
    }
}

fn format_number_literal(value: f64, rust: bool) -> String {
    let value = if value == 0.0 { 0.0 } else { value };
    if rust {
        if value.fract() == 0.0 {
            format!("{value:.1}")
        } else {
            value.to_string()
        }
    } else if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn emit_portability_note_scaffold(ir: &BehaviorIr, plan: &TargetPlan, module_stem: &str) -> String {
    format!(
        "Port scaffold for {language}\n\
==============================\n\
\n\
IR: {ir_id}\n\
Purpose: {purpose}\n\
Module: {module_stem}\n\
\n\
Source files:\n{files}\n\
\n\
Operations:\n{operations}\n\
\n\
This target language does not yet have a dedicated emitter crate. Use this\n\
structured note as the patch artifact until a language adapter exists.\n",
        language = target_language_label(plan.target_language),
        ir_id = ir.ir_id,
        purpose = ir.purpose,
        files = bullet_list(&ir.feature.files),
        operations = bullet_list(
            &ir.operations
                .iter()
                .map(|operation| operation.name.clone())
                .collect::<Vec<_>>()
        ),
    )
}

fn emit_generic_test_note(ir: &BehaviorIr, plan: &TargetPlan) -> String {
    format!(
        "Validation plan for {language}\n\
==============================\n\
\n\
IR: {ir_id}\n\
\n\
Commands to run after applying the patch:\n{commands}\n",
        language = target_language_label(plan.target_language),
        ir_id = ir.ir_id,
        commands = bullet_list(&plan.validation_plan.commands),
    )
}

fn typescript_operation_stubs(ir: &BehaviorIr) -> String {
    if ir.public_api.is_empty() {
        return "[]".to_string();
    }
    let entries = ir
        .public_api
        .iter()
        .map(|api| {
            format!(
                "  {{ name: {name}, kind: {kind}, sourceSignature: {signature}, evidenceIds: {evidence_ids} }}",
                name = json_string(&api.name),
                kind = json_string(&api.kind),
                signature = api
                    .signature
                    .as_ref()
                    .map(json_string)
                    .unwrap_or_else(|| "null".to_string()),
                evidence_ids = json_string(&api.evidence_ids),
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!("[\n{entries}\n]")
}

fn rust_operation_stubs(ir: &BehaviorIr) -> String {
    if ir.public_api.is_empty() {
        return "&[]".to_string();
    }
    let entries = ir
        .public_api
        .iter()
        .map(|api| {
            format!(
                "    PortOperationStub {{ name: {name}, kind: {kind}, source_signature: {signature}, evidence_ids: {evidence_ids} }}",
                name = rust_string(&api.name),
                kind = rust_string(&api.kind),
                signature = api
                    .signature
                    .as_ref()
                    .map(|signature| format!("Some({})", rust_string(signature)))
                    .unwrap_or_else(|| "None".to_string()),
                evidence_ids = rust_string_slice(&api.evidence_ids),
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!("&[\n{entries},\n]")
}

fn emit_typescript_api_stubs(ir: &BehaviorIr) -> String {
    let stubs = ir
        .public_api
        .iter()
        .map(|api| {
            if api_is_data_model(api) {
                emit_typescript_model_stub(api)
            } else {
                emit_typescript_function_stub(ir, api)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if stubs.trim().is_empty() {
        "// No public API contracts were selected for this feature slice.\n".to_string()
    } else {
        stubs
    }
}

fn emit_typescript_model_stub(api: &ApiContract) -> String {
    format!(
        "/** Source {kind}: {name}. */\n\
export interface {type_name} {{\n\
  readonly __sourceName: {source_name};\n\
  readonly __sourceKind: {source_kind};\n\
}}\n",
        kind = api.kind,
        name = api.name,
        type_name = port_type_name(api),
        source_name = json_string(&api.name),
        source_kind = json_string(&api.kind),
    )
}

fn emit_typescript_function_stub(ir: &BehaviorIr, api: &ApiContract) -> String {
    let function_name = typescript_identifier(&api.name, "portedOperation");
    if let Some(body) = translate_simple_numeric_body(api, typescript_identifier, false) {
        let params = typed_param_list(api, typescript_identifier, "number");
        let body = render_typescript_numeric_body(&body);
        return format!(
            "/**\n\
 * Source signature: {signature}\n\
 * Evidence: {evidence_ids}\n\
 * Semantics: translated from a simple numeric source body.\n\
 */\n\
export function {function_name}({params}): number {{\n\
{body}\
}}\n",
            signature = api.signature.as_deref().unwrap_or("unknown"),
            evidence_ids = api.evidence_ids.join(", "),
            function_name = function_name,
            params = params,
            body = body,
        );
    }
    let params = typed_param_list(api, typescript_identifier, "unknown");
    let message = format!(
        "Port operation `{}` from BehaviorIr `{}` requires implementation before use.",
        api.name, ir.ir_id
    );
    format!(
        "/**\n\
 * Source signature: {signature}\n\
 * Evidence: {evidence_ids}\n\
 */\n\
export function {function_name}({params}): never {{\n\
  throw new Error({message});\n\
}}\n",
        signature = api.signature.as_deref().unwrap_or("unknown"),
        evidence_ids = api.evidence_ids.join(", "),
        function_name = function_name,
        params = params,
        message = json_string(&message),
    )
}

fn emit_rust_api_stubs(ir: &BehaviorIr) -> String {
    let stubs = ir
        .public_api
        .iter()
        .map(|api| {
            if api_is_data_model(api) {
                emit_rust_model_stub(api)
            } else {
                emit_rust_function_stub(api)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if stubs.trim().is_empty() {
        "// No public API contracts were selected for this feature slice.\n".to_string()
    } else {
        format!(
            "// Generated from BehaviorIr {}; simple proven operations may be translated, unresolved operations return PortError.\n{}",
            rust_string(&ir.ir_id),
            stubs
        )
    }
}

fn emit_rust_model_stub(api: &ApiContract) -> String {
    format!(
        "/// Source {kind}: {name}.\n\
#[derive(Debug, Clone, PartialEq, Eq, Default)]\n\
pub struct {type_name} {{}}\n",
        kind = api.kind,
        name = api.name,
        type_name = port_type_name(api),
    )
}

fn emit_rust_function_stub(api: &ApiContract) -> String {
    let function_name = rust_identifier(&api.name, "ported_operation");
    if let Some(body) = translate_simple_numeric_body(api, rust_identifier, true) {
        let params = typed_param_list(api, rust_identifier, "f64");
        let body = render_rust_numeric_body(&body);
        return format!(
            "/// Source signature: {signature}\n\
/// Semantics: translated from a simple numeric source body.\n\
pub fn {function_name}({params}) -> Result<f64, PortError> {{\n\
{body}\
}}\n",
            signature = api.signature.as_deref().unwrap_or("unknown"),
            function_name = function_name,
            params = params,
            body = body,
        );
    }
    let params = rust_stub_param_list(api);
    format!(
        "/// Source signature: {signature}\n\
pub fn {function_name}({params}) -> Result<(), PortError> {{\n\
    Err(PortError::unimplemented({operation}))\n\
}}\n",
        signature = api.signature.as_deref().unwrap_or("unknown"),
        function_name = function_name,
        params = params,
        operation = rust_string(&api.name),
    )
}

fn typed_param_list(api: &ApiContract, mapper: fn(&str, &str) -> String, ty: &str) -> String {
    unique_target_params(api, mapper)
        .into_iter()
        .map(|param| format!("{param}: {ty}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn rust_stub_param_list(api: &ApiContract) -> String {
    unique_target_params(api, rust_identifier)
        .into_iter()
        .map(|param| format!("_{param}: impl std::fmt::Debug"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SimpleNumericBody {
    Return(String),
    LocalAssignments {
        assignments: Vec<NumericAssignment>,
        result: String,
    },
    GuardedReturns {
        guards: Vec<GuardedNumericReturn>,
        fallback: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GuardedNumericReturn {
    condition: String,
    expression: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NumericAssignment {
    target: String,
    expression: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SemanticLine<'a> {
    indent: usize,
    text: &'a str,
}

fn translate_simple_numeric_body(
    api: &ApiContract,
    mapper: fn(&str, &str) -> String,
    rust_numeric_literals: bool,
) -> Option<SimpleNumericBody> {
    let body = api.source_body.as_deref()?;
    let param_map = mapped_param_names(api, mapper);
    let lines = semantic_body_lines(body);
    translate_guarded_numeric_returns(&lines, &param_map, rust_numeric_literals)
        .or_else(|| translate_assignment_numeric_return(&lines, &param_map, mapper, rust_numeric_literals))
        .or_else(|| translate_unconditional_numeric_return(&lines, &param_map, rust_numeric_literals))
}

fn mapped_param_names(
    api: &ApiContract,
    mapper: fn(&str, &str) -> String,
) -> BTreeMap<String, String> {
    signature_param_names(api.signature.as_deref())
        .into_iter()
        .filter(|source| !matches!(source.as_str(), "self" | "cls" | "this"))
        .map(|source| {
            let target = mapper(&source, "arg");
            (source, target)
        })
        .collect()
}

fn semantic_body_lines(body: &str) -> Vec<SemanticLine<'_>> {
    body.lines()
        .filter_map(|line| {
            let text = line.trim();
            let text = text.strip_prefix('}').map(str::trim_start).unwrap_or(text);
            if text.is_empty()
                || text.starts_with('#')
                || text.starts_with("//")
                || text == "{"
                || text == "}"
                || text == "};"
                || is_function_header_line(text)
            {
                return None;
            }
            Some(SemanticLine {
                indent: leading_indent_width(line),
                text,
            })
        })
        .collect()
}

fn is_function_header_line(text: &str) -> bool {
    (text.starts_with("def ") && text.ends_with(':'))
        || ((text.starts_with("fn ") || text.starts_with("pub fn "))
            && text.contains('(')
            && (text.ends_with('{') || text.contains("->")))
}

fn leading_indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|ch| ch.is_whitespace())
        .map(|ch| if ch == '\t' { 4 } else { 1 })
        .sum()
}

fn translate_unconditional_numeric_return(
    lines: &[SemanticLine<'_>],
    param_map: &BTreeMap<String, String>,
    rust_numeric_literals: bool,
) -> Option<SimpleNumericBody> {
    let [line] = lines else {
        return None;
    };
    let expr = strip_return_expr(line.text)?;
    translate_numeric_expr(expr, param_map, rust_numeric_literals).map(SimpleNumericBody::Return)
}

fn translate_guarded_numeric_returns(
    lines: &[SemanticLine<'_>],
    param_map: &BTreeMap<String, String>,
    rust_numeric_literals: bool,
) -> Option<SimpleNumericBody> {
    let mut index = 0usize;
    let mut guards = Vec::new();
    while index + 1 < lines.len() {
        let Some(condition) = strip_guard_condition(lines[index].text) else {
            break;
        };
        let return_line = lines[index + 1];
        if return_line.indent <= lines[index].indent {
            return None;
        }
        let expression = strip_return_expr(return_line.text)?;
        guards.push(GuardedNumericReturn {
            condition: translate_numeric_condition(condition, param_map, rust_numeric_literals)?,
            expression: translate_numeric_expr(expression, param_map, rust_numeric_literals)?,
        });
        index += 2;
    }
    if guards.is_empty() || index >= lines.len() {
        return None;
    }
    let fallback = if strip_else_line(lines[index].text).is_some() {
        if index + 2 != lines.len() || lines[index + 1].indent <= lines[index].indent {
            return None;
        }
        strip_return_expr(lines[index + 1].text)?
    } else {
        if index + 1 != lines.len() {
            return None;
        }
        strip_return_expr(lines[index].text)?
    };
    Some(SimpleNumericBody::GuardedReturns {
        guards,
        fallback: translate_numeric_expr(fallback, param_map, rust_numeric_literals)?,
    })
}

fn translate_assignment_numeric_return(
    lines: &[SemanticLine<'_>],
    param_map: &BTreeMap<String, String>,
    mapper: fn(&str, &str) -> String,
    rust_numeric_literals: bool,
) -> Option<SimpleNumericBody> {
    if lines.len() < 2 {
        return None;
    }
    let mut env = param_map.clone();
    let mut assignments = Vec::new();
    for line in &lines[..lines.len() - 1] {
        let (source_target, source_expr) = strip_assignment(line.text)?;
        if env.contains_key(source_target) {
            return None;
        }
        let target = mapper(source_target, "local");
        let expression = translate_numeric_expr(source_expr, &env, rust_numeric_literals)?;
        env.insert(source_target.to_string(), target.clone());
        assignments.push(NumericAssignment { target, expression });
    }
    if assignments.is_empty() {
        return None;
    }
    let result = strip_return_expr(lines[lines.len() - 1].text)?;
    Some(SimpleNumericBody::LocalAssignments {
        assignments,
        result: translate_numeric_expr(result, &env, rust_numeric_literals)?,
    })
}

fn strip_return_expr(line: &str) -> Option<&str> {
    let expr = line
        .trim()
        .strip_prefix("return ")?
        .trim()
        .trim_end_matches(';')
        .trim();
    (!expr.is_empty()).then_some(expr)
}

fn strip_assignment(line: &str) -> Option<(&str, &str)> {
    let line = line.trim().trim_end_matches(';').trim();
    if line.contains("==") || line.contains("!=") || line.contains("<=") || line.contains(">=") {
        return None;
    }
    let (left, right) = line.split_once('=')?;
    let left = left.trim();
    let right = right.trim();
    if !is_identifier_token(left) || right.is_empty() {
        return None;
    }
    Some((left, right))
}

fn strip_guard_condition(line: &str) -> Option<&str> {
    let text = line.trim();
    let condition = text
        .strip_prefix("if ")
        .or_else(|| text.strip_prefix("elif "))?
        .trim()
        .trim_end_matches(':')
        .trim_end_matches('{')
        .trim();
    (!condition.is_empty()).then_some(condition)
}

fn strip_else_line(line: &str) -> Option<()> {
    let text = line.trim();
    matches!(text, "else:" | "else {").then_some(())
}

fn translate_numeric_condition(
    condition: &str,
    param_map: &BTreeMap<String, String>,
    rust_numeric_literals: bool,
) -> Option<String> {
    let condition = strip_balanced_outer_parens(condition.trim());
    if let Some(parts) = split_top_level_bool(condition, " or ") {
        return parts
            .into_iter()
            .map(|part| translate_numeric_condition(part, param_map, rust_numeric_literals))
            .collect::<Option<Vec<_>>>()
            .map(|parts| parts.join(" || "));
    }
    if let Some(parts) = split_top_level_bool(condition, " || ") {
        return parts
            .into_iter()
            .map(|part| translate_numeric_condition(part, param_map, rust_numeric_literals))
            .collect::<Option<Vec<_>>>()
            .map(|parts| parts.join(" || "));
    }
    if let Some(parts) = split_top_level_bool(condition, " and ") {
        return parts
            .into_iter()
            .map(|part| translate_numeric_condition(part, param_map, rust_numeric_literals))
            .collect::<Option<Vec<_>>>()
            .map(|parts| parts.join(" && "));
    }
    if let Some(parts) = split_top_level_bool(condition, " && ") {
        return parts
            .into_iter()
            .map(|part| translate_numeric_condition(part, param_map, rust_numeric_literals))
            .collect::<Option<Vec<_>>>()
            .map(|parts| parts.join(" && "));
    }
    let (left, operator, right) = split_numeric_comparison(condition)?;
    Some(format!(
        "{} {operator} {}",
        translate_numeric_expr(left.trim(), param_map, rust_numeric_literals)?,
        translate_numeric_expr(right.trim(), param_map, rust_numeric_literals)?
    ))
}

fn strip_balanced_outer_parens(mut value: &str) -> &str {
    loop {
        let trimmed = value.trim();
        if !(trimmed.starts_with('(') && trimmed.ends_with(')')) {
            return trimmed;
        }
        let mut depth = 0usize;
        let mut wraps = true;
        for (idx, ch) in trimmed.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth = match depth.checked_sub(1) {
                        Some(depth) => depth,
                        None => return trimmed,
                    };
                    if depth == 0 && idx + ch.len_utf8() < trimmed.len() {
                        wraps = false;
                        break;
                    }
                }
                _ => {}
            }
        }
        if !wraps || depth != 0 {
            return trimmed;
        }
        value = &trimmed[1..trimmed.len() - 1];
    }
}

fn split_top_level_bool<'a>(condition: &'a str, separator: &str) -> Option<Vec<&'a str>> {
    let mut parts = Vec::new();
    let mut paren_depth = 0usize;
    let mut start = 0usize;
    let mut index = 0usize;
    while index < condition.len() {
        let ch = condition[index..].chars().next()?;
        match ch {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.checked_sub(1)?,
            _ => {}
        }
        if paren_depth == 0 && condition[index..].starts_with(separator) {
            parts.push(condition[start..index].trim());
            index += separator.len();
            start = index;
            continue;
        }
        index += ch.len_utf8();
    }
    if parts.is_empty() {
        return None;
    }
    parts.push(condition[start..].trim());
    parts.iter().all(|part| !part.is_empty()).then_some(parts)
}

fn split_numeric_comparison(condition: &str) -> Option<(&str, &str, &str)> {
    let mut paren_depth = 0usize;
    let mut indices = condition.char_indices().peekable();
    while let Some((idx, ch)) = indices.next() {
        match ch {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.checked_sub(1)?,
            '<' | '>' | '=' | '!' if paren_depth == 0 => {
                let next = indices.peek().map(|(_, next)| *next);
                let operator_len = if matches!(next, Some('=')) { 2 } else { 1 };
                let operator = &condition[idx..idx + operator_len];
                if !matches!(operator, "<" | "<=" | ">" | ">=" | "==" | "!=") {
                    return None;
                }
                let right_start = idx + operator_len;
                return Some((&condition[..idx], operator, &condition[right_start..]));
            }
            _ => {}
        }
    }
    None
}

fn render_typescript_numeric_body(body: &SimpleNumericBody) -> String {
    match body {
        SimpleNumericBody::Return(expr) => format!("  return {expr};\n"),
        SimpleNumericBody::LocalAssignments {
            assignments,
            result,
        } => {
            let mut out = String::new();
            for assignment in assignments {
                out.push_str(&format!(
                    "  const {} = {};\n",
                    assignment.target, assignment.expression
                ));
            }
            out.push_str(&format!("  return {result};\n"));
            out
        }
        SimpleNumericBody::GuardedReturns { guards, fallback } => {
            let mut out = String::new();
            for guard in guards {
                out.push_str(&format!(
                    "  if ({}) {{\n    return {};\n  }}\n",
                    guard.condition, guard.expression
                ));
            }
            out.push_str(&format!("  return {fallback};\n"));
            out
        }
    }
}

fn render_rust_numeric_body(body: &SimpleNumericBody) -> String {
    match body {
        SimpleNumericBody::Return(expr) => format!("    Ok({expr})\n"),
        SimpleNumericBody::LocalAssignments {
            assignments,
            result,
        } => {
            let mut out = String::new();
            for assignment in assignments {
                out.push_str(&format!(
                    "    let {} = {};\n",
                    assignment.target, assignment.expression
                ));
            }
            out.push_str(&format!("    Ok({result})\n"));
            out
        }
        SimpleNumericBody::GuardedReturns { guards, fallback } => {
            let mut out = String::new();
            for guard in guards {
                out.push_str(&format!(
                    "    if {} {{\n        return Ok({});\n    }}\n",
                    guard.condition, guard.expression
                ));
            }
            out.push_str(&format!("    Ok({fallback})\n"));
            out
        }
    }
}

fn translate_numeric_expr(
    expr: &str,
    param_map: &BTreeMap<String, String>,
    rust_numeric_literals: bool,
) -> Option<String> {
    let mut out = String::new();
    let mut chars = expr.char_indices().peekable();
    let mut saw_value = false;
    let mut expect_operand = true;
    let mut paren_depth = 0usize;
    while let Some((_, ch)) = chars.peek().copied() {
        if ch.is_ascii_whitespace() {
            out.push(ch);
            chars.next();
            continue;
        }
        if ch == '(' {
            if !expect_operand {
                return None;
            }
            out.push(ch);
            paren_depth += 1;
            chars.next();
            continue;
        }
        if ch == ')' {
            if expect_operand || paren_depth == 0 {
                return None;
            }
            out.push(ch);
            paren_depth -= 1;
            chars.next();
            expect_operand = false;
            continue;
        }
        if is_numeric_operator(ch) {
            if expect_operand {
                if matches!(ch, '+' | '-') {
                    out.push(ch);
                    chars.next();
                    continue;
                }
                return None;
            }
            out.push(ch);
            chars.next();
            expect_operand = true;
            continue;
        }
        if ch.is_ascii_digit() {
            if !expect_operand {
                return None;
            }
            let start = chars.next().map(|(idx, _)| idx)?;
            let mut end = start + ch.len_utf8();
            let mut has_dot = false;
            while let Some((idx, next)) = chars.peek().copied() {
                if next.is_ascii_digit() {
                    end = idx + next.len_utf8();
                    chars.next();
                } else if next == '.' && !has_dot {
                    has_dot = true;
                    end = idx + next.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let literal = &expr[start..end];
            if literal.ends_with('.') {
                return None;
            }
            if rust_numeric_literals && !has_dot {
                out.push_str(literal);
                out.push_str(".0");
            } else {
                out.push_str(literal);
            }
            saw_value = true;
            expect_operand = false;
            continue;
        }
        if is_identifier_start(ch) {
            if !expect_operand {
                return None;
            }
            let start = chars.next().map(|(idx, _)| idx)?;
            let mut end = start + ch.len_utf8();
            while let Some((idx, next)) = chars.peek().copied() {
                if is_identifier_continue(next) {
                    end = idx + next.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let ident = &expr[start..end];
            let target = param_map.get(ident)?;
            out.push_str(target);
            saw_value = true;
            expect_operand = false;
            continue;
        }
        return None;
    }
    (saw_value && !expect_operand && paren_depth == 0).then_some(out)
}

fn is_numeric_operator(ch: char) -> bool {
    matches!(ch, '+' | '-' | '*' | '/' | '%')
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn is_identifier_token(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    is_identifier_start(first) && chars.all(is_identifier_continue)
}

fn unique_target_params(api: &ApiContract, mapper: fn(&str, &str) -> String) -> Vec<String> {
    let mut seen = BTreeMap::new();
    signature_param_names(api.signature.as_deref())
        .into_iter()
        .filter(|param| !matches!(param.as_str(), "self" | "cls" | "this"))
        .map(|param| mapper(&param, "arg"))
        .map(|param| {
            let count = seen.entry(param.clone()).or_insert(0usize);
            *count += 1;
            if *count == 1 {
                param
            } else {
                format!("{param}{count}")
            }
        })
        .collect()
}

fn signature_param_names(signature: Option<&str>) -> Vec<String> {
    let Some(signature) = signature else {
        return Vec::new();
    };
    let Some(start) = signature.find('(') else {
        return Vec::new();
    };
    let Some(end_offset) = signature[start + 1..].find(')') else {
        return Vec::new();
    };
    signature[start + 1..start + 1 + end_offset]
        .split(',')
        .filter_map(clean_param_name)
        .collect()
}

fn clean_param_name(raw: &str) -> Option<String> {
    let mut value = raw.trim();
    if value.is_empty() {
        return None;
    }
    value = value.trim_start_matches('*').trim_start_matches('&').trim();
    if let Some((left, _)) = value.split_once(':') {
        value = left.trim();
    }
    if let Some((left, _)) = value.split_once('=') {
        value = left.trim();
    }
    if let Some(last) = value.split_whitespace().last() {
        value = last;
    }
    let cleaned = value
        .trim_matches(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .trim_matches('_')
        .to_ascii_lowercase();
    (!cleaned.is_empty()).then_some(cleaned)
}

fn api_is_data_model(api: &ApiContract) -> bool {
    let kind = api.kind.to_ascii_lowercase();
    kind.contains("class")
        || kind.contains("struct")
        || kind.contains("model")
        || kind.contains("dataclass")
        || kind.contains("enum")
}

fn port_type_name(api: &ApiContract) -> String {
    format!("{}Port", pascal_case(&identifier_slug(&api.name)))
}

fn typescript_identifier(value: &str, fallback: &str) -> String {
    let ident = identifier_slug(value);
    let ident = if ident.is_empty() {
        fallback.to_string()
    } else {
        ident
    };
    if is_typescript_reserved(&ident) {
        format!("{ident}_value")
    } else {
        ident
    }
}

fn rust_identifier(value: &str, fallback: &str) -> String {
    let ident = identifier_slug(value);
    let ident = if ident.is_empty() {
        fallback.to_string()
    } else {
        ident
    };
    if is_rust_reserved(&ident) {
        format!("{ident}_value")
    } else {
        ident
    }
}

fn is_typescript_reserved(value: &str) -> bool {
    matches!(
        value,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
    )
}

fn is_rust_reserved(value: &str) -> bool {
    matches!(
        value,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
    )
}

fn json_string<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".to_string())
}

fn rust_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn rust_string_slice(values: &[String]) -> String {
    if values.is_empty() {
        "&[]".to_string()
    } else {
        let values = values
            .iter()
            .map(|value| rust_string(value))
            .collect::<Vec<_>>()
            .join(", ");
        format!("&[{values}]")
    }
}

fn bullet_list(values: &[String]) -> String {
    if values.is_empty() {
        "- none\n".to_string()
    } else {
        values
            .iter()
            .map(|value| format!("- {value}\n"))
            .collect::<String>()
    }
}

#[cfg(test)]
mod tests {
    use rustyred_thg_behavior_ir::{IdiomLevel, OperationKind, PatchStatus, TargetLanguage};
    use rustyred_thg_core::NodeRecord;
    use serde_json::json;

    use super::*;
    use crate::{
        CodeDependencySnapshot, CodeFileSnapshot, CodeSpecCompileOutput, ComposeProvenance,
        ReconstructionSpec,
    };

    #[test]
    fn feature_slice_selects_seeded_symbols_tests_docs_and_dependencies() {
        let spec = fixture_spec();
        let input = FeatureSliceInput {
            seed: Some("json module".to_string()),
            ..FeatureSliceInput::default()
        };

        let slice = feature_slice_from_reconstruction_spec(&spec, &input);

        assert_eq!(slice.repo_id, "repo:fixture");
        assert_eq!(slice.files, vec!["lightwood/codegen.py"]);
        assert_eq!(slice.entry_symbols, vec!["sym:codegen"]);
        assert_eq!(slice.tests, vec!["tests/test_codegen.py"]);
        assert_eq!(slice.docs, vec!["README.md"]);
        assert!(slice
            .dependencies
            .iter()
            .any(|dep| dep.name == "json.dumps"));
        assert!(slice
            .evidence
            .iter()
            .any(|evidence| evidence.kind == "symbol"));
    }

    #[test]
    fn behavior_ir_lowers_feature_slice_to_portable_contract() {
        let spec = fixture_spec();
        let input = FeatureSliceInput {
            seed: Some("json module".to_string()),
            ..FeatureSliceInput::default()
        };

        let ir = behavior_ir_from_reconstruction_spec(&spec, &input);

        assert_eq!(ir.version, BEHAVIOR_IR_VERSION);
        assert_eq!(ir.feature.version, FEATURE_SLICE_VERSION);
        assert_eq!(ir.public_api[0].name, "build_json_ai_module");
        assert!(ir.public_api[0]
            .source_body
            .as_deref()
            .is_some_and(|body| body.contains("return payload + 1")));
        assert_eq!(ir.operations[0].kind, OperationKind::ParserSerializer);
        assert!(ir
            .invariants
            .iter()
            .any(|invariant| invariant.summary.contains("structural inventory")));
        assert!(ir.portability_hazards.is_empty());
    }

    #[test]
    fn target_plan_emit_and_validate_create_reviewable_rust_patch() {
        let spec = fixture_spec();
        let slice_input = FeatureSliceInput {
            seed: Some("json module".to_string()),
            ..FeatureSliceInput::default()
        };
        let ir = behavior_ir_from_reconstruction_spec(&spec, &slice_input);
        let target_input = TargetPlanInput {
            target_language: TargetLanguage::Rust,
            ..TargetPlanInput::default()
        };

        let plan = target_plan_from_behavior_ir(&ir, &target_input);
        let patch = patch_set_from_behavior_ir(&ir, &plan);
        let validated = validate_patch_set(&plan, &patch);

        assert_eq!(plan.target_language, TargetLanguage::Rust);
        assert_eq!(plan.module_plan[0].module_path, "src/json_module.rs");
        assert_eq!(patch.status, PatchStatus::NeedsReview);
        assert_eq!(patch.files[0].path, "src/json_module.rs");
        assert!(patch.files[0].after.contains("pub struct PortMetadata"));
        assert!(patch.files[0].after.contains("pub const OPERATION_STUBS"));
        assert!(patch.files[0].after.contains("pub struct PortError"));
        assert!(patch.files[0]
            .after
            .contains("pub fn build_json_ai_module(payload: f64) -> Result<f64, PortError>"));
        assert!(patch.files[0].after.contains("Ok(payload + 1.0)"));
        assert!(patch.files[0]
            .after
            .contains("Generated output is a behavior scaffold"));
        assert!(patch.tests[0]
            .after
            .contains("assert_eq!(generated_port::build_json_ai_module(0.0).unwrap(), 1.0);"));
        assert_eq!(validated.receipts[0].command, "cargo test");
        assert_eq!(validated.receipts[0].status, "not_run");
    }

    #[test]
    fn target_plan_emit_creates_typescript_api_stubs() {
        let spec = fixture_spec();
        let slice_input = FeatureSliceInput {
            seed: Some("json module".to_string()),
            ..FeatureSliceInput::default()
        };
        let ir = behavior_ir_from_reconstruction_spec(&spec, &slice_input);
        let target_input = TargetPlanInput {
            target_language: TargetLanguage::TypeScript,
            ..TargetPlanInput::default()
        };

        let plan = target_plan_from_behavior_ir(&ir, &target_input);
        let patch = patch_set_from_behavior_ir(&ir, &plan);

        assert_eq!(patch.files[0].path, "src/json_module.ts");
        assert!(patch.files[0].after.contains("export const operationStubs"));
        assert!(patch.files[0]
            .after
            .contains("export function build_json_ai_module(payload: number): number"));
        assert!(patch.files[0].after.contains("return payload + 1;"));
        assert!(patch.tests[0]
            .after
            .contains("assert.equal(build_json_ai_module(0), 1);"));
    }

    #[test]
    fn semantic_return_translation_rejects_unsupported_python_shapes() {
        let api = ApiContract {
            name: "render_template".to_string(),
            kind: "function".to_string(),
            file_path: Some("lightwood/codegen.py".to_string()),
            signature: Some("def render_template(payload)".to_string()),
            source_body: Some(
                "def render_template(payload):\n    return template.render(payload)".to_string(),
            ),
            evidence_ids: Vec::new(),
        };

        assert_eq!(
            translate_simple_numeric_body(&api, typescript_identifier, false),
            None
        );
    }

    #[test]
    fn semantic_return_translation_rejects_incomplete_arithmetic() {
        let api = ApiContract {
            name: "broken_math".to_string(),
            kind: "function".to_string(),
            file_path: Some("lightwood/codegen.py".to_string()),
            signature: Some("def broken_math(payload)".to_string()),
            source_body: Some("def broken_math(payload):\n    return payload +".to_string()),
            evidence_ids: Vec::new(),
        };

        assert_eq!(
            translate_simple_numeric_body(&api, typescript_identifier, false),
            None
        );
    }

    #[test]
    fn semantic_guarded_return_translation_emits_control_flow() {
        let api = ApiContract {
            name: "clamp_value".to_string(),
            kind: "function".to_string(),
            file_path: Some("lightwood/math.py".to_string()),
            signature: Some("def clamp_value(value, low, high)".to_string()),
            source_body: Some(
                "def clamp_value(value, low, high):\n    if value < low:\n        return low\n    if value > high:\n        return high\n    return value"
                    .to_string(),
            ),
            evidence_ids: Vec::new(),
        };

        let typescript =
            translate_simple_numeric_body(&api, typescript_identifier, false).unwrap();
        assert_eq!(
            render_typescript_numeric_body(&typescript),
            "  if (value < low) {\n    return low;\n  }\n  if (value > high) {\n    return high;\n  }\n  return value;\n"
        );

        let rust = translate_simple_numeric_body(&api, rust_identifier, true).unwrap();
        assert_eq!(
            render_rust_numeric_body(&rust),
            "    if value < low {\n        return Ok(low);\n    }\n    if value > high {\n        return Ok(high);\n    }\n    Ok(value)\n"
        );
    }

    #[test]
    fn semantic_guarded_return_translation_accepts_elif_else_and_boolean_conditions() {
        let api = ApiContract {
            name: "clamp_value".to_string(),
            kind: "function".to_string(),
            file_path: Some("lightwood/math.py".to_string()),
            signature: Some("def clamp_value(value, low, high)".to_string()),
            source_body: Some(
                "def clamp_value(value, low, high):\n    if value < low or value == low:\n        return low\n    elif value > high and high != 0:\n        return high\n    else:\n        return value"
                    .to_string(),
            ),
            evidence_ids: Vec::new(),
        };

        let typescript =
            translate_simple_numeric_body(&api, typescript_identifier, false).unwrap();
        assert_eq!(
            render_typescript_numeric_body(&typescript),
            "  if (value < low || value == low) {\n    return low;\n  }\n  if (value > high && high != 0) {\n    return high;\n  }\n  return value;\n"
        );

        let examples =
            semantic_examples_for_api(&api, typescript_identifier, "portedOperation", false);
        assert_eq!(examples.len(), 3);
    }

    #[test]
    fn semantic_guarded_return_translation_accepts_brace_style_else() {
        let api = ApiContract {
            name: "floor_zero".to_string(),
            kind: "function".to_string(),
            file_path: Some("src/math.rs".to_string()),
            signature: Some("pub fn floor_zero(value: f64)".to_string()),
            source_body: Some(
                "pub fn floor_zero(value: f64) -> f64 {\n    if (value < 0) {\n        return 0;\n    } else {\n        return value;\n    }\n}"
                    .to_string(),
            ),
            evidence_ids: Vec::new(),
        };

        let rust = translate_simple_numeric_body(&api, rust_identifier, true).unwrap();
        assert_eq!(
            render_rust_numeric_body(&rust),
            "    if value < 0.0 {\n        return Ok(0.0);\n    }\n    Ok(value)\n"
        );
    }

    #[test]
    fn semantic_assignment_return_translation_emits_locals() {
        let api = ApiContract {
            name: "score_value".to_string(),
            kind: "function".to_string(),
            file_path: Some("lightwood/math.py".to_string()),
            signature: Some("def score_value(value, offset)".to_string()),
            source_body: Some(
                "def score_value(value, offset):\n    adjusted = value + offset\n    doubled = adjusted * 2\n    return doubled"
                    .to_string(),
            ),
            evidence_ids: Vec::new(),
        };

        let typescript =
            translate_simple_numeric_body(&api, typescript_identifier, false).unwrap();
        assert_eq!(
            render_typescript_numeric_body(&typescript),
            "  const adjusted = value + offset;\n  const doubled = adjusted * 2;\n  return doubled;\n"
        );

        let rust = translate_simple_numeric_body(&api, rust_identifier, true).unwrap();
        assert_eq!(
            render_rust_numeric_body(&rust),
            "    let adjusted = value + offset;\n    let doubled = adjusted * 2.0;\n    Ok(doubled)\n"
        );
    }

    #[test]
    fn target_plan_input_accepts_nested_typescript_target() {
        let input = target_plan_input_from_value(&json!({
            "target": {
                "language": "typescript",
                "idiom_level": "faithful",
                "target_project": {
                    "project_root": "/tmp/target",
                    "package_name": "demo"
                }
            }
        }))
        .unwrap();

        assert_eq!(input.target_language, TargetLanguage::TypeScript);
        assert_eq!(input.idiom_level, IdiomLevel::Faithful);
        assert_eq!(
            input.target_project.project_root.as_deref(),
            Some("/tmp/target")
        );
    }

    fn fixture_spec() -> ReconstructionSpec {
        ReconstructionSpec {
            source_ref: SourceRef {
                github_url: Some("https://github.com/mindsdb/lightwood.git".to_string()),
                repo_id: Some("repo:fixture".to_string()),
                sha: Some("abc123".to_string()),
                ..SourceRef::default()
            },
            code_spec: Some(CodeSpecCompileOutput {
                spec_node: NodeRecord::new(
                    "code-spec:fixture",
                    [crate::CODE_SPEC_LABEL],
                    json!({"repo_id": "repo:fixture"}),
                ),
                spec_edges: Vec::new(),
                files: vec![
                    CodeFileSnapshot {
                        file_id: "file:codegen".to_string(),
                        path: "lightwood/codegen.py".to_string(),
                        language: "Python".to_string(),
                        content_hash: None,
                    },
                    CodeFileSnapshot {
                        file_id: "file:test".to_string(),
                        path: "tests/test_codegen.py".to_string(),
                        language: "Python".to_string(),
                        content_hash: None,
                    },
                    CodeFileSnapshot {
                        file_id: "file:readme".to_string(),
                        path: "README.md".to_string(),
                        language: "Markdown".to_string(),
                        content_hash: None,
                    },
                ],
                symbols: vec![
                    CodeSymbolSnapshot {
                        symbol_id: "sym:codegen".to_string(),
                        file_id: Some("file:codegen".to_string()),
                        file_path: "lightwood/codegen.py".to_string(),
                        kind: "function".to_string(),
                        name: "build_json_ai_module".to_string(),
                        language: "Python".to_string(),
                        line: Some(10),
                        signature: Some("def build_json_ai_module(payload)".to_string()),
                        body: Some(
                            "def build_json_ai_module(payload):\n    return payload + 1"
                                .to_string(),
                        ),
                        call_names: vec!["json.dumps".to_string()],
                        dependency_names: vec!["template.render".to_string()],
                        parser_backed: true,
                    },
                    CodeSymbolSnapshot {
                        symbol_id: "sym:helper".to_string(),
                        file_id: Some("file:codegen".to_string()),
                        file_path: "lightwood/codegen.py".to_string(),
                        kind: "function".to_string(),
                        name: "align_imports".to_string(),
                        language: "Python".to_string(),
                        line: Some(20),
                        signature: Some("def align_imports(imports)".to_string()),
                        body: None,
                        call_names: Vec::new(),
                        dependency_names: Vec::new(),
                        parser_backed: true,
                    },
                ],
                dependency_edges: vec![CodeDependencySnapshot {
                    from_symbol_id: "sym:codegen".to_string(),
                    to_symbol_id: "sym:helper".to_string(),
                    edge_type: "CALLS_SYMBOL".to_string(),
                }],
                file_count: 3,
                symbol_count: 2,
                structure_count: 0,
                member_count: 0,
                dependency_edge_count: 1,
                artifact_hash: "hash".to_string(),
                spec_body: "fixture".to_string(),
            }),
            features: Vec::new(),
            obligations: vec![crate::CodeImplementationObligation {
                tenant_id: "Travis-Gilbert".to_string(),
                repo_id: "repo:fixture".to_string(),
                obligation_id: "obligation:coverage".to_string(),
                target_file: None,
                target_symbol_id: None,
                obligation: "Validate compiled source specification coverage".to_string(),
                rationale: "Compiled code specification covers structural inventory.".to_string(),
                evidence_ids: vec!["code-spec:fixture".to_string()],
                suggested_validators: vec!["validator:code:compiled-spec-coverage".to_string()],
                risks: vec!["medium-risk:source-spec-coverage-regression".to_string()],
                unknowns: Vec::new(),
            }],
            patterns: Vec::new(),
            binary: None,
            datawave_facts: Vec::new(),
            drift: Vec::new(),
            provenance: ComposeProvenance {
                ingest_path: "AlreadyInStore".to_string(),
                repo_id: "repo:fixture".to_string(),
                sha: Some("abc123".to_string()),
                compiler_version: crate::CODE_COMPILER_VERSION.to_string(),
                feature_version: crate::CODE_COMPILER_FEATURE_VERSION.to_string(),
                code_to_datawave: None,
            },
            code_files_count: 3,
            code_symbols_count: 2,
        }
    }
}
