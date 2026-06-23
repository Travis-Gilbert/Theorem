use crate::binding_store::{load_binding, persist_binding_run_result, BindingRuntimeError};
use rustyred_thg_core::GraphStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use std::fmt;
use theorem_harness_core::{
    run_intra_agent_loop_with_invoker, AgentBinding, AgentHead, BindingBudgetScope,
    BindingComposition, BindingError, BindingEventState, BindingIdentity, FakeIntraAgentLoopInput,
    GroundedClaim, HeadCostProfile, HeadInvocationError, HeadInvocationReceipt, HeadInvoker,
    HeadKind, HeadReliabilityProfile, HeadTransport, IntraAgentLoopError, ScratchpadRevision,
    TraceTier,
};

pub const DEFAULT_BINDING_ID: &str = "agent:theorem";
pub const THEOREM_AGENT_HEADS_ENV: &str = "THEOREM_AGENT_HEADS";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComposedAgentRunResult {
    pub binding_id: String,
    pub run_id: String,
    pub task: String,
    pub published_claims: Vec<GroundedClaim>,
    pub consensus_head_set: Vec<String>,
    pub alignment_verdict: Value,
    pub events: Vec<BindingEventState>,
    pub scratchpad_revisions: Vec<ScratchpadRevision>,
    pub invocation_receipts: Vec<HeadInvocationReceipt>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ComposedAgentRuntimeError {
    BindingStore(BindingRuntimeError),
    Binding(BindingError),
    Loop(IntraAgentLoopError),
    InvalidInput(String),
}

impl fmt::Display for ComposedAgentRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BindingStore(error) => write!(f, "{error}"),
            Self::Binding(error) => write!(f, "{error}"),
            Self::Loop(error) => write!(f, "{error}"),
            Self::InvalidInput(error) => write!(f, "{error}"),
        }
    }
}

impl Error for ComposedAgentRuntimeError {}

impl From<BindingRuntimeError> for ComposedAgentRuntimeError {
    fn from(value: BindingRuntimeError) -> Self {
        Self::BindingStore(value)
    }
}

impl From<BindingError> for ComposedAgentRuntimeError {
    fn from(value: BindingError) -> Self {
        Self::Binding(value)
    }
}

impl From<IntraAgentLoopError> for ComposedAgentRuntimeError {
    fn from(value: IntraAgentLoopError) -> Self {
        Self::Loop(value)
    }
}

pub type ComposedAgentRuntimeResult<T> = Result<T, ComposedAgentRuntimeError>;

pub fn run_composed_agent<S: GraphStore>(
    store: &mut S,
    binding_id: &str,
    task: &str,
    invoker: &impl HeadInvoker,
) -> ComposedAgentRuntimeResult<ComposedAgentRunResult> {
    let claims = vec![GroundedClaim::new(
        task.trim(),
        format!("binding:{binding_id}:task"),
    )];
    run_composed_agent_with_claims(store, binding_id, task, claims, invoker)
}

pub fn run_composed_agent_with_claims<S: GraphStore>(
    store: &mut S,
    binding_id: &str,
    task: &str,
    claims: Vec<GroundedClaim>,
    invoker: &impl HeadInvoker,
) -> ComposedAgentRuntimeResult<ComposedAgentRunResult> {
    let binding_id = normalize_binding_id(binding_id);
    let task = task.trim();
    if task.is_empty() {
        return Err(ComposedAgentRuntimeError::InvalidInput(
            "composed_agent_run requires task".to_string(),
        ));
    }
    let binding = match load_binding(store, &binding_id)? {
        Some(binding) => binding,
        None => default_theorem_binding(&binding_id)?,
    };
    let input = FakeIntraAgentLoopInput::new(task, claims);
    let result = run_intra_agent_loop_with_invoker(binding, input, invoker)?;
    persist_binding_run_result(store, &result.binding, &result.events)?;
    let policy_event = result
        .events
        .iter()
        .find(|event| event.event_type == "POLICY.CHECKED");
    let alignment_verdict = policy_event
        .map(|event| Value::Object(event.payload.clone()))
        .unwrap_or_else(|| json!({ "allowed": false, "reason": "policy_event_missing" }));
    let published_claims = if verdict_allowed(&alignment_verdict) {
        result
            .invocation_receipts
            .last()
            .map(|receipt| receipt.claims.clone())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(ComposedAgentRunResult {
        binding_id: binding_id.clone(),
        run_id: result.binding.lifecycle.run_id.clone(),
        task: task.to_string(),
        published_claims,
        consensus_head_set: result.binding.trace_scope.synthesis_heads.clone(),
        alignment_verdict,
        events: result.events,
        scratchpad_revisions: result.scratchpad_revisions,
        invocation_receipts: result.invocation_receipts,
    })
}

pub fn default_theorem_binding(binding_id: &str) -> Result<AgentBinding, BindingError> {
    let configured_heads = configured_agent_heads_from_env();
    let (active_head_set, heads) = if configured_heads.is_empty() {
        legacy_default_heads()
    } else {
        (
            configured_heads
                .iter()
                .map(|head| head.head_id.clone())
                .collect(),
            configured_heads,
        )
    };
    let mut binding = AgentBinding::new(
        BindingIdentity {
            agent_id: "theorem".to_string(),
            owner_id: "travis".to_string(),
            agent_name: "Theorem".to_string(),
            composition_hash: String::new(),
            version: 1,
            trust_tier: "first_party".to_string(),
            active_head_set,
        },
        BindingComposition { heads },
        BindingBudgetScope::new("theorem", 32_000.0, 2),
    )?;
    binding.lifecycle.run_id = normalize_binding_id(binding_id);
    Ok(binding)
}

fn legacy_default_heads() -> (Vec<String>, Vec<AgentHead>) {
    (
        vec!["claude".to_string(), "deepseek".to_string()],
        vec![
            head(
                "claude",
                "anthropic",
                std::env::var("ANTHROPIC_MODEL")
                    .unwrap_or_else(|_| "claude-3-5-sonnet-latest".to_string()),
                "env:ANTHROPIC_API_KEY",
                HeadTransport::Api,
            ),
            head(
                "deepseek",
                "deepseek",
                std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".to_string()),
                "env:DEEPSEEK_API_KEY",
                HeadTransport::Api,
            ),
        ],
    )
}

fn configured_agent_heads_from_env() -> Vec<AgentHead> {
    std::env::var(THEOREM_AGENT_HEADS_ENV)
        .ok()
        .map(|value| {
            split_csv(&value)
                .into_iter()
                .map(|head_id| env_configured_head(&head_id))
                .collect()
        })
        .unwrap_or_default()
}

fn env_configured_head(head_id: &str) -> AgentHead {
    let head_id = head_id.trim();
    let head_slug = env_slug(head_id);
    let provider = env_first([
        format!("THEOREM_AGENT_HEAD_{head_slug}_PROVIDER"),
        format!("{head_slug}_PROVIDER"),
    ])
    .unwrap_or_else(|| normalize_provider(head_id));
    let provider_slug = env_slug(&provider);
    let model = env_first([
        format!("THEOREM_AGENT_HEAD_{head_slug}_MODEL"),
        format!("{head_slug}_MODEL"),
        format!("{provider_slug}_MODEL"),
    ])
    .unwrap_or_else(|| default_model_for_provider(&provider));
    let credential_ref = env_first([
        format!("THEOREM_AGENT_HEAD_{head_slug}_CREDENTIAL_REF"),
        format!("THEOREM_AGENT_HEAD_{head_slug}_CREDENTIAL"),
    ])
    .unwrap_or_else(|| format!("env:{provider_slug}_API_KEY"));
    let transport = env_first([
        format!("THEOREM_AGENT_HEAD_{head_slug}_TRANSPORT"),
        format!("{head_slug}_TRANSPORT"),
    ])
    .as_deref()
    .and_then(parse_transport)
    .unwrap_or(HeadTransport::Api);
    head(head_id, &provider, model, &credential_ref, transport)
}

fn head(
    head_id: &str,
    provider: &str,
    model: impl Into<String>,
    credential_ref: &str,
    transport: HeadTransport,
) -> AgentHead {
    AgentHead {
        head_id: head_id.to_string(),
        display_name: head_id.to_string(),
        provider: provider.to_string(),
        model: model.into(),
        credential_ref: credential_ref.to_string(),
        transport,
        kind: HeadKind::ReasoningCore,
        capabilities: Vec::new(),
        cost_profile: HeadCostProfile::default(),
        reliability_profile: HeadReliabilityProfile::default(),
        allowed_tools: Vec::new(),
        trace_tier: TraceTier::Receipt,
    }
}

fn env_first<const N: usize>(names: [String; N]) -> Option<String> {
    names.into_iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn env_slug(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn normalize_provider(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "openapi" => "openai".to_string(),
        other => other.to_string(),
    }
}

fn default_model_for_provider(provider: &str) -> String {
    match normalize_provider(provider).as_str() {
        "anthropic" | "claude" => "claude-3-5-sonnet-latest".to_string(),
        "deepseek" => "deepseek-v4-flash".to_string(),
        "gemma" => "gemma3:latest".to_string(),
        "minimax" => "MiniMax-M3".to_string(),
        "mistral" => "mistral-large-latest".to_string(),
        "openai" => "gpt-4.1-mini".to_string(),
        "zhipu" => "glm-4-plus".to_string(),
        "ai21" => "jamba-large".to_string(),
        other => other.to_string(),
    }
}

fn parse_transport(value: &str) -> Option<HeadTransport> {
    match value.trim().to_ascii_lowercase().as_str() {
        "api" => Some(HeadTransport::Api),
        "mcp" => Some(HeadTransport::Mcp),
        "local" => Some(HeadTransport::Local),
        "hosted" => Some(HeadTransport::Hosted),
        _ => None,
    }
}

fn normalize_binding_id(binding_id: &str) -> String {
    let trimmed = binding_id.trim();
    if trimmed.is_empty() {
        DEFAULT_BINDING_ID.to_string()
    } else {
        trimmed.to_string()
    }
}

fn verdict_allowed(verdict: &Value) -> bool {
    verdict
        .get("allowed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

impl From<HeadInvocationError> for ComposedAgentRuntimeError {
    fn from(value: HeadInvocationError) -> Self {
        Self::Loop(IntraAgentLoopError::Invocation(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::InMemoryGraphStore;
    use std::sync::Mutex;
    use theorem_harness_core::FakeHeadInvoker;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn composed_agent_run_persists_binding_events_and_scratchpad() {
        let _env = ScopedEnv::new([]);
        let mut store = InMemoryGraphStore::new();
        let result = run_composed_agent_with_claims(
            &mut store,
            "agent:test",
            "publish",
            vec![GroundedClaim::new("grounded", "source:1")],
            &FakeHeadInvoker::default(),
        )
        .unwrap();

        assert_eq!(result.run_id, "agent:test");
        assert_eq!(result.consensus_head_set, vec!["claude", "deepseek"]);
        assert_eq!(result.events.last().unwrap().event_type, "RUN.CLOSED");
        assert!(load_binding(&store, "agent:test").unwrap().is_some());
        assert_eq!(
            crate::load_binding_events(&store, "agent:test")
                .unwrap()
                .len(),
            result.events.len()
        );
    }

    #[test]
    fn default_binding_uses_env_configured_api_heads() {
        let _env = ScopedEnv::new([
            (THEOREM_AGENT_HEADS_ENV, "mistral,openai,minimax,deepseek"),
            ("MISTRAL_MODEL", "mistral-small-latest"),
            ("OPENAI_MODEL", "gpt-4.1-mini"),
            ("MINIMAX_MODEL", "MiniMax-M3"),
            ("DEEPSEEK_MODEL", "deepseek-v4-flash"),
        ]);

        let binding = default_theorem_binding("agent:env").unwrap();

        assert_eq!(
            binding.identity.active_head_set,
            vec!["deepseek", "minimax", "mistral", "openai"]
        );
        let mistral = binding.head("mistral").unwrap();
        assert_eq!(mistral.provider, "mistral");
        assert_eq!(mistral.model, "mistral-small-latest");
        assert_eq!(mistral.credential_ref, "env:MISTRAL_API_KEY");
        assert_eq!(mistral.transport, HeadTransport::Api);
        let openai = binding.head("openai").unwrap();
        assert_eq!(openai.provider, "openai");
        assert_eq!(openai.credential_ref, "env:OPENAI_API_KEY");
        let minimax = binding.head("minimax").unwrap();
        assert_eq!(minimax.provider, "minimax");
        assert_eq!(minimax.model, "MiniMax-M3");
        assert_eq!(minimax.credential_ref, "env:MINIMAX_API_KEY");
        let deepseek = binding.head("deepseek").unwrap();
        assert_eq!(deepseek.model, "deepseek-v4-flash");
        assert_eq!(deepseek.transport, HeadTransport::Api);
    }

    #[test]
    fn env_configured_heads_support_openapi_alias_and_transport_override() {
        let _env = ScopedEnv::new([
            (THEOREM_AGENT_HEADS_ENV, "openapi,deepseek"),
            ("THEOREM_AGENT_HEAD_OPENAPI_MODEL", "gpt-4.1-mini"),
            ("THEOREM_AGENT_HEAD_DEEPSEEK_TRANSPORT", "mcp"),
        ]);

        let binding = default_theorem_binding("agent:env-alias").unwrap();

        let openapi = binding.head("openapi").unwrap();
        assert_eq!(openapi.provider, "openai");
        assert_eq!(openapi.model, "gpt-4.1-mini");
        assert_eq!(openapi.credential_ref, "env:OPENAI_API_KEY");
        let deepseek = binding.head("deepseek").unwrap();
        assert_eq!(deepseek.transport, HeadTransport::Mcp);
    }

    #[test]
    #[ignore = "requires THEOREM_LIVE_PROVIDER_TEST=1 and real provider keys"]
    fn live_provider_invoker_runs_three_head_binding_when_enabled() {
        if std::env::var("THEOREM_LIVE_PROVIDER_TEST").ok().as_deref() != Some("1") {
            eprintln!("set THEOREM_LIVE_PROVIDER_TEST=1 with live provider keys to run");
            return;
        }
        let configured_heads = std::env::var(THEOREM_AGENT_HEADS_ENV)
            .expect("set THEOREM_AGENT_HEADS=deepseek,mistral,minimax");
        assert!(
            configured_heads.contains("deepseek")
                && configured_heads.contains("mistral")
                && configured_heads.contains("minimax"),
            "THEOREM_AGENT_HEADS must include deepseek,mistral,minimax"
        );
        let mut store = InMemoryGraphStore::new();
        let invoker = crate::ProviderHeadInvoker::from_env().unwrap();

        let result = run_composed_agent_with_claims(
            &mut store,
            "agent:live-provider-test",
            "Return one grounded claim that this live provider test ran.",
            vec![GroundedClaim::new("live provider smoke input", "test:live")],
            &invoker,
        )
        .unwrap();

        assert!(result.invocation_receipts.len() >= 3);
        assert!(result
            .events
            .iter()
            .any(|event| event.event_type == "POLICY.CHECKED"));
        if verdict_allowed(&result.alignment_verdict) {
            assert!(!result.published_claims.is_empty());
        } else {
            assert!(result.published_claims.is_empty());
        }
    }

    struct ScopedEnv {
        saved: Vec<(String, Option<String>)>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl ScopedEnv {
        fn new<const N: usize>(pairs: [(&'static str, &'static str); N]) -> Self {
            let guard = ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut names = vec![
                THEOREM_AGENT_HEADS_ENV.to_string(),
                "MISTRAL_MODEL".to_string(),
                "OPENAI_MODEL".to_string(),
                "MINIMAX_MODEL".to_string(),
                "DEEPSEEK_MODEL".to_string(),
                "THEOREM_AGENT_HEAD_OPENAPI_MODEL".to_string(),
                "THEOREM_AGENT_HEAD_DEEPSEEK_TRANSPORT".to_string(),
            ];
            for (name, _) in pairs {
                names.push(name.to_string());
            }
            names.sort();
            names.dedup();
            let saved = names
                .into_iter()
                .map(|name| {
                    let value = std::env::var(&name).ok();
                    std::env::remove_var(&name);
                    (name, value)
                })
                .collect::<Vec<_>>();
            for (name, value) in pairs {
                std::env::set_var(name, value);
            }
            Self {
                saved,
                _guard: guard,
            }
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            for (name, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}
