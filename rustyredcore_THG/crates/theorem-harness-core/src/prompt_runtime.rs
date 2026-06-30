//! Request-time prompt and output adaptation for composed-agent invocations.
//!
//! This is the fast clock: pure, bounded logic that can condition one request
//! without depending on offline optimizers or provider-specific render code.

use crate::agent_binding::HeadKind;
use crate::agent_head_registry::ResolvedAgentHead;
use crate::config_ledger::ConfigState;
use crate::epistemic_fitness::{measure_fitness_traits, FitnessObservation};
use crate::head_invocation::{
    default_head_system_prompt, GroundedClaim, HeadInvocationError, HeadInvocationKind,
    HeadInvocationReceipt, HeadInvocationRequest, HeadInvoker, CRITIQUE_ROLE_INSTRUCTION,
    FAST_FIRST_HEAD_PROMPT_ADDENDUM, MODALITY_HEAD_PROMPT_ADDENDUM, PROPOSAL_ROLE_INSTRUCTION,
    SYNTHESIS_ROLE_INSTRUCTION, THEOREM_HEAD_SYSTEM_PROMPT_CORE, VERIFICATION_ROLE_INSTRUCTION,
    VERIFIER_HEAD_PROMPT_ADDENDUM,
};
use crate::memory_contracts::PrepareMemoryEvidence;
use crate::metrics_composite::FitnessTraitScores;
use crate::state_hash::stable_value_hash;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const INSTRUCTION_KEY_PREFIX: &str = "instruction.";
pub const EXEMPLAR_EVIDENCE_KIND: &str = "theorem.prompt.exemplar";
pub const DEFAULT_INVOCATION_SHAPE: &[HeadInvocationKind] = &[
    HeadInvocationKind::Proposal,
    HeadInvocationKind::Critique,
    HeadInvocationKind::Synthesis,
    HeadInvocationKind::Verification,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PromptRuntimeError {
    EmptyInstructionKey,
    NonInstructionKey(String),
    DuplicateInstructionKey(String),
    UnknownInstructionKey(String),
    NonStringConfigValue(String),
    InvalidInvocationKind(String),
    InvalidVariantKey(String),
}

impl fmt::Display for PromptRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyInstructionKey => write!(f, "instruction key cannot be empty"),
            Self::NonInstructionKey(key) => {
                write!(f, "prompt runtime key is not an instruction key: {key}")
            }
            Self::DuplicateInstructionKey(key) => {
                write!(f, "instruction key is already registered: {key}")
            }
            Self::UnknownInstructionKey(key) => write!(f, "unknown instruction key: {key}"),
            Self::NonStringConfigValue(key) => {
                write!(f, "instruction key must resolve to a string value: {key}")
            }
            Self::InvalidInvocationKind(kind) => write!(f, "invalid invocation kind: {kind}"),
            Self::InvalidVariantKey(key) => write!(f, "variant key is not registered: {key}"),
        }
    }
}

impl Error for PromptRuntimeError {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PromptInstructionSpec {
    pub key: String,
    pub head_kind: HeadKind,
    pub invocation_kind: HeadInvocationKind,
    pub default_instruction: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PromptInstructionRegistry {
    instructions: BTreeMap<String, PromptInstructionSpec>,
    by_target: BTreeMap<String, String>,
}

impl PromptInstructionRegistry {
    pub fn register(&mut self, spec: PromptInstructionSpec) -> Result<(), PromptRuntimeError> {
        validate_instruction_key(&spec.key)?;
        if self.instructions.contains_key(&spec.key) {
            return Err(PromptRuntimeError::DuplicateInstructionKey(spec.key));
        }
        self.by_target.insert(
            target_key(&spec.head_kind, spec.invocation_kind),
            spec.key.clone(),
        );
        self.instructions.insert(spec.key.clone(), spec);
        Ok(())
    }

    pub fn register_variant_key(
        &mut self,
        key: impl Into<String>,
        default_instruction: impl Into<String>,
    ) -> Result<(), PromptRuntimeError> {
        let key = key.into();
        validate_instruction_key(&key)?;
        if self.instructions.contains_key(&key) {
            return Err(PromptRuntimeError::DuplicateInstructionKey(key));
        }
        self.instructions.insert(
            key.clone(),
            PromptInstructionSpec {
                key,
                head_kind: HeadKind::ReasoningCore,
                invocation_kind: HeadInvocationKind::Synthesis,
                default_instruction: default_instruction.into(),
            },
        );
        Ok(())
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.instructions.contains_key(key)
    }

    pub fn key_for(
        &self,
        head_kind: &HeadKind,
        kind: HeadInvocationKind,
    ) -> Result<&str, PromptRuntimeError> {
        let target = target_key(head_kind, kind);
        self.by_target
            .get(&target)
            .map(String::as_str)
            .ok_or(PromptRuntimeError::UnknownInstructionKey(target))
    }

    pub fn resolve_key(
        &self,
        config: &ConfigState,
        key: &str,
    ) -> Result<String, PromptRuntimeError> {
        let spec = self
            .instructions
            .get(key)
            .ok_or_else(|| PromptRuntimeError::UnknownInstructionKey(key.to_string()))?;
        match config.values.get(key) {
            Some(Value::String(value)) => Ok(value.clone()),
            Some(_) => Err(PromptRuntimeError::NonStringConfigValue(key.to_string())),
            None => Ok(spec.default_instruction.clone()),
        }
    }

    pub fn resolve_for_head(
        &self,
        config: &ConfigState,
        head: &ResolvedAgentHead,
        kind: HeadInvocationKind,
    ) -> Result<ResolvedPromptInstruction, PromptRuntimeError> {
        let key = self.key_for(&head.kind, kind)?.to_string();
        let instruction_text = match config.values.get(&key) {
            Some(Value::String(value)) => value.clone(),
            Some(_) => return Err(PromptRuntimeError::NonStringConfigValue(key)),
            None => default_head_system_prompt(head, kind),
        };
        Ok(ResolvedPromptInstruction {
            instruction_key: key,
            instruction_text,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedPromptInstruction {
    pub instruction_key: String,
    pub instruction_text: String,
}

pub fn default_prompt_instruction_registry() -> PromptInstructionRegistry {
    let mut registry = PromptInstructionRegistry::default();
    for head_kind in [
        HeadKind::ReasoningCore,
        HeadKind::SpecializedCoder,
        HeadKind::Verifier,
        HeadKind::SkillPlugin,
    ] {
        for kind in DEFAULT_INVOCATION_SHAPE {
            let spec = PromptInstructionSpec {
                key: prompt_instruction_key(&head_kind, *kind),
                default_instruction: default_seed_instruction(&head_kind, *kind),
                head_kind: head_kind.clone(),
                invocation_kind: *kind,
            };
            registry
                .register(spec)
                .expect("default prompt instruction keys are valid and unique");
        }
    }
    registry
}

pub fn prompt_instruction_key(head_kind: &HeadKind, kind: HeadInvocationKind) -> String {
    format!(
        "{INSTRUCTION_KEY_PREFIX}head.{}.{}.system",
        head_kind_slug(head_kind),
        kind.as_str()
    )
}

pub fn validate_instruction_key(key: &str) -> Result<(), PromptRuntimeError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(PromptRuntimeError::EmptyInstructionKey);
    }
    if !key.starts_with(INSTRUCTION_KEY_PREFIX) {
        return Err(PromptRuntimeError::NonInstructionKey(key.to_string()));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskVariantSelection {
    pub task_type: String,
    pub instruction_key: String,
    pub invocation_shape: Vec<HeadInvocationKind>,
}

pub fn select_variant_for_task(
    config: &ConfigState,
    registry: &PromptInstructionRegistry,
    task_type: &str,
    fallback_head_kind: &HeadKind,
    fallback_kind: HeadInvocationKind,
) -> Result<TaskVariantSelection, PromptRuntimeError> {
    let task_type = task_type.trim();
    let slug = task_slug(task_type);
    let fallback_key = registry
        .key_for(fallback_head_kind, fallback_kind)?
        .to_string();
    let key_config = format!("runtime.task_type.{slug}.instruction_key");
    let shape_config = format!("runtime.task_type.{slug}.shape");
    let instruction_key = config
        .values
        .get(&key_config)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or(fallback_key);
    if !registry.contains_key(&instruction_key) {
        return Err(PromptRuntimeError::InvalidVariantKey(instruction_key));
    }
    let invocation_shape = config
        .values
        .get(&shape_config)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(parse_invocation_kind)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| DEFAULT_INVOCATION_SHAPE.to_vec());
    Ok(TaskVariantSelection {
        task_type: task_type.to_string(),
        instruction_key,
        invocation_shape,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExemplarRecord {
    pub intent_id: String,
    pub task: String,
    pub output: String,
    pub outcome: Value,
}

pub fn exemplar_evidence_from_accepted_run(
    intent_id: &str,
    task: &str,
    output: &str,
    outcome: Value,
) -> PrepareMemoryEvidence {
    let payload = Map::from_iter([
        (
            "intent_id".to_string(),
            Value::String(intent_id.to_string()),
        ),
        ("task".to_string(), Value::String(task.to_string())),
        ("output".to_string(), Value::String(output.to_string())),
        ("outcome".to_string(), outcome.clone()),
    ]);
    let evidence_id = format!(
        "evidence:prompt-exemplar:{}",
        stable_value_hash(&Value::Object(payload.clone()))
    );
    PrepareMemoryEvidence {
        evidence_id,
        kind: EXEMPLAR_EVIDENCE_KIND.to_string(),
        source: "runtime:accepted-run".to_string(),
        immutable: true,
        payload,
        rationale: "Accepted run captured as request-time prompt exemplar.".to_string(),
    }
}

pub fn exemplars_from_evidence(
    evidence: &[PrepareMemoryEvidence],
    intent_id: &str,
    limit: usize,
) -> Vec<ExemplarRecord> {
    evidence
        .iter()
        .filter(|item| item.kind == EXEMPLAR_EVIDENCE_KIND)
        .filter_map(exemplar_from_evidence)
        .filter(|example| example.intent_id == intent_id)
        .filter(|example| outcome_is_positive(&example.outcome))
        .take(limit)
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    ClaimBearing,
    Code,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriticRoute {
    EpistemicTraits,
    ExecutableChecks,
}

pub fn critic_route_for_output(kind: OutputKind) -> CriticRoute {
    match kind {
        OutputKind::ClaimBearing => CriticRoute::EpistemicTraits,
        OutputKind::Code => CriticRoute::ExecutableChecks,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AbsoluteFitnessBar {
    pub min_root_depth: f64,
    pub min_source_independence: f64,
    pub min_support_ratio: f64,
    pub min_claim_specificity: f64,
    pub min_temporal_spread: f64,
}

impl Default for AbsoluteFitnessBar {
    fn default() -> Self {
        Self {
            min_root_depth: 0.15,
            min_source_independence: 0.75,
            min_support_ratio: 0.75,
            min_claim_specificity: 0.35,
            min_temporal_spread: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeRefinementConfig {
    pub max_refinements: u32,
    pub fitness_bar: AbsoluteFitnessBar,
}

impl Default for RuntimeRefinementConfig {
    fn default() -> Self {
        Self {
            max_refinements: 1,
            fitness_bar: AbsoluteFitnessBar::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AbsoluteFitnessViolation {
    pub trait_name: String,
    pub observed: f64,
    pub minimum: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeRefinementDecision {
    pub critic_route: CriticRoute,
    pub should_refine: bool,
    #[serde(default)]
    pub observations: Vec<FitnessObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scores: Option<FitnessTraitScores>,
    #[serde(default)]
    pub violations: Vec<AbsoluteFitnessViolation>,
    pub feedback: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeRefinementResult {
    pub receipts: Vec<HeadInvocationReceipt>,
    pub decisions: Vec<RuntimeRefinementDecision>,
}

pub fn evaluate_runtime_refinement(
    receipt: &HeadInvocationReceipt,
    output_kind: OutputKind,
    config: &RuntimeRefinementConfig,
) -> RuntimeRefinementDecision {
    match critic_route_for_output(output_kind) {
        CriticRoute::ExecutableChecks => RuntimeRefinementDecision {
            critic_route: CriticRoute::ExecutableChecks,
            should_refine: false,
            observations: Vec::new(),
            scores: None,
            violations: Vec::new(),
            feedback: "Route code output to executable verifier checks: compile, tests, and lint."
                .to_string(),
        },
        CriticRoute::EpistemicTraits => {
            let observations = fitness_observations_from_claims(&receipt.claims);
            let scores = measure_fitness_traits(&observations);
            let violations = absolute_violations(&scores, &config.fitness_bar);
            RuntimeRefinementDecision {
                critic_route: CriticRoute::EpistemicTraits,
                should_refine: !violations.is_empty(),
                observations,
                scores: Some(scores),
                feedback: refinement_feedback(&violations),
                violations,
            }
        }
    }
}

pub fn refine_with_invoker<I: HeadInvoker>(
    invoker: &I,
    base_request: HeadInvocationRequest,
    initial_receipt: HeadInvocationReceipt,
    output_kind: OutputKind,
    config: RuntimeRefinementConfig,
) -> Result<RuntimeRefinementResult, HeadInvocationError> {
    let mut receipts = vec![initial_receipt];
    let mut decisions = Vec::new();
    let mut refinements_used = 0;
    loop {
        let decision = evaluate_runtime_refinement(
            receipts
                .last()
                .expect("refinement always starts with an initial receipt"),
            output_kind,
            &config,
        );
        let should_refine = decision.should_refine;
        let feedback = decision.feedback.clone();
        decisions.push(decision);
        if !should_refine || refinements_used >= config.max_refinements {
            break;
        }
        let mut request = base_request.clone();
        request.task = format!(
            "{}\n\nRuntime refinement feedback (attempt {}):\n{}",
            base_request.task,
            refinements_used + 1,
            feedback
        );
        request.invocation_id = request.computed_invocation_id();
        receipts.push(invoker.invoke(request)?);
        refinements_used += 1;
    }
    Ok(RuntimeRefinementResult {
        receipts,
        decisions,
    })
}

pub fn fitness_observations_from_claims(claims: &[GroundedClaim]) -> Vec<FitnessObservation> {
    claims
        .iter()
        .enumerate()
        .map(|(index, claim)| {
            let provenance = claim.provenance.trim();
            let supported = !provenance.is_empty()
                && !provenance.eq_ignore_ascii_case("unknown")
                && !provenance.eq_ignore_ascii_case("unsupported");
            FitnessObservation {
                root_depth: if supported { 1 } else { 0 },
                source_id: if supported {
                    provenance.to_string()
                } else {
                    "unsupported".to_string()
                },
                support_ratio: if supported { 1.0 } else { 0.0 },
                claim_specificity: claim_specificity(&claim.text),
                observed_at_ms: index as i64 * 1_000,
            }
        })
        .collect()
}

fn default_seed_instruction(head_kind: &HeadKind, kind: HeadInvocationKind) -> String {
    let mut instruction = String::from(THEOREM_HEAD_SYSTEM_PROMPT_CORE);
    instruction.push_str("\n\nCurrent invocation role: ");
    instruction.push_str(match kind {
        HeadInvocationKind::Proposal => PROPOSAL_ROLE_INSTRUCTION,
        HeadInvocationKind::Critique => CRITIQUE_ROLE_INSTRUCTION,
        HeadInvocationKind::Synthesis => SYNTHESIS_ROLE_INSTRUCTION,
        HeadInvocationKind::Verification => VERIFICATION_ROLE_INSTRUCTION,
    });
    if *head_kind == HeadKind::Verifier || kind == HeadInvocationKind::Verification {
        instruction.push_str("\n\n");
        instruction.push_str(VERIFIER_HEAD_PROMPT_ADDENDUM);
    }
    if *head_kind == HeadKind::SkillPlugin {
        instruction.push_str("\n\n");
        instruction.push_str(MODALITY_HEAD_PROMPT_ADDENDUM);
    }
    if *head_kind == HeadKind::ReasoningCore && kind == HeadInvocationKind::Proposal {
        instruction.push_str("\n\n");
        instruction.push_str(FAST_FIRST_HEAD_PROMPT_ADDENDUM);
    }
    instruction
}

fn target_key(head_kind: &HeadKind, kind: HeadInvocationKind) -> String {
    format!("{}:{}", head_kind_slug(head_kind), kind.as_str())
}

fn head_kind_slug(head_kind: &HeadKind) -> &'static str {
    match head_kind {
        HeadKind::ReasoningCore => "reasoning_core",
        HeadKind::SkillPlugin => "skill_plugin",
        HeadKind::SpecializedCoder => "specialized_coder",
        HeadKind::Verifier => "verifier",
    }
}

fn task_slug(task_type: &str) -> String {
    task_type
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn parse_invocation_kind(value: &str) -> Result<HeadInvocationKind, PromptRuntimeError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "proposal" => Ok(HeadInvocationKind::Proposal),
        "critique" => Ok(HeadInvocationKind::Critique),
        "synthesis" => Ok(HeadInvocationKind::Synthesis),
        "verification" => Ok(HeadInvocationKind::Verification),
        other => Err(PromptRuntimeError::InvalidInvocationKind(other.to_string())),
    }
}

fn exemplar_from_evidence(evidence: &PrepareMemoryEvidence) -> Option<ExemplarRecord> {
    let intent_id = evidence.payload.get("intent_id")?.as_str()?.to_string();
    let task = evidence.payload.get("task")?.as_str()?.to_string();
    let output = evidence.payload.get("output")?.as_str()?.to_string();
    let outcome = evidence.payload.get("outcome")?.clone();
    Some(ExemplarRecord {
        intent_id,
        task,
        output,
        outcome,
    })
}

fn outcome_is_positive(outcome: &Value) -> bool {
    outcome
        .get("accepted")
        .and_then(Value::as_bool)
        .or_else(|| outcome.get("tests_passed").and_then(Value::as_bool))
        .unwrap_or_else(|| {
            outcome
                .get("outcome")
                .or_else(|| outcome.get("status"))
                .and_then(Value::as_str)
                .is_some_and(|value| matches!(value, "accepted" | "success" | "passed"))
        })
}

fn absolute_violations(
    scores: &FitnessTraitScores,
    bar: &AbsoluteFitnessBar,
) -> Vec<AbsoluteFitnessViolation> {
    [
        ("root_depth", scores.root_depth, bar.min_root_depth),
        (
            "source_independence",
            scores.source_independence,
            bar.min_source_independence,
        ),
        ("support_ratio", scores.support_ratio, bar.min_support_ratio),
        (
            "claim_specificity",
            scores.claim_specificity,
            bar.min_claim_specificity,
        ),
        (
            "temporal_spread",
            scores.temporal_spread,
            bar.min_temporal_spread,
        ),
    ]
    .into_iter()
    .filter_map(|(trait_name, observed, minimum)| {
        (observed < minimum).then(|| AbsoluteFitnessViolation {
            trait_name: trait_name.to_string(),
            observed,
            minimum,
        })
    })
    .collect()
}

fn refinement_feedback(violations: &[AbsoluteFitnessViolation]) -> String {
    if violations.is_empty() {
        return "Runtime critic accepted the output.".to_string();
    }
    let details = violations
        .iter()
        .map(|violation| {
            format!(
                "{} observed {:.6}, required {:.6}",
                violation.trait_name, violation.observed, violation.minimum
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("Runtime critic requests refinement: {details}.")
}

fn claim_specificity(text: &str) -> f64 {
    let words = text.split_whitespace().count() as f64;
    (words / 8.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AgentHeadEndpoint, HeadCostProfile, HeadReliabilityProfile, HeadTransport, TraceTier,
    };
    use serde_json::json;
    use std::cell::RefCell;

    #[test]
    fn registry_resolves_default_and_config_instruction() {
        let registry = default_prompt_instruction_registry();
        let head = head(HeadKind::ReasoningCore);
        let key = prompt_instruction_key(&HeadKind::ReasoningCore, HeadInvocationKind::Synthesis);
        let empty = ConfigState::default();

        let default = registry
            .resolve_for_head(&empty, &head, HeadInvocationKind::Synthesis)
            .unwrap();

        assert_eq!(default.instruction_key, key);
        assert_eq!(
            default.instruction_text,
            default_head_system_prompt(&head, HeadInvocationKind::Synthesis)
        );

        let config = ConfigState {
            values: BTreeMap::from([(key.clone(), json!("configured synthesis prompt"))]),
            graph_version_id: None,
        };
        let configured = registry
            .resolve_for_head(&config, &head, HeadInvocationKind::Synthesis)
            .unwrap();
        assert_eq!(configured.instruction_text, "configured synthesis prompt");
    }

    #[test]
    fn registry_rejects_non_instruction_key() {
        let mut registry = PromptInstructionRegistry::default();
        let error = registry
            .register_variant_key("routing.weight.codex", "bad")
            .unwrap_err();
        assert!(matches!(error, PromptRuntimeError::NonInstructionKey(_)));
    }

    #[test]
    fn variant_selection_requires_registered_instruction_key() {
        let mut registry = default_prompt_instruction_registry();
        registry
            .register_variant_key("instruction.variant.code", "code prompt")
            .unwrap();
        let config = ConfigState {
            values: BTreeMap::from([
                (
                    "runtime.task_type.code.instruction_key".to_string(),
                    json!("instruction.variant.code"),
                ),
                (
                    "runtime.task_type.code.shape".to_string(),
                    json!(["proposal", "synthesis", "verification"]),
                ),
            ]),
            graph_version_id: None,
        };

        let selected = select_variant_for_task(
            &config,
            &registry,
            "code",
            &HeadKind::ReasoningCore,
            HeadInvocationKind::Synthesis,
        )
        .unwrap();

        assert_eq!(selected.instruction_key, "instruction.variant.code");
        assert_eq!(
            selected.invocation_shape,
            vec![
                HeadInvocationKind::Proposal,
                HeadInvocationKind::Synthesis,
                HeadInvocationKind::Verification
            ]
        );
    }

    #[test]
    fn exemplar_evidence_filters_by_kind_intent_and_positive_outcome() {
        let positive = exemplar_evidence_from_accepted_run(
            "intent:answer",
            "task",
            "output",
            json!({"accepted": true}),
        );
        let negative = exemplar_evidence_from_accepted_run(
            "intent:answer",
            "bad task",
            "bad output",
            json!({"accepted": false}),
        );
        let unrelated = exemplar_evidence_from_accepted_run(
            "intent:other",
            "other task",
            "other output",
            json!({"accepted": true}),
        );

        let examples =
            exemplars_from_evidence(&[positive, negative, unrelated], "intent:answer", 4);

        assert_eq!(examples.len(), 1);
        assert_eq!(examples[0].task, "task");
    }

    #[test]
    fn unsupported_claim_output_refines_once_with_specific_violation() {
        let request = request_fixture("answer");
        let initial = HeadInvocationReceipt::from_request(
            &request,
            "bad",
            Map::from_iter([("text".to_string(), json!("bad"))]),
            1.0,
        )
        .with_claims(vec![
            GroundedClaim::new("The answer relies on one source only.", "source:a"),
            GroundedClaim::new("The conclusion repeats that same source.", "source:a"),
        ]);
        let invoker = RefiningInvoker::default();

        let result = refine_with_invoker(
            &invoker,
            request,
            initial,
            OutputKind::ClaimBearing,
            RuntimeRefinementConfig::default(),
        )
        .unwrap();

        assert_eq!(result.receipts.len(), 2);
        assert_eq!(invoker.calls.borrow().len(), 1);
        assert!(invoker.calls.borrow()[0].contains("source_independence"));
        assert_eq!(
            result.decisions[0].critic_route,
            CriticRoute::EpistemicTraits
        );
        assert!(result.decisions[0]
            .violations
            .iter()
            .any(|violation| violation.trait_name == "source_independence"));
    }

    #[test]
    fn clean_claim_output_makes_no_extra_call() {
        let request = request_fixture("answer");
        let initial = HeadInvocationReceipt::from_request(
            &request,
            "ok",
            Map::from_iter([("text".to_string(), json!("ok"))]),
            1.0,
        )
        .with_claims(vec![
            GroundedClaim::new(
                "First specific grounded claim has enough detail.",
                "source:a",
            ),
            GroundedClaim::new(
                "Second specific grounded claim has enough detail.",
                "source:b",
            ),
        ]);
        let invoker = RefiningInvoker::default();

        let result = refine_with_invoker(
            &invoker,
            request,
            initial,
            OutputKind::ClaimBearing,
            RuntimeRefinementConfig::default(),
        )
        .unwrap();

        assert_eq!(result.receipts.len(), 1);
        assert!(invoker.calls.borrow().is_empty());
    }

    #[test]
    fn code_output_routes_to_executable_critic() {
        let request = request_fixture("write code");
        let receipt = HeadInvocationReceipt::from_request(
            &request,
            "code",
            Map::from_iter([("text".to_string(), json!("fn main() {}"))]),
            1.0,
        );

        let decision = evaluate_runtime_refinement(
            &receipt,
            OutputKind::Code,
            &RuntimeRefinementConfig::default(),
        );

        assert_eq!(decision.critic_route, CriticRoute::ExecutableChecks);
        assert!(!decision.should_refine);
        assert!(decision.feedback.contains("compile"));
    }

    #[derive(Default)]
    struct RefiningInvoker {
        calls: RefCell<Vec<String>>,
    }

    impl HeadInvoker for RefiningInvoker {
        fn invoke(
            &self,
            request: HeadInvocationRequest,
        ) -> Result<HeadInvocationReceipt, HeadInvocationError> {
            self.calls.borrow_mut().push(request.task.clone());
            Ok(HeadInvocationReceipt::from_request(
                &request,
                "refined",
                Map::from_iter([("text".to_string(), json!("refined"))]),
                1.0,
            )
            .with_claims(vec![
                GroundedClaim::new("A specific refined claim with enough detail.", "source:a"),
                GroundedClaim::new(
                    "Another specific refined claim with enough detail.",
                    "source:b",
                ),
            ]))
        }
    }

    fn request_fixture(task: &str) -> HeadInvocationRequest {
        HeadInvocationRequest::new(
            head(HeadKind::ReasoningCore),
            HeadInvocationKind::Synthesis,
            task,
            1,
            Vec::new(),
            Vec::new(),
            "2026-06-30T00:00:00Z",
        )
    }

    fn head(kind: HeadKind) -> ResolvedAgentHead {
        ResolvedAgentHead {
            head_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            provider: "openai".to_string(),
            model: "model".to_string(),
            kind,
            endpoint: AgentHeadEndpoint {
                transport: HeadTransport::Api,
                target: "fake://target".to_string(),
                fake: true,
            },
            credential_ref: "env:TEST".to_string(),
            capabilities: Vec::new(),
            cost_profile: HeadCostProfile::default(),
            reliability_profile: HeadReliabilityProfile::default(),
            allowed_tools: Vec::new(),
            trace_tier: TraceTier::Receipt,
        }
    }
}
