use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::config::SyncConfig;
use crate::status::ConnectionState;
use crate::{Result, SyncError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TenantToken {
    Missing,
    Present(String),
}

impl TenantToken {
    pub fn load(config: &SyncConfig) -> Result<Self> {
        if let Some(token) = config
            .token_env
            .as_ref()
            .filter(|token| !token.trim().is_empty())
        {
            return Ok(Self::Present(token.trim().to_string()));
        }
        load_token_file(&config.token_path)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Missing => None,
            Self::Present(token) => Some(token.as_str()),
        }
    }
}

#[derive(Clone)]
pub struct McpClient {
    base_url: String,
    tenant: String,
    token: TenantToken,
    client: reqwest::Client,
}

impl McpClient {
    pub fn new(base_url: impl Into<String>, tenant: impl Into<String>, token: TenantToken) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            tenant: tenant.into(),
            token,
            client: reqwest::Client::new(),
        }
    }

    pub fn unauthenticated(base_url: impl Into<String>, tenant: impl Into<String>) -> Self {
        Self::new(base_url, tenant, TenantToken::Missing)
    }

    pub async fn call_tool(&self, name: &str, mut arguments: Value) -> Result<Value> {
        if !arguments.is_object() {
            arguments = json!({});
        }
        if let Some(map) = arguments.as_object_mut() {
            map.entry("tenant".to_string())
                .or_insert_with(|| Value::String(self.tenant.clone()));
        }
        let id = next_rpc_id();
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments
            }
        });

        let mut request = self.client.post(&self.base_url).json(&payload);
        if let Some(token) = self.token.as_str() {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(SyncError::Auth(status.to_string()));
        }
        if !status.is_success() {
            return Err(SyncError::Mcp(format!("HTTP {status}: {text}")));
        }
        let body: Value = serde_json::from_str(&text)?;
        if let Some(error) = body.get("error") {
            return Err(SyncError::Mcp(error.to_string()));
        }
        let result = body
            .get("result")
            .ok_or_else(|| SyncError::Mcp("MCP response missing result".to_string()))?;

        // The MCP transport returns one of two shapes depending on server age:
        //   1. modern:  result.structuredContent = { ... }
        //   2. legacy:  result.content = [{ type: "text", text: "{...json...}" }]
        // The live `rustyredcore-theorem-production` server still ships shape (2)
        // for these verbs, so unwrap content[].text as JSON before falling back
        // to the raw result. Tests that mock structuredContent continue to work.
        if let Some(structured) = result.get("structuredContent").cloned() {
            return Ok(structured);
        }
        if let Some(content) = result.get("content").and_then(Value::as_array) {
            if let Some(text) = content
                .iter()
                .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .and_then(|block| block.get("text").and_then(Value::as_str))
            {
                if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                    return Ok(parsed);
                }
            }
        }
        Ok(result.clone())
    }

    pub async fn doctor(&self) -> ConnectionState {
        if self.token.as_str().is_none() && self.base_url.starts_with("https://") {
            return ConnectionState::Disconnected;
        }
        match self
            .call_tool(
                "rustyred_thg_graph_version_compile",
                json!({"include_payloads": false}),
            )
            .await
        {
            Ok(_) => ConnectionState::Connected,
            Err(SyncError::Auth(_)) => ConnectionState::TokenInvalid,
            Err(_) => ConnectionState::Disconnected,
        }
    }
}

fn load_token_file(path: &Path) -> Result<TenantToken> {
    match fs::read_to_string(path) {
        Ok(token) => {
            let token = token.trim();
            if token.is_empty() {
                Ok(TenantToken::Missing)
            } else {
                Ok(TenantToken::Present(token.to_string()))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(TenantToken::Missing),
        Err(error) => Err(error.into()),
    }
}

fn next_rpc_id() -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    format!("substrate-sync-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_token_file_is_not_an_error() {
        let token = load_token_file(Path::new("/tmp/theorem-substrate-sync-missing-token"))
            .expect("load token");
        assert_eq!(token, TenantToken::Missing);
    }
}
