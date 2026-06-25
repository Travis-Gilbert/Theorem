use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: impl Into<Value>, method: impl Into<String>, params: impl Serialize) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: id.into(),
            method: method.into(),
            params: Some(serde_json::to_value(params).expect("ACP params must serialize")),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),
    Response(JsonRpcResponse),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Implementation {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClientCapabilities {
    pub fs: FsCapabilities,
    pub terminal: bool,
}

impl ClientCapabilities {
    pub fn commonplace_host() -> Self {
        Self {
            fs: FsCapabilities {
                read_text_file: true,
                write_text_file: true,
            },
            terminal: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FsCapabilities {
    #[serde(rename = "readTextFile")]
    pub read_text_file: bool,
    #[serde(rename = "writeTextFile")]
    pub write_text_file: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InitializeRequest {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
    #[serde(rename = "clientCapabilities")]
    pub client_capabilities: ClientCapabilities,
    #[serde(rename = "clientInfo")]
    pub client_info: Implementation,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NewSessionRequest {
    pub cwd: String,
    #[serde(rename = "mcpServers")]
    pub mcp_servers: Vec<McpServer>,
    #[serde(
        rename = "additionalDirectories",
        skip_serializing_if = "Vec::is_empty",
        default
    )]
    pub additional_directories: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PromptRequest {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub prompt: Vec<ContentBlock>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ContentBlock {
    Text { text: String },
    ResourceLink { uri: String, name: Option<String> },
    Resource {
        uri: String,
        text: String,
        #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase", tag = "type")]
pub enum McpServer {
    Http {
        name: String,
        url: String,
        headers: Vec<HttpHeader>,
    },
    Sse {
        name: String,
        url: String,
        headers: Vec<HttpHeader>,
    },
    Stdio {
        name: String,
        command: String,
        args: Vec<String>,
        env: Vec<EnvVariable>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EnvVariable {
    pub name: String,
    pub value: String,
}

pub fn initialize_request(id: impl Into<Value>, client_version: impl Into<String>) -> JsonRpcRequest {
    JsonRpcRequest::new(
        id,
        "initialize",
        InitializeRequest {
            protocol_version: PROTOCOL_VERSION,
            client_capabilities: ClientCapabilities::commonplace_host(),
            client_info: Implementation {
                name: "CommonPlace ACP Host".to_string(),
                version: client_version.into(),
            },
        },
    )
}

pub fn new_session_request(
    id: impl Into<Value>,
    cwd: impl Into<String>,
    mcp_servers: Vec<McpServer>,
    additional_directories: Vec<String>,
) -> JsonRpcRequest {
    JsonRpcRequest::new(
        id,
        "session/new",
        NewSessionRequest {
            cwd: cwd.into(),
            mcp_servers,
            additional_directories,
        },
    )
}

pub fn prompt_request(
    id: impl Into<Value>,
    session_id: impl Into<String>,
    text: impl Into<String>,
) -> JsonRpcRequest {
    JsonRpcRequest::new(
        id,
        "session/prompt",
        PromptRequest {
            session_id: session_id.into(),
            prompt: vec![ContentBlock::text(text)],
        },
    )
}

pub fn session_update_envelope(session_id: impl Into<String>, update: Value) -> Value {
    json!({
        "sessionId": session_id.into(),
        "update": update,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_advertises_fs_and_terminal_capabilities() {
        let request = initialize_request(1, "0.1.0");
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["method"], "initialize");
        assert_eq!(value["params"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(
            value["params"]["clientCapabilities"]["fs"]["readTextFile"],
            true
        );
        assert_eq!(
            value["params"]["clientCapabilities"]["fs"]["writeTextFile"],
            true
        );
        assert_eq!(value["params"]["clientCapabilities"]["terminal"], true);
    }

    #[test]
    fn prompt_serializes_text_content_block() {
        let request = prompt_request("p1", "session-1", "hello");
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["method"], "session/prompt");
        assert_eq!(value["params"]["sessionId"], "session-1");
        assert_eq!(value["params"]["prompt"][0]["type"], "text");
        assert_eq!(value["params"]["prompt"][0]["text"], "hello");
    }
}
