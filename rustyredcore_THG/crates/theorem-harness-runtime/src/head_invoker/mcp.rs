use crate::head_invoker::{
    object_payload, prompt_for_request, provider_send_error, provider_summary,
    system_instruction_for_request, truncate_detail, CredentialResolver, EndpointMap,
};
use reqwest::blocking::{Client, RequestBuilder};
use serde_json::{json, Value};
use theorem_harness_core::{
    HeadInvocationError, HeadInvocationKind, HeadInvocationReceipt, HeadInvocationRequest,
    HeadTransport,
};

pub fn invoke_mcp_head(
    http: &Client,
    endpoints: &EndpointMap,
    credentials: &CredentialResolver,
    fallback_cost_units: f64,
    request: HeadInvocationRequest,
) -> Result<HeadInvocationReceipt, HeadInvocationError> {
    let endpoint = mcp_endpoint(endpoints, &request)?;
    let tool_name = mcp_tool_name(&request);
    let prompt = prompt_for_request(&request);
    let system_prompt = system_instruction_for_request(&request);
    let mut builder = http.post(&endpoint).json(&json!({
        "jsonrpc": "2.0",
        "id": request.invocation_id,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": {
                "prompt": prompt,
                "system_prompt": system_prompt,
                "task": request.task,
                "kind": request.kind.as_str(),
                "scratchpad_crdt": request.scratchpad_crdt,
                "context_membrane": request.context_membrane,
                "prior_context": request.prior_context
            }
        }
    }));
    builder = maybe_attach_bearer(builder, credentials, &request);
    let response = builder
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
        "transport": "mcp",
        "kind": request.kind.as_str(),
        "tool": tool_name,
        "text": text,
        "prior_context": request.prior_context,
        "mcp_result": body_json.get("result").cloned().unwrap_or_else(|| json!({}))
    }));
    Ok(HeadInvocationReceipt::from_request(
        &request,
        provider_summary(
            request.kind,
            payload.get("text").and_then(Value::as_str).unwrap_or(""),
        ),
        payload,
        fallback_cost_units,
    ))
}

fn mcp_endpoint(
    endpoints: &EndpointMap,
    request: &HeadInvocationRequest,
) -> Result<String, HeadInvocationError> {
    if let Some(endpoint) = endpoints.get(&request.head.provider, &HeadTransport::Mcp) {
        return Ok(endpoint.to_string());
    }
    if !request.head.endpoint.fake && request.head.endpoint.target.starts_with("http") {
        return Ok(request.head.endpoint.target.clone());
    }
    Err(HeadInvocationError::ProviderError {
        head_id: request.head.head_id.clone(),
        provider: request.head.provider.clone(),
        status: 0,
        detail: format!(
            "missing MCP endpoint for provider {}; set {} or endpoint map",
            request.head.provider,
            mcp_endpoint_env_name(&request.head.provider)
        ),
    })
}

fn mcp_endpoint_env_name(provider: &str) -> String {
    format!(
        "{}_MCP_URL",
        provider
            .trim()
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>()
    )
}

pub fn mcp_tool_name(request: &HeadInvocationRequest) -> String {
    let env_name = format!(
        "THEOREM_MCP_TOOL_{}_{}",
        request
            .head
            .provider
            .trim()
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>(),
        request.kind.as_str().to_ascii_uppercase()
    );
    if let Ok(tool_name) = std::env::var(env_name) {
        let tool_name = tool_name.trim();
        if !tool_name.is_empty() {
            return tool_name.to_string();
        }
    }

    let provider = request.head.provider.trim().to_ascii_lowercase();
    match (provider.as_str(), request.kind) {
        ("deepseek", HeadInvocationKind::Proposal) => "deepseek_reason".to_string(),
        ("deepseek", HeadInvocationKind::Critique) => "deepseek_critique".to_string(),
        ("deepseek", HeadInvocationKind::Synthesis) => "deepseek_synthesize".to_string(),
        ("deepseek", HeadInvocationKind::Verification) => "deepseek_critique".to_string(),
        _ => format!("{}_{}", provider, request.kind.as_str()),
    }
}

fn maybe_attach_bearer(
    builder: RequestBuilder,
    credentials: &CredentialResolver,
    request: &HeadInvocationRequest,
) -> RequestBuilder {
    match credentials.resolve(&request.head.credential_ref) {
        Ok(secret) if !secret.trim().is_empty() => builder.bearer_auth(secret),
        _ => builder,
    }
}

fn mcp_text(body: &Value) -> String {
    body.get("result")
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.get("text")
                        .and_then(Value::as_str)
                        .or_else(|| item.get("content").and_then(Value::as_str))
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .or_else(|| {
            body.get("result")
                .and_then(|result| result.get("structuredContent"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use theorem_harness_core::{
        AgentHeadEndpoint, HeadCostProfile, HeadKind, HeadReliabilityProfile, ResolvedAgentHead,
        TraceTier,
    };

    #[test]
    fn deepseek_tool_names_match_binding_kinds() {
        assert_eq!(
            mcp_tool_name(&request("deepseek", HeadInvocationKind::Proposal)),
            "deepseek_reason"
        );
        assert_eq!(
            mcp_tool_name(&request("deepseek", HeadInvocationKind::Critique)),
            "deepseek_critique"
        );
        assert_eq!(
            mcp_tool_name(&request("deepseek", HeadInvocationKind::Synthesis)),
            "deepseek_synthesize"
        );
    }

    #[test]
    fn mcp_text_reads_standard_content_blocks() {
        let body = json!({
            "result": {
                "content": [{ "type": "text", "text": "mcp answer" }]
            }
        });

        assert_eq!(mcp_text(&body), "mcp answer");
    }

    fn request(provider: &str, kind: HeadInvocationKind) -> HeadInvocationRequest {
        HeadInvocationRequest::new(
            ResolvedAgentHead {
                head_id: provider.to_string(),
                display_name: provider.to_string(),
                provider: provider.to_string(),
                model: "model".to_string(),
                kind: HeadKind::ReasoningCore,
                endpoint: AgentHeadEndpoint {
                    transport: HeadTransport::Mcp,
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
            kind,
            "task",
            0,
            Vec::new(),
            Vec::new(),
            "2026-06-08T00:00:00Z",
        )
    }
}
