use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use theorem_harness_core::{Priority, TargetHead};
use time::OffsetDateTime;

/// Worker head that can execute a dispatch job.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Head {
    Claude,
    Codex,
    #[default]
    Either,
}

impl Head {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Either => "either",
        }
    }

    pub fn from_harness(value: TargetHead) -> Self {
        match value {
            TargetHead::Claude => Self::Claude,
            TargetHead::Codex => Self::Codex,
            TargetHead::Either => Self::Either,
        }
    }
}

impl TryFrom<&str> for Head {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            "either" => Ok(Self::Either),
            other => Err(format!("unsupported dispatch head: {other}")),
        }
    }
}

/// Hot execution state in Postgres.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    #[default]
    Pending,
    Claimed,
    Running,
    Done,
    Failed,
    Dead,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Dead => "dead",
        }
    }
}

impl TryFrom<&str> for JobState {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "claimed" => Ok(Self::Claimed),
            "running" => Ok(Self::Running),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            "dead" => Ok(Self::Dead),
            other => Err(format!("unsupported dispatch state: {other}")),
        }
    }
}

/// Failure class controls whether a job can be retried.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailureClass {
    Retryable,
    Fatal,
}

/// Minimal job payload stored in Postgres. Secrets do not belong here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub job_id: String,
    pub title: String,
    pub repo: Option<String>,
    pub spec_ref: Option<String>,
    pub spec_inline: Option<String>,
    pub target_head: Head,
    #[serde(default)]
    pub not_before: Option<String>,
    pub source_task_id: Option<String>,
    #[serde(default)]
    pub max_attempts: Option<i16>,
}

impl Job {
    pub fn from_harness(job: &theorem_harness_core::Job) -> Self {
        Self {
            job_id: job.job_id.clone(),
            title: job.title.clone(),
            repo: Some(job.repo.clone()),
            spec_ref: job.spec_ref.clone(),
            spec_inline: job.spec_inline.clone(),
            target_head: Head::from_harness(job.target_head),
            not_before: job.not_before.clone(),
            source_task_id: job.source_task_id.clone(),
            max_attempts: None,
        }
    }

    pub fn into_harness_submission(self) -> theorem_harness_core::JobSubmission {
        theorem_harness_core::JobSubmission {
            job_id: Some(self.job_id.clone()),
            title: self.title,
            spec_ref: self.spec_ref,
            spec_inline: self.spec_inline,
            repo: self.repo.unwrap_or_default(),
            priority: None,
            target_head: Some(match self.target_head {
                Head::Claude => TargetHead::Claude,
                Head::Codex => TargetHead::Codex,
                Head::Either => TargetHead::Either,
            }),
            not_before: self.not_before,
            source_task_id: self.source_task_id,
            source_project_id: None,
            idempotency_key: Some(format!("dispatch:{}", self.job_id)),
        }
    }
}

/// A row claimed by a receiver. Includes execution metadata for receipts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimedJob {
    pub job_id: String,
    pub title: String,
    pub repo: Option<String>,
    pub spec_ref: Option<String>,
    pub spec_inline: Option<String>,
    pub target_head: Head,
    pub priority: i16,
    pub state: JobState,
    pub not_before: OffsetDateTime,
    pub claimed_by: Option<String>,
    pub claimed_at: Option<OffsetDateTime>,
    pub lease_expires_at: Option<OffsetDateTime>,
    pub attempts: i16,
    pub max_attempts: i16,
    pub result: Option<Value>,
    pub source_task_id: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl ClaimedJob {
    pub fn job_payload(&self) -> Job {
        Job {
            job_id: self.job_id.clone(),
            title: self.title.clone(),
            repo: self.repo.clone(),
            spec_ref: self.spec_ref.clone(),
            spec_inline: self.spec_inline.clone(),
            target_head: self.target_head,
            not_before: Some(self.not_before.unix_timestamp().to_string()),
            source_task_id: self.source_task_id.clone(),
            max_attempts: Some(self.max_attempts),
        }
    }

    pub fn into_harness_job(self) -> theorem_harness_core::Job {
        let submission = self.job_payload().into_harness_submission();
        let submitted_by = self
            .claimed_by
            .unwrap_or_else(|| "postgres-dispatch".to_string());
        let mut job = theorem_harness_core::Job::from_submission(submission, submitted_by)
            .expect("claimed dispatch job has a spec");
        job.priority = priority_to_harness(self.priority);
        job
    }
}

/// Reaper counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReapReport {
    pub requeued: u64,
    pub dead: u64,
}

/// Inspectable queue distribution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateCount {
    pub state: JobState,
    pub count: i64,
}

pub fn priority_from_harness(priority: Priority) -> i16 {
    match priority {
        Priority::P0 => 0,
        Priority::P1 => 50,
        Priority::P2 => 100,
    }
}

pub fn priority_to_harness(priority: i16) -> Priority {
    match priority {
        i if i <= 0 => Priority::P0,
        i if i <= 50 => Priority::P1,
        _ => Priority::P2,
    }
}

pub(crate) fn duration_seconds(duration: Duration) -> Result<f64, String> {
    let seconds = duration.as_secs_f64();
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err("duration must be positive and finite".to_string());
    }
    Ok(seconds)
}
