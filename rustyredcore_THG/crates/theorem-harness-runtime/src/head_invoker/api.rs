use crate::head_invoker::{
    object_payload, prompt_for_request, provider_send_error, provider_summary, truncate_detail,
    CredentialResolver, EndpointMap,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use theorem_harness_core::{
    HeadInvocationError, HeadInvocationReceipt, HeadInvocationRequest, HeadTransport,
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
        "zhipu" | "zai" | "z.ai" | "glm" => Some(ApiProviderProfile {
            provider: "zhipu",
            env_endpoint: "ZHIPU_CHAT_URL",
            default_endpoint: "https://open.bigmodel.cn/api/paas/v4/chat/completions",
            request_shape: ApiRequestShape::OpenAiChatCompletions,
        }),
        "minimax" => Some(ApiProviderProfile {
            provider: "minimax",
            env_endpoint: "MINIMAX_CHAT_URL",
            default_endpoint: "https://api.minimax.io/v1/chat/completions",
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
        "zhipu",
        "minimax",
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
            invoke_anthropic_messages(http, endpoint, secret, request)
        }
        ApiRequestShape::OpenAiChatCompletions => {
            invoke_openai_chat_completions(http, endpoint, secret, request)
        }
    }
}

fn invoke_anthropic_messages(
    http: &Client,
    endpoint: &str,
    api_key: String,
    request: HeadInvocationRequest,
) -> Result<HeadInvocationReceipt, HeadInvocationError> {
    let prompt = prompt_for_request(&request);
    let response = http
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
    let body_json = provider_json(&request, status.as_u16(), &body)?;
    let text = anthropic_text(&body_json);
    if text.trim().is_empty() {
        return Err(empty_text_error(&request, status.as_u16()));
    }
    let usage = body_json.get("usage").cloned().unwrap_or_else(|| json!({}));
    let payload = object_payload(json!({
        "provider": request.head.provider,
        "model": request.head.model,
        "transport": "api",
        "request_shape": "anthropic_messages",
        "kind": request.kind.as_str(),
        "text": text,
        "prior_context": request.prior_context,
        "usage": usage
    }));
    Ok(HeadInvocationReceipt::from_request(
        &request,
        provider_summary(
            request.kind,
            payload.get("text").and_then(Value::as_str).unwrap_or(""),
        ),
        payload,
        anthropic_cost_units(&body_json),
    ))
}

fn invoke_openai_chat_completions(
    http: &Client,
    endpoint: &str,
    api_key: String,
    request: HeadInvocationRequest,
) -> Result<HeadInvocationReceipt, HeadInvocationError> {
    let prompt = prompt_for_request(&request);
    let response = http
        .post(endpoint)
        .bearer_auth(api_key)
        .json(&json!({
            "model": request.head.model,
            "messages": [{ "role": "user", "content": prompt }],
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
    let payload = object_payload(json!({
        "provider": request.head.provider,
        "model": request.head.model,
        "transport": "api",
        "request_shape": "openai_chat_completions",
        "kind": request.kind.as_str(),
        "text": text,
        "reasoning": reasoning,
        "prior_context": request.prior_context,
        "usage": usage
    }));
    Ok(HeadInvocationReceipt::from_request(
        &request,
        provider_summary(
            request.kind,
            payload.get("text").and_then(Value::as_str).unwrap_or(""),
        ),
        payload,
        openai_cost_units(&body_json),
    ))
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

fn anthropic_cost_units(body: &Value) -> f64 {
    let usage = body.get("usage").unwrap_or(&Value::Null);
    usage
        .get("input_tokens")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        + usage
            .get("output_tokens")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
}

fn openai_cost_units(body: &Value) -> f64 {
    let usage = body.get("usage").unwrap_or(&Value::Null);
    usage
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
        })
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
            "zhipu",
            "minimax",
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
        assert_eq!(openai_cost_units(&body), 5.0);
    }

    #[test]
    fn anthropic_shape_extracts_text_and_usage() {
        let body = json!({
            "content": [{ "type": "text", "text": "hello" }],
            "usage": { "input_tokens": 4, "output_tokens": 6 }
        });

        assert_eq!(anthropic_text(&body), "hello");
        assert_eq!(anthropic_cost_units(&body), 10.0);
    }
}
