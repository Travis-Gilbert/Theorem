use crate::{
    heartbeat_presence, join_room, read_mentions_for_actor_in_room,
    run_configured_composed_agent_with_claims, write_message, write_record, ComposedAgentRunResult,
    ComposedAgentRuntimeError, CoordinationError, CoordinationMessageState,
    CoordinationPresenceState, CoordinationRecordState, CoordinationRoomState, JoinRoomInput,
    PresenceInput, WriteMessageInput, WriteRecordInput, DEFAULT_BINDING_ID,
};
use rustyred_thg_core::GraphStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::error::Error;
use std::fmt;
use theorem_harness_core::{GroundedClaim, HeadInvoker};

pub const DEFAULT_AGENT_ACTOR: &str = "theorem";
pub const DEFAULT_AGENT_SURFACE: &str = "theorem-agent-runner";
pub const DEFAULT_MENTION_LIMIT: usize = 8;
pub const DEFAULT_HEARTBEAT_TTL_SECONDS: u64 = 60;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentRoomRunnerConfig {
    pub tenant_slug: String,
    pub room_id: String,
    pub binding_id: String,
    pub actor_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub surface: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub worktree: String,
    #[serde(default = "default_heartbeat_ttl_seconds")]
    pub heartbeat_ttl_seconds: u64,
    #[serde(default = "default_mention_limit")]
    pub mention_limit: usize,
    #[serde(default)]
    pub reply_to_requester: bool,
}

impl AgentRoomRunnerConfig {
    pub fn new(
        tenant_slug: impl Into<String>,
        room_id: impl Into<String>,
        binding_id: impl Into<String>,
    ) -> Self {
        Self {
            tenant_slug: tenant_slug.into(),
            room_id: room_id.into(),
            binding_id: binding_id.into(),
            actor_id: DEFAULT_AGENT_ACTOR.to_string(),
            session_id: String::new(),
            surface: DEFAULT_AGENT_SURFACE.to_string(),
            repo: String::new(),
            branch: String::new(),
            task: String::new(),
            worktree: String::new(),
            heartbeat_ttl_seconds: DEFAULT_HEARTBEAT_TTL_SECONDS,
            mention_limit: DEFAULT_MENTION_LIMIT,
            reply_to_requester: false,
        }
    }

    pub fn theorem_default(tenant_slug: impl Into<String>, room_id: impl Into<String>) -> Self {
        Self::new(tenant_slug, room_id, DEFAULT_BINDING_ID)
    }

    fn actor_id(&self) -> String {
        nonempty_or(&self.actor_id, DEFAULT_AGENT_ACTOR)
    }

    fn binding_id(&self) -> String {
        nonempty_or(&self.binding_id, DEFAULT_BINDING_ID)
    }

    fn surface(&self) -> String {
        nonempty_or(&self.surface, DEFAULT_AGENT_SURFACE)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentRoomRunnerCycle {
    pub room: CoordinationRoomState,
    pub presence: CoordinationPresenceState,
    #[serde(default)]
    pub turns: Vec<AgentRoomRunnerTurn>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentRoomRunnerTurn {
    pub source_message_id: String,
    pub source_actor_id: String,
    pub status: AgentRoomRunnerTurnStatus,
    #[serde(default)]
    pub run: Option<ComposedAgentRunResult>,
    #[serde(default)]
    pub contribution: Option<CoordinationRecordState>,
    #[serde(default)]
    pub reflection: Option<CoordinationRecordState>,
    #[serde(default)]
    pub reply: Option<CoordinationMessageState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRoomRunnerTurnStatus {
    Contributed,
    Blocked,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentRoomRunnerError {
    Coordination(CoordinationError),
}

impl fmt::Display for AgentRoomRunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Coordination(error) => write!(f, "{error}"),
        }
    }
}

impl Error for AgentRoomRunnerError {}

impl From<CoordinationError> for AgentRoomRunnerError {
    fn from(value: CoordinationError) -> Self {
        Self::Coordination(value)
    }
}

pub fn run_agent_room_cycle<S: GraphStore, I: HeadInvoker>(
    store: &mut S,
    config: AgentRoomRunnerConfig,
    invoker: &I,
) -> Result<AgentRoomRunnerCycle, AgentRoomRunnerError> {
    let actor_id = config.actor_id();
    let binding_id = config.binding_id();
    let surface = config.surface();
    let room = join_room(
        store,
        JoinRoomInput {
            tenant_slug: config.tenant_slug.clone(),
            actor_id: actor_id.clone(),
            room_id: config.room_id.clone(),
            session_id: config.session_id.clone(),
            surface: surface.clone(),
            repo: config.repo.clone(),
            branch: config.branch.clone(),
            task: config.task.clone(),
            worktree: config.worktree.clone(),
            head: actor_id.clone(),
            changed_files: Vec::new(),
            lane: "agent-room-runner".to_string(),
            updated_at: String::new(),
        },
    )?;
    let presence = heartbeat_presence(
        store,
        PresenceInput {
            tenant_slug: config.tenant_slug.clone(),
            actor_id: actor_id.clone(),
            session_id: config.session_id.clone(),
            surface,
            status: "active".to_string(),
            worktree: config.worktree.clone(),
            branch: config.branch.clone(),
            head: actor_id.clone(),
            changed_files: Vec::new(),
            ttl_seconds: config.heartbeat_ttl_seconds.max(1),
            refreshed_at: String::new(),
            expires_at: String::new(),
        },
    )?;
    let mentions = read_mentions_for_actor_in_room(
        store,
        &config.tenant_slug,
        &config.room_id,
        &actor_id,
        true,
        config.mention_limit,
    )?;
    let mut turns = Vec::new();
    for mention in mentions {
        let claims = vec![GroundedClaim::new(
            mention.message.clone(),
            format!("coordination_message:{}", mention.message_id),
        )];
        let turn = match run_configured_composed_agent_with_claims(
            store,
            &binding_id,
            &mention.message,
            claims,
            invoker,
        ) {
            Ok(result) if alignment_allowed(&result) => {
                let contribution =
                    write_contribution(store, &config, &actor_id, &mention, &result)?;
                let reflection = write_turn_reflection(
                    store,
                    &config,
                    &actor_id,
                    &mention,
                    "Theorem agent contributed an alignment-gated room result.",
                    Some(&result),
                    None,
                )?;
                let reply = if config.reply_to_requester {
                    Some(write_requester_reply(
                        store,
                        &config,
                        &actor_id,
                        &mention,
                        contribution.summary.as_str(),
                    )?)
                } else {
                    None
                };
                AgentRoomRunnerTurn {
                    source_message_id: mention.message_id,
                    source_actor_id: mention.actor_id,
                    status: AgentRoomRunnerTurnStatus::Contributed,
                    run: Some(result),
                    contribution: Some(contribution),
                    reflection: Some(reflection),
                    reply,
                }
            }
            Ok(result) => {
                let reflection = write_turn_reflection(
                    store,
                    &config,
                    &actor_id,
                    &mention,
                    "Theorem agent blocked publication at the alignment gate.",
                    Some(&result),
                    Some(result.alignment_verdict.clone()),
                )?;
                AgentRoomRunnerTurn {
                    source_message_id: mention.message_id,
                    source_actor_id: mention.actor_id,
                    status: AgentRoomRunnerTurnStatus::Blocked,
                    run: Some(result),
                    contribution: None,
                    reflection: Some(reflection),
                    reply: None,
                }
            }
            Err(error) => {
                let reflection = write_turn_reflection(
                    store,
                    &config,
                    &actor_id,
                    &mention,
                    "Theorem agent blocked publication before room contribution.",
                    None,
                    Some(runtime_error_verdict(&error)),
                )?;
                AgentRoomRunnerTurn {
                    source_message_id: mention.message_id,
                    source_actor_id: mention.actor_id,
                    status: AgentRoomRunnerTurnStatus::Blocked,
                    run: None,
                    contribution: None,
                    reflection: Some(reflection),
                    reply: None,
                }
            }
        };
        turns.push(turn);
    }

    Ok(AgentRoomRunnerCycle {
        room,
        presence,
        turns,
    })
}

fn write_contribution<S: GraphStore>(
    store: &mut S,
    config: &AgentRoomRunnerConfig,
    actor_id: &str,
    mention: &CoordinationMessageState,
    result: &ComposedAgentRunResult,
) -> Result<CoordinationRecordState, CoordinationError> {
    let summary = contribution_summary(result);
    write_record(
        store,
        WriteRecordInput {
            tenant_slug: config.tenant_slug.clone(),
            room_id: config.room_id.clone(),
            actor_id: actor_id.to_string(),
            record_id: String::new(),
            record_type: "event".to_string(),
            title: "Theorem agent contribution".to_string(),
            summary,
            body: synthesis_text(result),
            metadata: contribution_metadata(mention, result),
            created_at: String::new(),
        },
    )
}

fn write_turn_reflection<S: GraphStore>(
    store: &mut S,
    config: &AgentRoomRunnerConfig,
    actor_id: &str,
    mention: &CoordinationMessageState,
    summary: &str,
    result: Option<&ComposedAgentRunResult>,
    blocked_verdict: Option<Value>,
) -> Result<CoordinationRecordState, CoordinationError> {
    let mut metadata = Map::new();
    metadata.insert("source".to_string(), json!("agent_room_runner"));
    metadata.insert("source_message_id".to_string(), json!(mention.message_id));
    metadata.insert("source_actor_id".to_string(), json!(mention.actor_id));
    if let Some(result) = result {
        metadata.insert("run_id".to_string(), json!(result.run_id.clone()));
        metadata.insert("binding_id".to_string(), json!(result.binding_id.clone()));
        metadata.insert(
            "alignment_verdict".to_string(),
            result.alignment_verdict.clone(),
        );
        metadata.insert(
            "consensus_head_set".to_string(),
            json!(result.consensus_head_set.clone()),
        );
    }
    if let Some(blocked_verdict) = blocked_verdict {
        metadata.insert("blocked_verdict".to_string(), blocked_verdict);
    }
    write_record(
        store,
        WriteRecordInput {
            tenant_slug: config.tenant_slug.clone(),
            room_id: config.room_id.clone(),
            actor_id: actor_id.to_string(),
            record_id: String::new(),
            record_type: "reflection".to_string(),
            title: "Theorem agent turn reflection".to_string(),
            summary: summary.to_string(),
            body: mention.message.clone(),
            metadata,
            created_at: String::new(),
        },
    )
}

fn write_requester_reply<S: GraphStore>(
    store: &mut S,
    config: &AgentRoomRunnerConfig,
    actor_id: &str,
    mention: &CoordinationMessageState,
    contribution_summary: &str,
) -> Result<CoordinationMessageState, CoordinationError> {
    write_message(
        store,
        WriteMessageInput {
            tenant_slug: config.tenant_slug.clone(),
            room_id: config.room_id.clone(),
            actor_id: actor_id.to_string(),
            message_id: String::new(),
            urgency: "info".to_string(),
            delivery: "passive".to_string(),
            message: format!(
                "@{} theorem contribution ready: {}",
                mention.actor_id, contribution_summary
            ),
            mentions: vec![mention.actor_id.clone()],
            metadata: Map::new(),
            created_at: String::new(),
        },
    )
}

fn contribution_metadata(
    mention: &CoordinationMessageState,
    result: &ComposedAgentRunResult,
) -> Map<String, Value> {
    let mut metadata = Map::new();
    metadata.insert("source".to_string(), json!("agent_room_runner"));
    metadata.insert("source_message_id".to_string(), json!(mention.message_id));
    metadata.insert("source_actor_id".to_string(), json!(mention.actor_id));
    metadata.insert("run_id".to_string(), json!(result.run_id.clone()));
    metadata.insert("binding_id".to_string(), json!(result.binding_id.clone()));
    metadata.insert(
        "alignment_verdict".to_string(),
        result.alignment_verdict.clone(),
    );
    metadata.insert(
        "consensus_head_set".to_string(),
        json!(result.consensus_head_set.clone()),
    );
    metadata.insert(
        "published_claims".to_string(),
        json!(result.published_claims.clone()),
    );
    metadata
}

fn alignment_allowed(result: &ComposedAgentRunResult) -> bool {
    result
        .alignment_verdict
        .get("allowed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && !result.published_claims.is_empty()
}

fn contribution_summary(result: &ComposedAgentRunResult) -> String {
    let claims = result
        .published_claims
        .iter()
        .map(|claim| claim.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
    if !claims.is_empty() {
        return truncate(&claims.join("; "), 240);
    }
    truncate(&synthesis_text(result), 240)
}

fn synthesis_text(result: &ComposedAgentRunResult) -> String {
    result
        .invocation_receipts
        .last()
        .and_then(|receipt| receipt.payload.get("text").and_then(Value::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "Theorem agent produced an alignment-gated response.".to_string())
}

fn runtime_error_verdict(error: &ComposedAgentRuntimeError) -> Value {
    json!({
        "allowed": false,
        "reason": "composed_agent_runtime_error",
        "detail": error.to_string()
    })
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn nonempty_or(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn default_heartbeat_ttl_seconds() -> u64 {
    DEFAULT_HEARTBEAT_TTL_SECONDS
}

fn default_mention_limit() -> usize {
    DEFAULT_MENTION_LIMIT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composed_agent::THEOREM_AGENT_HEADS_ENV;
    use crate::{
        list_presence, read_mentions_for_actor, read_records_for_room, write_message,
        WriteMessageInput,
    };
    use rustyred_thg_core::InMemoryGraphStore;
    use std::sync::Mutex;
    use theorem_harness_core::{
        FakeHeadInvoker, HeadInvocationError, HeadInvocationReceipt, HeadInvocationRequest,
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Test double whose every head invocation fails like an upstream provider
    /// outage. Used to prove the room runner surfaces a `ProviderError` as a
    /// blocked turn rather than panicking (T2 acceptance).
    struct ProviderFailingInvoker;

    impl HeadInvoker for ProviderFailingInvoker {
        fn invoke(
            &self,
            _request: HeadInvocationRequest,
        ) -> Result<HeadInvocationReceipt, HeadInvocationError> {
            Err(HeadInvocationError::ProviderError {
                head_id: "anthropic".to_string(),
                provider: "anthropic".to_string(),
                status: 503,
                detail: "simulated upstream provider failure".to_string(),
            })
        }
    }

    #[test]
    fn runner_heartbeats_consumes_room_mention_and_posts_contribution() {
        let _env = ScopedEnv::new([
            (THEOREM_AGENT_HEADS_ENV, "mistral"),
            ("MISTRAL_API_KEY", "mistral-test-secret"),
            ("MISTRAL_MODEL", "mistral-small-latest"),
        ]);
        let mut store = InMemoryGraphStore::new();
        write_room_mention(
            &mut store,
            "tenant-a",
            "room:a",
            "codex",
            "please check @theorem",
        );

        let cycle = run_agent_room_cycle(
            &mut store,
            AgentRoomRunnerConfig::theorem_default("tenant-a", "room:a"),
            &FakeHeadInvoker::default(),
        )
        .unwrap();

        assert_eq!(cycle.presence.actor_id, "theorem");
        assert_eq!(cycle.turns.len(), 1);
        assert_eq!(
            cycle.turns[0].status,
            AgentRoomRunnerTurnStatus::Contributed
        );
        assert!(cycle.turns[0].contribution.is_some());

        let presence = list_presence(&store, "tenant-a").unwrap();
        assert!(presence.iter().any(|record| record.actor_id == "theorem"));
        let records =
            read_records_for_room(&store, "tenant-a", "room:a", &["event".to_string()], 20)
                .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].actor_id, "theorem");
        assert_eq!(
            cycle.turns[0].run.as_ref().unwrap().consensus_head_set,
            vec!["mistral"]
        );
        let mentions =
            read_mentions_for_actor(&mut store, "tenant-a", "theorem", false, 20).unwrap();
        assert!(mentions.is_empty());
    }

    #[test]
    fn runner_wakes_configured_qwen_room_participant() {
        let _env = ScopedEnv::new([
            (THEOREM_AGENT_HEADS_ENV, "qwen"),
            ("QWEN_API_KEY", "qwen-test-secret"),
            ("QWEN_MODEL", "qwen3.7-max"),
        ]);
        let mut store = InMemoryGraphStore::new();
        write_room_mention(
            &mut store,
            "tenant-a",
            "room:a",
            "claude",
            "qwen should see @theorem",
        );

        let cycle = run_agent_room_cycle(
            &mut store,
            AgentRoomRunnerConfig::theorem_default("tenant-a", "room:a"),
            &FakeHeadInvoker::default(),
        )
        .unwrap();

        assert_eq!(cycle.turns.len(), 1);
        assert_eq!(
            cycle.turns[0].status,
            AgentRoomRunnerTurnStatus::Contributed
        );
        let run = cycle.turns[0].run.as_ref().expect("turn has run result");
        assert_eq!(run.consensus_head_set, vec!["qwen"]);
        assert_eq!(run.invocation_receipts.len(), 1);
        assert_eq!(run.invocation_receipts[0].head_id, "qwen");
        assert_eq!(
            run.alignment_verdict
                .get("single_head_mode")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn runner_does_not_consume_other_tenant_or_room_mentions() {
        let _env = ScopedEnv::new([
            (THEOREM_AGENT_HEADS_ENV, "mistral"),
            ("MISTRAL_API_KEY", "mistral-test-secret"),
            ("MISTRAL_MODEL", "mistral-small-latest"),
        ]);
        let mut store = InMemoryGraphStore::new();
        write_room_mention(&mut store, "tenant-a", "room:a", "codex", "a asks @theorem");
        write_room_mention(&mut store, "tenant-a", "room:b", "codex", "b asks @theorem");
        write_room_mention(
            &mut store,
            "tenant-b",
            "room:a",
            "codex",
            "tenant b asks @theorem",
        );

        run_agent_room_cycle(
            &mut store,
            AgentRoomRunnerConfig::theorem_default("tenant-a", "room:a"),
            &FakeHeadInvoker::default(),
        )
        .unwrap();

        let tenant_a_left =
            read_mentions_for_actor(&mut store, "tenant-a", "theorem", false, 20).unwrap();
        assert_eq!(tenant_a_left.len(), 1);
        assert_eq!(tenant_a_left[0].room_id, "room:b");
        let tenant_b_left =
            read_mentions_for_actor(&mut store, "tenant-b", "theorem", false, 20).unwrap();
        assert_eq!(tenant_b_left.len(), 1);
        let tenant_b_records =
            read_records_for_room(&store, "tenant-b", "room:a", &["event".to_string()], 20)
                .unwrap();
        assert!(tenant_b_records.is_empty());
    }

    #[test]
    fn runner_surfaces_provider_failure_as_blocked_turn_without_panic() {
        let _env = ScopedEnv::new([
            (THEOREM_AGENT_HEADS_ENV, "mistral"),
            ("MISTRAL_API_KEY", "mistral-test-secret"),
            ("MISTRAL_MODEL", "mistral-small-latest"),
        ]);
        // T2 acceptance: a provider outage must surface as a blocked turn
        // carrying the ProviderError, never panic the room runner. This swaps
        // ONLY the invoker versus the contributing FakeHeadInvoker case above,
        // so the provider failure is the sole cause of the Blocked outcome.
        let mut store = InMemoryGraphStore::new();
        write_room_mention(
            &mut store,
            "tenant-a",
            "room:a",
            "codex",
            "please check @theorem",
        );

        let cycle = run_agent_room_cycle(
            &mut store,
            AgentRoomRunnerConfig::theorem_default("tenant-a", "room:a"),
            &ProviderFailingInvoker,
        )
        .expect("runner returns Ok even when the provider fails");

        // Presence still posted; the turn is blocked, not contributed.
        assert_eq!(cycle.presence.actor_id, "theorem");
        assert_eq!(cycle.turns.len(), 1);
        assert_eq!(cycle.turns[0].status, AgentRoomRunnerTurnStatus::Blocked);
        assert!(cycle.turns[0].contribution.is_none());
        assert!(cycle.turns[0].run.is_none());

        // The provider failure is captured in the turn reflection verdict.
        let reflection = cycle.turns[0]
            .reflection
            .as_ref()
            .expect("blocked turn writes a reflection");
        let verdict = reflection
            .metadata
            .get("blocked_verdict")
            .expect("reflection carries the blocked verdict");
        assert_eq!(
            verdict.get("reason").and_then(Value::as_str),
            Some("composed_agent_runtime_error")
        );
        let detail = verdict
            .get("detail")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(
            detail.contains("provider anthropic") && detail.contains("status 503"),
            "verdict detail must surface the provider failure, got: {detail}"
        );

        // No room contribution event is written on a provider failure, but the
        // mention is still drained so the runner does not spin on it.
        let records =
            read_records_for_room(&store, "tenant-a", "room:a", &["event".to_string()], 20)
                .unwrap();
        assert!(records.is_empty());
        let mentions =
            read_mentions_for_actor(&mut store, "tenant-a", "theorem", false, 20).unwrap();
        assert!(mentions.is_empty());
    }

    fn write_room_mention(
        store: &mut InMemoryGraphStore,
        tenant: &str,
        room_id: &str,
        actor_id: &str,
        message: &str,
    ) {
        write_message(
            store,
            WriteMessageInput {
                tenant_slug: tenant.to_string(),
                room_id: room_id.to_string(),
                actor_id: actor_id.to_string(),
                message_id: String::new(),
                urgency: "info".to_string(),
                delivery: "passive".to_string(),
                message: message.to_string(),
                mentions: vec!["theorem".to_string()],
                metadata: Map::new(),
                created_at: String::new(),
            },
        )
        .unwrap();
    }

    struct ScopedEnv {
        saved: Vec<(String, Option<String>)>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl ScopedEnv {
        fn new<const N: usize>(pairs: [(&'static str, &'static str); N]) -> Self {
            let guard = ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut names = vec![
                THEOREM_AGENT_HEADS_ENV.to_string(),
                "AI21_API_KEY".to_string(),
                "ANTHROPIC_API_KEY".to_string(),
                "CLAUDE_API_KEY".to_string(),
                "DASHSCOPE_API_KEY".to_string(),
                "DEEPSEEK_API_KEY".to_string(),
                "GEMMA_API_KEY".to_string(),
                "MINIMAX_API_KEY".to_string(),
                "MISTRAL_API_KEY".to_string(),
                "MISTRAL_MODEL".to_string(),
                "OPENAI_API_KEY".to_string(),
                "QWEN_API_KEY".to_string(),
                "QWEN_MODEL".to_string(),
                "ZHIPU_API_KEY".to_string(),
            ];
            for (name, _) in pairs {
                names.push(name.to_string());
            }
            names.sort();
            names.dedup();
            let saved = names
                .into_iter()
                .map(|name| {
                    let value = std::env::var(&name).ok();
                    std::env::remove_var(&name);
                    (name, value)
                })
                .collect::<Vec<_>>();
            for (name, value) in pairs {
                std::env::set_var(name, value);
            }
            Self {
                saved,
                _guard: guard,
            }
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            for (name, value) in self.saved.drain(..) {
                if let Some(value) = value {
                    std::env::set_var(name, value);
                } else {
                    std::env::remove_var(name);
                }
            }
        }
    }
}
