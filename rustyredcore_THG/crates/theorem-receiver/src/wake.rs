//! Wake planning for coordination-room messages.
//!
//! This module turns `delivery = "wake"` room messages into local command plans
//! and, when explicitly asked, launches those plans through the local receiver.
//! It deliberately stays out of queue semantics: no queue claim, no global
//! ownership, and no hidden scheduler.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use theorem_harness_core::{LANE_CLAUDE, LANE_CODEX};

use crate::config::ReceiverConfig;
use crate::spawn::{build_spawn_plan, command_from_plan, SpawnPlan};
use crate::ReceiverResult;

// Wake planning only needs the recent window. Kept small also because the
// harness read_messages_for_room currently emits unescaped control chars/quotes
// in some older message bodies, which breaks strict JSON parse over large
// windows (harness serialization bug; the client sanitizes control chars but
// cannot recover unescaped quotes).
pub const DEFAULT_WAKE_MESSAGE_LIMIT: usize = 8;
pub const DEFAULT_WAKE_MAX_PLANS: usize = 5;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct WakeMessage {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub room_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub urgency: String,
    #[serde(default)]
    pub delivery: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub mentions: Vec<String>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    #[serde(default)]
    pub created_at: String,
}

impl WakeMessage {
    pub fn author(&self) -> &str {
        if self.author.trim().is_empty() {
            self.actor_id.trim()
        } else {
            self.author.trim()
        }
    }

    pub fn is_wake(&self) -> bool {
        self.delivery.trim().eq_ignore_ascii_case("wake")
    }

    pub fn targets_actor(&self, actor: &str) -> bool {
        let actor = actor.trim();
        self.mentions.is_empty() || self.mentions.iter().any(|mention| mention.trim() == actor)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WakeLedger {
    planned_message_ids: BTreeSet<String>,
}

impl WakeLedger {
    pub fn load_or_default(path: impl AsRef<Path>) -> ReceiverResult<Self> {
        let path = path.as_ref();
        match fs::read_to_string(path) {
            Ok(raw) => Ok(serde_json::from_str(&raw)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save(&self, path: impl AsRef<Path>) -> ReceiverResult<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn contains(&self, message_id: &str) -> bool {
        self.planned_message_ids.contains(message_id.trim())
    }

    pub fn mark_planned(&mut self, message_id: impl Into<String>) -> bool {
        let message_id = message_id.into();
        if message_id.trim().is_empty() {
            return false;
        }
        self.planned_message_ids.insert(message_id)
    }

    pub fn planned_message_ids(&self) -> impl Iterator<Item = &str> {
        self.planned_message_ids.iter().map(String::as_str)
    }

    fn unmark_planned(&mut self, message_id: &str) -> bool {
        self.planned_message_ids.remove(message_id.trim())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WakeCommandPlan {
    pub actor: String,
    pub lane: String,
    pub message_id: String,
    pub room_id: String,
    pub prompt: String,
    pub command: SpawnPlan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WakeSkipped {
    pub message_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WakeDryRunReport {
    pub plans: Vec<WakeCommandPlan>,
    pub skipped: Vec<WakeSkipped>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WakeSpawnOutcome {
    pub pid: u32,
    pub immediate_status: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WakeSpawned {
    pub actor: String,
    pub lane: String,
    pub message_id: String,
    pub room_id: String,
    pub pid: u32,
    pub immediate_status: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WakeLaunchFailed {
    pub actor: String,
    pub lane: String,
    pub message_id: String,
    pub room_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WakeRunReport {
    pub plans: Vec<WakeCommandPlan>,
    pub spawned: Vec<WakeSpawned>,
    pub failed: Vec<WakeLaunchFailed>,
    pub skipped: Vec<WakeSkipped>,
}

pub fn build_wake_dry_run_report(
    config: &ReceiverConfig,
    actor: &str,
    messages: &[WakeMessage],
    ledger: &WakeLedger,
    max_plans: usize,
) -> WakeDryRunReport {
    let mut report = WakeDryRunReport::default();
    let lane = lane_for_actor(actor);
    let mut planned_ids: BTreeSet<String> =
        ledger.planned_message_ids().map(str::to_string).collect();

    for message in messages {
        if !message.is_wake() {
            continue;
        }
        if !message.targets_actor(actor) {
            continue;
        }
        if message.author() == actor.trim() {
            report.skipped.push(skip(message, "self_authored"));
            continue;
        }
        if message.message_id.trim().is_empty() {
            report.skipped.push(skip(message, "missing_message_id"));
            continue;
        }
        if planned_ids.contains(message.message_id.trim()) {
            report.skipped.push(skip(message, "already_planned"));
            continue;
        }
        if report.plans.len() >= max_plans {
            report.skipped.push(skip(message, "storm_control"));
            continue;
        }

        let Some(lane) = lane else {
            report.skipped.push(skip(message, "unknown_actor_lane"));
            continue;
        };
        let Some(worktree) = worktree_for_message(config, message) else {
            report.skipped.push(skip(message, "no_mapped_worktree"));
            continue;
        };
        let prompt = build_wake_prompt(actor, message);
        let Some(command) = build_spawn_plan(lane, &prompt, worktree) else {
            report.skipped.push(skip(message, "unregistered_lane"));
            continue;
        };

        if !message.message_id.trim().is_empty() {
            planned_ids.insert(message.message_id.clone());
        }
        report.plans.push(WakeCommandPlan {
            actor: actor.trim().to_string(),
            lane: lane.to_string(),
            message_id: message.message_id.clone(),
            room_id: message.room_id.clone(),
            prompt,
            command,
        });
    }

    report
}

pub fn run_wake_report_with_spawner<P, S>(
    config: &ReceiverConfig,
    actor: &str,
    messages: &[WakeMessage],
    ledger: &mut WakeLedger,
    max_plans: usize,
    mut persist_ledger: P,
    mut spawn: S,
) -> WakeRunReport
where
    P: FnMut(&WakeLedger) -> Result<(), String>,
    S: FnMut(&SpawnPlan) -> Result<WakeSpawnOutcome, String>,
{
    let dry_run = build_wake_dry_run_report(config, actor, messages, ledger, max_plans);
    let mut report = WakeRunReport {
        plans: dry_run.plans,
        skipped: dry_run.skipped,
        ..WakeRunReport::default()
    };

    for plan in &report.plans {
        if !ledger.mark_planned(plan.message_id.clone()) {
            report.skipped.push(WakeSkipped {
                message_id: plan.message_id.clone(),
                reason: "already_planned".to_string(),
            });
            continue;
        }

        if let Err(error) = persist_ledger(ledger) {
            ledger.unmark_planned(&plan.message_id);
            report
                .failed
                .push(wake_failed(plan, format!("ledger_persist_failed: {error}")));
            continue;
        }

        match spawn(&plan.command) {
            Ok(outcome) => report.spawned.push(WakeSpawned {
                actor: plan.actor.clone(),
                lane: plan.lane.clone(),
                message_id: plan.message_id.clone(),
                room_id: plan.room_id.clone(),
                pid: outcome.pid,
                immediate_status: outcome.immediate_status,
            }),
            Err(error) => report
                .failed
                .push(wake_failed(plan, format!("spawn_failed: {error}"))),
        }
    }

    report
}

pub fn spawn_wake_command(plan: &SpawnPlan) -> Result<WakeSpawnOutcome, String> {
    let mut command = command_from_plan(plan);
    command.stdin(Stdio::null());
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let pid = child.id();
    let immediate_status = match child.try_wait() {
        Ok(Some(status)) => Some(status.to_string()),
        Ok(None) => None,
        Err(error) => Some(format!("try_wait error: {error}")),
    };
    Ok(WakeSpawnOutcome {
        pid,
        immediate_status,
    })
}

pub fn build_wake_prompt(actor: &str, message: &WakeMessage) -> String {
    format!(
        "Wake event for actor `{actor}` in coordination room `{room_id}`.\n\
Reason: message `{message_id}` from `{author}` used delivery=wake.\n\
Urgency: {urgency}\n\
Created: {created_at}\n\n\
Message:\n{body}\n\n\
Instructions:\n\
- Read the room messages and your pending mentions before acting.\n\
- Treat this wake as a nudge, not an assignment or ownership claim.\n\
- Do not enforce lanes, claim global ownership, or wait for a peer handshake.\n\
- If no action is needed, report that and close your intent.",
        actor = actor.trim(),
        room_id = nonempty(&message.room_id, "room:ungrouped"),
        message_id = nonempty(&message.message_id, "unknown-message"),
        author = nonempty(message.author(), "unknown-author"),
        urgency = nonempty(&message.urgency, "info"),
        created_at = nonempty(&message.created_at, "unknown-time"),
        body = message.message.trim()
    )
}

pub fn wake_dry_run_report_json(report: &WakeDryRunReport) -> Value {
    json!({
        "plans": report.plans.iter().map(wake_plan_json).collect::<Vec<_>>(),
        "skipped": report.skipped.iter().map(wake_skipped_json).collect::<Vec<_>>()
    })
}

pub fn wake_run_report_json(report: &WakeRunReport) -> Value {
    json!({
        "plans": report.plans.iter().map(wake_plan_json).collect::<Vec<_>>(),
        "spawned": report.spawned.iter().map(|spawned| {
            json!({
                "actor": spawned.actor,
                "lane": spawned.lane,
                "message_id": spawned.message_id,
                "room_id": spawned.room_id,
                "pid": spawned.pid,
                "immediate_status": spawned.immediate_status,
            })
        }).collect::<Vec<_>>(),
        "failed": report.failed.iter().map(|failed| {
            json!({
                "actor": failed.actor,
                "lane": failed.lane,
                "message_id": failed.message_id,
                "room_id": failed.room_id,
                "reason": failed.reason,
            })
        }).collect::<Vec<_>>(),
        "skipped": report.skipped.iter().map(wake_skipped_json).collect::<Vec<_>>()
    })
}

fn wake_plan_json(plan: &WakeCommandPlan) -> Value {
    json!({
        "actor": plan.actor,
        "lane": plan.lane,
        "message_id": plan.message_id,
        "room_id": plan.room_id,
        "prompt": plan.prompt,
        "command": {
            "program": plan.command.program,
            "args": plan.command.args,
            "cwd": plan.command.cwd.display().to_string(),
            "strip_env": plan.command.strip_env
        }
    })
}

fn lane_for_actor(actor: &str) -> Option<&'static str> {
    match actor.trim() {
        "claude" | "claude-code" => Some(LANE_CLAUDE),
        "codex" => Some(LANE_CODEX),
        _ => None,
    }
}

fn worktree_for_message<'a>(config: &'a ReceiverConfig, message: &WakeMessage) -> Option<&'a Path> {
    for key in ["repo", "source_repo", "target_repo"] {
        if let Some(repo) = message.metadata.get(key).and_then(Value::as_str) {
            if let Some(worktree) = config.worktree_for(repo) {
                return Some(worktree);
            }
        }
    }
    if config.worktrees.len() == 1 {
        return config.worktrees.values().next().map(|path| path.as_path());
    }
    None
}

fn skip(message: &WakeMessage, reason: &str) -> WakeSkipped {
    WakeSkipped {
        message_id: message.message_id.clone(),
        reason: reason.to_string(),
    }
}

fn wake_failed(plan: &WakeCommandPlan, reason: String) -> WakeLaunchFailed {
    WakeLaunchFailed {
        actor: plan.actor.clone(),
        lane: plan.lane.clone(),
        message_id: plan.message_id.clone(),
        room_id: plan.room_id.clone(),
        reason,
    }
}

fn wake_skipped_json(skipped: &WakeSkipped) -> Value {
    json!({
        "message_id": skipped.message_id,
        "reason": skipped.reason
    })
}

fn nonempty<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    let value = value.trim();
    if value.is_empty() {
        fallback
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn config() -> ReceiverConfig {
        ReceiverConfig::from_toml(
            r#"
harness_url = "https://example/mcp"

[worktrees]
"Travis-Gilbert/theorem" = "/repos/theorem"
"Travis-Gilbert/other" = "/repos/other"
"#,
        )
        .unwrap()
    }

    fn wake_message(id: &str) -> WakeMessage {
        let mut metadata = Map::new();
        metadata.insert("repo".to_string(), json!("Travis-Gilbert/theorem"));
        WakeMessage {
            tenant_slug: "default".to_string(),
            room_id: "repo:theorem:branch:main".to_string(),
            message_id: id.to_string(),
            actor_id: "travis".to_string(),
            urgency: "ask".to_string(),
            delivery: "wake".to_string(),
            message: "@codex please check the trigger".to_string(),
            mentions: vec!["codex".to_string()],
            metadata,
            created_at: "unix_ms:1".to_string(),
            ..WakeMessage::default()
        }
    }

    fn temp_ledger_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "theorem-receiver-{name}-{}-{nonce}.json",
            std::process::id()
        ))
    }

    #[test]
    fn wake_message_targets_actor_when_mentioned_or_broadcast() {
        let mut message = wake_message("msg-1");
        assert!(message.targets_actor("codex"));
        assert!(!message.targets_actor("claude-code"));
        message.mentions.clear();
        assert!(message.targets_actor("claude-code"));
    }

    #[test]
    fn dry_run_builds_codex_command_without_marking_the_ledger() {
        let ledger = WakeLedger::default();
        let report =
            build_wake_dry_run_report(&config(), "codex", &[wake_message("msg-1")], &ledger, 5);

        assert_eq!(report.skipped, Vec::<WakeSkipped>::new());
        assert_eq!(report.plans.len(), 1);
        let plan = &report.plans[0];
        assert_eq!(plan.lane, "codex");
        assert_eq!(plan.command.program, "codex");
        assert_eq!(plan.command.args[0], "exec");
        assert_eq!(plan.command.cwd, std::path::PathBuf::from("/repos/theorem"));
        assert!(plan.prompt.contains("not an assignment or ownership claim"));
        assert!(!ledger.contains("msg-1"));
    }

    #[test]
    fn ledger_prevents_duplicate_plans() {
        let mut ledger = WakeLedger::default();
        assert!(ledger.mark_planned("msg-1"));
        assert!(!ledger.mark_planned(""));

        let report =
            build_wake_dry_run_report(&config(), "codex", &[wake_message("msg-1")], &ledger, 5);
        assert!(report.plans.is_empty());
        assert_eq!(report.skipped[0].reason, "already_planned");
        assert_eq!(
            ledger.planned_message_ids().collect::<Vec<_>>(),
            vec!["msg-1"]
        );
    }

    #[test]
    fn ledger_round_trips_to_json_file() {
        let path = temp_ledger_path("ledger");
        let mut ledger = WakeLedger::default();
        assert!(ledger.mark_planned("msg-1"));
        ledger.save(&path).unwrap();

        let loaded = WakeLedger::load_or_default(&path).unwrap();
        assert!(loaded.contains("msg-1"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn wake_run_persists_before_spawning_and_reports_pid() {
        let messages = vec![wake_message("msg-1")];
        let mut ledger = WakeLedger::default();
        let mut persisted = Vec::new();
        let mut spawned = Vec::new();

        let report = run_wake_report_with_spawner(
            &config(),
            "codex",
            &messages,
            &mut ledger,
            5,
            |ledger| {
                persisted.push(
                    ledger
                        .planned_message_ids()
                        .map(str::to_string)
                        .collect::<Vec<_>>(),
                );
                Ok(())
            },
            |plan| {
                spawned.push(plan.program.clone());
                Ok(WakeSpawnOutcome {
                    pid: 42,
                    immediate_status: None,
                })
            },
        );

        assert!(ledger.contains("msg-1"));
        assert_eq!(persisted, vec![vec!["msg-1".to_string()]]);
        assert_eq!(spawned, vec!["codex".to_string()]);
        assert_eq!(report.spawned[0].pid, 42);
        assert!(report.failed.is_empty());
    }

    #[test]
    fn wake_run_does_not_spawn_when_ledger_persist_fails() {
        let messages = vec![wake_message("msg-1")];
        let mut ledger = WakeLedger::default();
        let mut spawn_count = 0;

        let report = run_wake_report_with_spawner(
            &config(),
            "codex",
            &messages,
            &mut ledger,
            5,
            |_| Err("disk full".to_string()),
            |_| {
                spawn_count += 1;
                Ok(WakeSpawnOutcome {
                    pid: 42,
                    immediate_status: None,
                })
            },
        );

        assert_eq!(spawn_count, 0);
        assert!(!ledger.contains("msg-1"));
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].reason, "ledger_persist_failed: disk full");
    }

    #[test]
    fn duplicate_message_ids_are_planned_once_per_batch() {
        let messages = vec![wake_message("msg-1"), wake_message("msg-1")];
        let report =
            build_wake_dry_run_report(&config(), "codex", &messages, &WakeLedger::default(), 5);

        assert_eq!(report.plans.len(), 1);
        assert_eq!(report.skipped[0].reason, "already_planned");
    }

    #[test]
    fn storm_control_caps_plans_per_pass() {
        let messages = vec![wake_message("msg-1"), wake_message("msg-2")];
        let report =
            build_wake_dry_run_report(&config(), "codex", &messages, &WakeLedger::default(), 1);

        assert_eq!(report.plans.len(), 1);
        assert_eq!(report.plans[0].message_id, "msg-1");
        assert_eq!(report.skipped[0].reason, "storm_control");
    }

    #[test]
    fn unknown_actor_is_visible_not_silent() {
        let mut message = wake_message("msg-1");
        message.mentions.clear();
        let report =
            build_wake_dry_run_report(&config(), "unknown", &[message], &WakeLedger::default(), 5);

        assert!(report.plans.is_empty());
        assert_eq!(report.skipped[0].reason, "unknown_actor_lane");
    }
}
