//! The real, process-spawning [`RunExecutor`] (phone-control handoff Part B
//! "real run executor" wiring).
//!
//! [`MockExecutor`](crate::runs::MockExecutor) is the scripted, cargo-testable
//! stand-in that the run channel ships with; this module is its production
//! counterpart. [`ProcessRunExecutor`] executes a submitted run **locally against
//! the working tree** by spawning a real child process and streaming its output
//! as run events:
//!
//! * the child is spawned with `cwd` = the run's root (the working tree),
//! * each line of the child's stdout becomes a [`RunEventKind::Trace`] event,
//! * stderr is drained on its own thread (so a full stderr pipe can never
//!   deadlock the run) and surfaced as a trailing trace if non-empty,
//! * when the child exits, a `git diff --stat` over the root is emitted as a
//!   [`RunEventKind::Diff`] (the change the run made to the tree),
//! * the terminal [`RunState`] is `Done` (exit 0), `Failed` (nonzero / spawn
//!   error), or `Stopped` (the run channel signalled a cooperative stop, which
//!   this executor honors by actually killing the child).
//!
//! # How a stop kills the process
//!
//! The run channel's [`stop`](crate::runs::RunRegistry::stop) sets a cooperative
//! cancel flag the executor observes via [`RunEventSink::is_cancelled`]. A naive
//! blocking `read_line` loop could not observe that flag while a silent child
//! (e.g. `sleep 60`) produces no output, so stdout is read on a dedicated reader
//! thread that pushes lines down an mpsc channel. The executor's main loop polls
//! that channel on a short timeout and re-checks the cancel flag each tick; on a
//! stop it calls [`Child::kill`](std::process::Child::kill) (a real SIGKILL),
//! waits for the child to reap, and returns [`RunState::Stopped`]. So a stop both
//! ends the run AND terminates the spawned process.
//!
//! # Command building (and the claude/codex follow-up)
//!
//! What to spawn is decided by a [`CommandFactory`] seam. The shipped factory is
//! [`ShellCommandFactory`], which runs the run's `intent` as a shell command
//! (`sh -c <intent>`) -- enough to drive a real local toolchain and to prove the
//! executor end-to-end over a trivial command. Wiring the
//! `theorem-receiver` `HeadAdapter` (claude / codex CLI detection, env policy
//! that strips `ANTHROPIC_API_KEY`, receipt parsing) so a run spawns a real agent
//! is the named enrichment follow-up: implement [`CommandFactory`] for the
//! selected head and pass it to [`ProcessRunExecutor::with_command_factory`]. The
//! streaming / stop / diff machinery here is head-agnostic and is reused as-is.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::runs::{RunEventKind, RunEventSink, RunExecutor, RunSpec, RunState};

/// How often the streaming loop wakes to re-check the cooperative cancel flag
/// while the child produces no stdout. Small enough that a stop is honored
/// promptly; large enough that an idle run does not busy-spin.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How the spawned program + arguments are derived from a [`RunSpec`]. The
/// shipped impl is [`ShellCommandFactory`]; a real-agent run plugs a
/// claude/codex builder in here (see the module docs).
pub trait CommandFactory: Send + Sync + 'static {
    /// Build the `(program, args)` to spawn for this run. Returning `None` means
    /// the run has nothing to execute (an empty command); the executor then fails
    /// the run cleanly rather than spawning a degenerate process.
    fn build(&self, spec: &RunSpec) -> Option<ResolvedCommand>;
}

/// A fully-resolved command to spawn: the program and its argument vector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl ResolvedCommand {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }
}

/// The default command factory: run the run's `intent` as a shell command
/// (`sh -c <intent>`). This is what makes the executor real over a trivial
/// command (and drives an arbitrary local toolchain). A blank intent yields no
/// command, so the executor fails the run instead of spawning an empty `sh -c`.
#[derive(Clone, Debug, Default)]
pub struct ShellCommandFactory;

impl CommandFactory for ShellCommandFactory {
    fn build(&self, spec: &RunSpec) -> Option<ResolvedCommand> {
        let intent = spec.intent.trim();
        if intent.is_empty() {
            return None;
        }
        Some(ResolvedCommand::new(
            "sh",
            vec!["-c".to_string(), intent.to_string()],
        ))
    }
}

/// A real [`RunExecutor`] that spawns a child process in the working tree and
/// streams its output as run events. The production counterpart to
/// [`MockExecutor`](crate::runs::MockExecutor).
///
/// Construct with [`shell`](Self::shell) for the default `sh -c <intent>`
/// behavior, or [`with_command_factory`](Self::with_command_factory) to plug in a
/// claude/codex command builder (the named follow-up).
pub struct ProcessRunExecutor {
    /// The working tree the child runs in (`cwd`) and that the `git diff --stat`
    /// is computed over.
    root: PathBuf,
    /// How to derive the spawned command from a [`RunSpec`].
    factory: Box<dyn CommandFactory>,
}

impl ProcessRunExecutor {
    /// A process executor rooted at `root` that runs each run's `intent` as a
    /// shell command. `root` is the working tree: the child's `cwd` and the
    /// directory the diff is computed over.
    pub fn shell(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            factory: Box::new(ShellCommandFactory),
        }
    }

    /// A process executor rooted at `root` with a custom [`CommandFactory`] (e.g.
    /// a claude/codex `HeadAdapter`-backed builder). The streaming / stop / diff
    /// machinery is identical to [`shell`](Self::shell); only the spawned command
    /// differs.
    pub fn with_command_factory(root: impl Into<PathBuf>, factory: impl CommandFactory) -> Self {
        Self {
            root: root.into(),
            factory: Box::new(factory),
        }
    }

    /// The working-tree root the executor spawns children in.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl RunExecutor for ProcessRunExecutor {
    fn start(&self, spec: &RunSpec, sink: &RunEventSink) -> RunState {
        // An already-cancelled run (stopped before dispatch reached us) never
        // spawns a process.
        if sink.is_cancelled() {
            return RunState::Stopped;
        }

        let Some(command) = self.factory.build(spec) else {
            sink.emit(
                RunEventKind::Trace,
                "no command to run for this run (empty intent)",
            );
            return RunState::Failed;
        };

        sink.emit(
            RunEventKind::Trace,
            format!(
                "spawning: {} {} (cwd: {})",
                command.program,
                command.args.join(" "),
                self.root.display()
            ),
        );

        let mut child = match spawn_child(&command, &self.root) {
            Ok(child) => child,
            Err(error) => {
                sink.emit(
                    RunEventKind::Trace,
                    format!("failed to spawn {}: {error}", command.program),
                );
                return RunState::Failed;
            }
        };

        // Drain stdout on a reader thread (lines -> channel) and stderr on its
        // own thread (into a buffer). Reading on threads means a silent child is
        // still cancellable: the main loop polls the channel with a timeout and
        // re-checks the cancel flag every tick.
        let stdout_rx = spawn_stdout_reader(&mut child);
        let stderr_handle = spawn_stderr_drainer(&mut child);

        let outcome = stream_until_exit(&mut child, &stdout_rx, sink);

        // Reap and collect stderr regardless of how we exited.
        let stderr = stderr_handle
            .join()
            .unwrap_or_else(|_| "stderr reader thread panicked".to_string());

        match outcome {
            StreamOutcome::Cancelled => {
                // The child was killed in stream_until_exit; wait so it is reaped.
                let _ = child.wait();
                surface_stderr(sink, &stderr);
                // A stop is an honored cancel, not a failure: no diff (the work
                // was interrupted), terminal state is Stopped.
                RunState::Stopped
            }
            StreamOutcome::Exited(status) => {
                surface_stderr(sink, &stderr);
                emit_diff(sink, &self.root);
                let success = status.success();
                if let Some(code) = status.code() {
                    sink.emit(RunEventKind::Trace, format!("process exited with code {code}"));
                } else {
                    sink.emit(RunEventKind::Trace, "process exited (terminated by signal)");
                }
                if success {
                    RunState::Done
                } else {
                    RunState::Failed
                }
            }
            StreamOutcome::WaitError(error) => {
                surface_stderr(sink, &stderr);
                sink.emit(RunEventKind::Trace, format!("error waiting on process: {error}"));
                RunState::Failed
            }
        }
    }
}

/// Spawn the child with piped stdout/stderr in `root`. stdin is null so a child
/// that reads stdin sees EOF rather than blocking on the terminal.
fn spawn_child(command: &ResolvedCommand, root: &Path) -> std::io::Result<Child> {
    Command::new(&command.program)
        .args(&command.args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

/// Take the child's stdout and read it line-by-line on a dedicated thread,
/// pushing each line down the returned channel. The channel closes (the sender
/// drops) when stdout reaches EOF, which is how the main loop detects that the
/// child has finished writing.
fn spawn_stdout_reader(child: &mut Child) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        thread::Builder::new()
            .name("run-stdout-reader".to_string())
            .spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    match line {
                        Ok(line) => {
                            // A send error means the main loop has gone away
                            // (cancel/exit handled): stop reading.
                            if tx.send(line).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
            .ok();
    }
    rx
}

/// Take the child's stderr and drain it on a dedicated thread into a buffer
/// (returned via the join handle), so a verbose stderr can never fill its pipe
/// and deadlock the child. Surfaced as a trailing trace by the caller.
fn spawn_stderr_drainer(child: &mut Child) -> thread::JoinHandle<String> {
    let stderr = child.stderr.take();
    thread::Builder::new()
        .name("run-stderr-drainer".to_string())
        .spawn(move || {
            let mut buffer = String::new();
            if let Some(stderr) = stderr {
                let mut reader = BufReader::new(stderr);
                // Read to end; ignore decode errors (lossy) so binary stderr does
                // not abort the drain.
                let mut bytes = Vec::new();
                use std::io::Read;
                if reader.read_to_end(&mut bytes).is_ok() {
                    buffer = String::from_utf8_lossy(&bytes).into_owned();
                }
            }
            buffer
        })
        .expect("spawn stderr drainer thread")
}

/// The result of streaming a child's stdout until it finished or was cancelled.
enum StreamOutcome {
    /// The run was cooperatively cancelled; the child has been killed.
    Cancelled,
    /// The child exited on its own with this status.
    Exited(std::process::ExitStatus),
    /// Waiting on the child failed.
    WaitError(std::io::Error),
}

/// Stream the child's stdout lines as Trace events until the child exits or a
/// cooperative cancel is observed. Polls the stdout channel on a short timeout so
/// the cancel flag is re-checked even while the child is silent; on a cancel it
/// kills the child and returns [`StreamOutcome::Cancelled`].
fn stream_until_exit(
    child: &mut Child,
    stdout_rx: &mpsc::Receiver<String>,
    sink: &RunEventSink,
) -> StreamOutcome {
    loop {
        // Honor a stop promptly, whether or not the child is producing output.
        if sink.is_cancelled() {
            let _ = child.kill();
            // Drain any already-buffered stdout lines so the trace is complete up
            // to the kill point (best-effort, non-blocking).
            while let Ok(line) = stdout_rx.try_recv() {
                sink.emit(RunEventKind::Trace, line);
            }
            return StreamOutcome::Cancelled;
        }

        // Wait for the next stdout line, but only briefly, so we loop back to the
        // cancel check on a silent child.
        match stdout_rx.recv_timeout(CANCEL_POLL_INTERVAL) {
            Ok(line) => {
                sink.emit(RunEventKind::Trace, line);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // No output this tick. If the child has already exited, finish;
                // otherwise loop back to the cancel check.
                match child.try_wait() {
                    Ok(Some(status)) => return drain_then_exit(stdout_rx, sink, status),
                    Ok(None) => continue,
                    Err(error) => return StreamOutcome::WaitError(error),
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // stdout reached EOF: the child is done writing. Wait for it to
                // exit and return its status.
                match child.wait() {
                    Ok(status) => return drain_then_exit(stdout_rx, sink, status),
                    Err(error) => return StreamOutcome::WaitError(error),
                }
            }
        }
    }
}

/// Drain any remaining buffered stdout lines (the reader may have queued a few
/// before the channel closed) then return the exit outcome, so no late line is
/// dropped from the trace.
fn drain_then_exit(
    stdout_rx: &mpsc::Receiver<String>,
    sink: &RunEventSink,
    status: std::process::ExitStatus,
) -> StreamOutcome {
    while let Ok(line) = stdout_rx.try_recv() {
        sink.emit(RunEventKind::Trace, line);
    }
    StreamOutcome::Exited(status)
}

/// Emit the child's stderr as a trailing trace, if any (split into lines so the
/// SSE consumer sees discrete events rather than one blob).
fn surface_stderr(sink: &RunEventSink, stderr: &str) {
    for line in stderr.lines() {
        if !line.trim().is_empty() {
            sink.emit(RunEventKind::Trace, format!("stderr: {line}"));
        }
    }
}

/// Compute `git diff --stat` over the working tree and emit it as a Diff event.
/// Always emits a Diff (even "no changes" / "diff unavailable") so a run always
/// surfaces a diff summary, per the run-channel contract; a non-git directory or
/// a git failure is reported as unavailable rather than failing the run.
fn emit_diff(sink: &RunEventSink, root: &Path) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--stat"])
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let summary = String::from_utf8_lossy(&output.stdout);
            let summary = summary.trim();
            if summary.is_empty() {
                sink.emit(RunEventKind::Diff, "no changes to the working tree");
            } else {
                sink.emit(RunEventKind::Diff, summary.to_string());
            }
        }
        Ok(_) => {
            // git ran but returned nonzero (e.g. not a git repository).
            sink.emit(RunEventKind::Diff, "diff unavailable (not a git working tree)");
        }
        Err(_) => {
            sink.emit(RunEventKind::Diff, "diff unavailable (git not found)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::{RunError, RunRegistry, RunState};
    use std::sync::Arc;
    use std::time::Duration;

    /// Poll a run's record until `done` holds or a bound elapses, yielding to the
    /// tokio runtime so the spawned blocking executor makes progress.
    async fn wait_until<F>(
        registry: &RunRegistry,
        run_id: &str,
        mut done: F,
    ) -> Option<crate::runs::RunRecord>
    where
        F: FnMut(&crate::runs::RunRecord) -> bool,
    {
        let bound = Duration::from_secs(20);
        let step = Duration::from_millis(20);
        let mut waited = Duration::ZERO;
        loop {
            if let Some(record) = registry.record(run_id) {
                if done(&record) {
                    return Some(record);
                }
            }
            if waited >= bound {
                return registry.record(run_id);
            }
            tokio::time::sleep(step).await;
            waited += step;
        }
    }

    #[test]
    fn shell_factory_builds_sh_c_and_rejects_blank_intent() {
        let factory = ShellCommandFactory;
        let built = factory.build(&RunSpec::tier_one("echo hi")).unwrap();
        assert_eq!(built.program, "sh");
        assert_eq!(built.args, vec!["-c".to_string(), "echo hi".to_string()]);
        // A blank intent has no command (the executor fails such a run cleanly).
        assert!(factory.build(&RunSpec::tier_one("   ")).is_none());
    }

    #[tokio::test]
    async fn process_executor_streams_real_stdout_and_ends_done_with_a_diff() {
        // A trivial REAL command (no claude/codex needed): two echoed lines.
        let dir = tempfile::tempdir().unwrap();
        let executor = ProcessRunExecutor::shell(dir.path());
        let registry = RunRegistry::new(Arc::new(executor));

        let run_id = registry.submit(RunSpec::tier_one("echo working; echo done"));

        let record = wait_until(&registry, &run_id, |r| r.state.is_terminal())
            .await
            .expect("run record exists");
        assert_eq!(
            record.state,
            RunState::Done,
            "a successful process run ends Done"
        );

        // The child's real stdout was streamed as Trace events.
        let traces: Vec<&str> = record
            .events
            .iter()
            .filter(|e| e.kind == RunEventKind::Trace)
            .map(|e| e.body.as_str())
            .collect();
        assert!(
            traces.contains(&"working"),
            "the first stdout line is a Trace event; got {traces:?}"
        );
        assert!(
            traces.contains(&"done"),
            "the second stdout line is a Trace event; got {traces:?}"
        );

        // A Diff event was surfaced (the dir is not a git repo, so it reports
        // unavailable -- but a Diff is always emitted).
        assert!(
            record.events.iter().any(|e| e.kind == RunEventKind::Diff),
            "the run surfaces a Diff event on completion"
        );

        // The lifecycle reached a terminal Done via a Status event.
        assert!(
            record
                .events
                .iter()
                .any(|e| e.kind == RunEventKind::Status && e.body == "done"),
            "the run emits a terminal done Status"
        );
    }

    #[tokio::test]
    async fn process_executor_surfaces_a_real_git_diff() {
        // In a real git working tree with an uncommitted change, the diff --stat
        // is non-empty and names the changed file.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Best-effort git init + a committed file; skip the assertion if git is
        // absent so the suite does not require git to be installed.
        let git_ok = Command::new("git")
            .arg("-C")
            .arg(root)
            .arg("init")
            .stdin(Stdio::null())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !git_ok {
            eprintln!("git not available; skipping real-diff assertion");
            return;
        }
        // Configure identity so commit works in CI-like environments.
        for args in [
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "t"],
        ] {
            let _ = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(&args)
                .output();
        }
        std::fs::write(root.join("tracked.txt"), "one\n").unwrap();
        let _ = Command::new("git").arg("-C").arg(root).args(["add", "."]).output();
        let _ = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["commit", "-m", "init"])
            .output();

        let executor = ProcessRunExecutor::shell(root);
        let registry = RunRegistry::new(Arc::new(executor));
        // The run modifies the tracked file, so `git diff --stat` is non-empty.
        let run_id = registry.submit(RunSpec::tier_one("printf 'two\\n' >> tracked.txt"));
        let record = wait_until(&registry, &run_id, |r| r.state.is_terminal())
            .await
            .expect("run record exists");
        assert_eq!(record.state, RunState::Done);
        let diff = record
            .events
            .iter()
            .find(|e| e.kind == RunEventKind::Diff)
            .expect("a Diff event");
        assert!(
            diff.body.contains("tracked.txt"),
            "the Diff names the changed file; got {:?}",
            diff.body
        );
    }

    #[tokio::test]
    async fn stop_kills_a_long_running_process_and_ends_stopped() {
        // A long sleep: it produces NO stdout, so this proves the executor is
        // cancellable even while the child is silent (the reader-thread design).
        let dir = tempfile::tempdir().unwrap();
        let executor = ProcessRunExecutor::shell(dir.path());
        let registry = RunRegistry::new(Arc::new(executor));

        // 120s sleep -- far longer than the test bound, so only a real kill ends it.
        let run_id = registry.submit(RunSpec::tier_one("sleep 120"));

        // Wait until it is actually Running (the spawn trace has been emitted and
        // the child is in flight).
        wait_until(&registry, &run_id, |r| r.state == RunState::Running)
            .await
            .expect("run record exists");

        // Stop it: the cooperative cancel makes the executor kill the child.
        registry.stop(&run_id).expect("stop an in-flight run");

        let stopped = wait_until(&registry, &run_id, |r| r.state.is_terminal())
            .await
            .expect("run record exists");
        assert_eq!(
            stopped.state,
            RunState::Stopped,
            "stopping a real process run ends Stopped (the child was killed)"
        );
        // A stopped run surfaces no Diff (the work was interrupted).
        assert!(
            !stopped.events.iter().any(|e| e.kind == RunEventKind::Diff),
            "a killed run does not emit a completion Diff"
        );

        // Stopping the now-terminal run is an InvalidState error.
        assert!(matches!(
            registry.stop(&run_id),
            Err(RunError::InvalidState { .. })
        ));
    }

    #[tokio::test]
    async fn empty_command_fails_the_run_without_spawning() {
        let dir = tempfile::tempdir().unwrap();
        let executor = ProcessRunExecutor::shell(dir.path());
        let registry = RunRegistry::new(Arc::new(executor));
        // A blank intent: ShellCommandFactory returns no command.
        let run_id = registry.submit(RunSpec::tier_one("   "));
        let record = wait_until(&registry, &run_id, |r| r.state.is_terminal())
            .await
            .expect("run record exists");
        assert_eq!(
            record.state,
            RunState::Failed,
            "a run with no command fails cleanly rather than spawning"
        );
    }

    #[tokio::test]
    async fn nonzero_exit_fails_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let executor = ProcessRunExecutor::shell(dir.path());
        let registry = RunRegistry::new(Arc::new(executor));
        let run_id = registry.submit(RunSpec::tier_one("echo oops; exit 3"));
        let record = wait_until(&registry, &run_id, |r| r.state.is_terminal())
            .await
            .expect("run record exists");
        assert_eq!(
            record.state,
            RunState::Failed,
            "a nonzero child exit fails the run"
        );
        // stdout was still streamed before the failure.
        assert!(
            record
                .events
                .iter()
                .any(|e| e.kind == RunEventKind::Trace && e.body == "oops"),
            "stdout before a nonzero exit is still streamed"
        );
    }
}
