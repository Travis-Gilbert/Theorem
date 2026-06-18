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
    HeadInvocationError, HeadInvocationKind, HeadInvocationReceipt, HeadInvocationRequest,
    HeadInvoker, HeadTransport,
};

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
        self.endpoints
            .get(&endpoint_key(provider, transport))
            .map(String::as_str)
    }
}

#[derive(Clone, Debug)]
pub struct RealHeadInvoker {
    http: reqwest::blocking::Client,
    credentials: CredentialResolver,
    endpoints: EndpointMap,
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

        match request.head.endpoint.transport {
            HeadTransport::Api => {
                api::invoke_api_head(&self.http, &self.endpoints, &self.credentials, request)
            }
            HeadTransport::Mcp => {
                mcp::invoke_mcp_head(&self.http, &self.endpoints, &self.credentials, request)
            }
            HeadTransport::Local | HeadTransport::Hosted => {
                Err(HeadInvocationError::ProviderError {
                    head_id: request.head.head_id.clone(),
                    provider: request.head.provider.clone(),
                    status: 0,
                    detail: format!(
                        "unsupported provider transport: {} over {:?}",
                        request.head.provider, request.head.endpoint.transport
                    ),
                })
            }
        }
    }
}

pub type ProviderHeadInvoker = RealHeadInvoker;

pub(crate) fn prompt_for_request(request: &HeadInvocationRequest) -> String {
    let mut prompt = format!(
        "Task:\n{}\n\nInvocation kind: {}\n",
        request.task,
        request.kind.as_str()
    );
    if !request.prior_context.is_empty() {
        prompt.push_str("\nPrior revisions:\n");
        for context in &request.prior_context {
            prompt.push_str(&format!(
                "- {} ({}) {}\n",
                context.revision_id,
                context.kind.as_str(),
                context.output_summary
            ));
            if let Some(text) = context.payload.get("text").and_then(Value::as_str) {
                prompt.push_str(text);
                prompt.push('\n');
            }
        }
    }
    if !request.claims.is_empty() {
        prompt.push_str("\nGrounding claims required:\n");
        for claim in &request.claims {
            prompt.push_str(&format!("- {} [{}]\n", claim.text, claim.provenance));
        }
    }
    prompt
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

pub(crate) fn provider_summary(kind: HeadInvocationKind, text: &str) -> String {
    let clipped = text.chars().take(96).collect::<String>();
    format!("provider {}: {}", kind.as_str(), clipped)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use theorem_harness_core::{
        AgentHeadEndpoint, HeadCostProfile, HeadKind, HeadReliabilityProfile, ResolvedAgentHead,
        TraceTier,
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
                kind: HeadInvocationKind::Proposal,
                output_summary: "proposal".to_string(),
                payload: object_payload(json!({ "text": "proposal body" })),
            }],
            Vec::new(),
            "2026-06-08T00:00:00Z",
        );

        let prompt = prompt_for_request(&request);

        assert!(prompt.contains("scratchrev:1"));
        assert!(prompt.contains("proposal body"));
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
