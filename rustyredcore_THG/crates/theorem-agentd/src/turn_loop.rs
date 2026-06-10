use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::AgentdConfig;
use crate::ledger::{append_ledger_entry, now_unix_ms, LedgerEntry};
use crate::mcp::McpRouter;
use crate::model::{ChatMessage, ModelClient, ModelDecision};
use crate::tools::ToolCatalog;
use crate::{AgentdError, AgentdResult};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TranscriptEvent {
    Context {
        tool: String,
        result: Value,
    },
    ModelDecision {
        raw_content: String,
        decision: ModelDecision,
    },
    ToolCall {
        name: String,
        arguments: Value,
    },
    ToolResult {
        name: String,
        result: Value,
    },
    Final {
        text: String,
    },
    Ledger {
        path: String,
        written: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transcript {
    pub turn_id: String,
    pub prompt: String,
    pub events: Vec<TranscriptEvent>,
}

impl Transcript {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            turn_id: format!("turn-{}", now_unix_ms()),
            prompt: prompt.into(),
            events: Vec::new(),
        }
    }
}

pub fn run_once(
    config: &AgentdConfig,
    model: &ModelClient,
    router: &McpRouter,
    catalog: &ToolCatalog,
    prompt: &str,
) -> AgentdResult<Transcript> {
    let mut transcript = Transcript::new(prompt);

    // Harness gossip scaffolding: heartbeat presence + open a live intent for
    // this turn, so the daemon is a real co-member of the room, not just a tool
    // caller. Best-effort: never let coordination noise break the turn.
    let _ = router.best_effort_call(
        "presence",
        json!({
            "actor": config.actor,
            "mode": "heartbeat",
            "status": "working",
            "surface": "theorem-agentd",
            "ttl_seconds": 120
        }),
    );
    let _ = router.best_effort_call(
        "coordination_intent",
        json!({
            "actor": config.actor,
            "room_id": config.default_room_id,
            "status": "working",
            "summary": format!(
                "theorem-agentd turn: {}",
                prompt.chars().take(160).collect::<String>()
            )
        }),
    );

    let grammar = catalog.gbnf_grammar();
    let system_prompt = catalog.system_prompt(&config.actor, &config.default_room_id);
    let context = router.best_effort_call(
        "coordination_context",
        json!({"room_id": config.default_room_id}),
    );
    transcript.events.push(TranscriptEvent::Context {
        tool: "coordination_context".to_string(),
        result: context.clone(),
    });
    let recall = router.best_effort_call("recall", json!({"query": prompt, "limit": 3}));
    transcript.events.push(TranscriptEvent::Context {
        tool: "recall".to_string(),
        result: recall.clone(),
    });

    let mut messages = vec![
        ChatMessage::system(system_prompt),
        ChatMessage::user(format!(
            "ROOM_CONTEXT\n{}\n\nRECALL\n{}\n\nUSER\n{}",
            serde_json::to_string_pretty(&context)?,
            serde_json::to_string_pretty(&recall)?,
            prompt
        )),
    ];

    for _ in 0..config.loop_config.max_iterations {
        let output = model.decide(&messages, catalog, &grammar)?;
        let usage = output.usage.clone();
        transcript.events.push(TranscriptEvent::ModelDecision {
            raw_content: output.raw_content.clone(),
            decision: output.decision.clone(),
        });
        match output.decision {
            ModelDecision::Final { text } => {
                // Turn-end: close the intent as done with the reply as the handoff.
                let _ = router.best_effort_call(
                    "coordination_intent",
                    json!({
                        "actor": config.actor,
                        "room_id": config.default_room_id,
                        "status": "done",
                        "summary": format!(
                            "theorem-agentd: {}",
                            text.chars().take(160).collect::<String>()
                        )
                    }),
                );
                transcript.events.push(TranscriptEvent::Final { text });
                append_ledger_entry(
                    &config.ledger.path,
                    &LedgerEntry::new(
                        transcript.turn_id.clone(),
                        prompt.to_string(),
                        "once".to_string(),
                        None,
                        usage,
                    ),
                )?;
                transcript.events.push(TranscriptEvent::Ledger {
                    path: config.ledger.path.display().to_string(),
                    written: true,
                });
                return Ok(transcript);
            }
            ModelDecision::ToolCall { name, arguments } => {
                catalog.validate_call(&name, &arguments)?;
                transcript.events.push(TranscriptEvent::ToolCall {
                    name: name.clone(),
                    arguments: arguments.clone(),
                });
                let result = router.call_tool(catalog, &name, arguments)?;
                transcript.events.push(TranscriptEvent::ToolResult {
                    name: name.clone(),
                    result: result.clone(),
                });
                append_ledger_entry(
                    &config.ledger.path,
                    &LedgerEntry::new(
                        transcript.turn_id.clone(),
                        prompt.to_string(),
                        "once".to_string(),
                        Some(name.clone()),
                        usage,
                    ),
                )?;
                messages.push(ChatMessage::assistant(output.raw_content));
                messages.push(ChatMessage::tool(format!(
                    "TOOL_RESULT {} {}",
                    name,
                    serde_json::to_string(&result)?
                )));
            }
        }
    }
    let _ = router.best_effort_call(
        "coordination_intent",
        json!({
            "actor": config.actor,
            "room_id": config.default_room_id,
            "status": "paused",
            "summary": "theorem-agentd: max_iterations reached without final reply"
        }),
    );
    Err(AgentdError::Model(format!(
        "max_iterations {} reached without final reply",
        config.loop_config.max_iterations
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RuleModelClient;

    #[test]
    fn transcript_records_tool_result_shape() {
        let mut transcript = Transcript::new("prompt");
        transcript.events.push(TranscriptEvent::ToolResult {
            name: "coordination_context".to_string(),
            result: json!({"ok": true}),
        });
        let raw = serde_json::to_string(&transcript).unwrap();
        assert!(raw.contains("coordination_context"));
    }

    #[test]
    fn deterministic_coordinate_call_is_schema_valid() {
        let catalog = ToolCatalog::default_catalog();
        let model = RuleModelClient {
            default_room_id: "repo:theorem:branch:main".to_string(),
            actor: "theorem-agentd".to_string(),
        };
        let output = model
            .decide(&[ChatMessage::user("have an agent fix the failing test")])
            .unwrap();
        let ModelDecision::ToolCall { name, arguments } = output.decision else {
            panic!("expected tool call");
        };
        assert_eq!(name, "coordinate");
        catalog.validate_call(&name, &arguments).unwrap();
        assert_eq!(arguments["delivery"], "wake");
        assert_eq!(arguments["mentions"][0], "codex");
    }
}
