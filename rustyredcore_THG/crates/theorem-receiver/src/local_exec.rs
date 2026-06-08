//! Local execution of a substrate-dispatched proof.
//!
//! The smoke-test finding (2026-06-07): `multihead_proof` runs inside the remote
//! Railway substrate, where the head's local checkout does not exist. There a
//! local `git`/`cargo` path and the working-tree `cwd` are both ENOENT, so the
//! `substrate_rerun` trust tier is only honest for what the container can see. A
//! proof of a head's *local* working tree has to run where the code lives: the
//! receiver's own checkout. That is what this module does, and the receiver
//! reports the receipt back through the existing dispatch-queue contract
//! (`job_complete` receipts), so no new MCP verb is required to close the loop.
//!
//! Plan-as-pure-data, mirroring `spawn.rs`: the proof shape ([`ProofPlan`]) is
//! built and asserted without touching the process table; only [`run_proof`]
//! executes anything.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::ReceiverResult;

/// The trust tier stamped on a locally-executed proof. Distinct from the remote
/// substrate's `substrate_rerun`: this rerun happened on the head's own checkout,
/// against the actual working tree, so its evidence is about the real code.
pub const TRUST_TIER_LOCAL: &str = "substrate_rerun_local";

/// How often a running child is polled for completion before the deadline.
const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// A fully-resolved local proof, as pure data (testable without running).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofPlan {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub timeout: Duration,
}

impl ProofPlan {
    pub fn new(
        command: impl Into<String>,
        args: Vec<String>,
        cwd: impl Into<PathBuf>,
        timeout: Duration,
    ) -> Self {
        Self {
            command: command.into(),
            args,
            cwd: cwd.into(),
            timeout,
        }
    }
}

/// The receipt of a local proof run. Mirrors the fields the work graph records
/// for a `multihead_proof` receipt, plus `trust_tier = substrate_rerun_local`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProofReceipt {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    /// "passed" iff the child exited 0 and did not time out; "failed" otherwise.
    pub status: String,
    pub trust_tier: String,
}

impl ProofReceipt {
    pub fn passed(&self) -> bool {
        self.status == "passed"
    }
}

/// Run a proof on the local checkout and return its receipt.
///
/// Output is captured through temp files rather than pipes so a verbose proof
/// (`cargo test` can emit megabytes) cannot dead-lock on a full pipe buffer while
/// we poll for the deadline: with pipes the child would block on write, never
/// exit, and be falsely killed as a timeout. Files never block the writer.
pub fn run_proof(plan: &ProofPlan) -> ReceiverResult<ProofReceipt> {
    let stamp = unique_stamp();
    let out_path = std::env::temp_dir().join(format!("theorem-proof-{stamp}.out"));
    let err_path = std::env::temp_dir().join(format!("theorem-proof-{stamp}.err"));

    let outcome = run_capture(plan, &out_path, &err_path);

    let stdout = fs::read_to_string(&out_path).unwrap_or_default();
    let stderr = fs::read_to_string(&err_path).unwrap_or_default();
    let _ = fs::remove_file(&out_path);
    let _ = fs::remove_file(&err_path);

    let (exit_code, timed_out) = outcome?;
    let passed = !timed_out && exit_code == Some(0);
    Ok(ProofReceipt {
        command: plan.command.clone(),
        args: plan.args.clone(),
        cwd: plan.cwd.display().to_string(),
        exit_code,
        stdout,
        stderr,
        timed_out,
        status: if passed { "passed" } else { "failed" }.to_string(),
        trust_tier: TRUST_TIER_LOCAL.to_string(),
    })
}

/// Spawn the child with output redirected to `out_path`/`err_path`, poll until it
/// exits or the deadline passes, and kill it on timeout. Returns
/// `(exit_code, timed_out)`; `exit_code` is `None` on a signal/timeout kill.
fn run_capture(
    plan: &ProofPlan,
    out_path: &Path,
    err_path: &Path,
) -> ReceiverResult<(Option<i32>, bool)> {
    let out_file = fs::File::create(out_path)?;
    let err_file = fs::File::create(err_path)?;
    let mut child = Command::new(&plan.command)
        .args(&plan.args)
        .current_dir(&plan.cwd)
        .stdin(Stdio::null())
        .stdout(out_file)
        .stderr(err_file)
        .spawn()?;

    let deadline = Instant::now() + plan.timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((status.code(), false));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok((None, true));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// A per-process-unique suffix for temp file names, without reaching for a clock
/// or randomness (a monotonic counter plus the pid is enough for uniqueness).
fn unique_stamp() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{n}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A proof that shells out to `sh -c <script>` (portable across the Linux CI
    /// container and a macOS dev box), run in the temp dir.
    fn sh(script: &str, timeout_ms: u64) -> ProofPlan {
        ProofPlan::new(
            "sh",
            vec!["-c".to_string(), script.to_string()],
            std::env::temp_dir(),
            Duration::from_millis(timeout_ms),
        )
    }

    #[test]
    fn plan_is_pure_data() {
        let plan = ProofPlan::new(
            "cargo",
            vec!["test".into(), "-p".into(), "theorem-receiver".into()],
            "/repos/theorem",
            Duration::from_secs(120),
        );
        assert_eq!(plan.command, "cargo");
        assert_eq!(plan.args.len(), 3);
        assert_eq!(plan.cwd, PathBuf::from("/repos/theorem"));
        assert_eq!(plan.timeout, Duration::from_secs(120));
    }

    #[test]
    fn exit_zero_passes_and_is_tagged_local() {
        let receipt = run_proof(&sh("exit 0", 5_000)).unwrap();
        assert_eq!(receipt.exit_code, Some(0));
        assert!(!receipt.timed_out);
        assert_eq!(receipt.status, "passed");
        assert!(receipt.passed());
        assert_eq!(receipt.trust_tier, TRUST_TIER_LOCAL);
    }

    #[test]
    fn nonzero_exit_fails_with_the_real_code() {
        let receipt = run_proof(&sh("exit 3", 5_000)).unwrap();
        assert_eq!(receipt.exit_code, Some(3));
        assert_eq!(receipt.status, "failed");
        assert!(!receipt.passed());
    }

    #[test]
    fn stdout_and_stderr_are_captured() {
        let receipt = run_proof(&sh("echo on-out; echo on-err 1>&2", 5_000)).unwrap();
        assert!(
            receipt.stdout.contains("on-out"),
            "stdout was {:?}",
            receipt.stdout
        );
        assert!(
            receipt.stderr.contains("on-err"),
            "stderr was {:?}",
            receipt.stderr
        );
    }

    #[test]
    fn a_proof_past_its_deadline_is_killed_and_fails() {
        // `sleep 5` cannot finish inside a 200ms budget: it must be killed.
        let receipt = run_proof(&sh("sleep 5", 200)).unwrap();
        assert!(receipt.timed_out, "expected a timeout kill");
        assert_eq!(receipt.exit_code, None, "a killed child has no exit code");
        assert_eq!(receipt.status, "failed");
    }
}
