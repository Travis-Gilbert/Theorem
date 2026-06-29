use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{LocalModelError, LocalModelResult};

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

    pub fn validate_call(&self, name: &str, arguments: &Value) -> LocalModelResult<()> {
        let Some(definition) = self.get(name) else {
            return Err(LocalModelError::Tool(format!("unknown tool '{name}'")));
        };
        let Some(map) = arguments.as_object() else {
            return Err(LocalModelError::Tool(format!(
                "arguments for {name} must be a JSON object"
            )));
        };
        for required in definition.required_arguments() {
            if !map.contains_key(&required) {
                return Err(LocalModelError::Tool(format!(
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
            "You are {actor}, the local Theorem assistant daemon. You are a co-member of the harness coordination room, not a process that embeds other agents. Default room_id: {default_room_id}.\n\n{charter}\n\nReturn exactly one JSON object. For a final reply, return {{\"type\":\"final\",\"text\":\"...\"}}. For a tool call, return {{\"type\":\"tool_call\",\"name\":\"<tool name>\",\"arguments\":{{...}}}}.\n\nUse `coordinate` or `job_submit` when another head should act. Do not spawn Claude or Codex directly.\n\nAvailable tools:\n{tools}",
            charter = charter(),
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

/// The daemon's charter: principles and reasoning that shape how it acts, folded
/// into the system prompt (CHK030-032). Deliberately not tone instructions, and
/// deliberately not optimized for the operator's approval.
pub fn charter() -> &'static str {
    "Charter:\n\
- You steer and rank within bounded, enumerated choices. You submit jobs and relay milestones; you do not edit the graph's topology or another head's work directly. The receiver, not you, launches Claude or Codex.\n\
- You coordinate by footprint over the shared substrate: read others' live intent before acting, declare your own, and reserve @mentions for blocks and forks.\n\
- Capture, completion, and the job board are mechanical truth. Never let a task read more finished than the run actually is: a status is a claim you must be able to back with a receipt.\n\
- Disagreement is licensed. When the evidence or the operator's own stated goals point against a request, say so plainly and propose the better path. Do not shade your reasoning to win approval; an honest objection is worth more than an agreeable one.\n\
- Prefer the smallest action that moves the work, and leave a legible trace of why you took it."
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
            "coordination_intent",
            "harness",
            "Declare your live intent and footprint in the coordination room: what you are doing now and which files your hands are on. Set status=done to close it.",
            json!({
                "type": "object",
                "required": ["actor", "summary"],
                "properties": {
                    "actor": {"type": "string"},
                    "room_id": {"type": "string"},
                    "status": {"type": "string", "enum": ["working", "paused", "done"]},
                    "summary": {"type": "string"},
                    "footprint": {"type": "array", "items": {"type": "string"}}
                }
            }),
        ),
        tool(
            "presence",
            "harness",
            "Heartbeat your presence in the coordination room so peers know you are live right now.",
            json!({
                "type": "object",
                "required": [],
                "properties": {
                    "actor": {"type": "string"},
                    "mode": {"type": "string", "enum": ["get", "heartbeat", "end"]},
                    "status": {"type": "string"},
                    "ttl_seconds": {"type": "integer"}
                }
            }),
        ),
        tool(
            "encode",
            "harness",
            "Write a durable memory: a feedback signal, solution, decision, or postmortem learned this turn that should survive into future sessions.",
            json!({
                "type": "object",
                "required": ["content"],
                "properties": {
                    "content": {"type": "string"},
                    "kind": {"type": "string"},
                    "title": {"type": "string"},
                    "summary": {"type": "string"},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "actor": {"type": "string"}
                }
            }),
        ),
        // TickTick tools mirror the live MCP exactly: every call wraps its fields
        // in a `params` object, the task body is `content`, the list is
        // `project_id`, and priority is the int enum 0/1/3/5. Capture and relay
        // call these mechanically; these definitions also let the model run them
        // for ad-hoc operator requests.
        tool(
            "ticktick_list_projects",
            "ticktick",
            "List all TickTick projects/lists with their ids. Call first to discover a project_id.",
            json!({
                "type": "object",
                "required": ["params"],
                "properties": {
                    "params": {
                        "type": "object",
                        "properties": {
                            "response_format": {"type": "string", "enum": ["markdown", "json"]}
                        }
                    }
                }
            }),
        ),
        tool(
            "ticktick_get_project",
            "ticktick",
            "Get a TickTick project and all its tasks (with subtasks under `items`).",
            json!({
                "type": "object",
                "required": ["params"],
                "properties": {
                    "params": {
                        "type": "object",
                        "required": ["project_id"],
                        "properties": {
                            "project_id": {"type": "string"},
                            "response_format": {"type": "string", "enum": ["markdown", "json"]}
                        }
                    }
                }
            }),
        ),
        tool(
            "ticktick_search_tasks",
            "ticktick",
            "Search tasks within one TickTick project by text/priority/completion.",
            json!({
                "type": "object",
                "required": ["params"],
                "properties": {
                    "params": {
                        "type": "object",
                        "required": ["project_id"],
                        "properties": {
                            "project_id": {"type": "string"},
                            "query": {"type": "string"},
                            "priority": {"type": "integer", "enum": [0, 1, 3, 5]},
                            "include_completed": {"type": "boolean"},
                            "response_format": {"type": "string", "enum": ["markdown", "json"]}
                        }
                    }
                }
            }),
        ),
        tool(
            "ticktick_create_task",
            "ticktick",
            "Create a TickTick task. `content` is the body; priority is 0/1/3/5.",
            json!({
                "type": "object",
                "required": ["params"],
                "properties": {
                    "params": {
                        "type": "object",
                        "required": ["title", "project_id"],
                        "properties": {
                            "title": {"type": "string"},
                            "project_id": {"type": "string"},
                            "content": {"type": "string"},
                            "priority": {"type": "integer", "enum": [0, 1, 3, 5]},
                            "due_date": {"type": "string"}
                        }
                    }
                }
            }),
        ),
        tool(
            "ticktick_update_task",
            "ticktick",
            "Update a TickTick task. Provide task_id and project_id; only provided fields change. `content` is the body.",
            json!({
                "type": "object",
                "required": ["params"],
                "properties": {
                    "params": {
                        "type": "object",
                        "required": ["task_id", "project_id"],
                        "properties": {
                            "task_id": {"type": "string"},
                            "project_id": {"type": "string"},
                            "title": {"type": "string"},
                            "content": {"type": "string"},
                            "priority": {"type": "integer", "enum": [0, 1, 3, 5]}
                        }
                    }
                }
            }),
        ),
        tool(
            "ticktick_complete_task",
            "ticktick",
            "Mark a TickTick task complete. Requires project_id and task_id.",
            json!({
                "type": "object",
                "required": ["params"],
                "properties": {
                    "params": {
                        "type": "object",
                        "required": ["project_id", "task_id"],
                        "properties": {
                            "project_id": {"type": "string"},
                            "task_id": {"type": "string"}
                        }
                    }
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
        let missing = catalog.validate_call("coordinate", &json!({"actor": "localmodel"}));
        assert!(missing.is_err());
        catalog
            .validate_call(
                "coordinate",
                &json!({
                    "actor": "localmodel",
                    "room_id": "repo:theorem:branch:main",
                    "message": "@codex fix this",
                    "urgency": "ask"
                }),
            )
            .unwrap();
    }

    #[test]
    fn system_prompt_contains_the_charter() {
        // CHK032: the charter is inspectable in the prompt the daemon builds.
        let catalog = ToolCatalog::default_catalog();
        let prompt = catalog.system_prompt("theorem-localmodel", "repo:theorem:branch:main");
        assert!(prompt.contains("Charter:"));
        assert!(prompt.contains("Disagreement is licensed"));
        assert!(prompt.contains("mechanical truth"));
    }

    #[test]
    fn ticktick_tools_match_the_live_params_shape() {
        // The live TickTick MCP wraps every call in `params`; a flat schema would
        // make the model's calls fail. The catalog must require `params`.
        let catalog = ToolCatalog::default_catalog();
        for name in [
            "ticktick_get_project",
            "ticktick_update_task",
            "ticktick_complete_task",
        ] {
            let tool = catalog.get(name).expect("ticktick tool present");
            assert_eq!(
                tool.schema["required"][0], "params",
                "{name} requires params"
            );
        }
        // The drifted name is gone.
        assert!(catalog.get("ticktick_list_tasks").is_none());
    }
}
