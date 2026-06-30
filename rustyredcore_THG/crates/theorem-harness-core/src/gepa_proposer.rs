use crate::agent_binding::HeadKind;
use crate::agent_head_registry::ResolvedAgentHead;
use crate::attribution::ConfigAttributionTable;
use crate::config_ledger::{
    ConfigDelta, ConfigLedger, ConfigLedgerError, ConfigState, ConfigValueDelta,
};
use crate::epistemic_fitness::FitnessGateResult;
use crate::gepa_feedback::FeedbackPoint;
use crate::head_invocation::{
    HeadInvocationKind, HeadInvocationReceipt, HeadInvocationRequest, HeadInvoker,
    CRITIQUE_ROLE_INSTRUCTION, FAST_FIRST_HEAD_PROMPT_ADDENDUM, MODALITY_HEAD_PROMPT_ADDENDUM,
    PROPOSAL_ROLE_INSTRUCTION, SYNTHESIS_ROLE_INSTRUCTION, THEOREM_HEAD_SYSTEM_PROMPT_CORE,
    VERIFICATION_ROLE_INSTRUCTION, VERIFIER_HEAD_PROMPT_ADDENDUM,
};
use crate::improvement_rate::ImprovementRate;
use crate::loop_gate::{close_loop_if_allowed, LoopClosureInput, LoopGateState, LoopGateVerdict};
use crate::shadow_eval::{evaluate_shadow, ShadowEvalInput, ShadowEvalResult};
use crate::state_hash::stable_value_hash;
use crate::types::AgentRunState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Write as _};

pub const INSTRUCTION_KEY_PREFIX: &str = "instruction.";
pub const USER_PROMPT_IMPROVER_KEY: &str = "instruction.user_prompt_improver";
pub const HEAD_SYSTEM_CORE_KEY: &str = "instruction.head.system.core";
pub const HEAD_ROLE_PROPOSAL_KEY: &str = "instruction.head.role.proposal";
pub const HEAD_ROLE_CRITIQUE_KEY: &str = "instruction.head.role.critique";
pub const HEAD_ROLE_SYNTHESIS_KEY: &str = "instruction.head.role.synthesis";
pub const HEAD_ROLE_VERIFICATION_KEY: &str = "instruction.head.role.verification";
pub const HEAD_ADDENDUM_FAST_FIRST_KEY: &str = "instruction.head.addendum.fast_first";
pub const HEAD_ADDENDUM_VERIFIER_KEY: &str = "instruction.head.addendum.verifier";
pub const HEAD_ADDENDUM_MODALITY_KEY: &str = "instruction.head.addendum.modality";

pub const USER_PROMPT_IMPROVER_SEED_INSTRUCTION: &str = r#"Rewrite the user's submitted prompt into a clearer task request while preserving intent, constraints, tone, and any named files or acceptance criteria. Do not add requirements the user did not ask for. Return only the improved prompt."#;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GepaProposalError {
    DuplicateInstructionKey(String),
    EmptyInstructionKey,
    NonInstructionKey(String),
    EmptyInstructionValue(String),
    UnknownInstructionKey(String),
    NonStringConfigValue(String),
    EmptyUserPrompt,
    PromptImproverFailed(String),
    MissingOutcome {
        intent_id: String,
        session_id: String,
    },
    Ledger(ConfigLedgerError),
}

impl fmt::Display for GepaProposalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateInstructionKey(key) => {
                write!(f, "instruction key is already registered: {key}")
            }
            Self::EmptyInstructionKey => write!(f, "instruction key cannot be empty"),
            Self::NonInstructionKey(key) => {
                write!(f, "GEPA can only optimize instruction keys, got: {key}")
            }
            Self::EmptyInstructionValue(key) => {
                write!(f, "instruction value cannot be empty for key: {key}")
            }
            Self::UnknownInstructionKey(key) => write!(f, "unknown instruction key: {key}"),
            Self::NonStringConfigValue(key) => {
                write!(f, "instruction key must resolve to a string value: {key}")
            }
            Self::EmptyUserPrompt => write!(f, "user prompt cannot be empty"),
            Self::PromptImproverFailed(error) => write!(f, "prompt improver failed: {error}"),
            Self::MissingOutcome {
                intent_id,
                session_id,
            } => write!(
                f,
                "session {session_id} for intent {intent_id} is missing captured outcome"
            ),
            Self::Ledger(error) => write!(f, "{error}"),
        }
    }
}

impl Error for GepaProposalError {}

impl From<ConfigLedgerError> for GepaProposalError {
    fn from(value: ConfigLedgerError) -> Self {
        Self::Ledger(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum InstructionSource {
    UserPromptImprover,
    HeadSystemCore,
    HeadRole { role: String },
    HeadAddendum { name: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstructionKeySpec {
    pub key: String,
    pub description: String,
    pub default_value: String,
    pub source: InstructionSource,
}

impl InstructionKeySpec {
    pub fn new(
        key: impl Into<String>,
        description: impl Into<String>,
        default_value: impl Into<String>,
        source: InstructionSource,
    ) -> Result<Self, GepaProposalError> {
        let spec = Self {
            key: key.into(),
            description: description.into(),
            default_value: default_value.into(),
            source,
        };
        validate_instruction_spec(&spec)?;
        Ok(spec)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InstructionKeyRegistry {
    pub entries: BTreeMap<String, InstructionKeySpec>,
}

impl InstructionKeyRegistry {
    pub fn register(&mut self, spec: InstructionKeySpec) -> Result<(), GepaProposalError> {
        validate_instruction_spec(&spec)?;
        if self.entries.contains_key(&spec.key) {
            return Err(GepaProposalError::DuplicateInstructionKey(spec.key));
        }
        self.entries.insert(spec.key.clone(), spec);
        Ok(())
    }

    pub fn is_instruction_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub fn resolve(&self, config: &ConfigState, key: &str) -> Result<String, GepaProposalError> {
        let spec = self
            .entries
            .get(key)
            .ok_or_else(|| GepaProposalError::UnknownInstructionKey(key.to_string()))?;
        match config.values.get(key) {
            Some(Value::String(value)) => Ok(value.clone()),
            Some(_) => Err(GepaProposalError::NonStringConfigValue(key.to_string())),
            None => Ok(spec.default_value.clone()),
        }
    }

    pub fn seed_config_state(&self, config: &mut ConfigState) -> Result<(), GepaProposalError> {
        for (key, spec) in &self.entries {
            match config.values.get(key) {
                Some(Value::String(_)) => {}
                Some(_) => return Err(GepaProposalError::NonStringConfigValue(key.clone())),
                None => {
                    config
                        .values
                        .insert(key.clone(), Value::String(spec.default_value.clone()));
                }
            }
        }
        Ok(())
    }

    pub fn seed_candidate(
        &self,
        config: &ConfigState,
        key: &str,
    ) -> Result<BTreeMap<String, String>, GepaProposalError> {
        Ok(BTreeMap::from([(
            key.to_string(),
            self.resolve(config, key)?,
        )]))
    }
}

pub fn default_gepa_instruction_registry() -> InstructionKeyRegistry {
    let mut registry = InstructionKeyRegistry::default();
    for spec in default_instruction_specs() {
        registry
            .register(spec)
            .expect("default GEPA instruction keys should be valid and unique");
    }
    registry
}

pub fn head_role_instruction_key(kind: HeadInvocationKind) -> &'static str {
    match kind {
        HeadInvocationKind::Proposal => HEAD_ROLE_PROPOSAL_KEY,
        HeadInvocationKind::Critique => HEAD_ROLE_CRITIQUE_KEY,
        HeadInvocationKind::Synthesis => HEAD_ROLE_SYNTHESIS_KEY,
        HeadInvocationKind::Verification => HEAD_ROLE_VERIFICATION_KEY,
    }
}

pub fn configured_head_system_prompt(
    registry: &InstructionKeyRegistry,
    config: &ConfigState,
    head: &ResolvedAgentHead,
    kind: HeadInvocationKind,
) -> Result<String, GepaProposalError> {
    let mut prompt = registry.resolve(config, HEAD_SYSTEM_CORE_KEY)?;
    prompt.push_str("\n\nCurrent invocation role: ");
    prompt.push_str(&registry.resolve(config, head_role_instruction_key(kind))?);
    if is_fast_first_head(head) && kind == HeadInvocationKind::Proposal {
        prompt.push_str("\n\n");
        prompt.push_str(&registry.resolve(config, HEAD_ADDENDUM_FAST_FIRST_KEY)?);
    }
    if kind == HeadInvocationKind::Verification || head.kind == HeadKind::Verifier {
        prompt.push_str("\n\n");
        prompt.push_str(&registry.resolve(config, HEAD_ADDENDUM_VERIFIER_KEY)?);
    }
    if is_modality_head(head) {
        prompt.push_str("\n\n");
        prompt.push_str(&registry.resolve(config, HEAD_ADDENDUM_MODALITY_KEY)?);
    }
    if !head.capabilities.is_empty() {
        prompt.push_str("\n\nKnown strengths for this head: ");
        prompt.push_str(&head.capabilities.join(", "));
        prompt.push('.');
    }
    Ok(prompt)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserPromptImproverRequest {
    pub instruction_key: String,
    pub instruction: String,
    pub user_prompt: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserPromptImproverReceipt {
    pub instruction_key: String,
    pub instruction_hash: String,
    pub user_prompt_hash: String,
    pub improved_prompt: String,
    pub model_calls: u32,
}

pub fn serve_user_prompt_improver<F>(
    registry: &InstructionKeyRegistry,
    config: &ConfigState,
    user_prompt: &str,
    forward_once: F,
) -> Result<UserPromptImproverReceipt, GepaProposalError>
where
    F: FnOnce(UserPromptImproverRequest) -> Result<String, String>,
{
    let user_prompt = user_prompt.trim();
    if user_prompt.is_empty() {
        return Err(GepaProposalError::EmptyUserPrompt);
    }
    let instruction = registry.resolve(config, USER_PROMPT_IMPROVER_KEY)?;
    let request = UserPromptImproverRequest {
        instruction_key: USER_PROMPT_IMPROVER_KEY.to_string(),
        instruction: instruction.clone(),
        user_prompt: user_prompt.to_string(),
    };
    let improved_prompt = forward_once(request).map_err(GepaProposalError::PromptImproverFailed)?;
    Ok(UserPromptImproverReceipt {
        instruction_key: USER_PROMPT_IMPROVER_KEY.to_string(),
        instruction_hash: stable_value_hash(&json!({
            "instruction_key": USER_PROMPT_IMPROVER_KEY,
            "instruction": instruction,
        })),
        user_prompt_hash: stable_value_hash(&json!({
            "user_prompt": user_prompt,
        })),
        improved_prompt,
        model_calls: 1,
    })
}

pub fn serve_user_prompt_improver_with_head<I: HeadInvoker>(
    registry: &InstructionKeyRegistry,
    config: &ConfigState,
    user_prompt: &str,
    head: ResolvedAgentHead,
    created_at: impl Into<String>,
    invoker: &I,
) -> Result<UserPromptImproverReceipt, GepaProposalError> {
    let created_at = created_at.into();
    serve_user_prompt_improver(registry, config, user_prompt, |request| {
        let invocation = HeadInvocationRequest::new(
            head.clone(),
            HeadInvocationKind::Synthesis,
            prompt_improver_task(&request.user_prompt),
            0,
            Vec::new(),
            Vec::new(),
            created_at.clone(),
        )
        .with_head_system_prompt(request.instruction);
        invoker
            .invoke(invocation)
            .map(|receipt| prompt_improver_text(&receipt))
            .map_err(|error| error.to_string())
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GepaTrainSession {
    pub session_id: String,
    pub intent_id: String,
    pub input: Value,
    pub trace: AgentRunState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Value>,
    pub feedback: FeedbackPoint,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrainExample {
    pub intent_id: String,
    pub input: Value,
    pub trace: AgentRunState,
    pub outcome: Value,
    pub feedback: String,
    pub score: f64,
    pub axes: BTreeMap<String, f64>,
}

pub fn export_trainset_for_intent(
    sessions: &[GepaTrainSession],
    intent_id: &str,
) -> Result<Vec<TrainExample>, GepaProposalError> {
    sessions
        .iter()
        .filter(|session| session.intent_id == intent_id)
        .map(|session| {
            let outcome =
                session
                    .outcome
                    .clone()
                    .ok_or_else(|| GepaProposalError::MissingOutcome {
                        intent_id: session.intent_id.clone(),
                        session_id: session.session_id.clone(),
                    })?;
            Ok(TrainExample {
                intent_id: session.intent_id.clone(),
                input: session.input.clone(),
                trace: session.trace.clone(),
                outcome,
                feedback: session.feedback.feedback.clone(),
                score: session.feedback.score,
                axes: session.feedback.axes.as_axis_map(),
            })
        })
        .collect()
}

pub fn trainset_jsonl(examples: &[TrainExample]) -> Result<String, serde_json::Error> {
    let mut output = String::new();
    for example in examples {
        output.push_str(&serde_json::to_string(example)?);
        output.push('\n');
    }
    Ok(output)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GepaInstructionCandidate {
    pub gepa_run_id: String,
    pub candidate_id: String,
    pub instruction_key: String,
    pub optimized_instruction: String,
    #[serde(default)]
    pub parents: Vec<String>,
    #[serde(default)]
    pub val_subscores: BTreeMap<String, f64>,
    #[serde(default)]
    pub lineage: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GepaGateRouteResult {
    pub delta: ConfigDelta,
    pub shadow: ShadowEvalResult,
    pub verdict: LoopGateVerdict,
}

pub struct GepaGateRouteInput<'a> {
    pub candidate: &'a GepaInstructionCandidate,
    pub registry: &'a InstructionKeyRegistry,
    pub shadow_input: ShadowEvalInput,
    pub attribution: &'a ConfigAttributionTable,
    pub rate: &'a ImprovementRate,
    pub fitness: &'a FitnessGateResult,
}

pub fn ingest_gepa_candidate(
    registry: &InstructionKeyRegistry,
    config: &ConfigState,
    candidate: &GepaInstructionCandidate,
) -> Result<ConfigDelta, GepaProposalError> {
    let before = registry.resolve(config, &candidate.instruction_key)?;
    if candidate.optimized_instruction.trim().is_empty() {
        return Err(GepaProposalError::EmptyInstructionValue(
            candidate.instruction_key.clone(),
        ));
    }
    Ok(ConfigDelta {
        delta_id: gepa_delta_id(candidate),
        description: format!(
            "GEPA candidate {} from run {} for {}",
            candidate.candidate_id, candidate.gepa_run_id, candidate.instruction_key
        ),
        values: vec![ConfigValueDelta {
            key: candidate.instruction_key.clone(),
            before: Value::String(before),
            after: Value::String(candidate.optimized_instruction.clone()),
        }],
        graph_version_before: config.graph_version_id.clone(),
        graph_version_after: None,
    })
}

pub fn route_gepa_candidate_through_gate(
    state: &mut LoopGateState,
    ledger: &mut ConfigLedger,
    config: &mut ConfigState,
    input: GepaGateRouteInput<'_>,
) -> Result<GepaGateRouteResult, GepaProposalError> {
    let delta = ingest_gepa_candidate(input.registry, config, input.candidate)?;
    let attribution_key = delta
        .values
        .first()
        .map(|value| value.key.clone())
        .expect("GEPA instruction deltas always carry one value");
    let mut shadow_input = input.shadow_input;
    shadow_input.delta_id = delta.delta_id.clone();
    let shadow = evaluate_shadow(shadow_input);
    let verdict = close_loop_if_allowed(
        state,
        ledger,
        config,
        LoopClosureInput {
            delta: delta.clone(),
            attribution_key: &attribution_key,
            attribution: input.attribution,
            shadow: &shadow,
            rate: input.rate,
            fitness: input.fitness,
        },
    )?;
    Ok(GepaGateRouteResult {
        delta,
        shadow,
        verdict,
    })
}

pub fn gepa_val_subscores(points: &[(String, FeedbackPoint)]) -> BTreeMap<String, f64> {
    points
        .iter()
        .map(|(instance_id, point)| (instance_id.clone(), point.score))
        .collect()
}

fn default_instruction_specs() -> Vec<InstructionKeySpec> {
    vec![
        InstructionKeySpec::new(
            USER_PROMPT_IMPROVER_KEY,
            "User prompt improver served as one cheap rewrite before downstream execution.",
            USER_PROMPT_IMPROVER_SEED_INSTRUCTION,
            InstructionSource::UserPromptImprover,
        )
        .unwrap(),
        InstructionKeySpec::new(
            HEAD_SYSTEM_CORE_KEY,
            "Shared composed-agent head system prompt core.",
            THEOREM_HEAD_SYSTEM_PROMPT_CORE,
            InstructionSource::HeadSystemCore,
        )
        .unwrap(),
        InstructionKeySpec::new(
            HEAD_ROLE_PROPOSAL_KEY,
            "Instruction for proposal head invocations.",
            PROPOSAL_ROLE_INSTRUCTION,
            InstructionSource::HeadRole {
                role: "proposal".to_string(),
            },
        )
        .unwrap(),
        InstructionKeySpec::new(
            HEAD_ROLE_CRITIQUE_KEY,
            "Instruction for critique head invocations.",
            CRITIQUE_ROLE_INSTRUCTION,
            InstructionSource::HeadRole {
                role: "critique".to_string(),
            },
        )
        .unwrap(),
        InstructionKeySpec::new(
            HEAD_ROLE_SYNTHESIS_KEY,
            "Instruction for synthesis head invocations.",
            SYNTHESIS_ROLE_INSTRUCTION,
            InstructionSource::HeadRole {
                role: "synthesis".to_string(),
            },
        )
        .unwrap(),
        InstructionKeySpec::new(
            HEAD_ROLE_VERIFICATION_KEY,
            "Instruction for verification head invocations.",
            VERIFICATION_ROLE_INSTRUCTION,
            InstructionSource::HeadRole {
                role: "verification".to_string(),
            },
        )
        .unwrap(),
        InstructionKeySpec::new(
            HEAD_ADDENDUM_FAST_FIRST_KEY,
            "Instruction addendum for fast-first heads.",
            FAST_FIRST_HEAD_PROMPT_ADDENDUM,
            InstructionSource::HeadAddendum {
                name: "fast_first".to_string(),
            },
        )
        .unwrap(),
        InstructionKeySpec::new(
            HEAD_ADDENDUM_VERIFIER_KEY,
            "Instruction addendum for verifier heads.",
            VERIFIER_HEAD_PROMPT_ADDENDUM,
            InstructionSource::HeadAddendum {
                name: "verifier".to_string(),
            },
        )
        .unwrap(),
        InstructionKeySpec::new(
            HEAD_ADDENDUM_MODALITY_KEY,
            "Instruction addendum for modality-specialized heads.",
            MODALITY_HEAD_PROMPT_ADDENDUM,
            InstructionSource::HeadAddendum {
                name: "modality".to_string(),
            },
        )
        .unwrap(),
    ]
}

fn validate_instruction_spec(spec: &InstructionKeySpec) -> Result<(), GepaProposalError> {
    let key = spec.key.trim();
    if key.is_empty() {
        return Err(GepaProposalError::EmptyInstructionKey);
    }
    if !key.starts_with(INSTRUCTION_KEY_PREFIX) {
        return Err(GepaProposalError::NonInstructionKey(spec.key.clone()));
    }
    if spec.default_value.trim().is_empty() {
        return Err(GepaProposalError::EmptyInstructionValue(spec.key.clone()));
    }
    Ok(())
}

fn gepa_delta_id(candidate: &GepaInstructionCandidate) -> String {
    format!(
        "gepa:{}:{}",
        sanitize_id_part(&candidate.gepa_run_id),
        sanitize_id_part(&candidate.candidate_id)
    )
}

fn sanitize_id_part(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        let mut encoded = String::new();
        for byte in trimmed.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') {
                encoded.push(byte as char);
            } else {
                write!(&mut encoded, "~{byte:02X}").expect("write to string");
            }
        }
        encoded
    }
}

fn is_fast_first_head(head: &ResolvedAgentHead) -> bool {
    let identity = format!(
        "{} {} {} {}",
        head.head_id, head.display_name, head.provider, head.model
    )
    .to_ascii_lowercase();
    identity.contains("flash")
        || head
            .capabilities
            .iter()
            .any(|capability| matches_capability(capability, &["fast_first", "low_latency"]))
}

fn is_modality_head(head: &ResolvedAgentHead) -> bool {
    head.kind == HeadKind::SkillPlugin
        || head.capabilities.iter().any(|capability| {
            matches_capability(
                capability,
                &[
                    "ocr",
                    "vision",
                    "transcription",
                    "audio",
                    "image_generation",
                    "generation",
                ],
            )
        })
}

fn matches_capability(capability: &str, needles: &[&str]) -> bool {
    let normalized = capability.trim().to_ascii_lowercase();
    needles.iter().any(|needle| normalized.contains(needle))
}

fn prompt_improver_task(user_prompt: &str) -> String {
    format!(
        "Improve the following user prompt for downstream execution. Return only the improved prompt.\n\nUser prompt:\n{user_prompt}"
    )
}

fn prompt_improver_text(receipt: &HeadInvocationReceipt) -> String {
    ["text", "content", "improved_prompt", "output"]
        .iter()
        .find_map(|key| {
            receipt
                .payload
                .get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| receipt.output_summary.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_binding::{HeadCostProfile, HeadReliabilityProfile, HeadTransport, TraceTier};
    use crate::agent_head_registry::AgentHeadEndpoint;
    use crate::attribution::{ConfigAttributionTable, ConfigRunAttribution};
    use crate::epistemic_fitness::{
        check_epistemic_fitness, measure_fitness_traits, FitnessObservation,
    };
    use crate::gepa_feedback::{gepa_feedback_point, GepaFeedbackInput, ReservoirFeedback};
    use crate::improvement_rate::{composite_point, compute_improvement_rate};
    use crate::session_metrics::SessionMetricsState;
    use crate::shadow_eval::ShadowEvalInput;
    use crate::types::{AgentStepState, Payload};
    use crate::FakeHeadInvoker;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn registry_resolves_and_seeds_instruction_values() {
        let registry = default_gepa_instruction_registry();
        let mut config = ConfigState::default();

        registry.seed_config_state(&mut config).unwrap();

        assert_eq!(
            registry
                .resolve(&config, HEAD_SYSTEM_CORE_KEY)
                .unwrap()
                .as_str(),
            THEOREM_HEAD_SYSTEM_PROMPT_CORE
        );
        config
            .values
            .insert(HEAD_SYSTEM_CORE_KEY.to_string(), json!("configured core"));
        assert_eq!(
            registry.resolve(&config, HEAD_SYSTEM_CORE_KEY).unwrap(),
            "configured core"
        );
        assert_eq!(
            registry
                .seed_candidate(&config, HEAD_SYSTEM_CORE_KEY)
                .unwrap()
                .get(HEAD_SYSTEM_CORE_KEY),
            Some(&"configured core".to_string())
        );
    }

    #[test]
    fn registry_rejects_non_instruction_tuning_keys() {
        let error = InstructionKeySpec::new(
            "routing.weight.codex",
            "numeric tuning key",
            "0.6",
            InstructionSource::UserPromptImprover,
        )
        .unwrap_err();

        assert_eq!(
            error,
            GepaProposalError::NonInstructionKey("routing.weight.codex".to_string())
        );
    }

    #[test]
    fn seeded_config_reproduces_default_head_prompt() {
        let registry = default_gepa_instruction_registry();
        let mut config = ConfigState::default();
        registry.seed_config_state(&mut config).unwrap();
        let head = resolved_head(
            "gemini-flash",
            HeadKind::ReasoningCore,
            vec!["fast_first".to_string(), "rust".to_string()],
        );

        let configured =
            configured_head_system_prompt(&registry, &config, &head, HeadInvocationKind::Proposal)
                .unwrap();

        assert_eq!(
            configured,
            crate::head_invocation::default_head_system_prompt(&head, HeadInvocationKind::Proposal)
        );
    }

    #[test]
    fn configured_head_prompt_reads_overridden_instruction_values() {
        let registry = default_gepa_instruction_registry();
        let mut config = ConfigState::default();
        registry.seed_config_state(&mut config).unwrap();
        config.values.insert(
            HEAD_ROLE_SYNTHESIS_KEY.to_string(),
            json!("merge the strongest grounded answer from the shared document."),
        );
        let head = resolved_head("synthesis", HeadKind::ReasoningCore, Vec::new());

        let configured =
            configured_head_system_prompt(&registry, &config, &head, HeadInvocationKind::Synthesis)
                .unwrap();

        assert!(configured.contains("merge the strongest grounded answer"));
        assert!(!configured.contains(SYNTHESIS_ROLE_INSTRUCTION));
    }

    #[test]
    fn prompt_improver_serves_one_forward_pass_from_config() {
        let registry = default_gepa_instruction_registry();
        let mut config = ConfigState::default();
        registry.seed_config_state(&mut config).unwrap();
        config.values.insert(
            USER_PROMPT_IMPROVER_KEY.to_string(),
            json!("Rewrite into a precise implementation request."),
        );
        let mut calls = 0;

        let receipt =
            serve_user_prompt_improver(&registry, &config, "  make it work  ", |request| {
                calls += 1;
                assert_eq!(
                    request.instruction,
                    "Rewrite into a precise implementation request."
                );
                assert_eq!(request.user_prompt, "make it work");
                Ok("Implement the requested behavior and validate it.".to_string())
            })
            .unwrap();

        assert_eq!(calls, 1);
        assert_eq!(receipt.model_calls, 1);
        assert_eq!(
            receipt.improved_prompt,
            "Implement the requested behavior and validate it."
        );
    }

    #[test]
    fn prompt_improver_can_be_served_by_one_head_invocation() {
        let registry = default_gepa_instruction_registry();
        let mut config = ConfigState::default();
        registry.seed_config_state(&mut config).unwrap();
        let head = resolved_head("synthesis", HeadKind::ReasoningCore, Vec::new());

        let receipt = serve_user_prompt_improver_with_head(
            &registry,
            &config,
            "make this clearer",
            head,
            "2026-06-30T00:00:00Z",
            &FakeHeadInvoker::default(),
        )
        .unwrap();

        assert_eq!(receipt.model_calls, 1);
        assert_eq!(receipt.improved_prompt, "fake synthesis");
    }

    #[test]
    fn trainset_export_requires_captured_outcome() {
        let session = train_session("intent:prompt", "session:1", Some(json!("rewritten")));
        let examples = export_trainset_for_intent(&[session], "intent:prompt").unwrap();

        assert_eq!(examples.len(), 1);
        assert!(trainset_jsonl(&examples)
            .unwrap()
            .contains("\"intent_id\":\"intent:prompt\""));

        let missing = train_session("intent:prompt", "session:2", None);
        assert_eq!(
            export_trainset_for_intent(&[missing], "intent:prompt").unwrap_err(),
            GepaProposalError::MissingOutcome {
                intent_id: "intent:prompt".to_string(),
                session_id: "session:2".to_string()
            }
        );
    }

    #[test]
    fn candidate_ingestion_produces_preconditioned_delta_and_inverse() {
        let registry = default_gepa_instruction_registry();
        let mut config = ConfigState {
            values: BTreeMap::new(),
            graph_version_id: Some("graph:v1".to_string()),
        };
        registry.seed_config_state(&mut config).unwrap();
        let candidate = prompt_candidate("run:1", "cand:1", "Rewrite crisply.");

        let delta = ingest_gepa_candidate(&registry, &config, &candidate).unwrap();

        assert_eq!(delta.delta_id, "gepa:run~3A1:cand~3A1");
        assert_eq!(delta.values[0].key, USER_PROMPT_IMPROVER_KEY);
        assert_eq!(
            delta.values[0].before,
            json!(USER_PROMPT_IMPROVER_SEED_INSTRUCTION)
        );
        assert_eq!(delta.values[0].after, json!("Rewrite crisply."));
        assert_eq!(delta.inverse().values[0].before, json!("Rewrite crisply."));
    }

    #[test]
    fn candidate_delta_id_encodes_delimiters_inside_parts() {
        let first = prompt_candidate("run:1", "cand:2", "Rewrite crisply.");
        let second = prompt_candidate("run", "1:cand:2", "Rewrite crisply.");

        assert_eq!(gepa_delta_id(&first), "gepa:run~3A1:cand~3A2");
        assert_eq!(gepa_delta_id(&second), "gepa:run:1~3Acand~3A2");
        assert_ne!(gepa_delta_id(&first), gepa_delta_id(&second));
    }

    #[test]
    fn gate_route_rejects_productivity_positive_source_collapse() {
        let registry = default_gepa_instruction_registry();
        let mut config = ConfigState::default();
        registry.seed_config_state(&mut config).unwrap();
        let original_config = config.clone();
        let mut ledger = ConfigLedger::default();
        let mut gate_state = LoopGateState::default();
        let candidate = prompt_candidate("run:1", "cand:source-collapse", "Use one source only.");
        let attribution = ConfigAttributionTable::credit_runs(&[ConfigRunAttribution {
            config_keys: vec![USER_PROMPT_IMPROVER_KEY.to_string()],
            composite_delta: 0.2,
        }]);
        let rate = stable_rate();
        let before = vec![
            obs("source:a", 0),
            obs("source:b", 1_000),
            obs("source:c", 2_000),
        ];
        let after = vec![
            obs("source:a", 0),
            obs("source:a", 1_000),
            obs("source:a", 2_000),
        ];
        let fitness = check_epistemic_fitness(
            measure_fitness_traits(&before),
            measure_fitness_traits(&after),
        );

        let result = route_gepa_candidate_through_gate(
            &mut gate_state,
            &mut ledger,
            &mut config,
            GepaGateRouteInput {
                candidate: &candidate,
                registry: &registry,
                shadow_input: improving_shadow_input(),
                attribution: &attribution,
                rate: &rate,
                fitness: &fitness,
            },
        )
        .unwrap();

        assert!(!result.verdict.accepted);
        assert_eq!(
            result.verdict.rejection,
            Some(crate::loop_gate::LoopGateRejection::FitnessDegraded)
        );
        assert!(result.shadow.composite_delta > 0.0);
        assert!(ledger.entries.is_empty());
        assert_eq!(config, original_config);
    }

    #[test]
    fn gepa_val_subscores_use_productivity_only() {
        let point = gepa_feedback_point(GepaFeedbackInput {
            metric: metric("session:1", "candidate", true, 1_000),
            fitness: None,
            shadow: None,
            reservoir: ReservoirFeedback::default(),
        });

        let subscores = gepa_val_subscores(&[("instance:1".to_string(), point.clone())]);

        assert_eq!(subscores.get("instance:1"), Some(&point.score));
    }

    fn train_session(
        intent_id: &str,
        session_id: &str,
        outcome: Option<Value>,
    ) -> GepaTrainSession {
        let mut trace = AgentRunState::new("improve prompt", "codex", Payload::new());
        trace.run_id = session_id.to_string();
        trace = trace.with_step(
            AgentStepState::new(session_id, "model_call", Payload::new()).with_step_id("step:1"),
        );
        GepaTrainSession {
            session_id: session_id.to_string(),
            intent_id: intent_id.to_string(),
            input: json!({"user_prompt": "make this better"}),
            trace,
            outcome,
            feedback: gepa_feedback_point(GepaFeedbackInput {
                metric: metric(session_id, "candidate", true, 1_000),
                fitness: None,
                shadow: None,
                reservoir: ReservoirFeedback::default(),
            }),
        }
    }

    fn prompt_candidate(
        run_id: &str,
        candidate_id: &str,
        optimized_instruction: &str,
    ) -> GepaInstructionCandidate {
        GepaInstructionCandidate {
            gepa_run_id: run_id.to_string(),
            candidate_id: candidate_id.to_string(),
            instruction_key: USER_PROMPT_IMPROVER_KEY.to_string(),
            optimized_instruction: optimized_instruction.to_string(),
            parents: Vec::new(),
            val_subscores: BTreeMap::new(),
            lineage: json!({"engine": "gepa"}),
        }
    }

    fn resolved_head(
        head_id: &str,
        kind: HeadKind,
        capabilities: Vec<String>,
    ) -> ResolvedAgentHead {
        ResolvedAgentHead {
            head_id: head_id.to_string(),
            display_name: head_id.to_string(),
            provider: "test".to_string(),
            model: head_id.to_string(),
            kind,
            endpoint: AgentHeadEndpoint {
                transport: HeadTransport::Api,
                target: "test://head".to_string(),
                fake: true,
            },
            credential_ref: "credential:test".to_string(),
            capabilities,
            cost_profile: HeadCostProfile::default(),
            reliability_profile: HeadReliabilityProfile::default(),
            allowed_tools: Vec::new(),
            trace_tier: TraceTier::default(),
        }
    }

    fn improving_shadow_input() -> ShadowEvalInput {
        ShadowEvalInput {
            delta_id: String::new(),
            baseline_metrics: (0..60)
                .map(|index| metric(&format!("baseline:{index}"), "baseline", true, 10_000))
                .collect(),
            candidate_metrics: (0..60)
                .map(|index| metric(&format!("candidate:{index}"), "candidate", true, 4_000))
                .collect(),
            run_pairs: Vec::new(),
            safety_violations: Vec::new(),
            baseline_mode: "baseline".to_string(),
            candidate_mode: "candidate".to_string(),
        }
    }

    fn stable_rate() -> ImprovementRate {
        let points = [0.1, 0.2, 0.3, 0.4, 0.5]
            .into_iter()
            .enumerate()
            .map(|(index, value)| composite_point(format!("p{index}"), value, None))
            .collect::<Vec<_>>();
        compute_improvement_rate(&points, 5).unwrap()
    }

    fn metric(session_id: &str, mode: &str, complete: bool, tokens: i64) -> SessionMetricsState {
        SessionMetricsState {
            total_input_tokens: tokens / 2,
            total_output_tokens: tokens - (tokens / 2),
            total_tool_calls: 2,
            task_completion: complete,
            pairformer_mode: mode.to_string(),
            task_category: "godel".to_string(),
            workstream_id: "gepa".to_string(),
            session_id: session_id.to_string(),
            total_tokens: tokens,
        }
    }

    fn obs(source_id: &str, observed_at_ms: i64) -> FitnessObservation {
        FitnessObservation {
            root_depth: 4,
            source_id: source_id.to_string(),
            support_ratio: 0.8,
            claim_specificity: 0.8,
            observed_at_ms,
        }
    }
}
