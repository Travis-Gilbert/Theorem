pub mod api;
pub mod credentials;
pub mod mcp;

pub use api::{api_provider_profile, default_api_profiles, ApiProviderProfile, ApiRequestShape};
pub use credentials::{CredentialResolutionError, CredentialResolver};
pub use mcp::mcp_tool_name;

use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;
use theorem_harness_core::{
    default_head_system_prompt, prompt_instruction_key, HeadInvocationError, HeadInvocationKind,
    HeadInvocationReceipt, HeadInvocationRequest, HeadInvoker, HeadKind, HeadTransport,
};
use theorem_prompt::{MarkerRenderer, PromptSpec, Renderer};

const DEFAULT_PROVIDER_HEAD_COST_UNITS: f64 = 1.0;
const DEFAULT_LOCAL_OPENAI_CHAT_URL: &str = "http://127.0.0.1:8080/v1/chat/completions";

#[derive(Clone, Debug, Default)]
pub struct EndpointMap {
    endpoints: BTreeMap<String, String>,
}

impl EndpointMap {
    pub fn from_env() -> Self {
        let mut map = Self::default();
        for profile in default_api_profiles() {
            let endpoint = std::env::var(profile.env_endpoint)
                .unwrap_or_else(|_| profile.default_endpoint.to_string());
            map.insert(profile.provider, &HeadTransport::Api, endpoint);
        }
        if let Ok(endpoint) = std::env::var("DEEPSEEK_MCP_URL") {
            map.insert("deepseek", &HeadTransport::Mcp, endpoint);
        }
        let local_endpoint = std::env::var("THEOREM_LOCAL_OPENAI_URL")
            .unwrap_or_else(|_| DEFAULT_LOCAL_OPENAI_CHAT_URL.to_string());
        map.insert("local", &HeadTransport::Local, local_endpoint.clone());
        map.insert("gemma", &HeadTransport::Local, local_endpoint);
        if let Some(hosted_endpoint) = hosted_openai_endpoint_from_env() {
            map.insert("hosted", &HeadTransport::Hosted, hosted_endpoint);
        }
        map
    }

    pub fn insert(
        &mut self,
        provider: impl AsRef<str>,
        transport: &HeadTransport,
        endpoint: impl Into<String>,
    ) {
        self.endpoints
            .insert(endpoint_key(provider.as_ref(), transport), endpoint.into());
    }

    pub fn get(&self, provider: &str, transport: &HeadTransport) -> Option<&str> {
        let endpoint = self
            .endpoints
            .get(&endpoint_key(provider, transport))
            .map(String::as_str);
        if endpoint.is_some() {
            return endpoint;
        }
        match transport {
            HeadTransport::Local => self
                .endpoints
                .get(&endpoint_key("local", transport))
                .map(String::as_str),
            HeadTransport::Hosted => self
                .endpoints
                .get(&endpoint_key("hosted", transport))
                .map(String::as_str),
            HeadTransport::Api | HeadTransport::Mcp => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RealHeadInvoker {
    http: reqwest::blocking::Client,
    credentials: CredentialResolver,
    endpoints: EndpointMap,
    fallback_cost_units: f64,
}

impl RealHeadInvoker {
    pub fn from_env() -> Result<Self, HeadInvocationError> {
        Self::new(EndpointMap::from_env())
    }

    pub fn new(endpoints: EndpointMap) -> Result<Self, HeadInvocationError> {
        Self::with_credentials(endpoints, CredentialResolver::new())
    }

    pub fn with_credentials(
        endpoints: EndpointMap,
        credentials: CredentialResolver,
    ) -> Result<Self, HeadInvocationError> {
        Self::with_credentials_and_cost_units(
            endpoints,
            credentials,
            configured_provider_head_cost_units(),
        )
    }

    pub fn with_credentials_and_cost_units(
        endpoints: EndpointMap,
        credentials: CredentialResolver,
        fallback_cost_units: f64,
    ) -> Result<Self, HeadInvocationError> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|error| HeadInvocationError::ProviderError {
                head_id: "real-head-invoker".to_string(),
                provider: "reqwest".to_string(),
                status: 0,
                detail: error.to_string(),
            })?;
        Ok(Self {
            http,
            credentials,
            endpoints,
            fallback_cost_units,
        })
    }
}

impl HeadInvoker for RealHeadInvoker {
    fn invoke(
        &self,
        request: HeadInvocationRequest,
    ) -> Result<HeadInvocationReceipt, HeadInvocationError> {
        if request.task.trim().is_empty() {
            return Err(HeadInvocationError::EmptyTask {
                head_id: request.head.head_id,
                kind: request.kind,
            });
        }
        if request.head.kind == HeadKind::SkillPlugin {
            return Err(HeadInvocationError::SkillPluginDenied {
                head_id: request.head.head_id,
            });
        }

        match request.head.endpoint.transport {
            HeadTransport::Api => api::invoke_api_head(
                &self.http,
                &self.endpoints,
                &self.credentials,
                self.fallback_cost_units,
                request,
            ),
            HeadTransport::Mcp => mcp::invoke_mcp_head(
                &self.http,
                &self.endpoints,
                &self.credentials,
                self.fallback_cost_units,
                request,
            ),
            HeadTransport::Local | HeadTransport::Hosted => invoke_openai_compatible_transport(
                &self.http,
                &self.endpoints,
                &self.credentials,
                self.fallback_cost_units,
                request,
            ),
        }
    }
}

pub type ProviderHeadInvoker = RealHeadInvoker;

pub(crate) fn prompt_for_request(request: &HeadInvocationRequest) -> String {
    let spec = prompt_spec_for_request(request);
    let rendered = MarkerRenderer.render(&spec);
    let mut prompt = rendered
        .messages
        .iter()
        .find(|message| message.role == "user")
        .map(|message| message.content.clone())
        .unwrap_or_default();
    prompt.push_str(
        "\nReturn a concise answer. End with a line `Claims JSON:` followed by a JSON array of objects with `text` and `provenance` fields for the claims you assert.\n",
    );
    prompt
}

pub(crate) fn prompt_spec_for_request(request: &HeadInvocationRequest) -> PromptSpec {
    PromptSpec::from_request(
        request,
        prompt_instruction_key(&request.head.kind, request.kind),
        system_instruction_for_request(request),
    )
}

pub(crate) fn system_instruction_for_request(request: &HeadInvocationRequest) -> String {
    if request.head_system_prompt.trim().is_empty() {
        default_head_system_prompt(&request.head, request.kind)
    } else {
        request.head_system_prompt.clone()
    }
}

pub(crate) fn provider_send_error(
    request: &HeadInvocationRequest,
    error: reqwest::Error,
) -> HeadInvocationError {
    if error.is_timeout() {
        HeadInvocationError::Timeout {
            head_id: request.head.head_id.clone(),
            provider: request.head.provider.clone(),
        }
    } else {
        HeadInvocationError::ProviderError {
            head_id: request.head.head_id.clone(),
            provider: request.head.provider.clone(),
            status: error.status().map(|status| status.as_u16()).unwrap_or(0),
            detail: error.to_string(),
        }
    }
}

pub(crate) fn provider_summary(_kind: HeadInvocationKind, text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .chars()
        .take(280)
        .collect()
}

pub(crate) fn object_payload(value: Value) -> serde_json::Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    }
}

pub(crate) fn truncate_detail(detail: &str) -> String {
    detail.chars().take(512).collect()
}

fn endpoint_key(provider: &str, transport: &HeadTransport) -> String {
    format!(
        "{}:{}",
        provider.trim().to_ascii_lowercase(),
        transport_slug(transport)
    )
}

fn transport_slug(transport: &HeadTransport) -> &'static str {
    match transport {
        HeadTransport::Api => "api",
        HeadTransport::Mcp => "mcp",
        HeadTransport::Local => "local",
        HeadTransport::Hosted => "hosted",
    }
}

fn configured_provider_head_cost_units() -> f64 {
    std::env::var("THEOREM_PROVIDER_HEAD_COST_UNITS")
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(DEFAULT_PROVIDER_HEAD_COST_UNITS)
}

fn hosted_openai_endpoint_from_env() -> Option<String> {
    std::env::var("THEOREM_HOSTED_OPENAI_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("THEOREM_LITELLM_CHAT_URL")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            std::env::var("THEOREM_LITELLM_BASE_URL")
                .ok()
                .map(|value| chat_completions_endpoint(&value))
                .filter(|value| !value.is_empty())
        })
}

fn chat_completions_endpoint(base_url: &str) -> String {
    let base_url = base_url.trim().trim_end_matches('/');
    if base_url.ends_with("/chat/completions") {
        base_url.to_string()
    } else if base_url.ends_with("/v1") {
        format!("{base_url}/chat/completions")
    } else {
        format!("{base_url}/v1/chat/completions")
    }
}

fn invoke_openai_compatible_transport(
    http: &reqwest::blocking::Client,
    endpoints: &EndpointMap,
    credentials: &CredentialResolver,
    fallback_cost_units: f64,
    request: HeadInvocationRequest,
) -> Result<HeadInvocationReceipt, HeadInvocationError> {
    let transport = request.head.endpoint.transport.clone();
    let Some(endpoint) = endpoints.get(&request.head.provider, &transport) else {
        return Err(HeadInvocationError::ProviderError {
            head_id: request.head.head_id.clone(),
            provider: request.head.provider.clone(),
            status: 0,
            detail: format!(
                "missing endpoint for provider {} over {:?}",
                request.head.provider, transport
            ),
        });
    };
    let api_key = match transport {
        HeadTransport::Local => credentials
            .resolve_optional_local(&request.head.credential_ref)
            .map_err(|error| HeadInvocationError::ProviderError {
                head_id: request.head.head_id.clone(),
                provider: request.head.provider.clone(),
                status: 0,
                detail: error.detail(),
            })?,
        HeadTransport::Hosted => Some(credentials.resolve(&request.head.credential_ref).map_err(
            |error| HeadInvocationError::ProviderError {
                head_id: request.head.head_id.clone(),
                provider: request.head.provider.clone(),
                status: 0,
                detail: error.detail(),
            },
        )?),
        HeadTransport::Api | HeadTransport::Mcp => unreachable!("handled by caller"),
    };
    api::invoke_openai_chat_completions(
        http,
        endpoint,
        api_key,
        transport,
        fallback_cost_units,
        request,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use theorem_harness_core::{
        AgentHeadEndpoint, ContextMembranePrime, HeadCostProfile, HeadKind, HeadReliabilityProfile,
        ResolvedAgentHead, TraceTier,
    };

    #[test]
    fn credential_resolver_supports_env_and_secret_store_refs() {
        std::env::set_var("TEST_HEAD_INVOKER_KEY", "env-secret");
        let resolver = CredentialResolver::new().with_secret("runtime-key", "secret-value");

        assert_eq!(
            resolver.resolve("env:TEST_HEAD_INVOKER_KEY").unwrap(),
            "env-secret"
        );
        assert_eq!(
            resolver.resolve("secret:runtime-key").unwrap(),
            "secret-value"
        );
        assert_eq!(
            resolver.resolve("secret-store:runtime-key").unwrap(),
            "secret-value"
        );
        std::env::remove_var("TEST_HEAD_INVOKER_KEY");
    }

    #[test]
    fn endpoint_map_registers_default_api_profiles() {
        let endpoints = EndpointMap::from_env();

        assert!(endpoints.get("anthropic", &HeadTransport::Api).is_some());
        assert!(endpoints.get("minimax", &HeadTransport::Api).is_some());
        assert!(endpoints.get("gemma", &HeadTransport::Api).is_some());
    }

    #[test]
    fn prompt_includes_prior_revision_content() {
        let request = HeadInvocationRequest::new_with_context(
            head("anthropic", HeadTransport::Api),
            HeadInvocationKind::Critique,
            "review",
            2,
            vec!["scratchrev:1".to_string()],
            vec![theorem_harness_core::RevisionContext {
                revision_id: "scratchrev:1".to_string(),
                kind: HeadInvocationKind::Proposal.as_str().to_string(),
                output_summary: "proposal".to_string(),
                payload: object_payload(json!({ "text": "proposal body" })),
            }],
            Vec::new(),
            "2026-06-08T00:00:00Z",
        )
        .with_context_membrane(vec![ContextMembranePrime::new(
            "context:ambient",
            "ambient intelligence",
            "scope was primed at run start",
            "test:prompt",
            0.75,
        )]);

        let prompt = prompt_for_request(&request);

        assert!(prompt.contains("scratchrev:1"));
        assert!(prompt.contains("proposal body"));
        assert!(prompt.contains("Shared CRDT scratchpad"));
        assert!(prompt.contains("scratchpad.crdt.scratchpad_default"));
        assert!(prompt.contains("Context membrane primes"));
        assert!(prompt.contains("context:ambient"));
    }

    fn head(provider: &str, transport: HeadTransport) -> ResolvedAgentHead {
        ResolvedAgentHead {
            head_id: provider.to_string(),
            display_name: provider.to_string(),
            provider: provider.to_string(),
            model: "model".to_string(),
            kind: HeadKind::ReasoningCore,
            endpoint: AgentHeadEndpoint {
                transport,
                target: "fake://target".to_string(),
                fake: true,
            },
            credential_ref: "env:TEST_PROVIDER_KEY".to_string(),
            capabilities: Vec::new(),
            cost_profile: HeadCostProfile::default(),
            reliability_profile: HeadReliabilityProfile::default(),
            allowed_tools: Vec::new(),
            trace_tier: TraceTier::Receipt,
        }
    }
}
