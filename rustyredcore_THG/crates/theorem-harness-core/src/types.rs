use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub type Payload = Map<String, Value>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentStepState {
    pub step_id: String,
    pub run_id: String,
    pub kind: String,
    #[serde(default)]
    pub payload: Payload,
    #[serde(default = "now_string")]
    pub created_at: String,
}

impl AgentStepState {
    pub fn new(run_id: impl Into<String>, kind: impl Into<String>, payload: Payload) -> Self {
        Self {
            step_id: prefixed_id("step"),
            run_id: run_id.into(),
            kind: kind.into(),
            payload,
            created_at: now_string(),
        }
    }

    pub fn with_step_id(mut self, step_id: impl Into<String>) -> Self {
        self.step_id = step_id.into();
        self
    }

    pub fn at(mut self, created_at: impl Into<String>) -> Self {
        self.created_at = created_at.into();
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentRunState {
    pub run_id: String,
    pub task: String,
    pub actor: String,
    #[serde(default)]
    pub scope: Payload,
    #[serde(default = "running_status")]
    pub status: String,
    #[serde(default)]
    pub steps: Vec<AgentStepState>,
    #[serde(default)]
    pub search_runs: Vec<Payload>,
    #[serde(default)]
    pub artifacts: Vec<Payload>,
    #[serde(default)]
    pub memory_patches: Vec<Payload>,
    #[serde(default)]
    pub validations: Vec<Payload>,
    #[serde(default = "now_string")]
    pub created_at: String,
    #[serde(default = "now_string")]
    pub updated_at: String,
}

impl AgentRunState {
    pub fn new(task: impl Into<String>, actor: impl Into<String>, scope: Payload) -> Self {
        Self {
            run_id: prefixed_id("run"),
            task: task.into(),
            actor: actor.into(),
            scope,
            status: running_status(),
            steps: Vec::new(),
            search_runs: Vec::new(),
            artifacts: Vec::new(),
            memory_patches: Vec::new(),
            validations: Vec::new(),
            created_at: now_string(),
            updated_at: now_string(),
        }
    }

    pub fn with_step(mut self, step: AgentStepState) -> Self {
        self.steps.push(step);
        self.updated_at = now_string();
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunState {
    pub run_id: String,
    pub task: String,
    pub actor: String,
    #[serde(default)]
    pub scope: Payload,
    #[serde(default = "created_status")]
    pub status: String,
    #[serde(default)]
    pub task_signature: String,
    #[serde(default)]
    pub host: Payload,
    #[serde(default)]
    pub profile: Option<Payload>,
    #[serde(default)]
    pub toolkit: Option<Payload>,
    #[serde(default)]
    pub context: Option<Payload>,
    #[serde(default)]
    pub cache_events: Vec<Payload>,
    #[serde(default)]
    pub validators: Vec<Payload>,
    #[serde(default)]
    pub outcome: Option<Payload>,
    #[serde(default)]
    pub learning_patches: Vec<Payload>,
    #[serde(default)]
    pub federation_signals: Vec<Payload>,
    #[serde(default)]
    pub last_event_seq: u64,
    #[serde(default = "now_string")]
    pub created_at: String,
    #[serde(default = "now_string")]
    pub updated_at: String,
    #[serde(default)]
    pub workstream_id: String,
    #[serde(default)]
    pub agent_host: String,
    #[serde(default)]
    pub agent_model: String,
}

impl RunState {
    pub fn new(task: impl Into<String>, actor: impl Into<String>, scope: Payload) -> Self {
        Self {
            run_id: prefixed_id("harnessrun"),
            task: task.into(),
            actor: actor.into(),
            scope,
            status: created_status(),
            task_signature: String::new(),
            host: Payload::new(),
            profile: None,
            toolkit: None,
            context: None,
            cache_events: Vec::new(),
            validators: Vec::new(),
            outcome: None,
            learning_patches: Vec::new(),
            federation_signals: Vec::new(),
            last_event_seq: 0,
            created_at: now_string(),
            updated_at: now_string(),
            workstream_id: String::new(),
            agent_host: String::new(),
            agent_model: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventState {
    #[serde(default = "event_id")]
    pub event_id: String,
    pub run_id: String,
    pub seq: u64,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub payload: Payload,
    #[serde(default)]
    pub state_hash_before: String,
    #[serde(default)]
    pub state_hash_after: String,
    #[serde(default = "now_string")]
    pub created_at: String,
    #[serde(default)]
    pub workstream_id: String,
    #[serde(default)]
    pub agent_host: String,
    #[serde(default)]
    pub agent_model: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub commit_sha: String,
    #[serde(default = "private_scope")]
    pub privacy_scope: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransitionInput {
    #[serde(default)]
    pub run_id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub payload: Payload,
    #[serde(default)]
    pub actor: String,
    #[serde(default)]
    pub idempotency_key: String,
    #[serde(default = "now_string")]
    pub created_at: String,
}

impl TransitionInput {
    pub fn new(event_type: impl Into<String>, payload: Payload) -> Self {
        Self {
            run_id: String::new(),
            event_type: event_type.into(),
            payload,
            actor: String::new(),
            idempotency_key: String::new(),
            created_at: now_string(),
        }
    }

    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = run_id.into();
        self
    }

    pub fn at(mut self, created_at: impl Into<String>) -> Self {
        self.created_at = created_at.into();
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuardViolation {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub required_state: String,
    #[serde(default)]
    pub received_state: String,
    #[serde(default)]
    pub missing_fields: Vec<String>,
    #[serde(default)]
    pub details: Payload,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransitionResult {
    pub run: RunState,
    pub event: EventState,
    #[serde(default)]
    pub effects: Vec<Value>,
    pub state_hash_before: String,
    pub state_hash_after: String,
}

pub fn prefixed_id(prefix: &str) -> String {
    format!("{prefix}:{}", Uuid::new_v4().simple())
}

fn created_status() -> String {
    "created".to_string()
}

fn running_status() -> String {
    "running".to_string()
}

fn event_id() -> String {
    prefixed_id("event")
}

fn private_scope() -> String {
    "private".to_string()
}

pub fn now_string() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => format!("{}.{:09}Z", duration.as_secs(), duration.subsec_nanos()),
        Err(_) => "0.000000000Z".to_string(),
    }
}
