use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use theorem_harness_core::{
    HeadInvocationError, HeadInvocationKind, HeadInvocationReceipt, HeadInvocationRequest,
    HeadInvoker, HeadTransport,
};

#[derive(Clone, Debug, Default)]
pub struct CredentialResolver;

impl CredentialResolver {
    pub fn resolve(&self, credential_ref: &str) -> Result<String, String> {
        let Some(env_name) = credential_ref.trim().strip_prefix("env:") else {
            return Err(format!(
                "unsupported credential reference {credential_ref}; expected env:NAME"
            ));
        };
        std::env::var(env_name).map_err(|_| format!("missing environment credential {env_name}"))
    }
}

#[derive(Clone, Debug, Default)]
pub struct EndpointMap {
    endpoints: BTreeMap<String, String>,
}

impl EndpointMap {
    pub fn from_env() -> Self {
        let mut map = Self::default();
        map.insert(
            "anthropic",
            &HeadTransport::Api,
            std::env::var("ANTHROPIC_MESSAGES_URL")
                .unwrap_or_else(|_| "https://api.anthropic.com/v1/messages".to_string()),
        );
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
pub struct ProviderHeadInvoker {
    http: reqwest::blocking::Client,
    credentials: CredentialResolver,
    endpoints: EndpointMap,
}

impl ProviderHeadInvoker {
    pub fn from_env() -> Result<Self, HeadInvocationError> {
        Self::new(EndpointMap::from_env())
    }

    pub fn new(endpoints: EndpointMap) -> Result<Self, HeadInvocationError> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|error| HeadInvocationError::ProviderError {
                head_id: "provider-invoker".to_string(),
                provider: "reqwest".to_string(),
                status: 0,
                detail: error.to_string(),
            })?;
        Ok(Self {
            http,
            credentials: CredentialResolver,
            endpoints,
        })
    }

    fn invoke_anthropic(
        &self,
        request: HeadInvocationRequest,
    ) -> Result<HeadInvocationReceipt, HeadInvocationError> {
        let endpoint = self
            .endpoints
            .get(&request.head.provider, &request.head.endpoint.transport)
            .unwrap_or("https://api.anthropic.com/v1/messages");
        let api_key = self
            .credentials
            .resolve(&request.head.credential_ref)
            .map_err(|detail| HeadInvocationError::ProviderError {
                head_id: request.head.head_id.clone(),
                provider: request.head.provider.clone(),
                status: 0,
                detail,
            })?;
        let prompt = prompt_for_request(&request);
        let response = self
            .http
            .post(endpoint)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&json!({
                "model": request.head.model,
                "max_tokens": 1024,
                "messages": [{ "role": "user", "content": prompt }]
            }))
            .send()
            .map_err(|error| provider_send_error(&request, error))?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|error| provider_send_error(&request, error))?;
        if !status.is_success() {
            return Err(HeadInvocationError::ProviderError {
                head_id: request.head.head_id.clone(),
                provider: request.head.provider.clone(),
                status: status.as_u16(),
                detail: truncate_detail(&body),
            });
        }
        let body_json: Value =
            serde_json::from_str(&body).map_err(|error| HeadInvocationError::ProviderError {
                head_id: request.head.head_id.clone(),
                provider: request.head.provider.clone(),
                status: status.as_u16(),
                detail: format!("invalid provider JSON: {error}"),
            })?;
        let text = body_json
            .get("content")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find_map(|item| item.get("text")))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if text.is_empty() {
            return Err(HeadInvocationError::ProviderError {
                head_id: request.head.head_id.clone(),
                provider: request.head.provider.clone(),
                status: status.as_u16(),
                detail: "provider returned no text content".to_string(),
            });
        }
        let usage = body_json.get("usage").cloned().unwrap_or_else(|| json!({}));
        let input_tokens = usage
            .get("input_tokens")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let output_tokens = usage
            .get("output_tokens")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let payload = object_payload(json!({
            "provider": request.head.provider,
            "model": request.head.model,
            "kind": request.kind.as_str(),
            "text": text,
            "prior_context": request.prior_context,
            "usage": usage
        }));
        Ok(HeadInvocationReceipt::from_request(
            &request,
            provider_summary(request.kind, &text),
            payload,
            input_tokens + output_tokens,
        ))
    }

    fn invoke_mcp_peer(
        &self,
        request: HeadInvocationRequest,
    ) -> Result<HeadInvocationReceipt, HeadInvocationError> {
        let endpoint = self
            .endpoints
            .get(&request.head.provider, &request.head.endpoint.transport)
            .ok_or_else(|| HeadInvocationError::ProviderError {
                head_id: request.head.head_id.clone(),
                provider: request.head.provider.clone(),
                status: 0,
                detail: "missing MCP endpoint; set DEEPSEEK_MCP_URL or endpoint map".to_string(),
            })?;
        let tool_name = match request.kind {
            HeadInvocationKind::Proposal => "deepseek_reason",
            HeadInvocationKind::Critique => "deepseek_critique",
            HeadInvocationKind::Synthesis => "deepseek_synthesize",
        };
        let prompt = prompt_for_request(&request);
        let response = self
            .http
            .post(endpoint)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": request.invocation_id,
                "method": "tools/call",
                "params": {
                    "name": tool_name,
                    "arguments": {
                        "prompt": prompt,
                        "task": request.task,
                        "kind": request.kind.as_str(),
                        "prior_context": request.prior_context
                    }
                }
            }))
            .send()
            .map_err(|error| provider_send_error(&request, error))?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|error| provider_send_error(&request, error))?;
        if !status.is_success() {
            return Err(HeadInvocationError::ProviderError {
                head_id: request.head.head_id.clone(),
                provider: request.head.provider.clone(),
                status: status.as_u16(),
                detail: truncate_detail(&body),
            });
        }
        let body_json: Value =
            serde_json::from_str(&body).map_err(|error| HeadInvocationError::ProviderError {
                head_id: request.head.head_id.clone(),
                provider: request.head.provider.clone(),
                status: status.as_u16(),
                detail: format!("invalid MCP JSON: {error}"),
            })?;
        if let Some(error) = body_json.get("error") {
            return Err(HeadInvocationError::ProviderError {
                head_id: request.head.head_id.clone(),
                provider: request.head.provider.clone(),
                status: 0,
                detail: truncate_detail(&error.to_string()),
            });
        }
        let text = mcp_text(&body_json);
        if text.trim().is_empty() {
            return Err(HeadInvocationError::ProviderError {
                head_id: request.head.head_id.clone(),
                provider: request.head.provider.clone(),
                status: 0,
                detail: "MCP peer returned no text content".to_string(),
            });
        }
        let payload = object_payload(json!({
            "provider": request.head.provider,
            "model": request.head.model,
            "kind": request.kind.as_str(),
            "tool": tool_name,
            "text": text,
            "prior_context": request.prior_context,
            "mcp_result": body_json.get("result").cloned().unwrap_or_else(|| json!({}))
        }));
        Ok(HeadInvocationReceipt::from_request(
            &request,
            provider_summary(request.kind, &text),
            payload,
            0.0,
        ))
    }
}

impl HeadInvoker for ProviderHeadInvoker {
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
        let provider = request.head.provider.to_ascii_lowercase();
        match (&*provider, &request.head.endpoint.transport) {
            ("anthropic", HeadTransport::Api) => self.invoke_anthropic(request),
            ("deepseek", HeadTransport::Mcp) => self.invoke_mcp_peer(request),
            _ => Err(HeadInvocationError::ProviderError {
                head_id: request.head.head_id.clone(),
                provider: request.head.provider.clone(),
                status: 0,
                detail: format!(
                    "unsupported provider transport: {} over {:?}",
                    request.head.provider, request.head.endpoint.transport
                ),
            }),
        }
    }
}

fn prompt_for_request(request: &HeadInvocationRequest) -> String {
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

fn provider_send_error(
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

fn provider_summary(kind: HeadInvocationKind, text: &str) -> String {
    let clipped = text.chars().take(96).collect::<String>();
    format!("provider {}: {}", kind.as_str(), clipped)
}

fn mcp_text(body: &Value) -> String {
    body.get("result")
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find_map(|item| item.get("text").and_then(Value::as_str))
        })
        .or_else(|| {
            body.get("result")
                .and_then(|result| result.get("structuredContent"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .to_string()
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

fn object_payload(value: Value) -> serde_json::Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    }
}

fn truncate_detail(detail: &str) -> String {
    detail.chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use theorem_harness_core::{
        AgentHeadEndpoint, HeadCostProfile, HeadKind, HeadReliabilityProfile, ResolvedAgentHead,
        RevisionContext, TraceTier,
    };

    #[test]
    fn credential_resolver_rejects_raw_material_and_requires_env_refs() {
        let resolver = CredentialResolver;
        let error = resolver.resolve("sk-not-allowed").unwrap_err();
        assert!(error.contains("expected env:NAME"));
    }

    #[test]
    fn mcp_peer_without_endpoint_fails_typed_provider_error() {
        let invoker = ProviderHeadInvoker::new(EndpointMap::default()).unwrap();
        let error = invoker
            .invoke(request("deepseek", HeadTransport::Mcp))
            .unwrap_err();

        assert!(matches!(
            error,
            HeadInvocationError::ProviderError {
                provider,
                status: 0,
                ..
            } if provider == "deepseek"
        ));
    }

    #[test]
    fn prompt_includes_prior_revision_content() {
        let mut request = request("anthropic", HeadTransport::Api);
        request.prior_context = vec![RevisionContext {
            revision_id: "scratchrev:1".to_string(),
            kind: HeadInvocationKind::Proposal,
            output_summary: "proposal".to_string(),
            payload: object_payload(json!({ "text": "body from proposal" })),
        }];

        let prompt = prompt_for_request(&request);

        assert!(prompt.contains("scratchrev:1"));
        assert!(prompt.contains("body from proposal"));
    }

    fn request(provider: &str, transport: HeadTransport) -> HeadInvocationRequest {
        HeadInvocationRequest::new(
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
            },
            HeadInvocationKind::Proposal,
            "task",
            0,
            Vec::new(),
            Vec::new(),
            "2026-06-08T00:00:00Z",
        )
    }
}
