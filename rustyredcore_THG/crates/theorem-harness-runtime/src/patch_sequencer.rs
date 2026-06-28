use std::error::Error;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::json;
use theorem_harness_core::{stable_value_hash, Receipt};

pub type PatchSequencerResult<T> = Result<T, PatchSequencerError>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PatchProposal {
    pub patch_id: String,
    pub node_id: String,
    pub owner: String,
    pub base_commit: String,
    pub diff: String,
    #[serde(default)]
    pub file_scope: Vec<String>,
    #[serde(default = "default_claimed_status")]
    pub claimed_status: String,
}

impl PatchProposal {
    pub fn new(
        patch_id: impl Into<String>,
        node_id: impl Into<String>,
        owner: impl Into<String>,
        base_commit: impl Into<String>,
        diff: impl Into<String>,
    ) -> Self {
        Self {
            patch_id: patch_id.into(),
            node_id: node_id.into(),
            owner: owner.into(),
            base_commit: base_commit.into(),
            diff: diff.into(),
            file_scope: Vec::new(),
            claimed_status: default_claimed_status(),
        }
    }

    pub fn with_file_scope(mut self, file_scope: Vec<String>) -> Self {
        self.file_scope = file_scope;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchApplyStatus {
    Applied,
    Conflict,
    BaseNotAncestor,
    DirtyWorktree,
    NoChanges,
}

impl PatchApplyStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Conflict => "conflict",
            Self::BaseNotAncestor => "base_not_ancestor",
            Self::DirtyWorktree => "dirty_worktree",
            Self::NoChanges => "no_changes",
        }
    }

    pub fn is_accepted(self) -> bool {
        matches!(self, Self::Applied)
    }

    pub fn should_reopen_node(self) -> bool {
        matches!(self, Self::Conflict | Self::BaseNotAncestor)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PatchApplyReceipt {
    pub patch_id: String,
    pub node_id: String,
    pub owner: String,
    pub base_commit: String,
    pub head_before: String,
    pub head_after: String,
    pub status: PatchApplyStatus,
    pub command: String,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    pub receipt: Receipt,
}

impl PatchApplyReceipt {
    pub fn is_accepted(&self) -> bool {
        self.status.is_accepted()
    }

    pub fn should_reopen_node(&self) -> bool {
        self.status.should_reopen_node()
    }

    pub fn reopen_base_commit(&self) -> Option<&str> {
        self.should_reopen_node()
            .then_some(self.head_before.as_str())
    }
}

#[derive(Clone, Debug)]
pub struct PatchSequencer {
    repo_path: PathBuf,
    apply_lock: Arc<Mutex<()>>,
}

impl PatchSequencer {
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
            apply_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    pub fn apply_proposal(
        &self,
        proposal: &PatchProposal,
    ) -> PatchSequencerResult<PatchApplyReceipt> {
        let _guard = self
            .apply_lock
            .lock()
            .map_err(|_| PatchSequencerError::LockPoisoned)?;

        let head_before = git_stdout(&self.repo_path, ["rev-parse", "--verify", "HEAD"])?;
        if let Some(dirty) = dirty_tracked_worktree(&self.repo_path)? {
            return Ok(self.receipt(
                proposal,
                &head_before,
                &head_before,
                PatchApplyStatus::DirtyWorktree,
                "git status --porcelain=v1 --untracked-files=no",
                "",
                &dirty,
            ));
        }

        if !git_status(
            &self.repo_path,
            ["merge-base", "--is-ancestor", &proposal.base_commit, "HEAD"],
        )?
        .success
        {
            return Ok(self.receipt(
                proposal,
                &head_before,
                &head_before,
                PatchApplyStatus::BaseNotAncestor,
                "git merge-base --is-ancestor <base> HEAD",
                "",
                "",
            ));
        }

        let normalized_diff;
        let patch_stdin = if proposal.diff.ends_with('\n') {
            proposal.diff.as_bytes()
        } else {
            normalized_diff = format!("{}\n", proposal.diff);
            normalized_diff.as_bytes()
        };
        let apply = git_with_stdin(
            &self.repo_path,
            ["apply", "--3way", "--index", "-"],
            patch_stdin,
        )?;
        if !apply.success {
            let _ = git_status(&self.repo_path, ["reset", "--hard", "HEAD"]);
            let head_after = git_stdout(&self.repo_path, ["rev-parse", "--verify", "HEAD"])?;
            return Ok(self.receipt(
                proposal,
                &head_before,
                &head_after,
                PatchApplyStatus::Conflict,
                "git apply --3way --index -",
                &apply.stdout,
                &apply.stderr,
            ));
        }

        if git_status(&self.repo_path, ["diff", "--cached", "--quiet"])?.success {
            return Ok(self.receipt(
                proposal,
                &head_before,
                &head_before,
                PatchApplyStatus::NoChanges,
                "git apply --3way --index -",
                &apply.stdout,
                &apply.stderr,
            ));
        }

        let commit_message = format!(
            "apply patch proposal {} for node {}",
            proposal.patch_id, proposal.node_id
        );
        let commit = git_status_owned(
            &self.repo_path,
            vec![
                "commit".to_string(),
                "-m".to_string(),
                commit_message,
                "--no-gpg-sign".to_string(),
            ],
        )?;
        if !commit.success {
            let _ = git_status(&self.repo_path, ["reset", "--hard", "HEAD"]);
            return Err(PatchSequencerError::Git {
                command: "git commit -m <patch proposal>".to_string(),
                code: commit.code,
                stdout: commit.stdout,
                stderr: commit.stderr,
            });
        }

        let head_after = git_stdout(&self.repo_path, ["rev-parse", "--verify", "HEAD"])?;
        Ok(self.receipt(
            proposal,
            &head_before,
            &head_after,
            PatchApplyStatus::Applied,
            "git apply --3way --index - && git commit",
            &format!("{}\n{}", apply.stdout, commit.stdout),
            &format!("{}\n{}", apply.stderr, commit.stderr),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn receipt(
        &self,
        proposal: &PatchProposal,
        head_before: &str,
        head_after: &str,
        status: PatchApplyStatus,
        command: &str,
        stdout: &str,
        stderr: &str,
    ) -> PatchApplyReceipt {
        let artifact_hash = stable_value_hash(&json!({
            "patch_id": proposal.patch_id,
            "node_id": proposal.node_id,
            "owner": proposal.owner,
            "base_commit": proposal.base_commit,
            "head_before": head_before,
            "head_after": head_after,
            "status": status.as_str(),
            "diff": proposal.diff,
        }));
        PatchApplyReceipt {
            patch_id: proposal.patch_id.clone(),
            node_id: proposal.node_id.clone(),
            owner: proposal.owner.clone(),
            base_commit: proposal.base_commit.clone(),
            head_before: head_before.to_string(),
            head_after: head_after.to_string(),
            status,
            command: command.to_string(),
            stdout: truncate_receipt_output(stdout),
            stderr: truncate_receipt_output(stderr),
            receipt: Receipt {
                kind: "patch_apply".to_string(),
                command: command.to_string(),
                base_commit: proposal.base_commit.clone(),
                claimed_status: proposal.claimed_status.clone(),
                verified_status: Some(status.as_str().to_string()),
                artifact_hash,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatchSequencerError {
    Io(String),
    Git {
        command: String,
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    LockPoisoned,
}

impl fmt::Display for PatchSequencerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "patch sequencer io failed: {error}"),
            Self::Git {
                command,
                code,
                stdout,
                stderr,
            } => write!(
                f,
                "patch sequencer git command failed ({command}, code {code:?}): stdout={stdout:?} stderr={stderr:?}"
            ),
            Self::LockPoisoned => write!(f, "patch sequencer apply lock was poisoned"),
        }
    }
}

impl Error for PatchSequencerError {}

impl From<std::io::Error> for PatchSequencerError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GitCommandOutput {
    success: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn default_claimed_status() -> String {
    "applied".to_string()
}

fn dirty_tracked_worktree(repo_path: &Path) -> PatchSequencerResult<Option<String>> {
    let status = git_stdout(
        repo_path,
        ["status", "--porcelain=v1", "--untracked-files=no"],
    )?;
    Ok((!status.trim().is_empty()).then_some(status))
}

fn git_stdout<const N: usize>(repo_path: &Path, args: [&str; N]) -> PatchSequencerResult<String> {
    let command = format_git_command(&args);
    let output = git_status(repo_path, args)?;
    if output.success {
        Ok(output.stdout.trim().to_string())
    } else {
        Err(PatchSequencerError::Git {
            command,
            code: output.code,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

fn git_status<const N: usize>(
    repo_path: &Path,
    args: [&str; N],
) -> PatchSequencerResult<GitCommandOutput> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()?;
    Ok(GitCommandOutput {
        success: output.status.success(),
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn git_status_owned(repo_path: &Path, args: Vec<String>) -> PatchSequencerResult<GitCommandOutput> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()?;
    Ok(GitCommandOutput {
        success: output.status.success(),
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn git_with_stdin<const N: usize>(
    repo_path: &Path,
    args: [&str; N],
    stdin: &[u8],
) -> PatchSequencerResult<GitCommandOutput> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut input) = child.stdin.take() {
        input.write_all(stdin)?;
    }
    let output = child.wait_with_output()?;
    Ok(GitCommandOutput {
        success: output.status.success(),
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn format_git_command<const N: usize>(args: &[&str; N]) -> String {
    format!("git {}", args.join(" "))
}

fn truncate_receipt_output(output: &str) -> String {
    const MAX_OUTPUT_BYTES: usize = 4096;
    if output.len() <= MAX_OUTPUT_BYTES {
        output.to_string()
    } else {
        let mut end = MAX_OUTPUT_BYTES;
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...[truncated]", &output[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn sequencer_applies_non_overlapping_patches_to_same_file() {
        let repo = TempRepo::new("non-overlap");
        write_fixture(repo.path(), "old-alpha", "old-beta");
        git(repo.path(), ["add", "."]);
        git(repo.path(), ["commit", "-m", "base", "--no-gpg-sign"]);
        let base = git(repo.path(), ["rev-parse", "HEAD"]);

        let first = proposal_from_base(repo.path(), &base, "patch-a", "node-a", |content| {
            content.replace("old-alpha", "new-alpha")
        });
        let second = proposal_from_base(repo.path(), &base, "patch-b", "node-b", |content| {
            content.replace("old-beta", "new-beta")
        });

        let sequencer = PatchSequencer::new(repo.path());
        let first_receipt = sequencer.apply_proposal(&first).unwrap();
        let second_receipt = sequencer.apply_proposal(&second).unwrap();

        assert_eq!(first_receipt.status, PatchApplyStatus::Applied);
        assert_eq!(second_receipt.status, PatchApplyStatus::Applied);
        assert!(first_receipt.receipt.is_substrate_verified());
        assert!(second_receipt.receipt.is_substrate_verified());

        let content = fs::read_to_string(repo.path().join("router.rs")).unwrap();
        assert!(content.contains("new-alpha"));
        assert!(content.contains("new-beta"));
        assert_ne!(first_receipt.head_after, second_receipt.head_after);
        assert_clean(repo.path());
    }

    #[test]
    fn sequencer_rejects_conflict_and_reopens_against_current_head() {
        let repo = TempRepo::new("conflict");
        write_fixture(repo.path(), "shared-old", "stable");
        git(repo.path(), ["add", "."]);
        git(repo.path(), ["commit", "-m", "base", "--no-gpg-sign"]);
        let base = git(repo.path(), ["rev-parse", "HEAD"]);

        let first = proposal_from_base(repo.path(), &base, "patch-a", "node-a", |content| {
            content.replace("shared-old", "shared-first")
        });
        let second = proposal_from_base(repo.path(), &base, "patch-b", "node-b", |content| {
            content.replace("shared-old", "shared-second")
        });

        let sequencer = PatchSequencer::new(repo.path());
        let accepted = sequencer.apply_proposal(&first).unwrap();
        let rejected = sequencer.apply_proposal(&second).unwrap();

        assert_eq!(accepted.status, PatchApplyStatus::Applied);
        assert_eq!(rejected.status, PatchApplyStatus::Conflict);
        assert!(rejected.should_reopen_node());
        assert_eq!(
            rejected.reopen_base_commit(),
            Some(accepted.head_after.as_str())
        );
        assert!(!rejected.receipt.is_substrate_verified());

        let content = fs::read_to_string(repo.path().join("router.rs")).unwrap();
        assert!(content.contains("shared-first"));
        assert!(!content.contains("shared-second"));
        assert!(!content.contains("<<<<<<<"));
        assert_clean(repo.path());
    }

    #[test]
    fn sequencer_rejects_non_ancestor_base_before_apply() {
        let repo = TempRepo::new("base-not-ancestor");
        write_fixture(repo.path(), "old-alpha", "old-beta");
        git(repo.path(), ["add", "."]);
        git(repo.path(), ["commit", "-m", "base", "--no-gpg-sign"]);
        let base = git(repo.path(), ["rev-parse", "HEAD"]);
        let proposal = proposal_from_base(repo.path(), &base, "patch-a", "node-a", |content| {
            content.replace("old-alpha", "new-alpha")
        });

        git(repo.path(), ["checkout", "--orphan", "other-root"]);
        write_fixture(repo.path(), "other-alpha", "other-beta");
        git(repo.path(), ["add", "."]);
        git(repo.path(), ["commit", "-m", "other base", "--no-gpg-sign"]);

        let sequencer = PatchSequencer::new(repo.path());
        let receipt = sequencer.apply_proposal(&proposal).unwrap();

        assert_eq!(receipt.status, PatchApplyStatus::BaseNotAncestor);
        assert!(receipt.should_reopen_node());
        assert_clean(repo.path());
    }

    struct TempRepo {
        path: PathBuf,
    }

    impl TempRepo {
        fn new(name: &str) -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "theorem-harness-patch-sequencer-{name}-{}-{stamp}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            git(&path, ["init"]);
            git(&path, ["config", "user.email", "sequencer@example.test"]);
            git(&path, ["config", "user.name", "Patch Sequencer Test"]);
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_fixture(repo_path: &Path, alpha: &str, beta: &str) {
        fs::write(
            repo_path.join("router.rs"),
            format!(
                "fn alpha() -> &'static str {{\n    \"{alpha}\"\n}}\n\nfn beta() -> &'static str {{\n    \"{beta}\"\n}}\n"
            ),
        )
        .unwrap();
    }

    fn proposal_from_base<F>(
        repo_path: &Path,
        base: &str,
        patch_id: &str,
        node_id: &str,
        rewrite: F,
    ) -> PatchProposal
    where
        F: FnOnce(String) -> String,
    {
        git(repo_path, ["reset", "--hard", base]);
        let file_path = repo_path.join("router.rs");
        let content = fs::read_to_string(&file_path).unwrap();
        fs::write(&file_path, rewrite(content)).unwrap();
        let diff = git(repo_path, ["diff", "--binary"]);
        git(repo_path, ["reset", "--hard", base]);
        PatchProposal::new(patch_id, node_id, "test-head", base, diff)
            .with_file_scope(vec!["router.rs".to_string()])
    }

    fn assert_clean(repo_path: &Path) {
        let status = git(repo_path, ["status", "--porcelain=v1"]);
        assert!(status.trim().is_empty(), "{status}");
    }

    fn git<const N: usize>(repo_path: &Path, args: [&str; N]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed\nstdout={}\nstderr={}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }
}
