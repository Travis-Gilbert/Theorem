//! Job: the dispatch-queue unit.
//!
//! One job is one spec, one session, one run. The Job sits ABOVE runs in the
//! work hierarchy: do NOT merge it into [`crate::work_graph::TaskNode`], which is
//! the INTRA-run work graph. A Job is dispatched, becomes a run, and that run may
//! itself contain a TaskNode graph (multi-head-run-execution/HANDOFF.md).
//!
//! This module is pure domain logic with no GraphStore dependency. Persistence
//! and the six queue verbs (submit / status / cancel / promote / claim /
//! complete) live in `theorem-harness-runtime::job_queue`, keeping storage out
//! of the kernel the same way `event_log.rs` is kept out of the pure state
//! machine.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use crate::types::now_string;

/// Canonical lane identifier for the Claude Code CLI (`which claude`).
pub const LANE_CLAUDE: &str = "claude";
/// Canonical lane identifier for the Codex CLI (`which codex`).
pub const LANE_CODEX: &str = "codex";

/// What kind of work a job represents. Drives the receiver's intent framing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobKind {
    ImplementSpec,
    Feature,
    Edit,
    App,
    Investigation,
}

/// Queue priority. Declared highest-first so the derived `Ord` sorts P0 ahead of
/// P1 ahead of P2 under an ascending sort.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    P0,
    P1,
    /// Unspecified jobs join the back of the line; explicit urgency must be
    /// declared so a default-priority job never preempts a marked one.
    #[default]
    P2,
}

/// Which head a job targets. `Either` is claimable by whichever lane a receiver
/// has installed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetHead {
    ClaudeCode,
    Codex,
    #[default]
    Either,
}

impl TargetHead {
    /// True when at least one of the receiver's installed lanes can run this job.
    pub fn matches_lanes(&self, lanes: &[String]) -> bool {
        let has = |lane: &str| lanes.iter().any(|candidate| candidate == lane);
        match self {
            TargetHead::ClaudeCode => has(LANE_CLAUDE),
            TargetHead::Codex => has(LANE_CODEX),
            TargetHead::Either => has(LANE_CLAUDE) || has(LANE_CODEX),
        }
    }

    /// The lane a receiver should spawn for this job, given its installed lanes.
    /// `Either` prefers Claude when present, then falls back to Codex.
    pub fn preferred_lane(&self, lanes: &[String]) -> Option<&'static str> {
        let has = |lane: &str| lanes.iter().any(|candidate| candidate == lane);
        match self {
            TargetHead::ClaudeCode if has(LANE_CLAUDE) => Some(LANE_CLAUDE),
            TargetHead::Codex if has(LANE_CODEX) => Some(LANE_CODEX),
            TargetHead::Either if has(LANE_CLAUDE) => Some(LANE_CLAUDE),
            TargetHead::Either if has(LANE_CODEX) => Some(LANE_CODEX),
            _ => None,
        }
    }
}

/// The lifecycle status of a job. Transitions are validated by
/// [`JobStatus::can_transition`]; every applied transition appends a graph event
/// in the runtime so the lifecycle is replayable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobStatus {
    Queued,
    Claimed,
    Running,
    PrOpen,
    Verifying,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    /// Terminal states never transition further.
    pub fn is_terminal(&self) -> bool {
        matches!(self, JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled)
    }

    /// `job_cancel` accepts a Queued job, or a Claimed job that has not yet begun
    /// running (per the spec: "Queued (or Claimed-not-yet-running) to Cancelled").
    pub fn can_cancel(&self) -> bool {
        matches!(self, JobStatus::Queued | JobStatus::Claimed)
    }

    /// The legal forward transitions of the dispatch lifecycle.
    pub fn can_transition(from: JobStatus, to: JobStatus) -> bool {
        use JobStatus::*;
        matches!(
            (from, to),
            (Queued, Claimed)
                | (Queued, Cancelled)
                | (Claimed, Running)
                | (Claimed, Cancelled)
                | (Claimed, Failed)
                | (Running, PrOpen)
                | (Running, Verifying)
                | (Running, Done)
                | (Running, Failed)
                | (PrOpen, Verifying)
                | (PrOpen, Done)
                | (PrOpen, Failed)
                | (Verifying, Done)
                | (Verifying, Failed)
        )
    }
}

/// The flat input accepted by `job_submit`. Optional fields fall back to defaults
/// (`branch` -> `job/{job_id}`, `idempotency_key` -> hash(spec_ref + title),
/// `priority` -> P2, `target_head` -> Either).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSubmission {
    pub title: String,
    pub spec_ref: String,
    pub repo: String,
    pub kind: JobKind,
    #[serde(default)]
    pub priority: Option<Priority>,
    #[serde(default)]
    pub target_head: Option<TargetHead>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

/// A dispatch job: one spec, one session, one run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    /// `"job-" + ulid`.
    pub job_id: String,
    pub kind: JobKind,
    pub title: String,
    /// Repo path (`docs/plans/x/HANDOFF.md`) or harness doc_id.
    pub spec_ref: String,
    /// `"Travis-Gilbert/theorem"` etc.
    pub repo: String,
    /// Defaults to `job/{job_id}`.
    pub branch: Option<String>,
    pub priority: Priority,
    pub target_head: TargetHead,
    pub status: JobStatus,
    /// actor_id of the submitter.
    pub submitted_by: String,
    pub submitted_at: String,
    /// receiver id.
    pub claimed_by: Option<String>,
    pub claimed_at: Option<String>,
    pub closed_at: Option<String>,
    /// run_id once dispatched.
    pub session_ref: Option<String>,
    /// PR number or branch ref.
    pub pr_ref: Option<String>,
    /// Defaults to hash(spec_ref + title).
    pub idempotency_key: String,
    pub notes: Option<String>,
}

impl Job {
    /// Build a freshly-queued job from a submission.
    pub fn from_submission(submission: JobSubmission, submitted_by: impl Into<String>) -> Self {
        let job_id = new_job_id();
        let idempotency_key = submission
            .idempotency_key
            .clone()
            .filter(|key| !key.trim().is_empty())
            .unwrap_or_else(|| idempotency_key_for(&submission.spec_ref, &submission.title));
        let branch = submission
            .branch
            .clone()
            .filter(|branch| !branch.trim().is_empty())
            .unwrap_or_else(|| default_branch(&job_id));
        Self {
            job_id,
            kind: submission.kind,
            title: submission.title,
            spec_ref: submission.spec_ref,
            repo: submission.repo,
            branch: Some(branch),
            priority: submission.priority.unwrap_or_default(),
            target_head: submission.target_head.unwrap_or_default(),
            status: JobStatus::Queued,
            submitted_by: submitted_by.into(),
            submitted_at: now_string(),
            claimed_by: None,
            claimed_at: None,
            closed_at: None,
            session_ref: None,
            pr_ref: None,
            idempotency_key,
            notes: submission.notes,
        }
    }

    /// The branch this job's work lands on (`job/{job_id}` unless overridden).
    pub fn branch_ref(&self) -> String {
        self.branch
            .clone()
            .filter(|branch| !branch.trim().is_empty())
            .unwrap_or_else(|| default_branch(&self.job_id))
    }

    /// True when a receiver with these lanes and configured repos may claim this
    /// job: it must be Queued, in a repo the receiver maps, and target a lane the
    /// receiver has installed.
    pub fn claimable_by(&self, lanes: &[String], repos: &[String]) -> bool {
        self.status == JobStatus::Queued
            && repos.iter().any(|repo| repo == &self.repo)
            && self.target_head.matches_lanes(lanes)
    }
}

/// Mint a new job id: `"job-" + ulid`. ULID gives a Crockford base32, lexically
/// time-sortable id, a natural secondary key behind `submitted_at`.
pub fn new_job_id() -> String {
    format!("job-{}", Ulid::new())
}

/// The default branch for a job: `job/{job_id}`.
pub fn default_branch(job_id: &str) -> String {
    format!("job/{job_id}")
}

/// Deterministic idempotency key: `sha256(spec_ref \x1f title)` as lowercase hex.
/// A unit separator between the fields prevents `("ab", "c")` and `("a", "bc")`
/// from colliding.
pub fn idempotency_key_for(spec_ref: &str, title: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(spec_ref.as_bytes());
    hasher.update([0x1f]);
    hasher.update(title.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn submission() -> JobSubmission {
        JobSubmission {
            title: "Desktop app, Dia rebuild".to_string(),
            spec_ref: "docs/plans/theorem-desktop/HANDOFF.md".to_string(),
            repo: "Travis-Gilbert/theorem".to_string(),
            kind: JobKind::App,
            priority: None,
            target_head: None,
            branch: None,
            notes: None,
            idempotency_key: None,
        }
    }

    #[test]
    fn from_submission_applies_defaults() {
        let job = Job::from_submission(submission(), "claude.ai");
        assert!(job.job_id.starts_with("job-"));
        assert_eq!(job.status, JobStatus::Queued);
        assert_eq!(job.priority, Priority::P2);
        assert_eq!(job.target_head, TargetHead::Either);
        assert_eq!(job.branch_ref(), format!("job/{}", job.job_id));
        assert_eq!(
            job.idempotency_key,
            idempotency_key_for(&job.spec_ref, &job.title)
        );
        assert_eq!(job.submitted_by, "claude.ai");
    }

    #[test]
    fn idempotency_key_is_deterministic_and_separator_safe() {
        assert_eq!(
            idempotency_key_for("docs/plans/x/HANDOFF.md", "Title"),
            idempotency_key_for("docs/plans/x/HANDOFF.md", "Title")
        );
        // The unit separator prevents field-boundary collisions.
        assert_ne!(
            idempotency_key_for("ab", "c"),
            idempotency_key_for("a", "bc")
        );
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

        assert!(TargetHead::ClaudeCode.matches_lanes(&claude));
        assert!(!TargetHead::ClaudeCode.matches_lanes(&codex));
        assert!(TargetHead::Codex.matches_lanes(&codex));
        assert!(!TargetHead::Codex.matches_lanes(&claude));
        assert!(TargetHead::Either.matches_lanes(&claude));
        assert!(TargetHead::Either.matches_lanes(&codex));
        assert!(!TargetHead::Either.matches_lanes(&[]));

        // Acceptance criterion 3: a Codex-lane job never matches a claude-only receiver.
        assert!(!TargetHead::Codex.matches_lanes(&claude));
        assert_eq!(TargetHead::Either.preferred_lane(&both), Some(LANE_CLAUDE));
        assert_eq!(TargetHead::Either.preferred_lane(&codex), Some(LANE_CODEX));
        assert_eq!(TargetHead::Codex.preferred_lane(&claude), None);
    }

    #[test]
    fn claimable_requires_queued_repo_and_lane() {
        let job = Job::from_submission(submission(), "claude.ai");
        let lanes = vec![LANE_CLAUDE.to_string()];
        let repos = vec!["Travis-Gilbert/theorem".to_string()];
        assert!(job.claimable_by(&lanes, &repos));
        // Unmapped repo is never claimed (security fence).
        assert!(!job.claimable_by(&lanes, &["other/repo".to_string()]));

        let mut claimed = job.clone();
        claimed.status = JobStatus::Claimed;
        assert!(!claimed.claimable_by(&lanes, &repos));
    }

    #[test]
    fn lifecycle_transitions() {
        use JobStatus::*;
        assert!(JobStatus::can_transition(Queued, Claimed));
        assert!(JobStatus::can_transition(Queued, Cancelled));
        assert!(JobStatus::can_transition(Claimed, Running));
        assert!(JobStatus::can_transition(Running, Done));
        assert!(JobStatus::can_transition(Running, Failed));
        assert!(JobStatus::can_transition(PrOpen, Done));
        // Illegal jumps are rejected.
        assert!(!JobStatus::can_transition(Queued, Done));
        assert!(!JobStatus::can_transition(Done, Running));
        assert!(Done.is_terminal());
        assert!(Cancelled.is_terminal());
        assert!(Queued.can_cancel());
        assert!(Claimed.can_cancel());
        assert!(!Running.can_cancel());
    }
}
