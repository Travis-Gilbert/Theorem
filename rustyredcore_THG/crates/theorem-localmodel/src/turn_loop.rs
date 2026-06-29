use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::capture::{self, CaptureReport};
use crate::config::LocalModelConfig;
use crate::ledger::{append_ledger_entry, now_unix_ms, LedgerEntry};
use crate::mcp::McpRouter;
use crate::model::{ChatMessage, ModelClient, ModelDecision};
use crate::relay::{self, RelayReport};
use crate::tools::ToolCatalog;
use crate::{LocalModelError, LocalModelResult};

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
    config: &LocalModelConfig,
    model: &ModelClient,
    router: &McpRouter,
    catalog: &ToolCatalog,
    prompt: &str,
) -> LocalModelResult<Transcript> {
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
            "surface": "theorem-localmodel",
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
                "theorem-localmodel turn: {}",
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
    // Personal memory is the operator's: route recall to the named tenant.
    // Coordination, jobs, and shared substrate stay on the harness default tenant
    // (CHK023-026). The explicit tenant_slug wins over the client's default.
    let recall = router.best_effort_call(
        "recall",
        json!({
            "query": prompt,
            "limit": 3,
            "tenant_slug": config.operator_memory_tenant,
        }),
    );
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
                            "theorem-localmodel: {}",
                            text.chars().take(160).collect::<String>()
                        )
                    }),
                );
                transcript.events.push(TranscriptEvent::Final { text });
                let entry = LedgerEntry::new(
                    transcript.turn_id.clone(),
                    prompt.to_string(),
                    "once".to_string(),
                    None,
                    usage,
                );
                append_ledger_entry(&config.ledger.path, &entry)?;
                mirror_ledger_to_graph(config, router, &entry);
                transcript.events.push(TranscriptEvent::Ledger {
                    path: config.ledger.path.display().to_string(),
                    written: true,
                });
                return Ok(transcript);
            }
            ModelDecision::ToolCall {
                name,
                mut arguments,
            } => {
                // Personal-memory writes/reads the model initiates go to the
                // operator's tenant; coordination and job tools stay on default
                // (CHK024/CHK025).
                route_personal_memory_tenant(&name, &mut arguments, &config.operator_memory_tenant);
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
                let entry = LedgerEntry::new(
                    transcript.turn_id.clone(),
                    prompt.to_string(),
                    "once".to_string(),
                    Some(name.clone()),
                    usage,
                );
                append_ledger_entry(&config.ledger.path, &entry)?;
                mirror_ledger_to_graph(config, router, &entry);
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
            "summary": "theorem-localmodel: max_iterations reached without final reply"
        }),
    );
    Err(LocalModelError::Model(format!(
        "max_iterations {} reached without final reply",
        config.loop_config.max_iterations
    )))
}

/// What one full tick did, for logging and tests.
#[derive(Debug, Default)]
pub struct TickReport {
    pub capture: Option<CaptureReport>,
    pub relay: Option<RelayReport>,
    pub transcript: Option<Transcript>,
}

/// One full daemon tick: the mechanical Agent Queue capture sweep, the milestone
/// relay sweep, then the model turn. Capture and relay are best-effort so a
/// TickTick or harness hiccup logs and is skipped rather than stalling the loop;
/// each is gated by its own config switch.
pub fn run_tick(
    config: &LocalModelConfig,
    model: &ModelClient,
    router: &McpRouter,
    catalog: &ToolCatalog,
    prompt: &str,
) -> TickReport {
    let mut report = TickReport::default();

    if config.capture.enabled {
        match capture::run_capture(router, &config.capture, &config.actor) {
            Ok(captured) => {
                if !captured.captured.is_empty() {
                    eprintln!(
                        "[theorem-localmodel] captured {} Agent Queue task(s) into jobs",
                        captured.captured.len()
                    );
                }
                report.capture = Some(captured);
            }
            Err(error) => eprintln!("[theorem-localmodel] capture error: {error}"),
        }
    }

    if config.relay.enabled {
        match relay::run_relays(router, model, &config.actor) {
            Ok(relayed) => {
                if !relayed.relayed.is_empty() {
                    eprintln!(
                        "[theorem-localmodel] relayed {} milestone(s) to TickTick",
                        relayed.relayed.len()
                    );
                }
                report.relay = Some(relayed);
            }
            Err(error) => eprintln!("[theorem-localmodel] relay error: {error}"),
        }
    }

    match run_once(config, model, router, catalog, prompt) {
        Ok(transcript) => report.transcript = Some(transcript),
        Err(error) => eprintln!("[theorem-localmodel] tick turn error: {error}"),
    }
    report
}

/// Personal-memory tools whose calls are scoped to the operator's tenant. Every
/// other tool (coordination, jobs, presence, TickTick) stays on the harness
/// default tenant (CHK024).
fn is_personal_memory_tool(name: &str) -> bool {
    matches!(
        name,
        "recall"
            | "remember"
            | "encode"
            | "forget"
            | "self_note"
            | "self_revise"
            | "self_archive"
            | "self_recall_archive"
    )
}

/// Inject the operator memory tenant into a personal-memory tool call, unless the
/// model already set one. Coordination and job tools are left untouched so the
/// shared substrate stays on the default tenant.
fn route_personal_memory_tenant(name: &str, arguments: &mut Value, tenant: &str) {
    if !is_personal_memory_tool(name) {
        return;
    }
    if let Value::Object(map) = arguments {
        map.entry("tenant_slug".to_string())
            .or_insert_with(|| json!(tenant));
    }
}

/// Best-effort mirror of a ledger entry into the graph as a typed self-memory
/// receipt (the label factory, CHK027-029). The JSONL ledger stays the source of
/// truth; this accumulates a retrievable corpus beside the Claude Code and Codex
/// traces, namespaced by its own `kind` so it never pollutes belief recall.
/// Failures are swallowed so the loop never breaks on a mirror.
fn mirror_ledger_to_graph(config: &LocalModelConfig, router: &McpRouter, entry: &LedgerEntry) {
    if !config.ledger.mirror_to_graph {
        return;
    }
    let content = serde_json::to_string(entry).unwrap_or_default();
    let summary = match &entry.tool_name {
        Some(tool) => format!("localmodel turn {} called {tool}", entry.turn_id),
        None => format!("localmodel turn {} final reply", entry.turn_id),
    };
    let _ = router.best_effort_call(
        "self_note",
        json!({
            "actor": config.actor,
            "kind": "localmodel_ledger",
            "memory_node_type": "trace",
            "title": entry.turn_id,
            "summary": summary,
            "content": content,
            "tags": ["localmodel", "ledger", "label-factory"],
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RuleModelClient;

    #[test]
    fn personal_memory_tools_route_to_operator_tenant() {
        // CHK024/CHK025: personal memory goes to the named tenant; coordination
        // stays on default; an explicit tenant the model set is respected.
        let mut recall = json!({ "query": "x" });
        route_personal_memory_tenant("recall", &mut recall, "operator:travis");
        assert_eq!(recall["tenant_slug"], "operator:travis");

        let mut coordinate = json!({ "actor": "a", "room_id": "r" });
        route_personal_memory_tenant("coordinate", &mut coordinate, "operator:travis");
        assert!(coordinate.get("tenant_slug").is_none());

        let mut job = json!({ "title": "t", "repo": "r" });
        route_personal_memory_tenant("job_submit", &mut job, "operator:travis");
        assert!(job.get("tenant_slug").is_none());

        let mut explicit = json!({ "content": "x", "tenant_slug": "shared" });
        route_personal_memory_tenant("encode", &mut explicit, "operator:travis");
        assert_eq!(explicit["tenant_slug"], "shared");
    }

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
            actor: "theorem-localmodel".to_string(),
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
