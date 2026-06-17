//! The dispatch v2 launcher loop: list, start once, spawn, receipt.
//!
//! Outbound only. The graph is a board, not a lease table: the receiver polls
//! pending jobs, writes the one set-once `started_at`/`session_ref` note, then
//! launches a local head with the spec and harness context.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use theorem_harness_core::types::now_string;
use theorem_harness_core::{Job, LANE_CLAUDE, LANE_CODEX};

use crate::config::ReceiverConfig;
use crate::head::adapter_for;
use crate::lanes::detect_lanes;
use crate::spawn::command_from_plan;
use crate::{client::HarnessClient, ReceiverError, ReceiverResult};
use theorem_dispatch::{ClaimedJob, DispatchQueue, FailureClass, Head as DispatchHead, JobState};

/// How many stdout lines to retain in the exit receipt.
const STDOUT_TAIL_LINES: usize = 40;

/// What happened to one launched job.
#[derive(Clone, Debug)]
pub struct JobRunReport {
    pub job_id: String,
    pub lane: String,
    pub exit_code: Option<i32>,
    pub exit_receipt_written: bool,
}

/// Run the receiver launcher loop forever.
pub fn run_loop(config: &ReceiverConfig, client: &HarnessClient) -> ReceiverResult<()> {
    run_loop_until(config, client, || false)
}

/// Run the receiver launcher loop until `should_stop` returns true.
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

    if let Some(database_url) = config.dispatch_database_url() {
        return run_postgres_loop_until(
            config,
            client,
            &receiver_id,
            &lanes,
            &database_url,
            &should_stop,
        );
    }

    while !should_stop() {
        match next_launchable_job(client, config, &lanes) {
            Ok(Some(job)) => match start_and_run_job(config, client, &receiver_id, &lanes, job) {
                Ok(Some(report)) => log(&format!(
                    "job {} exited: lane={} exit={:?} receipt={}",
                    report.job_id, report.lane, report.exit_code, report.exit_receipt_written
                )),
                Ok(None) => {}
                Err(error) => log(&format!("job run error: {error}")),
            },
            Ok(None) => sleep_until_stop(config.claim_interval(), &should_stop),
            Err(error) => {
                log(&format!("list error: {error}; backing off"));
                sleep_until_stop(config.claim_interval(), &should_stop);
            }
        }
    }
    log(&format!("receiver {receiver_id} stopping"));
    Ok(())
}

fn run_postgres_loop_until(
    config: &ReceiverConfig,
    client: &HarnessClient,
    receiver_id: &str,
    lanes: &[String],
    database_url: &str,
    should_stop: &impl Fn() -> bool,
) -> ReceiverResult<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    let queue = runtime.block_on(DispatchQueue::connect(database_url))?;
    let heads = dispatch_heads_for_lanes(lanes);
    if heads.is_empty() {
        return Err(ReceiverError::Config(
            "no dispatch heads mapped from detected lanes".to_string(),
        ));
    }

    log(&format!(
        "receiver {receiver_id} using Postgres dispatch queue: heads={heads:?} lease={}s heartbeat={}s reap={}s",
        config.dispatch_lease_secs,
        config.dispatch_heartbeat_secs,
        config.dispatch_reap_interval_secs
    ));

    let mut next_reap = Instant::now();
    while !should_stop() {
        if Instant::now() >= next_reap {
            match runtime.block_on(queue.reap()) {
                Ok(report) if report.requeued > 0 || report.dead > 0 => log(&format!(
                    "reaped expired dispatch leases: requeued={} dead={}",
                    report.requeued, report.dead
                )),
                Ok(_) => {}
                Err(error) => log(&format!("dispatch reap error: {error}")),
            }
            next_reap = Instant::now() + config.dispatch_reap_interval();
        }

        match runtime.block_on(queue.claim_next_for_heads(
            receiver_id,
            &heads,
            config.dispatch_lease(),
        )) {
            Ok(Some(claimed)) => match start_and_run_dispatch_job(
                &runtime,
                &queue,
                config,
                client,
                receiver_id,
                lanes,
                claimed,
            ) {
                Ok(Some(report)) => log(&format!(
                    "dispatch job {} exited: lane={} exit={:?} receipt={}",
                    report.job_id, report.lane, report.exit_code, report.exit_receipt_written
                )),
                Ok(None) => {}
                Err(error) => log(&format!("dispatch job run error: {error}")),
            },
            Ok(None) => sleep_until_stop(config.claim_interval(), should_stop),
            Err(error) => {
                log(&format!("dispatch claim error: {error}; backing off"));
                sleep_until_stop(config.claim_interval(), should_stop);
            }
        }
    }

    log(&format!("receiver {receiver_id} stopping"));
    Ok(())
}

fn dispatch_heads_for_lanes(lanes: &[String]) -> Vec<DispatchHead> {
    let mut heads = Vec::new();
    if lanes.iter().any(|lane| lane == LANE_CLAUDE) {
        heads.push(DispatchHead::Claude);
    }
    if lanes.iter().any(|lane| lane == LANE_CODEX) {
        heads.push(DispatchHead::Codex);
    }
    heads
}

fn next_launchable_job(
    client: &HarnessClient,
    config: &ReceiverConfig,
    lanes: &[String],
) -> ReceiverResult<Option<Job>> {
    let repos = config.repos();
    let mut jobs = client.job_list(None, Some("pending"))?;
    jobs.retain(|job| {
        repos.iter().any(|repo| repo == &job.repo)
            && job.target_head.matches_lanes(lanes)
            && !not_before_is_future(job.not_before.as_deref())
    });
    jobs.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.submitted_at.cmp(&b.submitted_at))
            .then_with(|| a.job_id.cmp(&b.job_id))
    });
    Ok(jobs.into_iter().next())
}

fn start_and_run_job(
    config: &ReceiverConfig,
    client: &HarnessClient,
    receiver_id: &str,
    lanes: &[String],
    job: Job,
) -> ReceiverResult<Option<JobRunReport>> {
    let session_ref = format!("receiver:{receiver_id}:{}", job.job_id);
    let start = client.job_note(
        &job.job_id,
        receiver_id,
        &format!("starting local session {session_ref}"),
        Vec::new(),
        Some(session_ref.clone()),
        false,
    )?;
    if !start
        .get("applied")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        log(&format!("start race lost for {}", job.job_id));
        return Ok(None);
    }

    match run_job(config, client, receiver_id, lanes, &job, &session_ref) {
        Ok(report) => Ok(Some(report)),
        Err(error) => {
            let _ = client.job_note(
                &job.job_id,
                receiver_id,
                &format!("receiver abort before launch: {error}"),
                Vec::new(),
                None,
                true,
            );
            Err(error)
        }
    }
}

fn start_and_run_dispatch_job(
    runtime: &tokio::runtime::Runtime,
    queue: &DispatchQueue,
    config: &ReceiverConfig,
    client: &HarnessClient,
    receiver_id: &str,
    lanes: &[String],
    claimed: ClaimedJob,
) -> ReceiverResult<Option<JobRunReport>> {
    let job_payload = claimed.job_payload();
    let job = claimed.clone().into_harness_job();
    if let Err(error) = client.job_submit(job_payload.into_harness_submission(), receiver_id) {
        runtime.block_on(queue.fail(
            &job.job_id,
            FailureClass::Retryable,
            json!({
                "stage": "board_submit",
                "error": error.to_string(),
            }),
        ))?;
        return Err(error);
    }

    let session_ref = format!("receiver:{receiver_id}:{}", job.job_id);
    let start = client.job_note(
        &job.job_id,
        receiver_id,
        &format!("starting local session {session_ref}"),
        Vec::new(),
        Some(session_ref.clone()),
        false,
    )?;
    if !start
        .get("applied")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        runtime.block_on(queue.fail(
            &job.job_id,
            FailureClass::Fatal,
            json!({
                "stage": "board_start",
                "error": "start note was not applied",
                "response": start,
            }),
        ))?;
        log(&format!("dispatch start skipped for {}", job.job_id));
        return Ok(None);
    }

    runtime.block_on(queue.renew_lease(&job.job_id, config.dispatch_lease()))?;
    let heartbeat = match DispatchHeartbeat::start(
        queue.clone(),
        job.job_id.clone(),
        config.dispatch_lease(),
        config.dispatch_heartbeat_interval(),
    ) {
        Ok(heartbeat) => heartbeat,
        Err(error) => {
            let _ = client.job_note(
                &job.job_id,
                receiver_id,
                &format!("receiver abort before launch: {error}"),
                Vec::new(),
                None,
                true,
            );
            runtime.block_on(queue.fail(
                &job.job_id,
                FailureClass::Retryable,
                json!({
                    "stage": "dispatch_heartbeat",
                    "error": error.to_string(),
                }),
            ))?;
            return Err(error);
        }
    };

    let run_result = run_job(config, client, receiver_id, lanes, &job, &session_ref);
    heartbeat.stop();

    match run_result {
        Ok(report) => {
            if report.exit_code == Some(0) {
                runtime.block_on(queue.complete(
                    &job.job_id,
                    json!({
                        "exit_code": report.exit_code,
                        "lane": report.lane,
                        "exit_receipt_written": report.exit_receipt_written,
                    }),
                ))?;
                let _ = client.job_archive(&job.job_id, "done", receiver_id);
            } else {
                let state = runtime.block_on(queue.fail(
                    &job.job_id,
                    FailureClass::Retryable,
                    json!({
                        "exit_code": report.exit_code,
                        "lane": report.lane,
                        "exit_receipt_written": report.exit_receipt_written,
                    }),
                ))?;
                if state == JobState::Dead {
                    let _ = client.job_archive(&job.job_id, "dead-lettered", receiver_id);
                } else {
                    let _ = client.job_note(
                        &job.job_id,
                        receiver_id,
                        "dispatch retry scheduled after child exit",
                        Vec::new(),
                        None,
                        true,
                    );
                }
            }
            Ok(Some(report))
        }
        Err(error) => {
            let _ = client.job_note(
                &job.job_id,
                receiver_id,
                &format!("receiver abort before launch: {error}"),
                Vec::new(),
                None,
                true,
            );
            runtime.block_on(queue.fail(
                &job.job_id,
                FailureClass::Retryable,
                json!({
                    "stage": "receiver_run",
                    "error": error.to_string(),
                }),
            ))?;
            Err(error)
        }
    }
}

struct DispatchHeartbeat {
    stop: mpsc::Sender<()>,
    handle: thread::JoinHandle<()>,
}

impl DispatchHeartbeat {
    fn start(
        queue: DispatchQueue,
        job_id: String,
        lease: Duration,
        interval: Duration,
    ) -> ReceiverResult<Self> {
        let (stop, stop_rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name(format!("dispatch-heartbeat-{job_id}"))
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        log(&format!("dispatch heartbeat runtime error: {error}"));
                        return;
                    }
                };
                loop {
                    match stop_rx.recv_timeout(interval) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if let Err(error) = runtime.block_on(queue.renew_lease(&job_id, lease))
                            {
                                log(&format!("dispatch heartbeat failed for {job_id}: {error}"));
                            }
                        }
                    }
                }
            })?;
        Ok(Self { stop, handle })
    }

    fn stop(self) {
        let _ = self.stop.send(());
        let _ = self.handle.join();
    }
}

fn run_job(
    config: &ReceiverConfig,
    client: &HarnessClient,
    receiver_id: &str,
    lanes: &[String],
    job: &Job,
    session_ref: &str,
) -> ReceiverResult<JobRunReport> {
    let worktree = config.worktree_for(&job.repo).ok_or_else(|| {
        ReceiverError::Config(format!("no worktree mapped for repo {}", job.repo))
    })?;
    let lane = job.target_head.preferred_lane(lanes).ok_or_else(|| {
        ReceiverError::Protocol(format!(
            "job {} targets a lane this receiver does not have",
            job.job_id
        ))
    })?;
    let adapter = adapter_for(lane).ok_or_else(|| {
        ReceiverError::Protocol(format!("no head adapter registered for lane {lane}"))
    })?;

    let spec_text = resolve_spec_text(job, worktree)?;
    let context_packet = coordination_context_packet(client, &job.job_id);
    probe_harness(client).map_err(|error| {
        ReceiverError::Protocol(format!("harness probe failed before launch: {error}"))
    })?;
    let intent = build_launch_prompt(job, &spec_text, &context_packet, receiver_id, session_ref);
    let plan = adapter.spawn_plan(&intent, worktree);

    log(&format!(
        "starting {} ({:?}/{:?}) -> spawning {} in {}",
        job.job_id,
        job.priority,
        job.target_head,
        lane,
        worktree.display()
    ));

    let mut command = command_from_plan(&plan);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = client.job_note(
                &job.job_id,
                receiver_id,
                &format!("spawn error for {lane}: {error}"),
                Vec::new(),
                None,
                true,
            );
            return Err(ReceiverError::Io(error));
        }
    };

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
    log(&format!(
        "usage: job={} lane={} exit={:?}",
        job.job_id, lane, exit_code
    ));

    let receipt = json!({
        "source": "receiver_exit",
        "lane": lane,
        "exit_code": exit_code,
        "stdout_tail": tail.joined(),
        "branch_tip": branch_tip(worktree),
    });
    let written = client.job_note(
        &job.job_id,
        receiver_id,
        &format!("child exited: {receipt}"),
        Vec::new(),
        None,
        false,
    )?;
    Ok(JobRunReport {
        job_id: job.job_id.clone(),
        lane: lane.to_string(),
        exit_code,
        exit_receipt_written: written
            .get("applied")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

fn resolve_spec_text(job: &Job, worktree: &Path) -> ReceiverResult<String> {
    if let Some(spec_inline) = job.spec_inline.as_deref() {
        return Ok(spec_inline.to_string());
    }
    let Some(spec_ref) = job.spec_ref.as_deref() else {
        return Err(ReceiverError::Protocol(format!(
            "job {} has neither spec_ref nor spec_inline",
            job.job_id
        )));
    };
    let path = worktree.join(spec_ref);
    std::fs::read_to_string(&path).map_err(|error| {
        ReceiverError::Config(format!("cannot read spec_ref {}: {error}", path.display()))
    })
}

fn coordination_context_packet(client: &HarnessClient, job_id: &str) -> String {
    match client.call_tool(
        "coordination_context",
        json!({ "room_id": "repo:theorem:branch:main", "job_id": job_id }),
    ) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
        Err(error) => format!("coordination_context unavailable: {error}"),
    }
}

fn probe_harness(client: &HarnessClient) -> ReceiverResult<()> {
    client.tools_list().map(|_| ())
}

pub fn build_launch_prompt(
    job: &Job,
    spec_text: &str,
    context_packet: &str,
    actor: &str,
    session_ref: &str,
) -> String {
    format!(
        "SPEC\n----\n{spec_text}\n\nCONTEXT PACKET\n--------------\n{context_packet}\n\nFOOTER\n------\nactor: {actor}\njob_id: {job_id}\nroom_id: repo:theorem:branch:main\nsession_ref: {session_ref}\nrepo: {repo}\ntarget_head: {target_head:?}\n\nDoctrine:\n- Dependencies named in a spec are information for you, not gates. Check the tree, decide, and note your reasoning on the job.\n- Refresh with coordination_context if this packet looks stale.\n\nWhen you make progress, call job_note with commit refs or receipts. When the thread is complete, anyone may call job_archive with reason \"done\".",
        job_id = job.job_id,
        repo = job.repo,
        target_head = job.target_head,
    )
}

fn not_before_is_future(not_before: Option<&str>) -> bool {
    let Some(not_before) = not_before else {
        return false;
    };
    let Some(epoch) = not_before
        .split('.')
        .next()
        .and_then(|part| part.parse::<u64>().ok())
    else {
        return false;
    };
    let now_epoch = now_string()
        .split('.')
        .next()
        .and_then(|part| part.parse::<u64>().ok())
        .unwrap_or(0);
    epoch > now_epoch
}

fn branch_tip(worktree: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(worktree)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
    use theorem_harness_core::{Priority, TargetHead};

    fn job_fixture() -> Job {
        Job {
            job_id: "job-001".to_string(),
            title: "Dia".to_string(),
            spec_ref: Some("docs/plans/x/HANDOFF.md".to_string()),
            spec_inline: None,
            repo: "Travis-Gilbert/theorem".to_string(),
            priority: Priority::P0,
            target_head: TargetHead::Either,
            not_before: None,
            source_task_id: None,
            source_project_id: None,
            submitted_by: "claude.ai".to_string(),
            submitted_at: "1.000000000Z".to_string(),
            started_at: None,
            session_ref: None,
            archived_at: None,
            archived_reason: None,
            idempotency_key: "sha256:abc".to_string(),
            receipts: Vec::new(),
        }
    }

    #[test]
    fn tail_buffer_keeps_only_the_last_n_lines() {
        let mut tail = TailBuffer::new(2);
        tail.push("a".to_string());
        tail.push("b".to_string());
        tail.push("c".to_string());
        assert_eq!(tail.joined(), "b\nc");
    }

    #[test]
    fn launch_prompt_contains_spec_context_and_footer() {
        let job = job_fixture();
        let prompt = build_launch_prompt(
            &job,
            "Build the thing.",
            "{\"context\":true}",
            "receiver-a",
            "session-a",
        );
        assert!(prompt.contains("Build the thing."));
        assert!(prompt.contains("CONTEXT PACKET"));
        assert!(prompt.contains("receiver-a"));
        assert!(prompt.contains("job-001"));
        assert!(prompt.contains("Dependencies named in a spec are information"));
        assert!(prompt.contains("coordination_context"));
        assert!(prompt.contains("job_note"));
    }

    #[test]
    fn not_before_epoch_skips_future_only() {
        assert!(!not_before_is_future(None));
        assert!(!not_before_is_future(Some("1.000000000Z")));
        assert!(not_before_is_future(Some("9999999999.000000000Z")));
        assert!(!not_before_is_future(Some("not-a-timestamp")));
    }
}
