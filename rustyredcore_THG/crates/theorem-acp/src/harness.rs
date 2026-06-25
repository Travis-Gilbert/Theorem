use std::env;

use crate::protocol::{HttpHeader, McpServer};

pub const DEFAULT_HARNESS_MCP_URL: &str = "http://127.0.0.1:8380/mcp";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessMcpConfig {
    pub name: String,
    pub url: String,
    pub token: Option<String>,
}

impl HarnessMcpConfig {
    pub fn from_env() -> Self {
        Self {
            name: env::var("THEOREM_HARNESS_MCP_NAME")
                .unwrap_or_else(|_| "Theorems-Harness V2".to_string()),
            url: env::var("THEOREM_HARNESS_MCP_URL")
                .unwrap_or_else(|_| DEFAULT_HARNESS_MCP_URL.to_string()),
            token: env::var("THEOREM_HARNESS_TOKEN").ok().filter(|value| !value.is_empty()),
        }
    }

    pub fn into_mcp_server(self) -> McpServer {
        let mut headers = Vec::new();
        if let Some(token) = self.token {
            headers.push(HttpHeader {
                name: "Authorization".to_string(),
                value: format!("Bearer {token}"),
            });
        }
        McpServer::Http {
            name: self.name,
            url: self.url,
            headers,
        }
    }
}

pub fn default_harness_mcp_server() -> McpServer {
    HarnessMcpConfig::from_env().into_mcp_server()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_config_builds_acp_http_mcp_server() {
        let server = HarnessMcpConfig {
            name: "Harness".to_string(),
            url: "http://127.0.0.1:8380/mcp".to_string(),
            token: Some("secret".to_string()),
        }
        .into_mcp_server();

        let value = serde_json::to_value(server).unwrap();
        assert_eq!(value["type"], "http");
        assert_eq!(value["name"], "Harness");
        assert_eq!(value["url"], "http://127.0.0.1:8380/mcp");
        assert_eq!(value["headers"][0]["name"], "Authorization");
    }
}
