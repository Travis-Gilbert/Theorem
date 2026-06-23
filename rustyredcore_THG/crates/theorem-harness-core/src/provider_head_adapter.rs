//! Shared provider/head execution context for runtime adapters.
//!
//! This is intentionally smaller than the full composed-agent head invocation
//! loop. Service adapters can attach the provider/head/model/transport context
//! to receipts now, while the richer AgentHead registry and invocation loop can
//! build on the same shape later.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProviderHeadExecutionContext {
    pub adapter: String,
    pub head_id: String,
    pub provider: String,
    pub model: String,
    pub transport: String,
}

impl ProviderHeadExecutionContext {
    pub fn from_request_head(
        adapter: impl Into<String>,
        default_model: impl Into<String>,
        head: Option<&Value>,
    ) -> Self {
        let head = head.and_then(Value::as_object);
        Self {
            adapter: adapter.into(),
            head_id: string_field(head, "head_id")
                .unwrap_or_else(|| "theorem-grpc-local".to_string()),
            provider: string_field(head, "provider").unwrap_or_else(|| "theorem_grpc".to_string()),
            model: string_field(head, "model").unwrap_or_else(|| default_model.into()),
            transport: string_field(head, "transport").unwrap_or_else(|| "local".to_string()),
        }
    }

    pub fn to_payload(&self) -> Value {
        json!({
            "adapter": self.adapter,
            "head_id": self.head_id,
            "provider": self.provider,
            "model": self.model,
            "transport": self.transport,
        })
    }
}

fn string_field(head: Option<&serde_json::Map<String, Value>>, key: &str) -> Option<String> {
    head.and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_context_defaults_without_raw_credentials() {
        let context = ProviderHeadExecutionContext::from_request_head("adapter", "model-a", None);

        assert_eq!(context.head_id, "theorem-grpc-local");
        assert_eq!(context.provider, "theorem_grpc");
        assert_eq!(context.model, "model-a");
        assert_eq!(context.transport, "local");
        assert!(context.to_payload().get("credential").is_none());
    }

    #[test]
    fn provider_context_honors_request_head_metadata() {
        let head = json!({
            "head_id": "head:claude",
            "provider": "anthropic",
            "model": "claude",
            "transport": "api",
            "credential": "sk-not-copied"
        });

        let context =
            ProviderHeadExecutionContext::from_request_head("adapter", "fallback", Some(&head));

        assert_eq!(context.head_id, "head:claude");
        assert_eq!(context.provider, "anthropic");
        assert_eq!(context.model, "claude");
        assert_eq!(context.transport, "api");
        assert!(context.to_payload().get("credential").is_none());
    }
}
