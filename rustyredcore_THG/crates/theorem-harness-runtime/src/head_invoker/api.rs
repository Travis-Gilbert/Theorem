use crate::head_invoker::{
    object_payload, prompt_for_request, provider_send_error, provider_summary,
    system_instruction_for_request, truncate_detail, CredentialResolver, EndpointMap,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use theorem_harness_core::{
    GroundedClaim, HeadInvocationError, HeadInvocationReceipt, HeadInvocationRequest, HeadTransport,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApiRequestShape {
    AnthropicMessages,
    OpenAiChatCompletions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApiProviderProfile {
    pub provider: &'static str,
    pub env_endpoint: &'static str,
    pub default_endpoint: &'static str,
    pub request_shape: ApiRequestShape,
}

pub fn api_provider_profile(provider: &str) -> Option<ApiProviderProfile> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "anthropic" | "claude" => Some(ApiProviderProfile {
            provider: "anthropic",
            env_endpoint: "ANTHROPIC_MESSAGES_URL",
            default_endpoint: "https://api.anthropic.com/v1/messages",
            request_shape: ApiRequestShape::AnthropicMessages,
        }),
        "deepseek" => Some(ApiProviderProfile {
            provider: "deepseek",
            env_endpoint: "DEEPSEEK_CHAT_URL",
            default_endpoint: "https://api.deepseek.com/chat/completions",
            request_shape: ApiRequestShape::OpenAiChatCompletions,
        }),
        "openai" | "openapi" => Some(ApiProviderProfile {
            provider: "openai",
            env_endpoint: "OPENAI_CHAT_URL",
            default_endpoint: "https://api.openai.com/v1/chat/completions",
            request_shape: ApiRequestShape::OpenAiChatCompletions,
        }),
        "zhipu" | "zai" | "z.ai" | "glm" => Some(ApiProviderProfile {
            provider: "zhipu",
            env_endpoint: "ZHIPU_CHAT_URL",
            default_endpoint: "https://open.bigmodel.cn/api/paas/v4/chat/completions",
            request_shape: ApiRequestShape::OpenAiChatCompletions,
        }),
        "minimax" => Some(ApiProviderProfile {
            provider: "minimax",
            env_endpoint: "MINIMAX_CHAT_URL",
            default_endpoint: "https://api.minimaxi.com/v1/chat/completions",
            request_shape: ApiRequestShape::OpenAiChatCompletions,
        }),
        "qwen" | "dashscope" | "alibaba" | "aliyun" => Some(ApiProviderProfile {
            provider: "qwen",
            env_endpoint: "QWEN_CHAT_URL",
            default_endpoint:
                "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions",
            request_shape: ApiRequestShape::OpenAiChatCompletions,
        }),
        "mistral" => Some(ApiProviderProfile {
            provider: "mistral",
            env_endpoint: "MISTRAL_CHAT_URL",
            default_endpoint: "https://api.mistral.ai/v1/chat/completions",
            request_shape: ApiRequestShape::OpenAiChatCompletions,
        }),
        "ai21" | "jamba" => Some(ApiProviderProfile {
            provider: "ai21",
            env_endpoint: "AI21_CHAT_URL",
            default_endpoint: "https://api.ai21.com/studio/v1/chat/completions",
            request_shape: ApiRequestShape::OpenAiChatCompletions,
        }),
        "gemma" => Some(ApiProviderProfile {
            provider: "gemma",
            env_endpoint: "GEMMA_CHAT_URL",
            default_endpoint:
                "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions",
            request_shape: ApiRequestShape::OpenAiChatCompletions,
        }),
        _ => None,
    }
}

pub fn default_api_profiles() -> Vec<ApiProviderProfile> {
    [
        "anthropic",
        "deepseek",
        "openai",
        "zhipu",
        "minimax",
        "qwen",
        "mistral",
        "ai21",
        "gemma",
    ]
    .iter()
    .filter_map(|provider| api_provider_profile(provider))
    .collect()
}

pub fn invoke_api_head(
    http: &Client,
    endpoints: &EndpointMap,
    credentials: &CredentialResolver,
    fallback_cost_units: f64,
    request: HeadInvocationRequest,
) -> Result<HeadInvocationReceipt, HeadInvocationError> {
    let Some(profile) = api_provider_profile(&request.head.provider) else {
        return Err(HeadInvocationError::ProviderError {
            head_id: request.head.head_id.clone(),
            provider: request.head.provider.clone(),
            status: 0,
            detail: format!("unsupported API provider {}", request.head.provider),
        });
    };
    let endpoint = endpoints
        .get(&request.head.provider, &HeadTransport::Api)
        .or_else(|| endpoints.get(profile.provider, &HeadTransport::Api))
        .unwrap_or(profile.default_endpoint);
    let secret = credentials
        .resolve(&request.head.credential_ref)
        .map_err(|error| HeadInvocationError::ProviderError {
            head_id: request.head.head_id.clone(),
            provider: request.head.provider.clone(),
            status: 0,
            detail: error.detail(),
        })?;

    match profile.request_shape {
        ApiRequestShape::AnthropicMessages => {
            invoke_anthropic_messages(http, endpoint, secret, fallback_cost_units, request)
        }
        ApiRequestShape::OpenAiChatCompletions => invoke_openai_chat_completions(
            http,
            endpoint,
            Some(secret),
            HeadTransport::Api,
            fallback_cost_units,
            request,
        ),
    }
}

fn invoke_anthropic_messages(
    http: &Client,
    endpoint: &str,
    api_key: String,
    fallback_cost_units: f64,
    request: HeadInvocationRequest,
) -> Result<HeadInvocationReceipt, HeadInvocationError> {
    let prompt = prompt_for_request(&request);
    let system_instruction = system_instruction_for_request(&request);
    let response = http
        .post(endpoint)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": request.head.model,
            "max_tokens": 1024,
            "temperature": 0.2,
            "system": system_instruction,
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
    let body_json = provider_json(&request, status.as_u16(), &body)?;
    let text = anthropic_text(&body_json);
    if text.trim().is_empty() {
        return Err(empty_text_error(&request, status.as_u16()));
    }
    let usage = body_json.get("usage").cloned().unwrap_or_else(|| json!({}));
    let claims = claims_from_text(&request, &text);
    let cost_units = anthropic_cost_units(&body_json, fallback_cost_units);
    let payload = object_payload(json!({
        "provider": request.head.provider,
        "model": request.head.model,
        "transport": "api",
        "request_shape": "anthropic_messages",
        "kind": request.kind.as_str(),
        "text": text,
        "content": text,
        "prior_context": request.prior_context,
        "provider_response": {
            "model": body_json.get("model").cloned().unwrap_or(Value::Null),
            "finish_reason": body_json.get("stop_reason").cloned().unwrap_or(Value::Null),
            "usage": usage
        }
    }));
    let receipt = HeadInvocationReceipt::from_request(
        &request,
        provider_summary(
            request.kind,
            payload.get("text").and_then(Value::as_str).unwrap_or(""),
        ),
        payload,
        cost_units,
    );
    Ok(receipt_with_claims(receipt, claims))
}

pub(crate) fn invoke_openai_chat_completions(
    http: &Client,
    endpoint: &str,
    api_key: Option<String>,
    transport: HeadTransport,
    fallback_cost_units: f64,
    request: HeadInvocationRequest,
) -> Result<HeadInvocationReceipt, HeadInvocationError> {
    let prompt = prompt_for_request(&request);
    let system_instruction = system_instruction_for_request(&request);
    let mut builder = http.post(endpoint);
    if let Some(api_key) = api_key
        .as_deref()
        .map(str::trim)
        .filter(|api_key| !api_key.is_empty())
    {
        builder = builder.bearer_auth(api_key);
    }
    let response = builder
        .json(&json!({
            "model": request.head.model,
            "messages": [
                { "role": "system", "content": system_instruction },
                { "role": "user", "content": prompt }
            ],
            "temperature": 0.2,
            "max_tokens": 1024
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
    let body_json = provider_json(&request, status.as_u16(), &body)?;
    let text = openai_chat_text(&body_json);
    if text.trim().is_empty() {
        return Err(empty_text_error(&request, status.as_u16()));
    }
    let usage = body_json.get("usage").cloned().unwrap_or_else(|| json!({}));
    let reasoning = openai_reasoning_text(&body_json);
    let claims = claims_from_text(&request, &text);
    let cost_units = openai_cost_units(&body_json, fallback_cost_units);
    let payload = object_payload(json!({
        "provider": request.head.provider,
        "model": request.head.model,
        "transport": transport_name(&transport),
        "request_shape": "openai_chat_completions",
        "kind": request.kind.as_str(),
        "text": text,
        "content": text,
        "reasoning": reasoning,
        "prior_context": request.prior_context,
        "provider_response": {
            "model": provider_model(&body_json),
            "finish_reason": provider_finish_reason(&body_json),
            "usage": usage
        }
    }));
    let receipt = HeadInvocationReceipt::from_request(
        &request,
        provider_summary(
            request.kind,
            payload.get("text").and_then(Value::as_str).unwrap_or(""),
        ),
        payload,
        cost_units,
    );
    Ok(receipt_with_claims(receipt, claims))
}

fn transport_name(transport: &HeadTransport) -> &'static str {
    match transport {
        HeadTransport::Api => "api",
        HeadTransport::Mcp => "mcp",
        HeadTransport::Local => "local",
        HeadTransport::Hosted => "hosted",
    }
}

fn provider_json(
    request: &HeadInvocationRequest,
    status: u16,
    body: &str,
) -> Result<Value, HeadInvocationError> {
    serde_json::from_str(body).map_err(|error| HeadInvocationError::ProviderError {
        head_id: request.head.head_id.clone(),
        provider: request.head.provider.clone(),
        status,
        detail: format!("invalid provider JSON: {error}"),
    })
}

fn anthropic_text(body: &Value) -> String {
    body.get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let item_type = item.get("type").and_then(Value::as_str);
                    if item_type == Some("text") || item_type.is_none() {
                        item.get("text").and_then(Value::as_str)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn openai_chat_text(body: &Value) -> String {
    body.get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .map(content_text)
        .unwrap_or_default()
}

fn openai_reasoning_text(body: &Value) -> String {
    body.get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| {
            message
                .get("reasoning_content")
                .or_else(|| message.get("reasoning"))
                .or_else(|| message.get("thoughts"))
        })
        .map(content_text)
        .unwrap_or_default()
}

fn content_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("content").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn anthropic_cost_units(body: &Value, fallback: f64) -> f64 {
    let usage = body.get("usage").unwrap_or(&Value::Null);
    let cost = usage
        .get("input_tokens")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        + usage
            .get("output_tokens")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
    if cost > 0.0 {
        cost
    } else {
        fallback
    }
}

fn openai_cost_units(body: &Value, fallback: f64) -> f64 {
    let usage = body.get("usage").unwrap_or(&Value::Null);
    let cost = usage
        .get("total_tokens")
        .and_then(Value::as_f64)
        .unwrap_or_else(|| {
            usage
                .get("prompt_tokens")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                + usage
                    .get("completion_tokens")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
        });
    if cost > 0.0 {
        cost
    } else {
        fallback
    }
}

fn provider_model(body: &Value) -> Value {
    body.get("model").cloned().unwrap_or(Value::Null)
}

fn provider_finish_reason(body: &Value) -> Value {
    body.get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("finish_reason"))
        .cloned()
        .unwrap_or(Value::Null)
}

fn claims_from_text(request: &HeadInvocationRequest, text: &str) -> Vec<GroundedClaim> {
    parse_claims_block(text).unwrap_or_else(|| {
        vec![GroundedClaim::new(
            provider_summary(request.kind, text),
            format!("head:{}", request.head.head_id),
        )]
    })
}

fn parse_claims_block(text: &str) -> Option<Vec<GroundedClaim>> {
    let start = claims_json_start(text)?;
    let end = matching_json_array_end(&text[start..])?;
    let raw = &text[start..start + end];
    let value: Value = serde_json::from_str(raw).ok()?;
    let claims = value
        .as_array()?
        .iter()
        .filter_map(|item| {
            let text = item.get("text").and_then(Value::as_str)?.trim();
            let provenance = item.get("provenance").and_then(Value::as_str)?.trim();
            if text.is_empty() || provenance.is_empty() {
                None
            } else {
                Some(GroundedClaim::new(text, provenance))
            }
        })
        .collect::<Vec<_>>();
    if claims.is_empty() {
        None
    } else {
        Some(claims)
    }
}

fn claims_json_start(text: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    for marker in ["claims json:", "claims:", "grounded claims:"] {
        if let Some(marker_start) = lower.rfind(marker) {
            let after_marker = marker_start + marker.len();
            if let Some(array_start) = text[after_marker..].find('[') {
                return Some(after_marker + array_start);
            }
        }
    }
    text.rfind('[')
}

fn matching_json_array_end(raw: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, character) in raw.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '[' => depth += 1,
            ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index + character.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

fn receipt_with_claims(
    mut receipt: HeadInvocationReceipt,
    claims: Vec<GroundedClaim>,
) -> HeadInvocationReceipt {
    receipt.claims = claims;
    receipt.receipt_hash = receipt.computed_receipt_hash();
    receipt
}

fn empty_text_error(request: &HeadInvocationRequest, status: u16) -> HeadInvocationError {
    HeadInvocationError::ProviderError {
        head_id: request.head.head_id.clone(),
        provider: request.head.provider.clone(),
        status,
        detail: "provider returned no text content".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_cover_named_api_providers() {
        for provider in [
            "anthropic",
            "deepseek",
            "openai",
            "openapi",
            "zhipu",
            "minimax",
            "qwen",
            "dashscope",
            "mistral",
            "ai21",
            "gemma",
        ] {
            assert!(api_provider_profile(provider).is_some(), "{provider}");
        }
    }

    #[test]
    fn openai_shape_extracts_text_reasoning_and_usage() {
        let body = json!({
            "choices": [{
                "message": {
                    "content": "answer",
                    "reasoning_content": "reason"
                }
            }],
            "usage": { "prompt_tokens": 2, "completion_tokens": 3 }
        });

        assert_eq!(openai_chat_text(&body), "answer");
        assert_eq!(openai_reasoning_text(&body), "reason");
        assert_eq!(openai_cost_units(&body, 1.0), 5.0);
    }

    #[test]
    fn anthropic_shape_extracts_text_and_usage() {
        let body = json!({
            "content": [{ "type": "text", "text": "hello" }],
            "usage": { "input_tokens": 4, "output_tokens": 6 }
        });

        assert_eq!(anthropic_text(&body), "hello");
        assert_eq!(anthropic_cost_units(&body, 1.0), 10.0);
    }

    #[test]
    fn parses_claims_block_from_model_text() {
        let text = r#"Answer.

Claims JSON:
[
  {"text":"alpha","provenance":"source:a"},
  {"text":"beta","provenance":"source:b"}
]
"#;

        let claims = parse_claims_block(text).unwrap();

        assert_eq!(claims.len(), 2);
        assert_eq!(claims[0].text, "alpha");
        assert_eq!(claims[1].provenance, "source:b");
    }
}
