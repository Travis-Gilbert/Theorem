//! The claim loop: poll, claim, spawn, wait, close.
//!
//! Outbound only. Idle until a job is claimed; one job runs to completion before
//! the next is claimed (capacity-1 default). When a job completes the loop
//! re-claims immediately; otherwise it sleeps the claim interval.

use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;

use serde_json::{json, Value};
use theorem_harness_core::Job;

use crate::config::ReceiverConfig;
use crate::head::adapter_for;
use crate::lanes::detect_lanes;
use crate::spawn::command_from_plan;
use crate::{client::HarnessClient, ReceiverError, ReceiverResult};

/// How many stdout lines to retain as the fallback receipt tail.
const STDOUT_TAIL_LINES: usize = 40;

/// What happened to one claimed job.
#[derive(Clone, Debug)]
pub struct JobRunReport {
    pub job_id: String,
    pub lane: String,
    pub exit_code: Option<i32>,
    /// True when the receiver's defensive Failed close actually took effect,
    /// i.e. the child exited without calling job_complete itself.
    pub defensive_close_applied: bool,
}

/// Run the receiver claim loop forever. Detects lanes once at startup; a machine
/// with no installed head is a hard error.
pub fn run_loop(config: &ReceiverConfig, client: &HarnessClient) -> ReceiverResult<()> {
    run_loop_until(config, client, || false)
}

/// Run the receiver claim loop until `should_stop` returns true.
///
/// The standalone binary uses [`run_loop`] for the historical forever-loop
/// behavior. Embedded hosts, such as Theorem Desktop, use this cancellable
/// variant so app shutdown does not leave a receiver thread behind.
pub fn run_loop_until(
    config: &ReceiverConfig,
    client: &HarnessClient,
    should_stop: impl Fn() -> bool,
) -> ReceiverResult<()> {
    let lanes = detect_lanes();
    if lanes.is_empty() {
        return Err(ReceiverError::Config(
            "no lanes detected: install the claude or codex CLI".to_string(),
        ));
    }
    let receiver_id = config.resolved_receiver_id();
    let repos = config.repos();
    log(&format!(
        "receiver {receiver_id} up: lanes={lanes:?} repos={repos:?} interval={}s",
        config.claim_interval_secs
    ));

    while !should_stop() {
        match client.job_claim(&receiver_id, &lanes, &repos) {
            Ok(Some(job)) => match run_job(config, client, &lanes, job) {
                Ok(report) => log(&format!(
                    "job {} done: lane={} exit={:?} defensive_close={}",
                    report.job_id, report.lane, report.exit_code, report.defensive_close_applied
                )),
                Err(error) => log(&format!("job run error: {error}")),
            },
            Ok(None) => sleep_until_stop(config.claim_interval(), &should_stop),
            Err(error) => {
                // A transient claim error must not kill the loop.
                log(&format!("claim error: {error}; backing off"));
                sleep_until_stop(config.claim_interval(), &should_stop);
            }
        }
    }
    log(&format!("receiver {receiver_id} stopping"));
    Ok(())
}

/// Resolve, spawn, supervise, and close one claimed job.
fn run_job(
    config: &ReceiverConfig,
    client: &HarnessClient,
    lanes: &[String],
    job: Job,
) -> ReceiverResult<JobRunReport> {
    let worktree = config.worktree_for(&job.repo).ok_or_else(|| {
        ReceiverError::Config(format!("no worktree mapped for repo {}", job.repo))
    })?;
    let lane = job.target_head.preferred_lane(lanes).ok_or_else(|| {
        ReceiverError::Protocol(format!(
            "claimed job {} targets a lane this receiver does not have",
            job.job_id
        ))
    })?;
    let adapter = adapter_for(lane).ok_or_else(|| {
        ReceiverError::Protocol(format!("no head adapter registered for lane {lane}"))
    })?;
    let intent = adapter.intent_template(&job.spec_ref, &job.job_id);
    let plan = adapter.spawn_plan(&intent, worktree);

    log(&format!(
        "claimed {} ({:?}/{:?}) -> spawning {} in {}",
        job.job_id,
        job.priority,
        job.target_head,
        lane,
        worktree.display()
    ));

    let mut command = command_from_plan(&plan);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    // stderr is inherited so the head's diagnostics stream live to the operator.

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            // Could not even start the head: close the job Failed with the reason.
            let receipts = json!({
                "source": "receiver_fallback",
                "lane": lane,
                "spawn_error": error.to_string(),
            });
            let _ = client.job_complete(&job.job_id, "failed", None, None, receipts);
            return Err(ReceiverError::Io(error));
        }
    };

    // Tee stdout to the operator while retaining a tail for the fallback receipt.
    let mut tail = TailBuffer::new(STDOUT_TAIL_LINES);
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        let sink = std::io::stdout();
        for line in reader.lines() {
            let line = line?;
            let _ = writeln!(sink.lock(), "{line}");
            tail.push(line);
        }
    }

    let status = child.wait()?;
    let exit_code = status.code();

    // Per-job usage line: from 2026-06-15, claude -p draws the finite monthly
    // Agent SDK credit bucket, so the draw is logged to be measurable.
    log(&format!(
        "usage: job={} lane={} exit={:?}",
        job.job_id, lane, exit_code
    ));

    // Failed-on-exit fallback. This is unconditional and idempotent: if the head
    // already called job_complete (Done or Failed), the job is terminal and this
    // is a no-op (applied=false). If the head exited WITHOUT completing, this
    // closes the job Failed with the exit receipt.
    let receipts = adapter.parse_receipt(exit_code, &tail.joined());
    let outcome = client.job_complete(&job.job_id, "failed", None, None, receipts)?;
    let defensive_close_applied = outcome
        .get("applied")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(JobRunReport {
        job_id: job.job_id,
        lane: lane.to_string(),
        exit_code,
        defensive_close_applied,
    })
}

/// A bounded ring of the most recent lines.
struct TailBuffer {
    capacity: usize,
    lines: std::collections::VecDeque<String>,
}

impl TailBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            lines: std::collections::VecDeque::with_capacity(capacity),
        }
    }

    fn push(&mut self, line: String) {
        if self.lines.len() == self.capacity {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    fn joined(&self) -> String {
        self.lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

fn log(message: &str) {
    eprintln!("[theorem-receiver] {message}");
}

fn sleep_until_stop(interval: std::time::Duration, should_stop: &impl Fn() -> bool) {
    let step = std::time::Duration::from_millis(250);
    let mut slept = std::time::Duration::ZERO;
    while slept < interval && !should_stop() {
        let remaining = interval.saturating_sub(slept);
        let next = remaining.min(step);
        std::thread::sleep(next);
        slept += next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_buffer_keeps_only_the_last_n_lines() {
        let mut tail = TailBuffer::new(2);
        tail.push("a".to_string());
        tail.push("b".to_string());
        tail.push("c".to_string());
        assert_eq!(tail.joined(), "b\nc");
    }
}
