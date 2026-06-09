use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{AgentdError, AgentdResult};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub server: String,
    pub description: String,
    pub schema: Value,
}

impl ToolDefinition {
    pub fn required_arguments(&self) -> BTreeSet<String> {
        self.schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct ToolCatalog {
    tools: BTreeMap<String, ToolDefinition>,
}

impl ToolCatalog {
    pub fn default_catalog() -> Self {
        let mut catalog = Self {
            tools: BTreeMap::new(),
        };
        for tool in default_tools() {
            catalog.insert(tool);
        }
        catalog
    }

    pub fn insert(&mut self, definition: ToolDefinition) {
        self.tools.insert(definition.name.clone(), definition);
    }

    pub fn get(&self, name: &str) -> Option<&ToolDefinition> {
        self.tools.get(name)
    }

    pub fn tools(&self) -> impl Iterator<Item = &ToolDefinition> {
        self.tools.values()
    }

    pub fn validate_call(&self, name: &str, arguments: &Value) -> AgentdResult<()> {
        let Some(definition) = self.get(name) else {
            return Err(AgentdError::Tool(format!("unknown tool '{name}'")));
        };
        let Some(map) = arguments.as_object() else {
            return Err(AgentdError::Tool(format!(
                "arguments for {name} must be a JSON object"
            )));
        };
        for required in definition.required_arguments() {
            if !map.contains_key(&required) {
                return Err(AgentdError::Tool(format!(
                    "arguments for {name} missing required key '{required}'"
                )));
            }
        }
        Ok(())
    }

    pub fn system_prompt(&self, actor: &str, default_room_id: &str) -> String {
        let tools = self
            .tools()
            .map(|tool| {
                format!(
                    "- {} on server `{}`: {}\n  schema: {}",
                    tool.name, tool.server, tool.description, tool.schema
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "You are {actor}, the local Theorem assistant daemon. You are a co-member of the harness coordination room, not a process that embeds other agents. Default room_id: {default_room_id}.\n\nReturn exactly one JSON object. For a final reply, return {{\"type\":\"final\",\"text\":\"...\"}}. For a tool call, return {{\"type\":\"tool_call\",\"name\":\"<tool name>\",\"arguments\":{{...}}}}.\n\nUse `coordinate` or `job_submit` when another head should act. Do not spawn Claude or Codex directly.\n\nAvailable tools:\n{tools}"
        )
    }

    pub fn gbnf_grammar(&self) -> String {
        let names = self
            .tools()
            .map(|tool| format!("\"{}\"", escape_gbnf(&tool.name)))
            .collect::<Vec<_>>()
            .join(" | ");
        format!(
            r#"root ::= final | tool_call
final ::= "{{" ws "\"type\"" ws ":" ws "\"final\"" ws "," ws "\"text\"" ws ":" ws string ws "}}"
tool_call ::= "{{" ws "\"type\"" ws ":" ws "\"tool_call\"" ws "," ws "\"name\"" ws ":" ws tool_name ws "," ws "\"arguments\"" ws ":" ws object ws "}}"
tool_name ::= {names}
object ::= "{{" ws (member (ws "," ws member)*)? ws "}}"
member ::= string ws ":" ws value
array ::= "[" ws (value (ws "," ws value)*)? ws "]"
value ::= object | array | string | number | "true" | "false" | "null"
string ::= "\"" char* "\""
char ::= [^"\\] | "\\" (["\\/bfnrt] | "u" hex hex hex hex)
hex ::= [0-9a-fA-F]
number ::= "-"? ([0-9] | [1-9][0-9]+) ("." [0-9]+)? ([eE] [-+]? [0-9]+)?
ws ::= [ \t\n\r]*
"#
        )
    }

    /// The catalog as an OpenAI-style `tools` array. Tool-calling models (Gemma
    /// via llama-server) return structured `tool_calls` from this, which is far
    /// more reliable than constraining free-form output with a GBNF grammar that
    /// the model's chat template fights.
    pub fn openai_tools(&self) -> Vec<Value> {
        self.tools()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.schema
                    }
                })
            })
            .collect()
    }
}

fn default_tools() -> Vec<ToolDefinition> {
    vec![
        tool(
            "coordinate",
            "harness",
            "Post a coordination-room message. Use delivery=\"wake\" plus mentions when Codex or Claude Code should be woken.",
            json!({
                "type": "object",
                "required": ["actor", "room_id", "message", "urgency"],
                "properties": {
                    "actor": {"type": "string"},
                    "room_id": {"type": "string"},
                    "delivery": {"type": "string", "enum": ["passive", "wake"]},
                    "mentions": {"type": "array", "items": {"type": "string"}},
                    "message": {"type": "string"},
                    "metadata": {"type": "object"},
                    "urgency": {"type": "string", "enum": ["info", "ask", "block"]},
                    "wake": {"type": "boolean"}
                }
            }),
        ),
        tool(
            "job_submit",
            "harness",
            "Submit a dispatch-queue job for an existing receiver to claim.",
            json!({
                "type": "object",
                "required": ["title", "repo", "submitted_by"],
                "properties": {
                    "title": {"type": "string"},
                    "repo": {"type": "string"},
                    "submitted_by": {"type": "string"},
                    "priority": {"type": "string"},
                    "target_head": {"type": "string"},
                    "spec_ref": {"type": "string"},
                    "spec_inline": {"type": "string"},
                    "idempotency_key": {"type": "string"}
                }
            }),
        ),
        tool(
            "job_list",
            "harness",
            "List dispatch-queue jobs.",
            json!({
                "type": "object",
                "required": [],
                "properties": {
                    "repo": {"type": "string"},
                    "state": {"type": "string"}
                }
            }),
        ),
        tool(
            "read_messages_for_room",
            "harness",
            "Read recent coordination-room messages.",
            json!({
                "type": "object",
                "required": ["room_id"],
                "properties": {
                    "room_id": {"type": "string"},
                    "limit": {"type": "integer"}
                }
            }),
        ),
        tool(
            "coordination_context",
            "harness",
            "Read the current coordination-room context packet.",
            json!({
                "type": "object",
                "required": ["room_id"],
                "properties": {
                    "room_id": {"type": "string"},
                    "job_id": {"type": "string"}
                }
            }),
        ),
        tool(
            "recall",
            "harness",
            "Recall typed memory relevant to the current user turn.",
            json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer"},
                    "include_low_fitness": {"type": "boolean"}
                }
            }),
        ),
        tool(
            "remember",
            "harness",
            "Write typed memory when a durable learning should survive future turns.",
            json!({
                "type": "object",
                "required": ["kind", "content"],
                "properties": {
                    "kind": {"type": "string"},
                    "content": {"type": "string"},
                    "links": {"type": "array", "items": {"type": "string"}}
                }
            }),
        ),
        tool(
            "ticktick_list_tasks",
            "ticktick",
            "List TickTick tasks.",
            json!({
                "type": "object",
                "required": [],
                "properties": {
                    "query": {"type": "string"},
                    "project": {"type": "string"}
                }
            }),
        ),
        tool(
            "ticktick_create_task",
            "ticktick",
            "Create a TickTick task.",
            json!({
                "type": "object",
                "required": ["title"],
                "properties": {
                    "title": {"type": "string"},
                    "notes": {"type": "string"},
                    "due": {"type": "string"},
                    "project": {"type": "string"}
                }
            }),
        ),
        tool(
            "ticktick_update_task",
            "ticktick",
            "Update a TickTick task.",
            json!({
                "type": "object",
                "required": ["task_id"],
                "properties": {
                    "task_id": {"type": "string"},
                    "title": {"type": "string"},
                    "notes": {"type": "string"},
                    "due": {"type": "string"}
                }
            }),
        ),
        tool(
            "ticktick_complete_task",
            "ticktick",
            "Complete a TickTick task.",
            json!({
                "type": "object",
                "required": ["task_id"],
                "properties": {
                    "task_id": {"type": "string"}
                }
            }),
        ),
        tool(
            "gmail_search",
            "gmail",
            "Search Gmail messages.",
            json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer"}
                }
            }),
        ),
        tool(
            "gmail_draft",
            "gmail",
            "Create a Gmail draft.",
            json!({
                "type": "object",
                "required": ["to", "subject", "body"],
                "properties": {
                    "to": {"type": "string"},
                    "subject": {"type": "string"},
                    "body": {"type": "string"}
                }
            }),
        ),
        tool(
            "gmail_label",
            "gmail",
            "Apply a Gmail label.",
            json!({
                "type": "object",
                "required": ["message_id", "label"],
                "properties": {
                    "message_id": {"type": "string"},
                    "label": {"type": "string"}
                }
            }),
        ),
        tool(
            "calendar_list_events",
            "calendar",
            "List Google Calendar events.",
            json!({
                "type": "object",
                "required": [],
                "properties": {
                    "start": {"type": "string"},
                    "end": {"type": "string"},
                    "query": {"type": "string"}
                }
            }),
        ),
        tool(
            "calendar_create_event",
            "calendar",
            "Create a Google Calendar event.",
            json!({
                "type": "object",
                "required": ["title", "start", "end"],
                "properties": {
                    "title": {"type": "string"},
                    "start": {"type": "string"},
                    "end": {"type": "string"},
                    "attendees": {"type": "array", "items": {"type": "string"}},
                    "notes": {"type": "string"}
                }
            }),
        ),
        tool(
            "filesystem_read",
            "filesystem",
            "Read a local file through the configured filesystem MCP server.",
            json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string"}
                }
            }),
        ),
    ]
}

fn tool(name: &str, server: &str, description: &str, schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        server: server.to_string(),
        description: description.to_string(),
        schema,
    }
}

fn escape_gbnf(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar_is_closed_over_catalog_names() {
        let catalog = ToolCatalog::default_catalog();
        let grammar = catalog.gbnf_grammar();
        assert!(grammar.contains("\"coordinate\""));
        assert!(grammar.contains("\"ticktick_create_task\""));
        assert!(!grammar.contains("\"made_up_tool\""));
    }

    #[test]
    fn validates_required_arguments() {
        let catalog = ToolCatalog::default_catalog();
        let missing = catalog.validate_call("coordinate", &json!({"actor": "agentd"}));
        assert!(missing.is_err());
        catalog
            .validate_call(
                "coordinate",
                &json!({
                    "actor": "agentd",
                    "room_id": "repo:theorem:branch:main",
                    "message": "@codex fix this",
                    "urgency": "ask"
                }),
            )
            .unwrap();
    }
}
