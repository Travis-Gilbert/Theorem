use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::{ModelConfig, ModelProvider};
use crate::tools::ToolCatalog;
use crate::{LocalModelError, LocalModelResult};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelDecision {
    Final { text: String },
    ToolCall { name: String, arguments: Value },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// A base64-encoded image attached to a chat message. The OpenAI-compatible
/// client sends this as an `image_url` content part with a data URL.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputImage {
    /// Media type, e.g. "image/png" or "image/jpeg".
    pub media_type: String,
    /// Base64-encoded image bytes without a data URL prefix.
    pub data_base64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<InputImage>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
            images: Vec::new(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            images: Vec::new(),
        }
    }

    pub fn user_with_images(content: impl Into<String>, images: Vec<InputImage>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            images,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
            images: Vec::new(),
        }
    }

    pub fn tool(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Tool,
            content: content.into(),
            images: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelOutput {
    pub decision: ModelDecision,
    pub raw_content: String,
    pub usage: ModelUsage,
}

pub enum ModelClient {
    OpenAi(OpenAiModelClient),
    Rule(RuleModelClient),
}

impl ModelClient {
    pub fn from_config(
        config: ModelConfig,
        default_room_id: String,
        actor: String,
    ) -> LocalModelResult<Self> {
        match config.provider {
            ModelProvider::OpenAiCompatible => Ok(Self::OpenAi(OpenAiModelClient::new(config)?)),
            ModelProvider::Rule => Ok(Self::Rule(RuleModelClient {
                default_room_id,
                actor,
            })),
        }
    }

    pub fn decide(
        &self,
        messages: &[ChatMessage],
        catalog: &ToolCatalog,
        grammar: &str,
    ) -> LocalModelResult<ModelOutput> {
        match self {
            Self::OpenAi(client) => client.decide(messages, catalog, grammar),
            Self::Rule(client) => client.decide(messages),
        }
    }

    /// Compose one terse status line for a milestone relay. The prose is the
    /// model's (the loop keeps milestone summaries model-written); `fallback` is
    /// returned verbatim for the rule provider and whenever the model errors, so
    /// a relay always has a line. The caller still guarantees load-bearing facts
    /// (e.g. that a PR-opened line contains the PR URL).
    pub fn compose_line(&self, instruction: &str, fallback: &str) -> String {
        match self {
            Self::OpenAi(client) => client
                .compose_line(instruction)
                .ok()
                .map(|line| sanitize_line(&line))
                .filter(|line| !line.is_empty())
                .unwrap_or_else(|| fallback.to_string()),
            Self::Rule(_) => fallback.to_string(),
        }
    }
}

/// First non-empty line of a model reply, trimmed and length-capped, so a chatty
/// completion cannot overflow a task tracker field.
pub fn sanitize_line(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .chars()
        .take(280)
        .collect()
}

pub struct OpenAiModelClient {
    http: reqwest::blocking::Client,
    config: ModelConfig,
    api_key: Option<String>,
}

impl OpenAiModelClient {
    pub fn new(config: ModelConfig) -> LocalModelResult<Self> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .map_err(LocalModelError::from)?;
        let api_key = config
            .api_key_env
            .as_deref()
            .and_then(|env| std::env::var(env).ok())
            .filter(|token| !token.trim().is_empty());
        Ok(Self {
            http,
            config,
            api_key,
        })
    }

    pub fn decide(
        &self,
        messages: &[ChatMessage],
        catalog: &ToolCatalog,
        _grammar: &str,
    ) -> LocalModelResult<ModelOutput> {
        let url = chat_completions_url(&self.config.base_url);
        let request_messages = request_messages(messages);
        // Tool-calling models (Gemma via llama-server) return structured
        // `tool_calls` when given an OpenAI `tools` array. This is more reliable
        // than constraining free-form JSON with a GBNF grammar, which Gemma's
        // chat template fights (it leaks native `<|tool_call>` tokens). The
        // grammar is retained only for `--print-tool-grammar` and grammar-only
        // backends; the live path uses tools + tool_calls.
        let body = json!({
            "model": self.config.model,
            "messages": request_messages,
            "temperature": self.config.temperature,
            "max_tokens": self.config.max_tokens,
            "tools": catalog.openai_tools()
        });
        let mut request = self.http.post(url).json(&body);
        if let Some(api_key) = &self.api_key {
            request = request.bearer_auth(api_key);
        }
        let response = request.send()?.error_for_status()?;
        let value: Value = response.json()?;
        parse_chat_completion(&value)
    }

    /// A no-tools completion that returns the model's text for a milestone line.
    fn compose_line(&self, instruction: &str) -> LocalModelResult<String> {
        let url = chat_completions_url(&self.config.base_url);
        let body = json!({
            "model": self.config.model,
            "messages": [
                {
                    "role": "system",
                    "content": "You write one terse status line for a task tracker. Reply with a single line, no preamble, under 140 characters."
                },
                { "role": "user", "content": instruction }
            ],
            "temperature": self.config.temperature,
            "max_tokens": 120
        });
        let mut request = self.http.post(url).json(&body);
        if let Some(api_key) = &self.api_key {
            request = request.bearer_auth(api_key);
        }
        let response = request.send()?.error_for_status()?;
        let value: Value = response.json()?;
        if let Some(error) = value.get("error") {
            return Err(LocalModelError::Model(format!("model error: {error}")));
        }
        let content = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok(content)
    }
}

pub struct RuleModelClient {
    pub(crate) default_room_id: String,
    pub(crate) actor: String,
}

impl RuleModelClient {
    pub fn decide(&self, messages: &[ChatMessage]) -> LocalModelResult<ModelOutput> {
        if messages
            .iter()
            .any(|message| message.role == MessageRole::Tool)
        {
            return Ok(model_output(ModelDecision::Final {
                text: "Done.".to_string(),
            }));
        }
        let prompt = messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::User)
            .map(|message| message.content.to_lowercase())
            .unwrap_or_default();
        if prompt.contains("what are the agents working on") || prompt.contains("agents working on")
        {
            return Ok(model_output(ModelDecision::ToolCall {
                name: "coordination_context".to_string(),
                arguments: json!({"room_id": self.default_room_id}),
            }));
        }
        if prompt.contains("have an agent") || prompt.contains("fix the failing test") {
            return Ok(model_output(ModelDecision::ToolCall {
                name: "coordinate".to_string(),
                arguments: json!({
                    "actor": self.actor,
                    "room_id": self.default_room_id,
                    "delivery": "wake",
                    "mentions": ["codex"],
                    "message": "@codex Please inspect and fix the failing test on Theorem. Use the room context and leave verification receipts.",
                    "metadata": {
                        "repo": "Theorem",
                        "source": "theorem-localmodel"
                    },
                    "urgency": "ask",
                    "wake": true
                }),
            }));
        }
        Ok(model_output(ModelDecision::Final {
            text: "I do not need a tool for that turn.".to_string(),
        }))
    }
}

fn model_output(decision: ModelDecision) -> ModelOutput {
    let raw_content = match &decision {
        ModelDecision::Final { text } => json!({"type": "final", "text": text}).to_string(),
        ModelDecision::ToolCall { name, arguments } => json!({
            "type": "tool_call",
            "name": name,
            "arguments": arguments
        })
        .to_string(),
    };
    ModelOutput {
        decision,
        raw_content,
        usage: ModelUsage::default(),
    }
}

pub fn parse_chat_completion(value: &Value) -> LocalModelResult<ModelOutput> {
    if let Some(error) = value.get("error") {
        return Err(LocalModelError::Model(format!("model error: {error}")));
    }
    let message = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .ok_or_else(|| {
            LocalModelError::Model("chat completion missing choices[0].message".to_string())
        })?;

    // Prefer structured tool calls: llama-server parses Gemma's native tool-call
    // tokens into this shape. `arguments` is a JSON string.
    if let Some(call) = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .and_then(|calls| calls.first())
    {
        let function = call
            .get("function")
            .ok_or_else(|| LocalModelError::Model("tool_call missing function".to_string()))?;
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| LocalModelError::Model("tool_call missing function.name".to_string()))?;
        let arguments = match function.get("arguments") {
            Some(Value::String(raw)) => serde_json::from_str(raw).unwrap_or_else(|_| json!({})),
            Some(other) => other.clone(),
            None => json!({}),
        };
        return Ok(ModelOutput {
            decision: ModelDecision::ToolCall {
                name: name.to_string(),
                arguments,
            },
            raw_content: call.to_string(),
            usage: parse_usage(value),
        });
    }

    // Otherwise a final text reply. parse_model_content still tolerates a
    // content-embedded {"type":...} envelope as a fallback.
    let content = message.get("content").and_then(Value::as_str).unwrap_or("");
    Ok(ModelOutput {
        decision: parse_model_content(content)?,
        raw_content: content.to_string(),
        usage: parse_usage(value),
    })
}

pub fn parse_model_content(content: &str) -> LocalModelResult<ModelDecision> {
    let trimmed = content.trim();
    if trimmed.starts_with('{') {
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            return Ok(ModelDecision::Final {
                text: content.to_string(),
            });
        };
        if value.get("type").and_then(Value::as_str) == Some("tool_call") {
            let name = value
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| LocalModelError::Model("tool_call missing name".to_string()))?;
            let arguments = value
                .get("arguments")
                .cloned()
                .ok_or_else(|| LocalModelError::Model("tool_call missing arguments".to_string()))?;
            return Ok(ModelDecision::ToolCall {
                name: name.to_string(),
                arguments,
            });
        }
        if let Some(name) = value
            .get("tool")
            .or_else(|| value.get("name"))
            .and_then(Value::as_str)
        {
            let arguments = value.get("arguments").cloned().unwrap_or_else(|| json!({}));
            return Ok(ModelDecision::ToolCall {
                name: name.to_string(),
                arguments,
            });
        }
        if value.get("type").and_then(Value::as_str) == Some("final") {
            let text = value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            return Ok(ModelDecision::Final { text });
        }
        if let Some(text) = value.get("final").and_then(Value::as_str) {
            return Ok(ModelDecision::Final {
                text: text.to_string(),
            });
        }
    }
    Ok(ModelDecision::Final {
        text: content.to_string(),
    })
}

fn parse_usage(value: &Value) -> ModelUsage {
    let usage = value.get("usage").unwrap_or(&Value::Null);
    ModelUsage {
        prompt_tokens: usage.get("prompt_tokens").and_then(Value::as_u64),
        completion_tokens: usage.get("completion_tokens").and_then(Value::as_u64),
        total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
    }
}

fn chat_completions_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/v1/chat/completions")
    }
}

fn request_messages(messages: &[ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .map(|message| {
            if message.images.is_empty() {
                json!({
                    "role": role_name(&message.role),
                    "content": message.content
                })
            } else {
                let mut content = vec![json!({
                    "type": "text",
                    "text": message.content
                })];
                content.extend(message.images.iter().map(|image| {
                    json!({
                        "type": "image_url",
                        "image_url": {
                            "url": format!(
                                "data:{};base64,{}",
                                image.media_type, image.data_base64
                            )
                        }
                    })
                }));
                json!({
                    "role": role_name(&message.role),
                    "content": content
                })
            }
        })
        .collect()
}

fn role_name(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "user",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_call_envelope() {
        let decision = parse_model_content(
            r#"{"type":"tool_call","name":"coordination_context","arguments":{"room_id":"repo:theorem:branch:main"}}"#,
        )
        .unwrap();
        assert_eq!(
            decision,
            ModelDecision::ToolCall {
                name: "coordination_context".to_string(),
                arguments: json!({"room_id": "repo:theorem:branch:main"})
            }
        );
    }

    #[test]
    fn parse_chat_completion_reads_tool_calls() {
        let value = json!({
            "choices": [{"message": {"content": "", "tool_calls": [
                {"type": "function", "function": {"name": "coordination_context", "arguments": "{\"room_id\":\"repo:theorem:branch:main\"}"}}
            ]}}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        let output = parse_chat_completion(&value).unwrap();
        match output.decision {
            ModelDecision::ToolCall { name, arguments } => {
                assert_eq!(name, "coordination_context");
                assert_eq!(arguments["room_id"], "repo:theorem:branch:main");
            }
            other => panic!("expected tool call, got {other:?}"),
        }
    }

    #[test]
    fn parses_plain_final_text() {
        let decision = parse_model_content("hello").unwrap();
        assert_eq!(
            decision,
            ModelDecision::Final {
                text: "hello".to_string()
            }
        );
    }

    #[test]
    fn builds_chat_url() {
        assert_eq!(
            chat_completions_url("http://127.0.0.1:8080"),
            "http://127.0.0.1:8080/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("http://127.0.0.1:8080/v1"),
            "http://127.0.0.1:8080/v1/chat/completions"
        );
    }

    #[test]
    fn text_only_message_serializes_as_string_content() {
        let messages = request_messages(&[ChatMessage::user("hello")]);

        assert_eq!(messages[0], json!({"role": "user", "content": "hello"}));
        assert!(messages[0]["content"].is_string());
    }

    #[test]
    fn image_message_serializes_as_content_array() {
        let messages = request_messages(&[ChatMessage::user_with_images(
            "what is visible?",
            vec![InputImage {
                media_type: "image/png".to_string(),
                data_base64: "iVBORw0KGgo=".to_string(),
            }],
        )]);

        let content = messages[0]["content"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(
            content[0],
            json!({"type": "text", "text": "what is visible?"})
        );
        assert_eq!(
            content[1],
            json!({
                "type": "image_url",
                "image_url": {
                    "url": "data:image/png;base64,iVBORw0KGgo="
                }
            })
        );
    }

    #[test]
    fn multiple_images_emit_multiple_parts() {
        let messages = request_messages(&[ChatMessage::user_with_images(
            "compare these",
            vec![
                InputImage {
                    media_type: "image/png".to_string(),
                    data_base64: "first".to_string(),
                },
                InputImage {
                    media_type: "image/jpeg".to_string(),
                    data_base64: "second".to_string(),
                },
            ],
        )]);

        let content = messages[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 3);
        assert_eq!(content[0], json!({"type": "text", "text": "compare these"}));
        assert_eq!(
            content[1]["image_url"]["url"],
            "data:image/png;base64,first"
        );
        assert_eq!(
            content[2]["image_url"]["url"],
            "data:image/jpeg;base64,second"
        );
    }

    #[test]
    #[ignore = "live smoke: requires llama-server started with --mmproj"]
    fn live_image_turn_against_llama_server() {
        let Ok(base_url) = std::env::var("THEOREM_LOCALMODEL_LIVE_MODEL_BASE_URL") else {
            eprintln!("set THEOREM_LOCALMODEL_LIVE_MODEL_BASE_URL to run the live image smoke");
            return;
        };
        let model = std::env::var("THEOREM_LOCALMODEL_LIVE_MODEL_NAME")
            .unwrap_or_else(|_| "gemma-4-12b-it-q4".to_string());
        let config = ModelConfig {
            provider: ModelProvider::OpenAiCompatible,
            base_url,
            model,
            api_key_env: std::env::var("THEOREM_LOCALMODEL_LIVE_MODEL_API_KEY_ENV").ok(),
            temperature: 0.0,
            max_tokens: 120,
            request_timeout_secs: 120,
            grammar_constrained: true,
        };
        let client = OpenAiModelClient::new(config).unwrap();
        let catalog = ToolCatalog::default_catalog();
        let output = client
            .decide(
                &[ChatMessage::user_with_images(
                    "Describe this single-pixel test image briefly. Do not call tools.",
                    vec![InputImage {
                        media_type: "image/png".to_string(),
                        data_base64: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=".to_string(),
                    }],
                )],
                &catalog,
                &catalog.gbnf_grammar(),
            )
            .unwrap();

        assert!(!output.raw_content.trim().is_empty());
    }
}
