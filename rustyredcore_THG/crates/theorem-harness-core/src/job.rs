//! Job: the dispatch-board unit.
//!
//! A Job is no longer a guarded lifecycle machine. Dispatch v2 treats it as a
//! durable thread: a spec to launch, set-once start metadata, optional archival
//! metadata, and append-only receipts from any actor. Infrastructure owns only
//! the "started once" launch invariant; agents coordinate the rest through the
//! harness room and receipts.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use crate::types::now_string;

/// Canonical lane identifier for the Claude Code CLI (`which claude`).
pub const LANE_CLAUDE: &str = "claude";
/// Canonical lane identifier for the Codex CLI (`which codex`).
pub const LANE_CODEX: &str = "codex";

/// Queue priority. It is a hint only, but it still sorts P0 before P1 before P2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    P0,
    P1,
    #[default]
    P2,
}

/// Which head a job prefers. The value is a launch hint, not an ownership rule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetHead {
    #[serde(rename = "claude", alias = "ClaudeCode")]
    Claude,
    #[serde(rename = "codex", alias = "Codex")]
    Codex,
    #[default]
    #[serde(rename = "either", alias = "Either")]
    Either,
}

impl TargetHead {
    /// True when at least one installed lane can run this job.
    pub fn matches_lanes(&self, lanes: &[String]) -> bool {
        self.preferred_lane(lanes).is_some()
    }

    /// Pick the lane a receiver should spawn. `Either` preserves the old local
    /// preference: Claude when present, then Codex.
    pub fn preferred_lane(&self, lanes: &[String]) -> Option<&'static str> {
        let has = |lane: &str| lanes.iter().any(|candidate| candidate == lane);
        match self {
            TargetHead::Claude if has(LANE_CLAUDE) => Some(LANE_CLAUDE),
            TargetHead::Codex if has(LANE_CODEX) => Some(LANE_CODEX),
            TargetHead::Either if has(LANE_CLAUDE) => Some(LANE_CLAUDE),
            TargetHead::Either if has(LANE_CODEX) => Some(LANE_CODEX),
            _ => None,
        }
    }
}

/// The flat input accepted by `job_submit`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSubmission {
    pub title: String,
    #[serde(default)]
    pub spec_ref: Option<String>,
    #[serde(default)]
    pub spec_inline: Option<String>,
    pub repo: String,
    #[serde(default)]
    pub priority: Option<Priority>,
    #[serde(default)]
    pub target_head: Option<TargetHead>,
    #[serde(default)]
    pub not_before: Option<String>,
    /// TickTick task id this job was captured from (Agent Queue capture path).
    /// Carried so the loop can relay milestones back to the originating task
    /// without inferring it. See `docs/plans/local-loop/`.
    #[serde(default)]
    pub source_task_id: Option<String>,
    /// TickTick project (list) id the source task lived in.
    #[serde(default)]
    pub source_project_id: Option<String>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

impl JobSubmission {
    /// `spec_ref` when present, else a content-addressed inline spec identity.
    pub fn spec_identity(&self) -> Option<String> {
        self.spec_ref
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .or_else(|| {
                self.spec_inline
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| inline_spec_identity(value))
            })
    }
}

/// One receipt appended to the job thread.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobReceipt {
    pub actor: String,
    pub at: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
}

impl JobReceipt {
    pub fn new(actor: impl Into<String>, text: impl Into<String>, refs: Vec<String>) -> Self {
        Self {
            actor: actor.into(),
            at: now_string(),
            text: text.into(),
            refs,
        }
    }
}

/// A dispatch job: one spec/thread, not a guarded state machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    /// `"job-" + ulid`.
    pub job_id: String,
    pub title: String,
    /// Repo path (`docs/plans/x/HANDOFF.md`) or harness doc_id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_ref: Option<String>,
    /// Inline spec text when the caller does not want a repo path/doc id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_inline: Option<String>,
    /// `"Travis-Gilbert/theorem"` etc.
    pub repo: String,
    pub priority: Priority,
    pub target_head: TargetHead,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_before: Option<String>,
    /// TickTick task id this job was captured from, when it came in through the
    /// Agent Queue capture path. The loop resolves the task to relay back to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_task_id: Option<String>,
    /// TickTick project (list) id the source task lived in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_project_id: Option<String>,
    /// actor_id of the submitter.
    pub submitted_by: String,
    pub submitted_at: String,
    /// Set once by the receiver that starts the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// run/session id once launched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_ref: Option<String>,
    /// Set when someone archives the thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_reason: Option<String>,
    /// Defaults to hash(spec_identity + title).
    pub idempotency_key: String,
    #[serde(default)]
    pub receipts: Vec<JobReceipt>,
}

impl Job {
    /// Build a pending job from a submission.
    pub fn from_submission(
        submission: JobSubmission,
        submitted_by: impl Into<String>,
    ) -> Result<Self, String> {
        let spec_identity = submission
            .spec_identity()
            .ok_or_else(|| "job_submit requires spec_ref or spec_inline".to_string())?;
        let job_id = new_job_id();
        let idempotency_key = submission
            .idempotency_key
            .clone()
            .filter(|key| !key.trim().is_empty())
            .unwrap_or_else(|| idempotency_key_for(&spec_identity, &submission.title));
        Ok(Self {
            job_id,
            title: submission.title,
            spec_ref: submission.spec_ref.filter(|value| !value.trim().is_empty()),
            spec_inline: submission
                .spec_inline
                .filter(|value| !value.trim().is_empty()),
            repo: submission.repo,
            priority: submission.priority.unwrap_or_default(),
            target_head: submission.target_head.unwrap_or_default(),
            not_before: submission
                .not_before
                .filter(|value| !value.trim().is_empty()),
            source_task_id: submission
                .source_task_id
                .filter(|value| !value.trim().is_empty()),
            source_project_id: submission
                .source_project_id
                .filter(|value| !value.trim().is_empty()),
            submitted_by: submitted_by.into(),
            submitted_at: now_string(),
            started_at: None,
            session_ref: None,
            archived_at: None,
            archived_reason: None,
            idempotency_key,
            receipts: Vec::new(),
        })
    }

    /// Derived board state. This is intentionally a string, not a lifecycle enum.
    pub fn derived_state(&self) -> &'static str {
        if self.archived_at.is_some() {
            "archived"
        } else if self.started_at.is_some() {
            "started"
        } else {
            "pending"
        }
    }

    pub fn is_pending(&self) -> bool {
        self.started_at.is_none() && self.archived_at.is_none()
    }

    pub fn spec_text_or_ref(&self) -> Option<&str> {
        self.spec_inline
            .as_deref()
            .or_else(|| self.spec_ref.as_deref())
    }
}

/// Mint a new job id: `"job-" + ulid`.
pub fn new_job_id() -> String {
    format!("job-{}", Ulid::new())
}

/// Deterministic idempotency key: `sha256(spec_identity \x1f title)`.
pub fn idempotency_key_for(spec_identity: &str, title: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(spec_identity.as_bytes());
    hasher.update([0x1f]);
    hasher.update(title.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn inline_spec_identity(spec_inline: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(spec_inline.as_bytes());
    format!("inline:sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn submission() -> JobSubmission {
        JobSubmission {
            title: "Desktop app, Dia rebuild".to_string(),
            spec_ref: Some("docs/plans/theorem-desktop/HANDOFF.md".to_string()),
            spec_inline: None,
            repo: "Travis-Gilbert/theorem".to_string(),
            priority: None,
            target_head: None,
            not_before: None,
            source_task_id: None,
            source_project_id: None,
            idempotency_key: None,
        }
    }

    #[test]
    fn from_submission_applies_defaults() {
        let job = Job::from_submission(submission(), "claude.ai").unwrap();
        assert!(job.job_id.starts_with("job-"));
        assert_eq!(job.derived_state(), "pending");
        assert_eq!(job.priority, Priority::P2);
        assert_eq!(job.target_head, TargetHead::Either);
        assert_eq!(
            job.idempotency_key,
            idempotency_key_for(job.spec_ref.as_deref().unwrap(), &job.title)
        );
        assert_eq!(job.submitted_by, "claude.ai");
    }

    #[test]
    fn spec_ref_or_spec_inline_is_required() {
        let mut submission = submission();
        submission.spec_ref = None;
        let error = Job::from_submission(submission, "x").unwrap_err();
        assert!(error.contains("spec_ref or spec_inline"));
    }

    #[test]
    fn from_submission_carries_source_correspondence() {
        let mut submission = submission();
        submission.source_task_id = Some("tt-task-123".to_string());
        submission.source_project_id = Some("tt-list-agent-queue".to_string());
        let job = Job::from_submission(submission, "theorem-agentd").unwrap();
        assert_eq!(job.source_task_id.as_deref(), Some("tt-task-123"));
        assert_eq!(
            job.source_project_id.as_deref(),
            Some("tt-list-agent-queue")
        );
        // Blank correspondence ids are treated as absent, like other hints.
        let mut blank = submission_with_source("", "");
        blank.title = "blank".to_string();
        let job = Job::from_submission(blank, "theorem-agentd").unwrap();
        assert!(job.source_task_id.is_none());
        assert!(job.source_project_id.is_none());
    }

    fn submission_with_source(task_id: &str, project_id: &str) -> JobSubmission {
        let mut s = submission();
        s.source_task_id = Some(task_id.to_string());
        s.source_project_id = Some(project_id.to_string());
        s
    }

    #[test]
    fn inline_spec_identity_is_content_addressed() {
        let mut submission = submission();
        submission.spec_ref = None;
        submission.spec_inline = Some("Build the thing.".to_string());
        let job = Job::from_submission(submission, "codex").unwrap();
        assert!(job.idempotency_key.starts_with("sha256:"));
        assert_eq!(job.spec_inline.as_deref(), Some("Build the thing."));
    }

    #[test]
    fn priority_orders_p0_first() {
        let mut order = vec![Priority::P2, Priority::P0, Priority::P1];
        order.sort();
        assert_eq!(order, vec![Priority::P0, Priority::P1, Priority::P2]);
    }

    #[test]
    fn target_head_lane_matching() {
        let claude = vec![LANE_CLAUDE.to_string()];
        let codex = vec![LANE_CODEX.to_string()];
        let both = vec![LANE_CLAUDE.to_string(), LANE_CODEX.to_string()];

        assert!(TargetHead::Claude.matches_lanes(&claude));
        assert!(!TargetHead::Claude.matches_lanes(&codex));
        assert!(TargetHead::Codex.matches_lanes(&codex));
        assert!(!TargetHead::Codex.matches_lanes(&claude));
        assert!(TargetHead::Either.matches_lanes(&claude));
        assert!(TargetHead::Either.matches_lanes(&codex));
        assert!(!TargetHead::Either.matches_lanes(&[]));

        assert_eq!(TargetHead::Either.preferred_lane(&both), Some(LANE_CLAUDE));
        assert_eq!(TargetHead::Either.preferred_lane(&codex), Some(LANE_CODEX));
        assert_eq!(TargetHead::Codex.preferred_lane(&claude), None);
    }
}
