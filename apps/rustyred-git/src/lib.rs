//! rustyred-git: W2, git-as-truth.
//!
//! `WorkspaceRepo` is a thin, real git VCS over a working-tree directory:
//! `init`, `commit_all`, `head`, `read_file_at_head`, `create_branch`,
//! `current_branch`. It is what gives the materialized working tree (W0's
//! DocTree, W3's materialize-to-run) a git history GitHub will accept;
//! graph-version (already built) versions the knowledge graph, this versions
//! the code. Two version-control systems for two kinds of state.
//!
//! Backend: `WorkspaceRepo` drives the installed `git` CLI for the full local
//! VCS surface, while `GixWorkspaceRepo` carries the pure-Rust no-git path as it
//! lands behind the same domain API. No backend trait yet: the API is still
//! small enough that the split can stay explicit until checkout/merge/push move
//! to gix as well.
//!
//! With disk available again, this crate also exposes [`GixWorkspaceRepo`]: a
//! pure-Rust `gix` slice for init/open/head/current-branch/read-at-HEAD/clone
//! plus materialized-tree commit-all, force branch checkout, and local bare-repo
//! push by preparing a ref update/object closure, encoding that closure into a
//! verified packfile and receive-pack request body, copying the closure, and
//! updating the remote ref. It also sends the verified receive-pack body through
//! a real local receive-pack transport and a mock-proven authenticated smart
//! HTTP receive-pack path. It performs pure-gix merge/conflict surfacing for
//! divergent branches and tree-level diff for branches/worktrees, plus plain
//! branch snapshots for parallel agent isolation. The pure-gix path also reports
//! materialized-worktree status for commit decisions and true staged/unstaged/
//! untracked index status. The ignored live GitHub push/PR smoke is green when
//! run with a configured remote and token.
//!
//! Plan: docs/plans/rustyred-code-workspace/W2-git-as-truth.md

use std::collections::HashSet;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const GITHUB_API_VERSION: &str = "2026-03-10";

/// Errors from a git operation.
#[derive(Debug)]
pub enum GitError {
    /// Local filesystem I/O failed.
    Io(std::io::Error),
    /// The `git` binary could not be spawned (not installed, or no permission).
    Spawn(std::io::Error),
    /// `git` ran but exited non-zero.
    Command {
        args: Vec<String>,
        code: Option<i32>,
        stderr: String,
    },
    /// The pure-Rust `gix` backend surfaced an error.
    Gix(String),
    /// The GitHub REST remote surfaced an error.
    GitHub(String),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::Io(error) => write!(f, "io: {error}"),
            GitError::Spawn(error) => write!(f, "spawning git: {error}"),
            GitError::Command { args, code, stderr } => {
                write!(f, "git {args:?} exited {code:?}: {stderr}")
            }
            GitError::Gix(error) => write!(f, "gix: {error}"),
            GitError::GitHub(error) => write!(f, "github: {error}"),
        }
    }
}

impl std::error::Error for GitError {}

/// The result of merging one branch into the current branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    /// A clean merge (or fast-forward); the resulting HEAD commit hash.
    Merged(String),
    /// The merge left conflicts in these paths (the tree is mid-merge; the
    /// caller resolves and commits, or aborts). Code is merged through git, so
    /// a conflict is surfaced here, never silently auto-resolved.
    Conflict(Vec<String>),
}

/// Receipt from a pure-gix local bare-repo push.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GixPushReceipt {
    /// Branch pushed, without the `refs/heads/` prefix.
    pub branch: String,
    /// The previous remote branch tip, if the branch already existed.
    pub old_head: Option<String>,
    /// The new remote branch tip.
    pub new_head: String,
    /// Number of unique objects walked and copied/confirmed in the remote ODB.
    pub objects_copied: usize,
}

/// Receipt from sending a prepared push through a real receive-pack transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GixReceivePackPushReceipt {
    /// Branch pushed, without the `refs/heads/` prefix.
    pub branch: String,
    /// Fully-qualified remote ref that receive-pack updated.
    pub remote_ref: String,
    /// The previous remote branch tip advertised by receive-pack, if present.
    pub old_head: Option<String>,
    /// The new remote branch tip.
    pub new_head: String,
    /// Capabilities advertised by receive-pack.
    pub advertised_capabilities: Vec<String>,
    /// Capabilities this sender requested on the first command line.
    pub requested_capabilities: Vec<String>,
    /// Number of objects encoded into the transmitted packfile.
    pub objects_sent: usize,
    /// Status lines returned by receive-pack, e.g. `unpack ok`, `ok refs/heads/main`.
    pub status: Vec<String>,
}

/// Receipt from sending a prepared push through smart HTTP receive-pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GixHttpPushReceipt {
    /// Smart-HTTP git remote URL, e.g. `https://github.com/owner/repo.git`.
    pub remote_url: String,
    /// Branch pushed, without the `refs/heads/` prefix.
    pub branch: String,
    /// Fully-qualified remote ref that receive-pack updated.
    pub remote_ref: String,
    /// The previous remote branch tip advertised by receive-pack, if present.
    pub old_head: Option<String>,
    /// The new local branch tip.
    pub new_head: String,
    /// Capabilities advertised by receive-pack.
    pub advertised_capabilities: Vec<String>,
    /// Capabilities this sender requested on the first command line.
    pub requested_capabilities: Vec<String>,
    /// Number of objects encoded into the transmitted packfile.
    pub objects_sent: usize,
    /// Status lines returned by receive-pack, e.g. `unpack ok`, `ok refs/heads/main`.
    pub status: Vec<String>,
}

/// Push-ready ref update, object closure, and packfile for a branch.
///
/// This is the offline-verifiable half of send-pack: the branch refspec, the
/// expected remote old value, the new local tip, and the exact object ids that a
/// remote without the new tip needs, encoded as a verified packfile. A live
/// protocol sender can use [`GixPreparedPush::receive_pack_request`] as the
/// request body; the local bare push path consumes the same object closure to
/// keep the proof honest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GixPreparedPush {
    /// Branch pushed, without the `refs/heads/` prefix.
    pub branch: String,
    /// Fully-qualified remote ref to update, e.g. `refs/heads/main`.
    pub remote_ref: String,
    /// Push refspec, e.g. `refs/heads/main:refs/heads/main`.
    pub refspec: String,
    /// The expected remote branch tip, if the remote branch already exists.
    pub old_head: Option<String>,
    /// The new local branch tip.
    pub new_head: String,
    /// Object ids that must be sent to let the remote accept `new_head`.
    pub objects_to_send: Vec<String>,
    /// Packfile containing exactly `objects_to_send`, ready for send-pack's pack
    /// body once the protocol negotiation/ref update has been written.
    pub packfile: GixPushPackfile,
}

/// In-memory packfile prepared for a push.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GixPushPackfile {
    /// Raw git packfile bytes (`PACK` header, encoded objects, trailing hash).
    pub bytes: Vec<u8>,
    /// Hex checksum/trailer of `bytes`.
    pub checksum: String,
    /// Number of objects encoded in `bytes`.
    pub object_count: usize,
}

/// A receive-pack request body ready for a smart-git push transport.
///
/// The request contains the packet-line command/ref update, a flush packet, and
/// the raw packfile bytes. The actual socket/HTTP/SSH transport is deliberately
/// outside this receipt so it can be live-tested only when a remote and token
/// exist, while the wire body remains locally verifiable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GixReceivePackRequest {
    /// Remote ref being updated, e.g. `refs/heads/main`.
    pub remote_ref: String,
    /// Expected remote head sent in the receive-pack command line.
    pub old_head: String,
    /// New local head sent in the receive-pack command line.
    pub new_head: String,
    /// Capabilities requested on the first command line.
    pub capabilities: Vec<String>,
    /// Complete request body: first command pkt-line, flush pkt-line, raw pack.
    pub bytes: Vec<u8>,
    /// Byte offset where the raw `PACK` bytes begin.
    pub packfile_offset: usize,
}

impl GixPreparedPush {
    /// Encode this prepared push as a receive-pack request body.
    ///
    /// This is the last offline-verifiable step before a live network push: it
    /// binds the fast-forward-checked ref update to the exact verified packfile
    /// already prepared from the object closure. A live transport should derive
    /// `capabilities` from the server advertisement; tests pass an explicit list
    /// to avoid pretending we negotiated with a server.
    pub fn receive_pack_request<S: AsRef<str>>(
        &self,
        capabilities: &[S],
    ) -> Result<GixReceivePackRequest, GitError> {
        let old_head = self
            .old_head
            .clone()
            .unwrap_or_else(|| "0".repeat(self.new_head.len()));
        let capabilities = capabilities
            .iter()
            .map(|capability| capability.as_ref().to_string())
            .collect::<Vec<_>>();

        let mut first_line = format!("{} {} {}", old_head, self.new_head, self.remote_ref);
        if !capabilities.is_empty() {
            first_line.push('\0');
            first_line.push_str(&capabilities.join(" "));
        }
        first_line.push('\n');

        let mut bytes = Vec::new();
        gix::protocol::transport::packetline::blocking_io::encode::write_packet_line(
            &gix::protocol::transport::packetline::PacketLineRef::Data(first_line.as_bytes()),
            &mut bytes,
        )
        .map_err(GitError::Io)?;
        gix::protocol::transport::packetline::blocking_io::encode::flush_to_write(&mut bytes)
            .map_err(GitError::Io)?;
        let packfile_offset = bytes.len();
        bytes
            .write_all(&self.packfile.bytes)
            .map_err(GitError::Io)?;

        Ok(GixReceivePackRequest {
            remote_ref: self.remote_ref.clone(),
            old_head,
            new_head: self.new_head.clone(),
            capabilities,
            bytes,
            packfile_offset,
        })
    }
}

/// A coarse file-level diff status for W2 porcelain.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiffStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
}

/// One repository-relative path changed between two committed/materialized trees.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiffEntry {
    /// File-level change kind.
    pub status: DiffStatus,
    /// Current/new path for additions, modifications, renames, and copies; old
    /// path for deletions.
    pub path: String,
    /// Previous path for renames/copies.
    pub old_path: Option<String>,
}

/// Pure-gix status receipt for the materialized worktree.
///
/// This is the W2-generated-worktree porcelain surface: it compares the files W0
/// and W3 materialized on disk to `HEAD` without creating a commit or moving a
/// ref. It is intentionally commit-candidate status, not full staged/unstaged
/// index porcelain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GixWorktreeStatus {
    /// Current branch name, or `None` for detached HEAD.
    pub current_branch: Option<String>,
    /// Current HEAD commit hash, or `None` on an unborn branch.
    pub head: Option<String>,
    /// File-level changes in repository-relative path order.
    pub changes: Vec<DiffEntry>,
}

impl GixWorktreeStatus {
    /// Whether the materialized worktree matches `HEAD` exactly.
    pub fn is_clean(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Pure-gix status receipt split across HEAD, index, and worktree.
///
/// `staged` is the diff from `HEAD` to the index. `unstaged` is tracked-file
/// drift from the index to the worktree. `untracked` lists source files present
/// on disk with no index entry. This is the richer git-porcelain surface for
/// flows that need staged/unstaged separation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GixIndexStatus {
    /// Current branch name, or `None` for detached HEAD.
    pub current_branch: Option<String>,
    /// Current HEAD commit hash, or `None` on an unborn branch.
    pub head: Option<String>,
    /// Changes staged in the git index relative to HEAD.
    pub staged: Vec<DiffEntry>,
    /// Tracked worktree changes not staged in the git index.
    pub unstaged: Vec<DiffEntry>,
    /// Worktree paths not tracked by the git index.
    pub untracked: Vec<String>,
}

impl GixIndexStatus {
    /// Whether HEAD, index, and worktree all agree and no untracked files exist.
    pub fn is_clean(&self) -> bool {
        self.staged.is_empty() && self.unstaged.is_empty() && self.untracked.is_empty()
    }
}

/// Run `git -C <root> <args>`, returning stdout bytes on success.
fn run_git_in(root: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(GitError::Spawn)?;
    if !output.status.success() {
        return Err(GitError::Command {
            args: args.iter().map(|s| (*s).to_string()).collect(),
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(output.stdout)
}

fn gix_error(error: impl std::fmt::Display) -> GitError {
    GitError::Gix(error.to_string())
}

fn github_error(error: impl std::fmt::Display) -> GitError {
    GitError::GitHub(error.to_string())
}

fn git_smart_http_authorization(token: &str) -> String {
    use base64::Engine as _;

    let credentials = format!("x-access-token:{token}");
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(credentials.as_bytes())
    )
}

fn git_signature() -> gix::actor::SignatureRef<'static> {
    use gix::bstr::ByteSlice;

    gix::actor::SignatureRef {
        name: b"RustyRed Workspace".as_bstr(),
        email: b"workspace@rustyred.local".as_bstr(),
        time: "0 +0000",
    }
}

fn branch_ref_name(name: &str) -> Result<gix::refs::FullName, GitError> {
    use gix::bstr::ByteSlice;

    let full_name = gix::refs::Category::LocalBranch
        .to_full_name(name.as_bytes().as_bstr())
        .map_err(gix_error)?;
    gix::validate::reference::branch_name(full_name.as_bstr()).map_err(gix_error)?;
    Ok(full_name)
}

/// A real git repository over a working-tree directory.
#[derive(Clone, Debug)]
pub struct WorkspaceRepo {
    root: PathBuf,
}

impl WorkspaceRepo {
    /// Open an existing repo rooted at `dir` (no validation beyond the path; git
    /// commands will surface "not a git repository" if it is not one).
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self { root: dir.into() }
    }

    /// The working-tree root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Initialize a new repository at `dir` (creating it if needed) on branch
    /// `main`, with a local commit identity so commits work without any global
    /// git config (CI, fresh containers).
    pub fn init(dir: impl Into<PathBuf>) -> Result<Self, GitError> {
        let root = dir.into();
        std::fs::create_dir_all(&root).map_err(GitError::Io)?;
        run_git_in(&root, &["init", "--quiet", "-b", "main"])?;
        // Local identity (repo-scoped, not global) so `commit` never fails on an
        // unconfigured host.
        run_git_in(&root, &["config", "user.email", "workspace@rustyred.local"])?;
        run_git_in(&root, &["config", "user.name", "RustyRed Workspace"])?;
        Ok(Self { root })
    }

    /// Stage every change and commit it. Returns the new commit hash.
    pub fn commit_all(&self, message: &str) -> Result<String, GitError> {
        run_git_in(&self.root, &["add", "-A"])?;
        run_git_in(&self.root, &["commit", "--quiet", "-m", message])?;
        // head() is Some right after a successful commit.
        Ok(self.head()?.unwrap_or_default())
    }

    /// The current HEAD commit hash, or `None` on an unborn branch (no commits
    /// yet). A failed `rev-parse HEAD` is the unborn signal, not an error.
    pub fn head(&self) -> Result<Option<String>, GitError> {
        match run_git_in(&self.root, &["rev-parse", "HEAD"]) {
            Ok(out) => Ok(Some(String::from_utf8_lossy(&out).trim().to_string())),
            // ponytail: an unborn HEAD makes `rev-parse HEAD` exit non-zero; we
            // read that specific failure as None. Spawn failures still propagate.
            Err(GitError::Command { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Read a path's bytes from the COMMITTED tree at HEAD (`git show HEAD:path`),
    /// not the working directory. `None` if the path is not in the commit. This
    /// is the read-back that proves a commit actually captured a file.
    pub fn read_file_at_head(&self, rel: &str) -> Result<Option<Vec<u8>>, GitError> {
        match run_git_in(&self.root, &["show", &format!("HEAD:{rel}")]) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(GitError::Command { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Create and switch to a new branch off the current HEAD.
    pub fn create_branch(&self, name: &str) -> Result<(), GitError> {
        run_git_in(&self.root, &["checkout", "--quiet", "-b", name])?;
        Ok(())
    }

    /// Switch to an existing branch.
    pub fn checkout(&self, name: &str) -> Result<(), GitError> {
        run_git_in(&self.root, &["checkout", "--quiet", name])?;
        Ok(())
    }

    /// The current branch name.
    pub fn current_branch(&self) -> Result<String, GitError> {
        let out = run_git_in(&self.root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    /// Merge `branch` into the current branch (a real three-way merge). A clean
    /// merge auto-commits and returns [`MergeOutcome::Merged`]; conflicting edits
    /// return [`MergeOutcome::Conflict`] with the conflicted paths (the tree is
    /// left mid-merge for the caller to resolve). Any other failure (e.g.
    /// unrelated histories) is a real error.
    pub fn merge(&self, branch: &str) -> Result<MergeOutcome, GitError> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["merge", "--no-edit", branch])
            .output()
            .map_err(GitError::Spawn)?;
        if output.status.success() {
            return Ok(MergeOutcome::Merged(self.head()?.unwrap_or_default()));
        }
        // Non-zero exit: a conflict lists unmerged paths; anything else is a
        // genuine failure, not a conflict.
        let conflicts = self.conflicted_paths()?;
        if conflicts.is_empty() {
            return Err(GitError::Command {
                args: vec!["merge".to_string(), branch.to_string()],
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        Ok(MergeOutcome::Conflict(conflicts))
    }

    /// Paths currently in a conflicted (unmerged) state. `git diff` exits 0 even
    /// mid-conflict, so this is safe to call after a failed merge.
    pub fn conflicted_paths(&self) -> Result<Vec<String>, GitError> {
        let out = run_git_in(&self.root, &["diff", "--name-only", "--diff-filter=U"])?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    }

    /// Abort an in-progress merge, restoring the pre-merge HEAD.
    pub fn merge_abort(&self) -> Result<(), GitError> {
        run_git_in(&self.root, &["merge", "--abort"])?;
        Ok(())
    }

    /// Register a remote (a URL or a local filesystem path, for a local
    /// bare-repo round-trip).
    pub fn add_remote(&self, name: &str, url: &str) -> Result<(), GitError> {
        run_git_in(&self.root, &["remote", "add", name, url])?;
        Ok(())
    }

    /// Push `branch` to `remote`.
    pub fn push(&self, remote: &str, branch: &str) -> Result<(), GitError> {
        run_git_in(&self.root, &["push", "--quiet", remote, branch])?;
        Ok(())
    }

    /// Clone `url` (a URL or local path) into `into`, returning the new repo.
    pub fn clone(url: &str, into: impl Into<PathBuf>) -> Result<Self, GitError> {
        let into = into.into();
        let output = Command::new("git")
            .args(["clone", "--quiet", url])
            .arg(&into)
            .output()
            .map_err(GitError::Spawn)?;
        if !output.status.success() {
            return Err(GitError::Command {
                args: vec!["clone".to_string(), url.to_string()],
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        Ok(Self { root: into })
    }
}

/// A pure-Rust `gix` repository over a working-tree directory.
///
/// This is the W2 gix backend slice that does not shell out for reads, HEAD
/// inspection, initialization, clone, materialized-tree commits, or force branch
/// checkout. It can also push by copying the prepared object closure into a
/// local bare repository, by sending the verified request body through local
/// receive-pack, and by using smart HTTP with injected token auth. It performs
/// clean merges/conflict surfacing and tree-level diff without the git CLI. It
/// also materializes committed branch snapshots for parallel agent isolation.
/// The ignored live GitHub remote/token smoke proves branch push plus draft PR
/// creation against a real GitHub remote.
#[derive(Clone, Debug)]
pub struct GixWorkspaceRepo {
    root: PathBuf,
}

impl GixWorkspaceRepo {
    /// Open an existing repository rooted at `dir`.
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self { root: dir.into() }
    }

    /// The working-tree root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Initialize a repository using the pure-Rust `gix` backend.
    pub fn init(dir: impl Into<PathBuf>) -> Result<Self, GitError> {
        let root = dir.into();
        std::fs::create_dir_all(&root).map_err(GitError::Io)?;
        gix::init(&root).map_err(gix_error)?;
        Ok(Self { root })
    }

    /// Write the whole materialized worktree (excluding `.git`) into git
    /// objects and create a commit on `HEAD` using the pure-Rust `gix` backend.
    ///
    /// This is intentionally narrower than `git add -A`: it treats the
    /// materialized directory as generated source state, writes a fresh tree
    /// directly, and advances the current branch. Checkout, index-conflict
    /// semantics, merge, and push remain separate W2 follow-up primitives.
    pub fn commit_all_from_worktree(&self, message: &str) -> Result<String, GitError> {
        let repo = gix::open(&self.root).map_err(gix_error)?;
        let tree_id = write_worktree_tree(&repo, &self.root)?;
        let parents = match repo.head().map_err(gix_error)? {
            head if head.is_unborn() => Vec::new(),
            head => vec![head.into_peeled_id().map_err(gix_error)?.detach()],
        };
        let signature = git_signature();
        let commit = repo
            .commit_as(signature, signature, "HEAD", message, tree_id, parents)
            .map_err(gix_error)?;
        write_index_for_tree(&repo, tree_id)?;
        Ok(commit.to_string())
    }

    /// Create and switch to a new branch off the current HEAD using `gix`.
    pub fn create_branch(&self, name: &str) -> Result<(), GitError> {
        let repo = gix::open(&self.root).map_err(gix_error)?;
        let head_id = repo
            .head()
            .map_err(gix_error)?
            .into_peeled_id()
            .map_err(gix_error)?
            .detach();
        let branch_ref = branch_ref_name(name)?;
        repo.reference(
            branch_ref.clone(),
            head_id,
            gix::refs::transaction::PreviousValue::MustNotExist,
            format!("branch: Created from HEAD for RustyRed workspace: {name}"),
        )
        .map_err(gix_error)?;
        checkout_branch_force(&repo, branch_ref)
    }

    /// Force-checkout an existing branch using `gix`.
    ///
    /// This treats the worktree as W0/W3 generated materialized state: it clears
    /// tracked/untracked files outside `.git`, swings `HEAD` to the branch, and
    /// writes the branch tree back to disk. Dirty-worktree preservation and
    /// porcelain prompts are intentionally not modeled here.
    pub fn checkout(&self, name: &str) -> Result<(), GitError> {
        let repo = gix::open(&self.root).map_err(gix_error)?;
        let branch_ref = branch_ref_name(name)?;
        repo.find_reference(branch_ref.as_bstr())
            .map_err(gix_error)?;
        checkout_branch_force(&repo, branch_ref)
    }

    /// Prepare the ref update, object closure, and packfile for pushing `branch`.
    ///
    /// `remote_old_head` is the value advertised by the remote branch, if it
    /// exists. If it is not an ancestor of the local branch tip, this rejects the
    /// push as non-fast-forward before any network or remote mutation happens.
    pub fn prepare_push_ref_update(
        &self,
        branch: &str,
        remote_old_head: Option<&str>,
    ) -> Result<GixPreparedPush, GitError> {
        let repo = gix::open(&self.root).map_err(gix_error)?;
        let old_head = remote_old_head
            .map(|head| gix::ObjectId::from_hex(head.as_bytes()).map_err(gix_error))
            .transpose()?;
        let (prepared, _) = prepare_push_for_repo(&repo, branch, old_head)?;
        Ok(prepared)
    }

    /// Prepare the receive-pack wire body for pushing `branch`.
    ///
    /// This is transport-agnostic: the remote advertisement supplies
    /// `remote_old_head` and the negotiated `capabilities`, while this method
    /// performs the local safety/object/packfile work and returns the bytes a
    /// smart-git sender writes after selecting `git-receive-pack`.
    pub fn prepare_receive_pack_request<S: AsRef<str>>(
        &self,
        branch: &str,
        remote_old_head: Option<&str>,
        capabilities: &[S],
    ) -> Result<GixReceivePackRequest, GitError> {
        self.prepare_push_ref_update(branch, remote_old_head)?
            .receive_pack_request(capabilities)
    }

    /// Push `branch` to a local bare repository through a real receive-pack
    /// transport.
    ///
    /// Unlike [`push_to_local_bare`](Self::push_to_local_bare), this drives the
    /// smart-git server path: it handshakes with `git-receive-pack`, derives the
    /// advertised old head and capabilities, writes the prepared receive-pack
    /// request body, and requires the server's `report-status` confirmation.
    /// This proves the W2 wire artifact against an actual transport while the
    /// GitHub/network smoke remains separately token-gated.
    pub fn push_to_local_receive_pack(
        &self,
        remote_dir: impl AsRef<Path>,
        branch: &str,
    ) -> Result<GixReceivePackPushReceipt, GitError> {
        use gix::bstr::ByteSlice;
        use gix::protocol::transport::{
            client::{
                blocking_io::{file, ReadlineBufRead, Transport},
                MessageKind, WriteMode,
            },
            packetline::PacketLineRef,
            Protocol, Service,
        };

        let local = gix::open(&self.root).map_err(gix_error)?;
        let remote_path = remote_dir.as_ref();
        let remote_path_bstr = gix::path::into_bstr(remote_path).into_owned();
        let mut transport =
            file::connect(remote_path_bstr, Protocol::V1, false).map_err(|never| match never {})?;
        let branch_ref = branch_ref_name(branch)?;
        let remote_ref = branch_ref.as_bstr().to_str_lossy().into_owned();
        let (advertised_capabilities, old_head) = {
            let mut handshake = transport
                .handshake(Service::ReceivePack, &[])
                .map_err(gix_error)?;
            let advertised_capabilities = transport_capabilities(&handshake.capabilities);
            if !handshake.capabilities.contains("report-status") {
                return Err(GitError::Gix(
                    "receive-pack did not advertise report-status".to_string(),
                ));
            }
            let old_head = read_advertised_receive_pack_head(handshake.refs.take(), &remote_ref)?;
            (advertised_capabilities, old_head)
        };

        let (prepared, _) = prepare_push_for_repo(&local, branch, old_head)?;
        let requested_capabilities = vec!["report-status".to_string()];
        let request = prepared.receive_pack_request(&requested_capabilities)?;
        let receipt_old_head = prepared.old_head.clone();
        let objects_sent = prepared.packfile.object_count;
        let writer = transport
            .request(WriteMode::Binary, MessageKind::Flush, false)
            .map_err(gix_error)?;
        let (mut raw_writer, mut reader) = writer.into_parts();
        raw_writer.write_all(&request.bytes).map_err(GitError::Io)?;
        raw_writer.flush().map_err(GitError::Io)?;
        drop(raw_writer);

        let mut status = Vec::new();
        while let Some(line) = reader.readline() {
            let line = line.map_err(GitError::Io)?.map_err(gix_error)?;
            if let PacketLineRef::Data(data) = line {
                status.push(data.as_bstr().to_str_lossy().trim_end().to_string());
            }
        }
        ensure_receive_pack_status(&status, &prepared.remote_ref)?;

        Ok(GixReceivePackPushReceipt {
            branch: branch.to_string(),
            remote_ref,
            old_head: receipt_old_head,
            new_head: prepared.new_head,
            advertised_capabilities,
            requested_capabilities,
            objects_sent,
            status,
        })
    }

    /// Push `branch` through the smart HTTP `git-receive-pack` protocol.
    ///
    /// Authentication is injected as GitHub-compatible Basic auth
    /// (`x-access-token:TOKEN`) so the existing GitHub App installation-token
    /// resolver can remain the credential source outside this crate. The live
    /// GitHub smoke is still token/remote gated, but this method is mockable end
    /// to end: advertise refs, prepare the checked update, POST the raw
    /// receive-pack request bytes, and require `report-status`.
    pub fn push_to_http_receive_pack(
        &self,
        remote_url: &str,
        token: &str,
        branch: &str,
    ) -> Result<GixHttpPushReceipt, GitError> {
        let local = gix::open(&self.root).map_err(gix_error)?;
        let remote_url = remote_url.trim_end_matches('/').to_string();
        let branch_ref = branch_ref_name(branch)?;
        let remote_ref = {
            use gix::bstr::ByteSlice;
            branch_ref.as_bstr().to_str_lossy().into_owned()
        };
        let http = reqwest::blocking::Client::new();
        let advertise_url = format!("{remote_url}/info/refs?service=git-receive-pack");
        let advertise = http
            .get(&advertise_url)
            .header("Accept", "application/x-git-receive-pack-advertisement")
            .header("Authorization", git_smart_http_authorization(token))
            .header("Connection", "close")
            .header("User-Agent", "rustyred-git")
            .send()
            .map_err(github_error)?;
        let advertise_status = advertise.status();
        if !advertise_status.is_success() {
            let body = advertise.text().unwrap_or_default();
            return Err(GitError::GitHub(format!(
                "receive-pack advertise failed with {advertise_status}: {body}"
            )));
        }
        let advertisement = advertise.bytes().map_err(github_error)?;
        let advertised = parse_smart_http_receive_pack_advertisement(&advertisement, &remote_ref)?;
        if !advertised
            .capabilities
            .iter()
            .any(|capability| capability == "report-status")
        {
            return Err(GitError::Gix(
                "receive-pack did not advertise report-status".to_string(),
            ));
        }

        let (prepared, _) = prepare_push_for_repo_with_known_remote(
            &local,
            branch,
            advertised.old_head,
            &advertised.heads,
        )?;
        let requested_capabilities = vec!["report-status".to_string()];
        let request = prepared.receive_pack_request(&requested_capabilities)?;
        let receipt_old_head = prepared.old_head.clone();
        let objects_sent = prepared.packfile.object_count;
        let receive_pack_url = format!("{remote_url}/git-receive-pack");
        let response = http
            .post(&receive_pack_url)
            .header("Accept", "application/x-git-receive-pack-result")
            .header("Authorization", git_smart_http_authorization(token))
            .header("Connection", "close")
            .header("Content-Type", "application/x-git-receive-pack-request")
            .header("User-Agent", "rustyred-git")
            .body(request.bytes)
            .send()
            .map_err(github_error)?;
        let status_code = response.status();
        if !status_code.is_success() {
            let body = response.text().unwrap_or_default();
            return Err(GitError::GitHub(format!(
                "receive-pack push failed with {status_code}: {body}"
            )));
        }
        let response_body = response.bytes().map_err(github_error)?;
        let status = parse_receive_pack_status(&response_body)?;
        ensure_receive_pack_status(&status, &prepared.remote_ref)?;

        Ok(GixHttpPushReceipt {
            remote_url,
            branch: branch.to_string(),
            remote_ref,
            old_head: receipt_old_head,
            new_head: prepared.new_head,
            advertised_capabilities: advertised.capabilities,
            requested_capabilities,
            objects_sent,
            status,
        })
    }

    /// Push `branch` to a local bare repository without invoking the git CLI.
    ///
    /// This is not the network transport-send path. It is the filesystem-remote W2
    /// slice: prepare the same ref update/object closure and packfile a protocol
    /// sender would use, copy those objects into the bare repository's object
    /// database, and fast-forward/update the destination branch ref. Live
    /// GitHub/HTTP/SSH push remains a separate protocol slice.
    pub fn push_to_local_bare(
        &self,
        remote_dir: impl AsRef<Path>,
        branch: &str,
    ) -> Result<GixPushReceipt, GitError> {
        let local = gix::open(&self.root).map_err(gix_error)?;
        let remote = gix::open(remote_dir.as_ref()).map_err(gix_error)?;
        if remote.workdir().is_some() {
            return Err(GitError::Gix(
                "push_to_local_bare requires a bare repository".to_string(),
            ));
        }

        let branch_ref = branch_ref_name(branch)?;
        let mut local_ref = local
            .find_reference(branch_ref.as_bstr())
            .map_err(gix_error)?;
        let new_head = local_ref.peel_to_id().map_err(gix_error)?.detach();
        let old_head = remote
            .try_find_reference(branch_ref.as_bstr())
            .map_err(gix_error)?
            .map(|mut reference| reference.peel_to_id().map(|id| id.detach()))
            .transpose()
            .map_err(gix_error)?;

        let (prepared, object_ids) = prepare_push_for_repo(&local, branch, old_head)?;
        let objects_copied = copy_selected_objects(&local, &remote, &object_ids)?;
        remote
            .reference(
                branch_ref,
                new_head,
                gix::refs::transaction::PreviousValue::Any,
                format!("push: RustyRed workspace updated {branch}"),
            )
            .map_err(gix_error)?;

        Ok(GixPushReceipt {
            branch: branch.to_string(),
            old_head: prepared.old_head,
            new_head: prepared.new_head,
            objects_copied,
        })
    }

    /// Merge `branch` into the current branch using only `gix`.
    ///
    /// A clean divergent merge creates a two-parent merge commit on `HEAD` and
    /// force-materializes the merged tree. If git would consider the merge
    /// unresolved, HEAD and the worktree are left unchanged and the conflicting
    /// repository-relative paths are returned.
    pub fn merge(&self, branch: &str) -> Result<MergeOutcome, GitError> {
        use gix::bstr::ByteSlice;

        let repo = gix::open(&self.root).map_err(gix_error)?;
        let ours = repo
            .head()
            .map_err(gix_error)?
            .into_peeled_id()
            .map_err(gix_error)?
            .detach();
        let branch_ref = branch_ref_name(branch)?;
        let mut their_ref = repo
            .find_reference(branch_ref.as_bstr())
            .map_err(gix_error)?;
        let theirs = their_ref.peel_to_id().map_err(gix_error)?.detach();

        if commit_reaches(&repo, ours, theirs)? {
            return Ok(MergeOutcome::Merged(ours.to_string()));
        }
        if commit_reaches(&repo, theirs, ours)? {
            fast_forward_head(&repo, theirs)?;
            return Ok(MergeOutcome::Merged(theirs.to_string()));
        }

        let labels = gix::merge::blob::builtin_driver::text::Labels {
            ancestor: None,
            current: Some(b"HEAD".as_bstr()),
            other: Some(branch.as_bytes().as_bstr()),
        };
        let mut outcome = repo
            .merge_commits(
                ours,
                theirs,
                labels,
                repo.tree_merge_options().map_err(gix_error)?.into(),
            )
            .map_err(gix_error)?;
        let unresolved = gix::merge::tree::TreatAsUnresolved::git();
        if outcome.tree_merge.has_unresolved_conflicts(unresolved) {
            return Ok(MergeOutcome::Conflict(gix_conflict_paths(
                &outcome.tree_merge.conflicts,
                unresolved,
            )));
        }

        let tree_id = outcome.tree_merge.tree.write().map_err(gix_error)?.detach();
        let signature = git_signature();
        let commit = repo
            .commit_as(
                signature,
                signature,
                "HEAD",
                format!("Merge branch '{branch}'"),
                tree_id,
                vec![ours, theirs],
            )
            .map_err(gix_error)?;
        checkout_head_tree_force(&repo)?;
        Ok(MergeOutcome::Merged(commit.to_string()))
    }

    /// Diff two committed branch tips using `gix` tree-to-tree diff.
    ///
    /// The returned entries describe the changes needed to turn `base` into
    /// `head`. This is the branch-review primitive the push/PR path will use.
    pub fn diff_branches(&self, base: &str, head: &str) -> Result<Vec<DiffEntry>, GitError> {
        let repo = gix::open(&self.root).map_err(gix_error)?;
        let base_tree = branch_tree(&repo, base)?;
        let head_tree = branch_tree(&repo, head)?;
        diff_trees(&base_tree, &head_tree)
    }

    /// Diff the current materialized worktree against the committed `HEAD`.
    ///
    /// This writes temporary blob/tree objects for the materialized files, but it
    /// does not create a commit or move any ref. It is a cheap porcelain check for
    /// W3's run/sync loop before deciding whether to commit.
    pub fn diff_worktree_to_head(&self) -> Result<Vec<DiffEntry>, GitError> {
        let repo = gix::open(&self.root).map_err(gix_error)?;
        let head_tree = repo
            .head_commit()
            .map_err(gix_error)?
            .tree()
            .map_err(gix_error)?;
        let worktree_tree_id = write_worktree_tree(&repo, &self.root)?;
        let worktree_tree = repo.find_tree(worktree_tree_id).map_err(gix_error)?;
        diff_trees(&head_tree, &worktree_tree)
    }

    /// Report materialized-worktree status for W2 commit decisions.
    ///
    /// The status is a commit-candidate receipt over the W0/W3 materialized
    /// source tree. It does not distinguish staged from unstaged changes because
    /// this gix backend commits generated source state by writing a fresh tree.
    pub fn worktree_status(&self) -> Result<GixWorktreeStatus, GitError> {
        let head = self.head()?;
        let changes = if head.is_some() {
            self.diff_worktree_to_head()?
        } else {
            added_entries_from_worktree(&self.root)?
        };
        Ok(GixWorktreeStatus {
            current_branch: self.current_branch()?,
            head,
            changes,
        })
    }

    /// Stage the whole materialized worktree into the git index.
    ///
    /// Like [`commit_all_from_worktree`](Self::commit_all_from_worktree), this
    /// treats disk as generated source state and writes a fresh index from the
    /// materialized tree. It does not create a commit or move `HEAD`.
    pub fn stage_all_from_worktree(&self) -> Result<GixIndexStatus, GitError> {
        let repo = gix::open(&self.root).map_err(gix_error)?;
        let tree_id = write_worktree_tree(&repo, &self.root)?;
        write_index_for_tree(&repo, tree_id)?;
        self.index_status()
    }

    /// Report true git porcelain status split into staged, unstaged, and
    /// untracked buckets using the persisted git index.
    pub fn index_status(&self) -> Result<GixIndexStatus, GitError> {
        let repo = gix::open(&self.root).map_err(gix_error)?;
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        let mut untracked = Vec::new();
        let iter = repo
            .status(gix::progress::Discard)
            .map_err(gix_error)?
            .untracked_files(gix::status::UntrackedFiles::Files)
            .into_iter(Vec::new())
            .map_err(gix_error)?;

        for item in iter {
            match item.map_err(gix_error)? {
                gix::status::Item::TreeIndex(change) => {
                    if let Some(entry) = diff_entry_from_index_change(change) {
                        staged.push(entry);
                    }
                }
                gix::status::Item::IndexWorktree(item) => {
                    collect_index_worktree_status(item, &mut unstaged, &mut untracked);
                }
            }
        }

        staged.sort();
        unstaged.sort();
        untracked.sort();
        Ok(GixIndexStatus {
            current_branch: self.current_branch()?,
            head: self.head()?,
            staged,
            unstaged,
            untracked,
        })
    }

    /// Materialize a committed branch tree into an isolated directory.
    ///
    /// The destination is a plain source snapshot, not a second git repository:
    /// it gives a parallel agent a clean branch-shaped filesystem to inspect or
    /// edit without touching the main materialized worktree. A later W2
    /// porcelain slice can promote this into full git worktrees.
    pub fn materialize_branch_snapshot(
        &self,
        branch: &str,
        into: impl AsRef<Path>,
    ) -> Result<(), GitError> {
        let repo = gix::open(&self.root).map_err(gix_error)?;
        let tree_id = branch_tree(&repo, branch)?.id().detach();
        checkout_tree_to_dir(&repo, tree_id, into.as_ref(), false)
    }

    /// The current HEAD commit hash, or `None` on an unborn branch.
    pub fn head(&self) -> Result<Option<String>, GitError> {
        let repo = gix::open(&self.root).map_err(gix_error)?;
        let head = repo.head().map_err(gix_error)?;
        if head.is_unborn() {
            return Ok(None);
        }
        let id = head.into_peeled_id().map_err(gix_error)?;
        Ok(Some(id.to_string()))
    }

    /// The current branch name, or `None` for detached HEAD.
    pub fn current_branch(&self) -> Result<Option<String>, GitError> {
        let repo = gix::open(&self.root).map_err(gix_error)?;
        Ok(repo
            .head_name()
            .map_err(gix_error)?
            .map(|name| name.shorten().to_string()))
    }

    /// Read a path's bytes from the committed tree at HEAD.
    pub fn read_file_at_head(&self, rel: &str) -> Result<Option<Vec<u8>>, GitError> {
        let repo = gix::open(&self.root).map_err(gix_error)?;
        let commit = match repo.head_commit() {
            Ok(commit) => commit,
            Err(_) => return Ok(None),
        };
        let tree = commit.tree().map_err(gix_error)?;
        let Some(entry) = tree.lookup_entry_by_path(rel).map_err(gix_error)? else {
            return Ok(None);
        };
        let blob = entry
            .object()
            .map_err(gix_error)?
            .try_into_blob()
            .map_err(|error| gix_error(format!("{error}")))?;
        Ok(Some(blob.data.clone()))
    }

    /// Clone `url` (a URL or local path) into `into` using `gix`.
    pub fn clone(url: &str, into: impl Into<PathBuf>) -> Result<Self, GitError> {
        let into = into.into();
        let interrupt = std::sync::atomic::AtomicBool::new(false);
        let mut prepare = gix::prepare_clone(url, &into).map_err(gix_error)?;
        let (mut checkout, _) = prepare
            .fetch_then_checkout(gix::progress::Discard, &interrupt)
            .map_err(gix_error)?;
        checkout
            .main_worktree(gix::progress::Discard, &interrupt)
            .map_err(gix_error)?;
        Ok(Self { root: into })
    }
}

fn branch_tree<'repo>(
    repo: &'repo gix::Repository,
    branch: &str,
) -> Result<gix::Tree<'repo>, GitError> {
    let branch_ref = branch_ref_name(branch)?;
    repo.find_reference(branch_ref.as_bstr())
        .map_err(gix_error)?
        .peel_to_commit()
        .map_err(gix_error)?
        .tree()
        .map_err(gix_error)
}

fn transport_capabilities(
    capabilities: &gix::protocol::transport::client::Capabilities,
) -> Vec<String> {
    use gix::bstr::ByteSlice;

    capabilities
        .iter()
        .map(|capability| {
            let name = capability.name().to_str_lossy();
            match capability.value() {
                Some(value) => format!("{name}={}", value.to_str_lossy()),
                None => name.into_owned(),
            }
        })
        .collect()
}

struct ReceivePackAdvertisement {
    old_head: Option<gix::ObjectId>,
    heads: Vec<gix::ObjectId>,
    capabilities: Vec<String>,
}

fn read_advertised_receive_pack_head<'a>(
    mut refs: Option<Box<dyn gix::protocol::transport::client::blocking_io::ReadlineBufRead + 'a>>,
    remote_ref: &str,
) -> Result<Option<gix::ObjectId>, GitError> {
    use gix::bstr::ByteSlice;
    use gix::protocol::transport::packetline::PacketLineRef;

    let Some(refs) = refs.as_mut() else {
        return Ok(None);
    };
    let mut advertised_head = None;
    while let Some(line) = refs.readline() {
        let line = line.map_err(GitError::Io)?.map_err(gix_error)?;
        let PacketLineRef::Data(data) = line else {
            continue;
        };
        let line = data.as_bstr().to_str_lossy();
        let Some((hex, name)) = line.trim_end().split_once(' ') else {
            continue;
        };
        if name.split('\0').next() == Some(remote_ref) {
            advertised_head = gix::ObjectId::from_hex(hex.as_bytes())
                .map(Some)
                .map_err(gix_error)?;
        }
    }
    Ok(advertised_head)
}

#[derive(Debug, PartialEq, Eq)]
enum SmartHttpPacketLine {
    Data(Vec<u8>),
    Flush,
}

fn parse_smart_http_packet_lines(bytes: &[u8]) -> Result<Vec<SmartHttpPacketLine>, GitError> {
    let mut lines = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        if bytes.len() - offset < 4 {
            return Err(GitError::Gix(
                "truncated smart HTTP packet-line length".to_string(),
            ));
        }
        let len = std::str::from_utf8(&bytes[offset..offset + 4])
            .ok()
            .and_then(|hex| usize::from_str_radix(hex, 16).ok())
            .ok_or_else(|| GitError::Gix("invalid smart HTTP packet-line length".to_string()))?;
        offset += 4;
        if len == 0 {
            lines.push(SmartHttpPacketLine::Flush);
            continue;
        }
        if len < 4 {
            return Err(GitError::Gix(format!(
                "invalid smart HTTP packet-line length {len}"
            )));
        }
        let data_len = len - 4;
        if bytes.len() - offset < data_len {
            return Err(GitError::Gix(
                "truncated smart HTTP packet-line payload".to_string(),
            ));
        }
        lines.push(SmartHttpPacketLine::Data(
            bytes[offset..offset + data_len].to_vec(),
        ));
        offset += data_len;
    }
    Ok(lines)
}

fn parse_smart_http_receive_pack_advertisement(
    bytes: &[u8],
    remote_ref: &str,
) -> Result<ReceivePackAdvertisement, GitError> {
    use gix::bstr::ByteSlice;

    let lines = parse_smart_http_packet_lines(bytes)?;
    let mut saw_service_header = false;
    let mut old_head = None;
    let mut heads = Vec::new();
    let mut capabilities = Vec::new();
    let mut first_ref_line = true;

    for line in lines {
        let SmartHttpPacketLine::Data(data) = line else {
            continue;
        };
        if !saw_service_header && data.as_slice() == b"# service=git-receive-pack\n" {
            saw_service_header = true;
            continue;
        }

        let mut data = data.as_slice();
        let line_capabilities = if first_ref_line {
            first_ref_line = false;
            if let Some(nul) = data.iter().position(|byte| *byte == 0) {
                let raw_capabilities = &data[nul + 1..];
                data = &data[..nul];
                raw_capabilities
                    .trim_with(|byte| byte.is_ascii_whitespace())
                    .split(|byte| *byte == b' ')
                    .filter(|capability| !capability.is_empty())
                    .map(|capability| capability.as_bstr().to_str_lossy().into_owned())
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        if !line_capabilities.is_empty() {
            capabilities = line_capabilities;
        }

        let line = data.as_bstr().to_str_lossy();
        let Some((hex, name)) = line.trim_end().split_once(' ') else {
            continue;
        };
        let head = gix::ObjectId::from_hex(hex.as_bytes()).map_err(gix_error)?;
        heads.push(head);
        if name == remote_ref {
            old_head = Some(head);
        }
    }

    Ok(ReceivePackAdvertisement {
        old_head,
        heads,
        capabilities,
    })
}

fn parse_receive_pack_status(bytes: &[u8]) -> Result<Vec<String>, GitError> {
    use gix::bstr::ByteSlice;

    Ok(parse_smart_http_packet_lines(bytes)?
        .into_iter()
        .filter_map(|line| match line {
            SmartHttpPacketLine::Data(data) => {
                Some(data.as_bstr().to_str_lossy().trim_end().to_string())
            }
            SmartHttpPacketLine::Flush => None,
        })
        .collect::<Vec<_>>())
}

fn ensure_receive_pack_status(status: &[String], remote_ref: &str) -> Result<(), GitError> {
    if !status.iter().any(|line| line == "unpack ok") {
        return Err(GitError::Gix(format!(
            "receive-pack did not report successful unpack: {status:?}"
        )));
    }
    let ok_ref = format!("ok {remote_ref}");
    if !status.iter().any(|line| line == &ok_ref) {
        return Err(GitError::Gix(format!(
            "receive-pack did not accept {remote_ref}: {status:?}"
        )));
    }
    if let Some(error) = status.iter().find(|line| line.starts_with("ng ")) {
        return Err(GitError::Gix(format!("receive-pack rejected ref: {error}")));
    }
    Ok(())
}

fn diff_trees(
    base_tree: &gix::Tree<'_>,
    head_tree: &gix::Tree<'_>,
) -> Result<Vec<DiffEntry>, GitError> {
    let mut entries = Vec::new();
    base_tree
        .changes()
        .map_err(gix_error)?
        .options(|options| {
            options.track_path();
            // Keep the first porcelain slice deterministic and blob-light:
            // renames are reported as delete+add until the richer diff slice.
            options.track_rewrites(None);
        })
        .for_each_to_obtain_tree(head_tree, |change| {
            if !change.entry_mode().is_tree() {
                entries.push(diff_entry_from_change(change));
            }
            Ok::<_, std::convert::Infallible>(std::ops::ControlFlow::Continue(()))
        })
        .map_err(gix_error)?;
    entries.sort();
    Ok(entries)
}

fn diff_entry_from_change(change: gix::object::tree::diff::Change<'_, '_, '_>) -> DiffEntry {
    use gix::bstr::ByteSlice;
    use gix::object::tree::diff::Change;

    match change {
        Change::Addition { location, .. } => DiffEntry {
            status: DiffStatus::Added,
            path: location.to_str_lossy().into_owned(),
            old_path: None,
        },
        Change::Deletion { location, .. } => DiffEntry {
            status: DiffStatus::Deleted,
            path: location.to_str_lossy().into_owned(),
            old_path: None,
        },
        Change::Modification { location, .. } => DiffEntry {
            status: DiffStatus::Modified,
            path: location.to_str_lossy().into_owned(),
            old_path: None,
        },
        Change::Rewrite {
            source_location,
            location,
            copy,
            ..
        } => DiffEntry {
            status: if copy {
                DiffStatus::Copied
            } else {
                DiffStatus::Renamed
            },
            path: location.to_str_lossy().into_owned(),
            old_path: Some(source_location.to_str_lossy().into_owned()),
        },
    }
}

fn diff_entry_from_index_change(change: gix::diff::index::Change) -> Option<DiffEntry> {
    use gix::bstr::ByteSlice;
    use gix::diff::index::Change;

    if change
        .entry_mode()
        .to_tree_entry_mode()
        .is_some_and(|mode| mode.is_tree())
    {
        return None;
    }

    Some(match change {
        Change::Addition { location, .. } => DiffEntry {
            status: DiffStatus::Added,
            path: location.as_ref().to_str_lossy().into_owned(),
            old_path: None,
        },
        Change::Deletion { location, .. } => DiffEntry {
            status: DiffStatus::Deleted,
            path: location.as_ref().to_str_lossy().into_owned(),
            old_path: None,
        },
        Change::Modification { location, .. } => DiffEntry {
            status: DiffStatus::Modified,
            path: location.as_ref().to_str_lossy().into_owned(),
            old_path: None,
        },
        Change::Rewrite {
            source_location,
            location,
            copy,
            ..
        } => DiffEntry {
            status: if copy {
                DiffStatus::Copied
            } else {
                DiffStatus::Renamed
            },
            path: location.as_ref().to_str_lossy().into_owned(),
            old_path: Some(source_location.as_ref().to_str_lossy().into_owned()),
        },
    })
}

fn collect_index_worktree_status(
    item: gix::status::index_worktree::Item,
    unstaged: &mut Vec<DiffEntry>,
    untracked: &mut Vec<String>,
) {
    use gix::bstr::ByteSlice;
    use gix::status::index_worktree::iter::Summary;

    let Some(summary) = item.summary() else {
        return;
    };
    let path = item.rela_path().to_str_lossy().into_owned();
    match summary {
        Summary::Added => untracked.push(path),
        Summary::Removed => unstaged.push(DiffEntry {
            status: DiffStatus::Deleted,
            path,
            old_path: None,
        }),
        Summary::Modified | Summary::TypeChange | Summary::Conflict | Summary::IntentToAdd => {
            unstaged.push(DiffEntry {
                status: DiffStatus::Modified,
                path,
                old_path: None,
            });
        }
        Summary::Renamed => unstaged.push(DiffEntry {
            status: DiffStatus::Renamed,
            path,
            old_path: None,
        }),
        Summary::Copied => unstaged.push(DiffEntry {
            status: DiffStatus::Copied,
            path,
            old_path: None,
        }),
    }
}

fn added_entries_from_worktree(root: &Path) -> Result<Vec<DiffEntry>, GitError> {
    let mut entries = Vec::new();
    collect_worktree_added_entries(root, root, &mut entries)?;
    entries.sort();
    Ok(entries)
}

fn collect_worktree_added_entries(
    root: &Path,
    dir: &Path,
    entries: &mut Vec<DiffEntry>,
) -> Result<(), GitError> {
    let mut children = std::fs::read_dir(dir)
        .map_err(GitError::Io)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(GitError::Io)?;
    children.sort_by_key(|entry| entry.file_name());

    for child in children {
        if child.file_name() == ".git" {
            continue;
        }
        let path = child.path();
        let meta = std::fs::symlink_metadata(&path).map_err(GitError::Io)?;
        if meta.file_type().is_dir() {
            collect_worktree_added_entries(root, &path, entries)?;
        } else if meta.file_type().is_file() || meta.file_type().is_symlink() {
            let rel = path
                .strip_prefix(root)
                .map_err(|error| gix_error(format!("worktree path escaped root: {error}")))?;
            entries.push(DiffEntry {
                status: DiffStatus::Added,
                path: repo_path_string(rel),
                old_path: None,
            });
        }
    }

    Ok(())
}

fn repo_path_string(path: &Path) -> String {
    path.iter()
        .map(|component| component.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn fast_forward_head(repo: &gix::Repository, new_head: gix::ObjectId) -> Result<(), GitError> {
    let head_name = repo
        .head_name()
        .map_err(gix_error)?
        .ok_or_else(|| GitError::Gix("cannot fast-forward a detached HEAD".to_string()))?;
    repo.reference(
        head_name,
        new_head,
        gix::refs::transaction::PreviousValue::Any,
        format!("merge: Fast-forward RustyRed workspace to {new_head}"),
    )
    .map_err(gix_error)?;
    checkout_head_tree_force(repo)
}

fn gix_conflict_paths(
    conflicts: &[gix::merge::tree::Conflict],
    unresolved: gix::merge::tree::TreatAsUnresolved,
) -> Vec<String> {
    use gix::bstr::ByteSlice;

    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for conflict in conflicts
        .iter()
        .filter(|conflict| conflict.is_unresolved(unresolved))
    {
        for path in [
            conflict.ours.location(),
            conflict.theirs.location(),
            conflict.ours.source_location(),
            conflict.theirs.source_location(),
        ] {
            if path.is_empty() {
                continue;
            }
            let path = path.to_str_lossy().into_owned();
            if seen.insert(path.clone()) {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths
}

fn checkout_branch_force(
    repo: &gix::Repository,
    branch_ref: gix::refs::FullName,
) -> Result<(), GitError> {
    repo.edit_reference(gix::refs::transaction::RefEdit {
        change: gix::refs::transaction::Change::Update {
            log: Default::default(),
            expected: gix::refs::transaction::PreviousValue::Any,
            new: gix::refs::Target::Symbolic(branch_ref),
        },
        name: "HEAD".try_into().expect("HEAD is a valid ref"),
        deref: false,
    })
    .map_err(gix_error)?;
    checkout_head_tree_force(repo)
}

fn checkout_head_tree_force(repo: &gix::Repository) -> Result<(), GitError> {
    let workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Gix("bare repository has no worktree".to_string()))?
        .to_path_buf();
    clear_worktree_except_git(&workdir)?;

    let tree_id = repo.head_tree_id().map_err(gix_error)?.detach();
    checkout_tree_to_dir(repo, tree_id, &workdir, true)
}

fn checkout_tree_to_dir(
    repo: &gix::Repository,
    tree_id: gix::ObjectId,
    destination: &Path,
    write_index: bool,
) -> Result<(), GitError> {
    std::fs::create_dir_all(destination).map_err(GitError::Io)?;
    clear_worktree_except_git(destination)?;
    let mut index = repo.index_from_tree(&tree_id).map_err(gix_error)?;
    let mut opts = repo
        .checkout_options(gix::worktree::stack::state::attributes::Source::IdMapping)
        .map_err(gix_error)?;
    opts.destination_is_initially_empty = true;
    opts.overwrite_existing = true;

    let interrupt = std::sync::atomic::AtomicBool::new(false);
    gix::worktree::state::checkout(
        &mut index,
        destination,
        repo.objects.clone().into_arc().map_err(gix_error)?,
        &gix::progress::Discard,
        &gix::progress::Discard,
        &interrupt,
        opts,
    )
    .map_err(gix_error)?;
    if write_index {
        index.write(Default::default()).map_err(gix_error)?;
    }
    Ok(())
}

fn write_index_for_tree(repo: &gix::Repository, tree_id: gix::ObjectId) -> Result<(), GitError> {
    let mut index = repo.index_from_tree(&tree_id).map_err(gix_error)?;
    index.write(Default::default()).map_err(gix_error)?;
    Ok(())
}

fn clear_worktree_except_git(root: &Path) -> Result<(), GitError> {
    for entry in std::fs::read_dir(root).map_err(GitError::Io)? {
        let entry = entry.map_err(GitError::Io)?;
        if entry.file_name() == ".git" {
            continue;
        }
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(GitError::Io)?;
        if meta.file_type().is_dir() {
            std::fs::remove_dir_all(path).map_err(GitError::Io)?;
        } else {
            std::fs::remove_file(path).map_err(GitError::Io)?;
        }
    }
    Ok(())
}

fn prepare_push_for_repo(
    local: &gix::Repository,
    branch: &str,
    old_head: Option<gix::ObjectId>,
) -> Result<(GixPreparedPush, Vec<gix::ObjectId>), GitError> {
    prepare_push_for_repo_with_known_remote(local, branch, old_head, &[])
}

fn prepare_push_for_repo_with_known_remote(
    local: &gix::Repository,
    branch: &str,
    old_head: Option<gix::ObjectId>,
    known_remote_heads: &[gix::ObjectId],
) -> Result<(GixPreparedPush, Vec<gix::ObjectId>), GitError> {
    use gix::bstr::ByteSlice;

    let branch_ref = branch_ref_name(branch)?;
    let remote_ref = branch_ref.as_bstr().to_str_lossy().into_owned();
    let refspec = format!("{remote_ref}:{remote_ref}");
    gix::refspec::parse(
        refspec.as_bytes().as_bstr(),
        gix::refspec::parse::Operation::Push,
    )
    .map_err(gix_error)?;

    let mut local_ref = local
        .find_reference(branch_ref.as_bstr())
        .map_err(gix_error)?;
    let new_head = local_ref.peel_to_id().map_err(gix_error)?.detach();

    if let Some(old) = old_head {
        if old != new_head && !commit_reaches(local, new_head, old)? {
            return Err(GitError::Gix(format!(
                "non-fast-forward push rejected for {branch}: remote {old} is not an ancestor of {new_head}"
            )));
        }
    }

    let mut remote_boundaries = HashSet::new();
    if let Some(old) = old_head {
        remote_boundaries.insert(old);
    }
    remote_boundaries.extend(known_remote_heads.iter().copied());

    let mut new_objects = collect_reachable_objects_until(local, new_head, &remote_boundaries)?;
    if let Some(old) = old_head {
        for id in collect_reachable_objects(local, old).unwrap_or_default() {
            new_objects.remove(&id);
        }
    }
    for known in known_remote_heads {
        if Some(*known) == old_head {
            continue;
        }
        let Ok(known_objects) = collect_reachable_objects(local, *known) else {
            continue;
        };
        for id in known_objects {
            new_objects.remove(&id);
        }
    }
    let mut object_ids = new_objects.into_iter().collect::<Vec<_>>();
    object_ids.sort_by_key(|id| id.to_string());
    let objects_to_send = object_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let packfile = build_push_packfile(local, &object_ids)?;

    Ok((
        GixPreparedPush {
            branch: branch.to_string(),
            remote_ref,
            refspec,
            old_head: old_head.map(|id| id.to_string()),
            new_head: new_head.to_string(),
            objects_to_send,
            packfile,
        },
        object_ids,
    ))
}

fn build_push_packfile(
    local: &gix::Repository,
    object_ids: &[gix::ObjectId],
) -> Result<GixPushPackfile, GitError> {
    let mut offset = gix_pack::data::header::encode(gix_pack::data::Version::V2, 0).len() as u64;
    let mut entries = Vec::with_capacity(object_ids.len());
    for id in object_ids {
        let object = local.find_object(*id).map_err(gix_error)?;
        let data = gix::objs::Data {
            kind: object.kind,
            object_hash: local.object_hash(),
            data: object.data.as_slice(),
        };
        let entry =
            gix_pack::data::input::Entry::from_data_obj(&data, offset).map_err(gix_error)?;
        offset += entry.bytes_in_pack();
        entries.push(Ok(entry));
    }

    let mut output = Cursor::new(Vec::new());
    let checksum = {
        let mut writer = gix_pack::data::input::EntriesToBytesIter::new(
            entries.into_iter(),
            &mut output,
            gix_pack::data::Version::V2,
            local.object_hash(),
        );
        for entry in writer.by_ref() {
            entry.map_err(gix_error)?;
        }
        writer
            .digest()
            .ok_or_else(|| GitError::Gix("pack writer did not produce a checksum".to_string()))?
            .to_string()
    };

    Ok(GixPushPackfile {
        bytes: output.into_inner(),
        checksum,
        object_count: object_ids.len(),
    })
}

fn collect_reachable_objects(
    local: &gix::Repository,
    tip: gix::ObjectId,
) -> Result<HashSet<gix::ObjectId>, GitError> {
    collect_reachable_objects_until(local, tip, &HashSet::new())
}

fn collect_reachable_objects_until(
    local: &gix::Repository,
    tip: gix::ObjectId,
    stop_at: &HashSet<gix::ObjectId>,
) -> Result<HashSet<gix::ObjectId>, GitError> {
    let mut seen = HashSet::new();
    collect_object_recursive_until(local, tip, stop_at, &mut seen)?;
    Ok(seen)
}

fn collect_object_recursive_until(
    local: &gix::Repository,
    id: gix::ObjectId,
    stop_at: &HashSet<gix::ObjectId>,
    seen: &mut HashSet<gix::ObjectId>,
) -> Result<(), GitError> {
    if stop_at.contains(&id) {
        return Ok(());
    }
    if !seen.insert(id) {
        return Ok(());
    }

    let object = local.find_object(id).map_err(gix_error)?;
    let kind = object.kind;
    match kind {
        gix::objs::Kind::Commit => {
            let commit = object.try_into_commit().map_err(gix_error)?;
            collect_object_recursive_until(
                local,
                commit.tree_id().map_err(gix_error)?.detach(),
                stop_at,
                seen,
            )?;
            let parents = commit
                .parent_ids()
                .map(|parent| parent.detach())
                .collect::<Vec<_>>();
            for parent in parents {
                collect_object_recursive_until(local, parent, stop_at, seen)?;
            }
        }
        gix::objs::Kind::Tree => {
            let tree = object.try_into_tree().map_err(gix_error)?;
            for entry in tree.decode().map_err(gix_error)?.entries {
                collect_object_recursive_until(local, entry.oid.to_owned(), stop_at, seen)?;
            }
        }
        gix::objs::Kind::Tag => {
            let tag = object.try_into_tag().map_err(gix_error)?;
            let tag_ref = tag.decode().map_err(gix_error)?;
            let target = gix::ObjectId::from_hex(tag_ref.target.as_ref()).map_err(gix_error)?;
            collect_object_recursive_until(local, target, stop_at, seen)?;
        }
        gix::objs::Kind::Blob => {}
    }

    Ok(())
}

fn copy_selected_objects(
    local: &gix::Repository,
    remote: &gix::Repository,
    object_ids: &[gix::ObjectId],
) -> Result<usize, GitError> {
    use gix::objs::Write;

    for id in object_ids {
        let object = local.find_object(*id).map_err(gix_error)?;
        remote
            .objects
            .write_buf(object.kind, object.data.as_slice())
            .map_err(gix_error)?;
    }
    Ok(object_ids.len())
}

fn commit_reaches(
    repo: &gix::Repository,
    start: gix::ObjectId,
    target: gix::ObjectId,
) -> Result<bool, GitError> {
    let mut stack = vec![start];
    let mut seen = HashSet::new();

    while let Some(id) = stack.pop() {
        if id == target {
            return Ok(true);
        }
        if !seen.insert(id) {
            continue;
        }
        let object = repo.find_object(id).map_err(gix_error)?;
        if object.kind != gix::objs::Kind::Commit {
            continue;
        }
        let commit = object.try_into_commit().map_err(gix_error)?;
        stack.extend(commit.parent_ids().map(|parent| parent.detach()));
    }

    Ok(false)
}

fn write_worktree_tree(repo: &gix::Repository, dir: &Path) -> Result<gix::ObjectId, GitError> {
    let mut entries = Vec::new();
    let mut children = std::fs::read_dir(dir)
        .map_err(GitError::Io)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(GitError::Io)?;
    children.sort_by_key(|entry| entry.file_name());

    for child in children {
        let file_name = child.file_name();
        if file_name == ".git" {
            continue;
        }
        let path = child.path();
        let meta = std::fs::symlink_metadata(&path).map_err(GitError::Io)?;
        let file_type = meta.file_type();
        let filename = file_name.to_string_lossy().as_bytes().to_vec().into();
        let (mode, oid) = if file_type.is_dir() {
            (
                gix::objs::tree::EntryKind::Tree.into(),
                write_worktree_tree(repo, &path)?,
            )
        } else if file_type.is_symlink() {
            (
                gix::objs::tree::EntryKind::Link.into(),
                repo.write_blob(
                    std::fs::read_link(&path)
                        .map_err(GitError::Io)?
                        .to_string_lossy()
                        .as_bytes(),
                )
                .map_err(gix_error)?
                .detach(),
            )
        } else if file_type.is_file() {
            let mode = if is_executable(&meta) {
                gix::objs::tree::EntryKind::BlobExecutable
            } else {
                gix::objs::tree::EntryKind::Blob
            };
            (
                mode.into(),
                repo.write_blob(std::fs::read(&path).map_err(GitError::Io)?)
                    .map_err(gix_error)?
                    .detach(),
            )
        } else {
            continue;
        };
        entries.push(gix::objs::tree::Entry {
            mode,
            filename,
            oid,
        });
    }

    entries.sort();
    let tree = gix::objs::Tree { entries };
    Ok(repo.write_object(tree).map_err(gix_error)?.detach())
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    false
}

/// GitHub pull request creation input for W2's remote/PR path.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PullRequestInput {
    pub title: String,
    pub head: String,
    pub base: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maintainer_can_modify: Option<bool>,
}

impl PullRequestInput {
    pub fn new(title: impl Into<String>, head: impl Into<String>, base: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            head: head.into(),
            base: base.into(),
            body: None,
            draft: None,
            maintainer_can_modify: None,
        }
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn draft(mut self, draft: bool) -> Self {
        self.draft = Some(draft);
        self
    }

    pub fn maintainer_can_modify(mut self, value: bool) -> Self {
        self.maintainer_can_modify = Some(value);
        self
    }
}

/// The minimal PR receipt the workspace needs after opening a pull request.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct PullRequestReceipt {
    pub number: u64,
    pub html_url: String,
}

/// Thin GitHub REST client for W2 push/PR-out.
///
/// Authentication is injected as a bearer token so `GithubApp` token minting in
/// `apps/theorem-harness-server` can remain the credential source while this
/// crate owns only the git/PR transport.
#[derive(Clone, Debug)]
pub struct GitHubClient {
    base_url: String,
    token: String,
    user_agent: String,
}

impl GitHubClient {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            base_url: "https://api.github.com".to_string(),
            token: token.into(),
            user_agent: "rustyred-git".to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// Create a pull request via `POST /repos/{owner}/{repo}/pulls`.
    ///
    /// This follows GitHub's documented REST shape: recommended GitHub media
    /// type, bearer auth, API version header, and JSON title/head/base/body/etc.
    pub fn create_pull_request(
        &self,
        owner: &str,
        repo: &str,
        input: &PullRequestInput,
    ) -> Result<PullRequestReceipt, GitError> {
        let url = format!("{}/repos/{owner}/{repo}/pulls", self.base_url);
        let response = reqwest::blocking::Client::new()
            .post(&url)
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {}", self.token))
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", &self.user_agent)
            .json(input)
            .send()
            .map_err(github_error)?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            return Err(GitError::GitHub(format!(
                "create pull request failed with {status}: {body}"
            )));
        }
        response.json().map_err(github_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;

    fn unique_temp_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut dir = std::env::temp_dir();
        dir.push(format!("rustyred-git-{tag}-{}-{n}", std::process::id()));
        dir
    }

    fn assert_prepared_packfile_matches_object_closure(
        prepared: &GixPreparedPush,
        repo_dir: &Path,
    ) {
        let repo = gix::open(repo_dir).expect("open gix repo");
        assert_eq!(
            prepared.packfile.object_count,
            prepared.objects_to_send.len()
        );
        assert!(prepared.packfile.bytes.starts_with(b"PACK"));

        let header = prepared.packfile.bytes[..12]
            .try_into()
            .expect("pack header length");
        let (version, count) = gix_pack::data::header::decode(header).expect("decode pack header");
        assert_eq!(version, gix_pack::data::Version::V2);
        assert_eq!(count as usize, prepared.packfile.object_count);

        let checksum = gix::ObjectId::from_hex(prepared.packfile.checksum.as_bytes())
            .expect("pack checksum is hex");
        let checksum_len = repo.object_hash().len_in_bytes();
        assert_eq!(
            &prepared.packfile.bytes[prepared.packfile.bytes.len() - checksum_len..],
            checksum.as_slice()
        );

        let mut entries = gix_pack::data::input::BytesToEntriesIter::new_from_header(
            std::io::BufReader::new(prepared.packfile.bytes.as_slice()),
            gix_pack::data::input::Mode::Verify,
            gix_pack::data::input::EntryDataMode::Ignore,
            repo.object_hash(),
        )
        .expect("parse pack header");
        let mut parsed = 0usize;
        for entry in &mut entries {
            entry.expect("pack entry verifies");
            parsed += 1;
        }
        assert_eq!(parsed, prepared.packfile.object_count);
    }

    fn assert_receive_pack_request_matches_prepared(
        request: &GixReceivePackRequest,
        prepared: &GixPreparedPush,
        expected_old_head: &str,
        expected_capabilities: &[&str],
    ) {
        use gix::protocol::transport::packetline::{decode, PacketLineRef};

        assert_eq!(request.remote_ref, prepared.remote_ref);
        assert_eq!(request.old_head, expected_old_head);
        assert_eq!(request.new_head, prepared.new_head);
        assert_eq!(
            request.capabilities,
            expected_capabilities
                .iter()
                .map(|capability| (*capability).to_string())
                .collect::<Vec<_>>()
        );

        let (line, consumed) = match decode::streaming(&request.bytes).expect("decode first line") {
            decode::Stream::Complete {
                line: PacketLineRef::Data(data),
                bytes_consumed,
            } => (data, bytes_consumed),
            other => panic!("expected first receive-pack command line, got {other:?}"),
        };
        let command = std::str::from_utf8(line).expect("command utf8");
        let command = command
            .strip_suffix('\n')
            .expect("receive-pack command line ends in LF");
        let (ref_update, capabilities) = command
            .split_once('\0')
            .expect("first receive-pack command carries capabilities after NUL");
        assert_eq!(
            ref_update,
            format!(
                "{} {} {}",
                expected_old_head, prepared.new_head, prepared.remote_ref
            )
        );
        assert_eq!(capabilities, expected_capabilities.join(" "));
        assert_eq!(
            &request.bytes[consumed..consumed + 4],
            b"0000",
            "command list is terminated by a flush packet"
        );
        assert_eq!(request.packfile_offset, consumed + 4);
        assert_eq!(
            &request.bytes[request.packfile_offset..],
            prepared.packfile.bytes.as_slice(),
            "raw verified packfile follows the receive-pack flush"
        );
    }

    // W2 first slice: a real local git repo. init -> (unborn HEAD) -> write +
    // commit -> HEAD is a real hash -> read files back from the COMMITTED tree
    // -> branch. (Divergent-commit merge is the next slice.)
    #[test]
    fn init_commit_read_back_and_branch() {
        let root = unique_temp_dir("repo");
        let repo = WorkspaceRepo::init(&root).expect("init");

        assert_eq!(
            repo.head().expect("head"),
            None,
            "HEAD is unborn before the first commit"
        );

        std::fs::write(root.join("hello.txt"), b"hello git").expect("write");
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        std::fs::write(root.join("src/main.rs"), b"fn main() {}").expect("write");

        let hash = repo.commit_all("initial").expect("commit");
        assert_eq!(hash.len(), 40, "a 40-char commit hash: {hash:?}");
        assert_eq!(repo.head().expect("head"), Some(hash), "HEAD == the commit");

        // Read back from the committed tree, not the working dir.
        assert_eq!(
            repo.read_file_at_head("hello.txt").expect("show"),
            Some(b"hello git".to_vec()),
            "committed file round-trips from the tree"
        );
        assert_eq!(
            repo.read_file_at_head("src/main.rs").expect("show"),
            Some(b"fn main() {}".to_vec()),
            "nested committed file round-trips"
        );
        assert_eq!(
            repo.read_file_at_head("not-committed.txt").expect("show"),
            None,
            "a path absent from the commit reads as None"
        );

        // Branching off the commit.
        assert_eq!(repo.current_branch().expect("branch"), "main");
        repo.create_branch("feature").expect("create branch");
        assert_eq!(repo.current_branch().expect("branch"), "feature");

        let _ = std::fs::remove_dir_all(&root);
    }

    // A second commit on a branch is independent of main: the two branches point
    // at different HEADs (the seed of the W2 acceptance's divergent-history test).
    #[test]
    fn branches_have_independent_heads() {
        let root = unique_temp_dir("diverge");
        let repo = WorkspaceRepo::init(&root).expect("init");
        std::fs::write(root.join("a.txt"), b"base").expect("write");
        let base = repo.commit_all("base").expect("commit");

        repo.create_branch("feature").expect("branch");
        std::fs::write(root.join("b.txt"), b"feature work").expect("write");
        let feature = repo.commit_all("feature commit").expect("commit");
        assert_ne!(feature, base, "the feature commit advances past base");

        repo.checkout("main").expect("checkout main");
        assert_eq!(
            repo.head().expect("head"),
            Some(base),
            "main still points at base; the feature commit did not touch it"
        );
        assert_eq!(
            repo.read_file_at_head("b.txt").expect("show"),
            None,
            "the feature-only file is not on main"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Create a bare repo at `dir` (a push target; no worktree).
    fn init_bare(dir: &std::path::Path) {
        std::fs::create_dir_all(dir).expect("mkdir bare");
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["init", "--bare", "-b", "main"])
            .status()
            .expect("git init --bare");
        assert!(status.success(), "git init --bare");
    }

    // W2 acceptance #1 (merge): base + two divergent branches touching different
    // files merge cleanly into one history with all files present.
    #[test]
    fn three_way_merge_clean() {
        let root = unique_temp_dir("merge");
        let repo = WorkspaceRepo::init(&root).expect("init");
        std::fs::write(root.join("a.txt"), b"base").expect("w");
        repo.commit_all("base").expect("c");

        repo.create_branch("feature").expect("b");
        std::fs::write(root.join("b.txt"), b"feature").expect("w");
        repo.commit_all("feature: add b").expect("c");

        repo.checkout("main").expect("co");
        std::fs::write(root.join("c.txt"), b"main side").expect("w");
        repo.commit_all("main: add c").expect("c");

        match repo.merge("feature").expect("merge") {
            MergeOutcome::Merged(hash) => assert_eq!(hash.len(), 40, "merge commit: {hash}"),
            other => panic!("expected a clean merge, got {other:?}"),
        }
        for (path, body) in [
            ("a.txt", &b"base"[..]),
            ("b.txt", &b"feature"[..]),
            ("c.txt", &b"main side"[..]),
        ] {
            assert_eq!(
                repo.read_file_at_head(path).expect("show").as_deref(),
                Some(body),
                "{path} present on the merged HEAD"
            );
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    // Conflicting edits to the same file surface as a Conflict (never a silent
    // auto-resolve); abort restores a clean tree.
    #[test]
    fn merge_conflict_is_surfaced() {
        let root = unique_temp_dir("conflict");
        let repo = WorkspaceRepo::init(&root).expect("init");
        std::fs::write(root.join("x.txt"), b"base\n").expect("w");
        repo.commit_all("base").expect("c");

        repo.create_branch("feature").expect("b");
        std::fs::write(root.join("x.txt"), b"feature change\n").expect("w");
        repo.commit_all("feature edits x").expect("c");

        repo.checkout("main").expect("co");
        std::fs::write(root.join("x.txt"), b"main change\n").expect("w");
        repo.commit_all("main edits x").expect("c");

        match repo.merge("feature").expect("merge") {
            MergeOutcome::Conflict(files) => {
                assert_eq!(files, vec!["x.txt".to_string()], "conflicted path")
            }
            other => panic!("expected a conflict, got {other:?}"),
        }
        repo.merge_abort().expect("abort");
        assert!(
            repo.conflicted_paths().expect("conflicted").is_empty(),
            "abort restores a clean tree"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // W2 acceptance #2 (clone/push): commit -> push to a local bare remote ->
    // clone it elsewhere -> the file round-trips. No network, no HTTP dependency.
    #[test]
    fn push_to_bare_then_clone_round_trip() {
        let bare = unique_temp_dir("bare");
        init_bare(&bare);
        let bare_url = bare.to_str().expect("bare path utf8");

        let work = unique_temp_dir("work");
        let repo = WorkspaceRepo::init(&work).expect("init");
        std::fs::write(work.join("shared.txt"), b"pushed content").expect("w");
        repo.commit_all("seed").expect("c");
        repo.add_remote("origin", bare_url).expect("remote");
        repo.push("origin", "main").expect("push");

        let cloned_dir = unique_temp_dir("clone");
        let cloned = WorkspaceRepo::clone(bare_url, &cloned_dir).expect("clone");
        assert_eq!(
            cloned.read_file_at_head("shared.txt").expect("show"),
            Some(b"pushed content".to_vec()),
            "the pushed file round-trips through the bare remote into a fresh clone"
        );

        let _ = std::fs::remove_dir_all(&bare);
        let _ = std::fs::remove_dir_all(&work);
        let _ = std::fs::remove_dir_all(&cloned_dir);
    }

    // W2 gix read slice: pure-Rust open/read over a real committed tree,
    // including trees produced outside the gix backend.
    #[test]
    fn gix_backend_reads_committed_tree_without_git_cli() {
        let root = unique_temp_dir("gix-read");
        let repo = WorkspaceRepo::init(&root).expect("init cli");
        std::fs::write(root.join("hello.txt"), b"hello from gix").expect("write");
        let cli_head = repo.commit_all("seed").expect("commit");

        let gix_repo = GixWorkspaceRepo::open(&root);
        assert_eq!(gix_repo.head().expect("gix head"), Some(cli_head));
        assert_eq!(
            gix_repo.current_branch().expect("branch").as_deref(),
            Some("main")
        );
        assert_eq!(
            gix_repo.read_file_at_head("hello.txt").expect("read"),
            Some(b"hello from gix".to_vec())
        );
        assert_eq!(
            gix_repo.read_file_at_head("missing.txt").expect("read"),
            None
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // W2 gix mutating slice: commit a materialized worktree using gix object
    // writes plus commit/ref update, then prove read-back from the committed
    // tree also stays inside the gix backend.
    #[test]
    fn gix_backend_commits_materialized_worktree_without_git_cli() {
        let root = unique_temp_dir("gix-commit");
        let repo = GixWorkspaceRepo::init(&root).expect("init gix");
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        std::fs::write(root.join("src/lib.rs"), b"pub fn one() -> u8 { 1 }\n").expect("write");
        std::fs::write(root.join("README.md"), b"# seed\n").expect("write");

        let first = repo
            .commit_all_from_worktree("seed from materialized tree")
            .expect("gix commit");
        assert_eq!(first.len(), 40, "gix commit hash: {first}");
        assert_eq!(repo.head().expect("head"), Some(first.clone()));
        assert_eq!(
            repo.read_file_at_head("src/lib.rs")
                .expect("read")
                .as_deref(),
            Some(&b"pub fn one() -> u8 { 1 }\n"[..])
        );
        assert_eq!(
            repo.read_file_at_head("README.md")
                .expect("read")
                .as_deref(),
            Some(&b"# seed\n"[..])
        );

        std::fs::write(root.join("src/lib.rs"), b"pub fn two() -> u8 { 2 }\n").expect("write");
        std::fs::remove_file(root.join("README.md")).expect("delete");
        let second = repo
            .commit_all_from_worktree("refresh materialized tree")
            .expect("gix commit");
        assert_ne!(second, first, "second commit advances HEAD");
        assert_eq!(repo.head().expect("head"), Some(second));
        assert_eq!(
            repo.read_file_at_head("src/lib.rs")
                .expect("read")
                .as_deref(),
            Some(&b"pub fn two() -> u8 { 2 }\n"[..])
        );
        assert_eq!(
            repo.read_file_at_head("README.md").expect("read"),
            None,
            "deleted worktree files disappear from the next materialized-tree commit"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // W2 gix branch/checkout slice: create a branch and force-materialize the
    // selected committed tree without invoking the git CLI. This is the
    // generated-worktree behavior W0/W3 need: branch switches rewrite disk to
    // the selected branch and remove stale files from the previous branch.
    #[test]
    fn gix_backend_creates_and_checks_out_branches_without_git_cli() {
        let root = unique_temp_dir("gix-branch");
        let repo = GixWorkspaceRepo::init(&root).expect("init gix");
        std::fs::write(root.join("base.txt"), b"base\n").expect("write base");
        let base = repo
            .commit_all_from_worktree("base materialized tree")
            .expect("commit base");

        repo.create_branch("feature").expect("create branch");
        assert_eq!(
            repo.current_branch().expect("branch").as_deref(),
            Some("feature")
        );
        std::fs::write(root.join("base.txt"), b"feature base\n").expect("write");
        std::fs::write(root.join("feature.txt"), b"feature only\n").expect("write");
        let feature = repo
            .commit_all_from_worktree("feature materialized tree")
            .expect("commit feature");
        assert_ne!(feature, base, "feature commit advances branch");

        repo.checkout("main").expect("checkout main");
        assert_eq!(
            repo.current_branch().expect("branch").as_deref(),
            Some("main")
        );
        assert_eq!(repo.head().expect("head"), Some(base.clone()));
        assert_eq!(
            std::fs::read(root.join("base.txt")).expect("read disk"),
            b"base\n"
        );
        assert!(
            !root.join("feature.txt").exists(),
            "force checkout removes stale branch-only files"
        );
        assert_eq!(
            repo.read_file_at_head("feature.txt").expect("read"),
            None,
            "main HEAD does not contain feature-only files"
        );

        repo.checkout("feature").expect("checkout feature");
        assert_eq!(
            repo.current_branch().expect("branch").as_deref(),
            Some("feature")
        );
        assert_eq!(repo.head().expect("head"), Some(feature));
        assert_eq!(
            std::fs::read(root.join("base.txt")).expect("read disk"),
            b"feature base\n"
        );
        assert_eq!(
            std::fs::read(root.join("feature.txt")).expect("read disk"),
            b"feature only\n"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // W2 gix merge slice: a clean divergent merge writes a real two-parent
    // commit and rematerializes the merged tree without invoking the git CLI.
    #[test]
    fn gix_backend_merges_clean_divergent_branches_without_git_cli() {
        let root = unique_temp_dir("gix-merge-clean");
        let repo = GixWorkspaceRepo::init(&root).expect("init gix");
        std::fs::write(root.join("a.txt"), b"base\n").expect("write base");
        repo.commit_all_from_worktree("base").expect("commit base");

        repo.create_branch("feature").expect("create feature");
        std::fs::write(root.join("b.txt"), b"feature\n").expect("write feature");
        let feature = repo
            .commit_all_from_worktree("feature adds b")
            .expect("commit feature");

        repo.checkout("main").expect("checkout main");
        std::fs::write(root.join("c.txt"), b"main\n").expect("write main");
        let main = repo
            .commit_all_from_worktree("main adds c")
            .expect("commit main");

        let merge = match repo.merge("feature").expect("merge feature") {
            MergeOutcome::Merged(hash) => hash,
            other => panic!("expected clean gix merge, got {other:?}"),
        };
        assert_ne!(merge, main, "merge commit advances main");
        assert_ne!(merge, feature, "merge commit is not just feature");
        assert_eq!(repo.head().expect("head"), Some(merge));
        for (path, body) in [
            ("a.txt", &b"base\n"[..]),
            ("b.txt", &b"feature\n"[..]),
            ("c.txt", &b"main\n"[..]),
        ] {
            assert_eq!(
                repo.read_file_at_head(path).expect("read").as_deref(),
                Some(body),
                "{path} present on merged HEAD"
            );
            assert_eq!(
                std::fs::read(root.join(path)).expect("read disk"),
                body,
                "{path} materialized after gix merge"
            );
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    // Same-file divergent edits surface as a gix conflict and leave HEAD plus
    // the materialized worktree on the pre-merge branch state.
    #[test]
    fn gix_backend_surfaces_merge_conflicts_without_git_cli() {
        let root = unique_temp_dir("gix-merge-conflict");
        let repo = GixWorkspaceRepo::init(&root).expect("init gix");
        std::fs::write(root.join("x.txt"), b"base\n").expect("write base");
        repo.commit_all_from_worktree("base").expect("commit base");

        repo.create_branch("feature").expect("create feature");
        std::fs::write(root.join("x.txt"), b"feature change\n").expect("write feature");
        repo.commit_all_from_worktree("feature edits x")
            .expect("commit feature");

        repo.checkout("main").expect("checkout main");
        std::fs::write(root.join("x.txt"), b"main change\n").expect("write main");
        let main = repo
            .commit_all_from_worktree("main edits x")
            .expect("commit main");

        match repo.merge("feature").expect("merge feature") {
            MergeOutcome::Conflict(paths) => {
                assert_eq!(paths, vec!["x.txt".to_string()], "conflicted path")
            }
            other => panic!("expected gix conflict, got {other:?}"),
        }
        assert_eq!(
            repo.head().expect("head"),
            Some(main),
            "conflict does not advance HEAD"
        );
        assert_eq!(
            std::fs::read(root.join("x.txt")).expect("read disk"),
            b"main change\n",
            "conflict does not rewrite the materialized tree"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // W2 gix porcelain slice: branch review diff reports committed-tree
    // additions, deletions, and modifications without invoking the git CLI.
    #[test]
    fn gix_backend_diffs_committed_branches_without_git_cli() {
        let root = unique_temp_dir("gix-diff-branches");
        let repo = GixWorkspaceRepo::init(&root).expect("init gix");
        std::fs::write(root.join("same.txt"), b"same\n").expect("write same");
        std::fs::write(root.join("edit.txt"), b"before\n").expect("write edit");
        std::fs::write(root.join("delete.txt"), b"delete me\n").expect("write delete");
        repo.commit_all_from_worktree("base").expect("commit base");

        repo.create_branch("feature").expect("create feature");
        std::fs::write(root.join("edit.txt"), b"after\n").expect("edit");
        std::fs::remove_file(root.join("delete.txt")).expect("delete");
        std::fs::write(root.join("add.txt"), b"add me\n").expect("add");
        repo.commit_all_from_worktree("feature changes")
            .expect("commit feature");

        repo.checkout("main").expect("checkout main");
        assert_eq!(
            repo.diff_branches("main", "feature").expect("diff"),
            vec![
                DiffEntry {
                    status: DiffStatus::Added,
                    path: "add.txt".to_string(),
                    old_path: None,
                },
                DiffEntry {
                    status: DiffStatus::Deleted,
                    path: "delete.txt".to_string(),
                    old_path: None,
                },
                DiffEntry {
                    status: DiffStatus::Modified,
                    path: "edit.txt".to_string(),
                    old_path: None,
                },
            ]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // W2 gix porcelain slice: dirty materialized files can be diffed against
    // HEAD without committing or moving refs.
    #[test]
    fn gix_backend_diffs_worktree_against_head_without_committing() {
        let root = unique_temp_dir("gix-diff-worktree");
        let repo = GixWorkspaceRepo::init(&root).expect("init gix");
        std::fs::write(root.join("same.txt"), b"same\n").expect("write same");
        std::fs::write(root.join("edit.txt"), b"before\n").expect("write edit");
        std::fs::write(root.join("delete.txt"), b"delete me\n").expect("write delete");
        let head = repo.commit_all_from_worktree("base").expect("commit base");

        std::fs::write(root.join("edit.txt"), b"after\n").expect("edit");
        std::fs::remove_file(root.join("delete.txt")).expect("delete");
        std::fs::write(root.join("add.txt"), b"add me\n").expect("add");

        assert_eq!(
            repo.diff_worktree_to_head().expect("diff"),
            vec![
                DiffEntry {
                    status: DiffStatus::Added,
                    path: "add.txt".to_string(),
                    old_path: None,
                },
                DiffEntry {
                    status: DiffStatus::Deleted,
                    path: "delete.txt".to_string(),
                    old_path: None,
                },
                DiffEntry {
                    status: DiffStatus::Modified,
                    path: "edit.txt".to_string(),
                    old_path: None,
                },
            ]
        );
        assert_eq!(
            repo.head().expect("head"),
            Some(head),
            "worktree diff does not commit or advance HEAD"
        );
        assert_eq!(
            repo.read_file_at_head("edit.txt").expect("read").as_deref(),
            Some(&b"before\n"[..]),
            "HEAD still sees the committed bytes"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // W2 gix porcelain slice: the generated-worktree status receipt reports
    // commit-candidate changes without requiring an index stage/unstage model.
    #[test]
    fn gix_backend_reports_worktree_status_for_commit_decisions() {
        let root = unique_temp_dir("gix-worktree-status");
        let repo = GixWorkspaceRepo::init(&root).expect("init gix");
        std::fs::create_dir_all(root.join("src")).expect("mkdir src");
        std::fs::write(root.join("src/lib.rs"), b"pub fn one() -> u8 { 1 }\n").expect("write lib");
        std::fs::write(root.join("README.md"), b"# status\n").expect("write readme");

        let unborn = repo.worktree_status().expect("unborn status");
        assert_eq!(unborn.current_branch.as_deref(), Some("main"));
        assert_eq!(unborn.head, None);
        assert!(!unborn.is_clean(), "unborn files are commit candidates");
        assert_eq!(
            unborn.changes,
            vec![
                DiffEntry {
                    status: DiffStatus::Added,
                    path: "README.md".to_string(),
                    old_path: None,
                },
                DiffEntry {
                    status: DiffStatus::Added,
                    path: "src/lib.rs".to_string(),
                    old_path: None,
                },
            ]
        );

        let head = repo
            .commit_all_from_worktree("seed status tree")
            .expect("commit seed");
        let clean = repo.worktree_status().expect("clean status");
        assert_eq!(clean.current_branch.as_deref(), Some("main"));
        assert_eq!(clean.head.as_deref(), Some(head.as_str()));
        assert!(clean.is_clean(), "status is clean immediately after commit");

        std::fs::write(root.join("src/lib.rs"), b"pub fn two() -> u8 { 2 }\n").expect("edit lib");
        std::fs::remove_file(root.join("README.md")).expect("delete readme");
        std::fs::write(root.join("src/new.rs"), b"pub fn new() {}\n").expect("add new");
        let dirty = repo.worktree_status().expect("dirty status");
        assert_eq!(dirty.head.as_deref(), Some(head.as_str()));
        assert_eq!(
            dirty.changes,
            vec![
                DiffEntry {
                    status: DiffStatus::Added,
                    path: "src/new.rs".to_string(),
                    old_path: None,
                },
                DiffEntry {
                    status: DiffStatus::Deleted,
                    path: "README.md".to_string(),
                    old_path: None,
                },
                DiffEntry {
                    status: DiffStatus::Modified,
                    path: "src/lib.rs".to_string(),
                    old_path: None,
                },
            ]
        );
        assert_eq!(
            repo.head().expect("head"),
            Some(head),
            "status does not create commits or move HEAD"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // W2 gix index porcelain slice: HEAD/index/worktree status distinguishes
    // staged changes, unstaged tracked changes, and untracked files.
    #[test]
    fn gix_backend_reports_staged_unstaged_and_untracked_status_without_git_cli() {
        let root = unique_temp_dir("gix-index-status");
        let repo = GixWorkspaceRepo::init(&root).expect("init gix");
        std::fs::write(root.join("tracked.txt"), b"base\n").expect("write base");
        let head = repo.commit_all_from_worktree("base").expect("commit base");

        let clean = repo.index_status().expect("clean index status");
        assert_eq!(clean.current_branch.as_deref(), Some("main"));
        assert_eq!(clean.head.as_deref(), Some(head.as_str()));
        assert!(clean.is_clean(), "commit-all refreshes the persisted index");

        std::fs::write(root.join("tracked.txt"), b"edited before stage\n").expect("edit tracked");
        std::fs::write(root.join("untracked.txt"), b"new file\n").expect("write untracked");
        let dirty = repo.index_status().expect("dirty index status");
        assert_eq!(dirty.staged, Vec::<DiffEntry>::new());
        assert_eq!(
            dirty.unstaged,
            vec![DiffEntry {
                status: DiffStatus::Modified,
                path: "tracked.txt".to_string(),
                old_path: None,
            }]
        );
        assert_eq!(dirty.untracked, vec!["untracked.txt".to_string()]);

        let staged = repo.stage_all_from_worktree().expect("stage worktree");
        assert_eq!(staged.head.as_deref(), Some(head.as_str()));
        assert_eq!(
            staged.staged,
            vec![
                DiffEntry {
                    status: DiffStatus::Added,
                    path: "untracked.txt".to_string(),
                    old_path: None,
                },
                DiffEntry {
                    status: DiffStatus::Modified,
                    path: "tracked.txt".to_string(),
                    old_path: None,
                },
            ]
        );
        assert_eq!(staged.unstaged, Vec::<DiffEntry>::new());
        assert_eq!(staged.untracked, Vec::<String>::new());

        std::fs::write(root.join("tracked.txt"), b"edited after stage\n")
            .expect("edit tracked again");
        let split = repo.index_status().expect("split index status");
        assert_eq!(
            split.staged, staged.staged,
            "index still holds the staged snapshot"
        );
        assert_eq!(
            split.unstaged,
            vec![DiffEntry {
                status: DiffStatus::Modified,
                path: "tracked.txt".to_string(),
                old_path: None,
            }],
            "same path can be both staged and dirty after another edit"
        );
        assert_eq!(
            repo.head().expect("head"),
            Some(head),
            "index status and staging do not move HEAD"
        );

        let next = repo
            .commit_all_from_worktree("commit latest materialized state")
            .expect("commit latest");
        let clean_after_commit = repo.index_status().expect("clean after commit");
        assert_eq!(clean_after_commit.head.as_deref(), Some(next.as_str()));
        assert!(
            clean_after_commit.is_clean(),
            "commit-all leaves HEAD, index, and worktree aligned"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // W2 gix isolation slice: materialize a branch into a separate plain
    // directory so a parallel agent can inspect/edit a branch-shaped tree
    // without touching the main worktree.
    #[test]
    fn gix_backend_materializes_branch_snapshot_for_isolated_agent_work() {
        let root = unique_temp_dir("gix-isolation-root");
        let repo = GixWorkspaceRepo::init(&root).expect("init gix");
        std::fs::write(root.join("base.txt"), b"base\n").expect("write base");
        repo.commit_all_from_worktree("base").expect("commit base");

        repo.create_branch("feature").expect("create feature");
        std::fs::write(root.join("base.txt"), b"feature base\n").expect("write feature base");
        std::fs::write(root.join("feature.txt"), b"feature only\n").expect("write feature");
        repo.commit_all_from_worktree("feature branch")
            .expect("commit feature");

        repo.checkout("main").expect("checkout main");
        std::fs::write(root.join("main.txt"), b"main only\n").expect("write main");
        repo.commit_all_from_worktree("main branch")
            .expect("commit main");

        let isolated = unique_temp_dir("gix-isolation-snapshot");
        std::fs::create_dir_all(&isolated).expect("mkdir isolated");
        std::fs::write(isolated.join("stale.txt"), b"stale\n").expect("write stale");
        repo.materialize_branch_snapshot("feature", &isolated)
            .expect("materialize feature");

        assert!(
            !isolated.join(".git").exists(),
            "snapshot is a plain materialized source tree"
        );
        assert!(
            !isolated.join("stale.txt").exists(),
            "re-materializing clears stale files"
        );
        assert_eq!(
            std::fs::read(isolated.join("base.txt")).expect("read isolated base"),
            b"feature base\n"
        );
        assert_eq!(
            std::fs::read(isolated.join("feature.txt")).expect("read isolated feature"),
            b"feature only\n"
        );
        assert!(
            !isolated.join("main.txt").exists(),
            "main-only files do not leak into the feature snapshot"
        );
        assert_eq!(
            std::fs::read(root.join("base.txt")).expect("read main base"),
            b"base\n",
            "main worktree remains checked out independently"
        );
        assert_eq!(
            std::fs::read(root.join("main.txt")).expect("read main"),
            b"main only\n"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&isolated);
    }

    // W2 gix local-remote slice: push to a local bare repository without the
    // git CLI by copying the reachable object closure into the remote ODB and
    // updating the branch ref. A second push proves fast-forward ref updates.
    #[test]
    fn gix_push_to_local_bare_round_trips_without_git_cli() {
        let bare = unique_temp_dir("gix-push-bare");
        gix::init_bare(&bare).expect("init bare with gix");
        let bare_url = bare.to_str().expect("bare path utf8");

        let work = unique_temp_dir("gix-push-work");
        let repo = GixWorkspaceRepo::init(&work).expect("init gix");
        std::fs::write(work.join("shared.txt"), b"first push\n").expect("write");
        let first = repo
            .commit_all_from_worktree("first local-bare push")
            .expect("commit first");
        let first_receipt = repo.push_to_local_bare(&bare, "main").expect("push first");
        assert_eq!(first_receipt.branch, "main");
        assert_eq!(first_receipt.old_head, None);
        assert_eq!(first_receipt.new_head, first);
        assert!(
            first_receipt.objects_copied >= 3,
            "commit + tree + blob copied/confirmed"
        );

        std::fs::write(work.join("shared.txt"), b"second push\n").expect("write");
        std::fs::write(work.join("next.txt"), b"new file\n").expect("write");
        let second = repo
            .commit_all_from_worktree("second local-bare push")
            .expect("commit second");
        let second_receipt = repo.push_to_local_bare(&bare, "main").expect("push second");
        assert_eq!(second_receipt.old_head, Some(first));
        assert_eq!(second_receipt.new_head, second);

        let cloned_dir = unique_temp_dir("gix-push-clone");
        let cloned = GixWorkspaceRepo::clone(bare_url, &cloned_dir).expect("gix clone");
        assert_eq!(
            cloned.read_file_at_head("shared.txt").expect("read"),
            Some(b"second push\n".to_vec())
        );
        assert_eq!(
            cloned.read_file_at_head("next.txt").expect("read"),
            Some(b"new file\n".to_vec())
        );

        let _ = std::fs::remove_dir_all(&bare);
        let _ = std::fs::remove_dir_all(&work);
        let _ = std::fs::remove_dir_all(&cloned_dir);
    }

    // W2 transport-send slice: send the prepared receive-pack body through a
    // real local receive-pack server. This uses gix's transport layer (which
    // spawns git-receive-pack for a filesystem remote) and proves the server
    // accepts the pack/ref update before a clone reads the committed tree.
    #[test]
    fn gix_receive_pack_transport_push_round_trips_against_local_bare() {
        let bare = unique_temp_dir("gix-receive-pack-bare");
        gix::init_bare(&bare).expect("init bare with gix");
        let bare_url = bare.to_str().expect("bare path utf8");

        let work = unique_temp_dir("gix-receive-pack-work");
        let repo = GixWorkspaceRepo::init(&work).expect("init gix");
        std::fs::write(work.join("shared.txt"), b"first receive-pack push\n").expect("write");
        let first = repo
            .commit_all_from_worktree("first receive-pack push")
            .expect("commit first");
        let first_receipt = repo
            .push_to_local_receive_pack(&bare, "main")
            .expect("receive-pack first");
        assert_eq!(first_receipt.branch, "main");
        assert_eq!(first_receipt.remote_ref, "refs/heads/main");
        assert_eq!(first_receipt.old_head, None);
        assert_eq!(first_receipt.new_head, first);
        assert!(
            first_receipt
                .advertised_capabilities
                .iter()
                .any(|capability| capability == "report-status"),
            "receive-pack advertised report-status: {:?}",
            first_receipt.advertised_capabilities
        );
        assert_eq!(
            first_receipt.requested_capabilities,
            vec!["report-status".to_string()]
        );
        assert!(first_receipt.status.contains(&"unpack ok".to_string()));
        assert!(first_receipt
            .status
            .contains(&"ok refs/heads/main".to_string()));
        assert!(first_receipt.objects_sent >= 3, "commit + tree + blob sent");

        std::fs::write(work.join("shared.txt"), b"second receive-pack push\n").expect("edit");
        std::fs::write(work.join("next.txt"), b"transport accepted\n").expect("add");
        let second = repo
            .commit_all_from_worktree("second receive-pack push")
            .expect("commit second");
        let second_receipt = repo
            .push_to_local_receive_pack(&bare, "main")
            .expect("receive-pack second");
        assert_eq!(second_receipt.old_head, Some(first));
        assert_eq!(second_receipt.new_head, second);
        assert!(second_receipt.status.contains(&"unpack ok".to_string()));
        assert!(second_receipt
            .status
            .contains(&"ok refs/heads/main".to_string()));

        let cloned_dir = unique_temp_dir("gix-receive-pack-clone");
        let cloned = GixWorkspaceRepo::clone(bare_url, &cloned_dir).expect("gix clone");
        assert_eq!(
            cloned.read_file_at_head("shared.txt").expect("read"),
            Some(b"second receive-pack push\n".to_vec())
        );
        assert_eq!(
            cloned.read_file_at_head("next.txt").expect("read"),
            Some(b"transport accepted\n".to_vec())
        );

        let _ = std::fs::remove_dir_all(&bare);
        let _ = std::fs::remove_dir_all(&work);
        let _ = std::fs::remove_dir_all(&cloned_dir);
    }

    // W2 push-ready protocol slice: prepare the refspec/ref-update plus the
    // minimal object closure, a verified V2 packfile, and a receive-pack request
    // body. The local bare push consumes the same object-closure artifact.
    #[test]
    fn gix_prepare_push_ref_update_reports_refspec_and_minimal_object_closure() {
        let bare = unique_temp_dir("gix-prepared-push-bare");
        gix::init_bare(&bare).expect("init bare with gix");

        let work = unique_temp_dir("gix-prepared-push-work");
        let repo = GixWorkspaceRepo::init(&work).expect("init gix");
        std::fs::write(work.join("shared.txt"), b"first push\n").expect("write");
        let first = repo
            .commit_all_from_worktree("first prepared push")
            .expect("commit first");

        let first_prepared = repo
            .prepare_push_ref_update("main", None)
            .expect("prepare first push");
        assert_eq!(first_prepared.branch, "main");
        assert_eq!(first_prepared.remote_ref, "refs/heads/main");
        assert_eq!(first_prepared.refspec, "refs/heads/main:refs/heads/main");
        assert_eq!(first_prepared.old_head, None);
        assert_eq!(first_prepared.new_head, first);
        assert!(
            first_prepared
                .objects_to_send
                .contains(&first_prepared.new_head),
            "first push sends the new commit object"
        );
        assert!(
            first_prepared.objects_to_send.len() >= 3,
            "commit + tree + blob form the first push closure"
        );
        assert_prepared_packfile_matches_object_closure(&first_prepared, &work);
        let capabilities = ["report-status", "side-band-64k"];
        let first_request = repo
            .prepare_receive_pack_request("main", None, &capabilities)
            .expect("first receive-pack request");
        assert_receive_pack_request_matches_prepared(
            &first_request,
            &first_prepared,
            &"0".repeat(first_prepared.new_head.len()),
            &capabilities,
        );

        repo.push_to_local_bare(&bare, "main")
            .expect("apply first prepared push");
        std::fs::write(work.join("shared.txt"), b"second push\n").expect("edit");
        std::fs::write(work.join("next.txt"), b"new file\n").expect("add");
        let second = repo
            .commit_all_from_worktree("second prepared push")
            .expect("commit second");

        let second_prepared = repo
            .prepare_push_ref_update("main", Some(&first_prepared.new_head))
            .expect("prepare second push");
        assert_eq!(
            second_prepared.old_head,
            Some(first_prepared.new_head.clone())
        );
        assert_eq!(second_prepared.new_head, second);
        assert!(
            second_prepared
                .objects_to_send
                .contains(&second_prepared.new_head),
            "second push sends the new commit object"
        );
        assert!(
            !second_prepared
                .objects_to_send
                .contains(second_prepared.old_head.as_ref().expect("old head")),
            "objects already reachable from the advertised remote head are excluded"
        );
        assert!(
            second_prepared.objects_to_send.len() < first_prepared.objects_to_send.len() + 3,
            "incremental push closure stays bounded to new objects"
        );
        assert_prepared_packfile_matches_object_closure(&second_prepared, &work);
        let second_request = second_prepared
            .receive_pack_request(&capabilities)
            .expect("second receive-pack request");
        assert_receive_pack_request_matches_prepared(
            &second_request,
            &second_prepared,
            second_prepared.old_head.as_deref().expect("old head"),
            &capabilities,
        );

        let _ = std::fs::remove_dir_all(&bare);
        let _ = std::fs::remove_dir_all(&work);
    }

    // A prepared push rejects non-fast-forward updates before any remote write.
    #[test]
    fn gix_prepare_push_ref_update_rejects_non_fast_forward() {
        let root = unique_temp_dir("gix-prepared-push-nff");
        let repo = GixWorkspaceRepo::init(&root).expect("init gix");
        std::fs::write(root.join("shared.txt"), b"base\n").expect("write base");
        repo.commit_all_from_worktree("base").expect("commit base");

        repo.create_branch("remote-main")
            .expect("create remote-shaped branch");
        std::fs::write(root.join("shared.txt"), b"remote side\n").expect("write remote");
        let remote_head = repo
            .commit_all_from_worktree("remote side")
            .expect("commit remote side");

        repo.checkout("main").expect("checkout main");
        std::fs::write(root.join("shared.txt"), b"local side\n").expect("write local");
        let local_head = repo
            .commit_all_from_worktree("local side")
            .expect("commit local side");
        assert_ne!(local_head, remote_head);

        let error = repo
            .prepare_push_ref_update("main", Some(&remote_head))
            .expect_err("non-fast-forward is rejected");
        assert!(
            error.to_string().contains("non-fast-forward push rejected"),
            "error names the push safety gate: {error}"
        );
        assert_eq!(
            repo.head().expect("head"),
            Some(local_head),
            "preparing a rejected push does not move local HEAD"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // W2 gix clone slice: clone through gix from a local bare remote, then read
    // the committed tree through gix. No network and no git CLI in the clone.
    #[test]
    fn gix_clone_from_bare_round_trips_committed_tree() {
        let bare = unique_temp_dir("gix-bare");
        init_bare(&bare);
        let bare_url = bare.to_str().expect("bare path utf8");

        let work = unique_temp_dir("gix-work");
        let repo = WorkspaceRepo::init(&work).expect("init cli");
        std::fs::write(work.join("shared.txt"), b"gix cloned content").expect("write");
        repo.commit_all("seed").expect("commit");
        repo.add_remote("origin", bare_url).expect("remote");
        repo.push("origin", "main").expect("push");

        let cloned_dir = unique_temp_dir("gix-clone");
        let cloned = GixWorkspaceRepo::clone(bare_url, &cloned_dir).expect("gix clone");
        assert_eq!(
            cloned.read_file_at_head("shared.txt").expect("read"),
            Some(b"gix cloned content".to_vec())
        );

        let _ = std::fs::remove_dir_all(&bare);
        let _ = std::fs::remove_dir_all(&work);
        let _ = std::fs::remove_dir_all(&cloned_dir);
    }

    #[derive(Debug)]
    struct CapturedHttpRequest {
        request: String,
        body: String,
    }

    #[derive(Debug)]
    struct CapturedRawHttpRequest {
        request: String,
        body: Vec<u8>,
    }

    fn accept_raw_http_request(
        listener: &TcpListener,
    ) -> (CapturedRawHttpRequest, std::net::TcpStream) {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut request = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = stream.read(&mut buf).expect("read headers");
            assert_ne!(n, 0, "client closed before request headers");
            request.extend_from_slice(&buf[..n]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("header terminator")
            + 4;
        let headers = String::from_utf8_lossy(&request[..header_end]).to_string();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("content length"))
            })
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let n = stream.read(&mut buf).expect("read body");
            assert_ne!(n, 0, "client closed before request body");
            request.extend_from_slice(&buf[..n]);
        }
        let body = request[header_end..header_end + content_length].to_vec();
        (
            CapturedRawHttpRequest {
                request: headers,
                body,
            },
            stream,
        )
    }

    fn write_http_response(
        stream: &mut std::net::TcpStream,
        status: &str,
        content_type: &str,
        body: &[u8],
    ) {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response.as_bytes()).expect("headers");
        stream.write_all(body).expect("body");
    }

    fn pkt_line(data: &[u8]) -> Vec<u8> {
        let mut bytes = format!("{:04x}", data.len() + 4).into_bytes();
        bytes.extend_from_slice(data);
        bytes
    }

    fn pkt_flush() -> Vec<u8> {
        b"0000".to_vec()
    }

    fn receive_pack_advertisement(old_head: &str, remote_ref: &str) -> Vec<u8> {
        receive_pack_advertisement_refs(&[(old_head, remote_ref)])
    }

    fn receive_pack_advertisement_refs(refs: &[(&str, &str)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(pkt_line(b"# service=git-receive-pack\n"));
        bytes.extend(pkt_flush());
        for (idx, (head, remote_ref)) in refs.iter().enumerate() {
            let line = if idx == 0 {
                format!("{head} {remote_ref}\0report-status object-format=sha1\n")
            } else {
                format!("{head} {remote_ref}\n")
            };
            bytes.extend(pkt_line(line.as_bytes()));
        }
        bytes.extend(pkt_flush());
        bytes
    }

    fn receive_pack_status(remote_ref: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(pkt_line(b"unpack ok\n"));
        bytes.extend(pkt_line(format!("ok {remote_ref}\n").as_bytes()));
        bytes.extend(pkt_flush());
        bytes
    }

    fn read_one_http_request(listener: TcpListener) -> CapturedHttpRequest {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut request = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = stream.read(&mut buf).expect("read headers");
            assert_ne!(n, 0, "client closed before request headers");
            request.extend_from_slice(&buf[..n]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("header terminator")
            + 4;
        let headers = String::from_utf8_lossy(&request[..header_end]).to_string();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("content length"))
            })
            .expect("content-length");
        while request.len() < header_end + content_length {
            let n = stream.read(&mut buf).expect("read body");
            assert_ne!(n, 0, "client closed before request body");
            request.extend_from_slice(&buf[..n]);
        }
        let body =
            String::from_utf8_lossy(&request[header_end..header_end + content_length]).to_string();
        let response_body =
            "{\"number\":42,\"html_url\":\"https://github.com/Travis-Gilbert/Theorem/pull/42\"}";
        let response = format!(
            "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).expect("response");
        CapturedHttpRequest {
            request: headers,
            body,
        }
    }

    // W2 smart-HTTP push slice: advertise refs over HTTP, prepare the checked
    // receive-pack request from the advertised old head, post the raw pack body
    // with GitHub-compatible git HTTP auth, and parse the server report-status
    // response. This is the network protocol shape the live GitHub smoke
    // exercises against a real remote/token.
    #[test]
    fn gix_http_receive_pack_push_uses_smart_http_and_git_basic_auth() {
        let work = unique_temp_dir("gix-http-push-work");
        let repo = GixWorkspaceRepo::init(&work).expect("init gix");
        std::fs::write(work.join("shared.txt"), b"remote base\n").expect("write base");
        let remote_head = repo
            .commit_all_from_worktree("remote advertised base")
            .expect("commit base");
        std::fs::write(work.join("shared.txt"), b"http pushed\n").expect("edit");
        std::fs::write(work.join("next.txt"), b"smart http body\n").expect("add");
        let new_head = repo
            .commit_all_from_worktree("smart http push")
            .expect("commit push");
        let expected = repo
            .prepare_push_ref_update("main", Some(&remote_head))
            .expect("expected push");
        let expected_request = expected
            .receive_pack_request(&["report-status"])
            .expect("expected receive-pack request");

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("addr");
        let expected_auth = git_smart_http_authorization("install-token").to_ascii_lowercase();
        let server = std::thread::spawn({
            let expected_request = expected_request.bytes.clone();
            let remote_head = remote_head.clone();
            let expected_auth = expected_auth.clone();
            move || {
                let (get, mut stream) = accept_raw_http_request(&listener);
                let get_lower = get.request.to_ascii_lowercase();
                assert!(
                    get.request
                        .starts_with("GET /repo.git/info/refs?service=git-receive-pack HTTP/1.1"),
                    "advertise request line: {}",
                    get.request.lines().next().unwrap_or_default()
                );
                assert!(get_lower.contains(&format!("authorization: {expected_auth}")));
                assert!(
                    get_lower.contains("accept: application/x-git-receive-pack-advertisement"),
                    "GET headers: {}",
                    get.request
                );
                assert!(
                    !get_lower.contains("git-protocol:"),
                    "v0 receive-pack sender should not opt into protocol v2: {}",
                    get.request
                );
                assert!(get.body.is_empty());
                write_http_response(
                    &mut stream,
                    "200 OK",
                    "application/x-git-receive-pack-advertisement",
                    &receive_pack_advertisement(&remote_head, "refs/heads/main"),
                );

                let (post, mut stream) = accept_raw_http_request(&listener);
                let post_lower = post.request.to_ascii_lowercase();
                assert!(
                    post.request
                        .starts_with("POST /repo.git/git-receive-pack HTTP/1.1"),
                    "receive-pack request line: {}",
                    post.request.lines().next().unwrap_or_default()
                );
                assert!(post_lower.contains(&format!("authorization: {expected_auth}")));
                assert!(
                    post_lower.contains("content-type: application/x-git-receive-pack-request"),
                    "POST headers: {}",
                    post.request
                );
                assert!(post_lower.contains("accept: application/x-git-receive-pack-result"));
                assert_eq!(post.body, expected_request);
                write_http_response(
                    &mut stream,
                    "200 OK",
                    "application/x-git-receive-pack-result",
                    &receive_pack_status("refs/heads/main"),
                );
            }
        });

        let receipt = repo
            .push_to_http_receive_pack(&format!("http://{addr}/repo.git"), "install-token", "main")
            .expect("smart HTTP push");
        server.join().expect("mock server");
        assert_eq!(receipt.remote_url, format!("http://{addr}/repo.git"));
        assert_eq!(receipt.branch, "main");
        assert_eq!(receipt.remote_ref, "refs/heads/main");
        assert_eq!(receipt.old_head, Some(remote_head));
        assert_eq!(receipt.new_head, new_head);
        assert_eq!(receipt.requested_capabilities, vec!["report-status"]);
        assert!(receipt
            .advertised_capabilities
            .contains(&"report-status".to_string()));
        assert_eq!(receipt.status, vec!["unpack ok", "ok refs/heads/main"]);
        assert!(receipt.objects_sent >= 4, "commit + tree + blobs sent");

        let _ = std::fs::remove_dir_all(&work);
    }

    // A new remote branch is not advertised, but the server does advertise refs
    // it already has (for example `main`). Smart-HTTP push must use those refs
    // as known remote objects so a PR branch push sends only the new commit/tree
    // delta instead of the whole repository closure.
    #[test]
    fn gix_http_receive_pack_new_branch_reuses_advertised_remote_heads() {
        let work = unique_temp_dir("gix-http-new-branch-work");
        let repo = GixWorkspaceRepo::init(&work).expect("init gix");
        std::fs::write(work.join("shared.txt"), b"base\n").expect("write base");
        let base = repo
            .commit_all_from_worktree("remote advertised base")
            .expect("commit base");
        repo.create_branch("rustyred-live-smoke")
            .expect("create feature branch");
        std::fs::write(work.join("next.txt"), b"small branch delta\n").expect("add branch file");
        let new_head = repo
            .commit_all_from_worktree("branch delta")
            .expect("commit branch");

        let base_id = gix::ObjectId::from_hex(base.as_bytes()).expect("base hex");
        let local = gix::open(&work).expect("open local");
        let (expected, _) = prepare_push_for_repo_with_known_remote(
            &local,
            "rustyred-live-smoke",
            None,
            &[base_id],
        )
        .expect("expected thin new-branch push");
        let expected_request = expected
            .receive_pack_request(&["report-status"])
            .expect("expected receive-pack request");
        let full_closure = repo
            .prepare_push_ref_update("rustyred-live-smoke", None)
            .expect("full new-branch push");
        assert!(
            expected.objects_to_send.len() < full_closure.objects_to_send.len(),
            "advertised remote base trims new-branch push closure"
        );
        assert!(
            !expected.objects_to_send.contains(&base),
            "remote-advertised base commit is not resent"
        );

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("addr");
        let server = std::thread::spawn({
            let expected_request = expected_request.bytes.clone();
            let base = base.clone();
            move || {
                let (_get, mut stream) = accept_raw_http_request(&listener);
                write_http_response(
                    &mut stream,
                    "200 OK",
                    "application/x-git-receive-pack-advertisement",
                    &receive_pack_advertisement_refs(&[(&base, "refs/heads/main")]),
                );

                let (post, mut stream) = accept_raw_http_request(&listener);
                assert_eq!(post.body, expected_request);
                write_http_response(
                    &mut stream,
                    "200 OK",
                    "application/x-git-receive-pack-result",
                    &receive_pack_status("refs/heads/rustyred-live-smoke"),
                );
            }
        });

        let receipt = repo
            .push_to_http_receive_pack(
                &format!("http://{addr}/repo.git"),
                "install-token",
                "rustyred-live-smoke",
            )
            .expect("smart HTTP new-branch push");
        server.join().expect("mock server");
        assert_eq!(receipt.old_head, None);
        assert_eq!(receipt.new_head, new_head);
        assert_eq!(receipt.objects_sent, expected.packfile.object_count);
        assert_eq!(
            receipt.status,
            vec!["unpack ok", "ok refs/heads/rustyred-live-smoke"]
        );

        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn gix_new_branch_push_from_shallow_clone_stops_at_advertised_remote_head() {
        let source = unique_temp_dir("gix-shallow-source");
        let source_repo = WorkspaceRepo::init(&source).expect("init source");
        std::fs::write(source.join("shared.txt"), b"first\n").expect("write first");
        source_repo.commit_all("first").expect("first commit");
        std::fs::write(source.join("shared.txt"), b"second\n").expect("write second");
        source_repo.commit_all("second").expect("second commit");

        let bare = unique_temp_dir("gix-shallow-bare");
        init_bare(&bare);
        source_repo
            .add_remote("origin", bare.to_str().expect("bare path utf8"))
            .expect("remote");
        source_repo.push("origin", "main").expect("push source");

        let shallow = unique_temp_dir("gix-shallow-clone");
        let output = Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                "--single-branch",
                "--branch",
                "main",
                "--quiet",
                &format!("file://{}", bare.display()),
            ])
            .arg(&shallow)
            .output()
            .expect("spawn shallow clone");
        assert!(
            output.status.success(),
            "shallow clone failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let git = GixWorkspaceRepo::open(&shallow);
        let advertised_base = git.head().expect("head").expect("base head");
        git.create_branch("rustyred-live-smoke")
            .expect("create smoke branch");
        std::fs::write(shallow.join("next.txt"), b"shallow branch delta\n")
            .expect("write branch file");
        let new_head = git
            .commit_all_from_worktree("branch from shallow clone")
            .expect("commit branch");
        let local = gix::open(&shallow).expect("open shallow");
        let base_id = gix::ObjectId::from_hex(advertised_base.as_bytes()).expect("base id");

        let (prepared, _) = prepare_push_for_repo_with_known_remote(
            &local,
            "rustyred-live-smoke",
            None,
            &[base_id],
        )
        .expect("prepare shallow new-branch push");
        assert_eq!(prepared.new_head, new_head);
        assert!(
            !prepared.objects_to_send.contains(&advertised_base),
            "advertised base commit is a traversal boundary, not a required local closure"
        );

        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&bare);
        let _ = std::fs::remove_dir_all(&shallow);
    }

    fn live_env(name: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| panic!("set {name} to run the live GitHub smoke"))
    }

    fn live_token() -> String {
        std::env::var("RUSTYRED_GIT_LIVE_TOKEN")
            .or_else(|_| std::env::var("GITHUB_TOKEN"))
            .expect("set RUSTYRED_GIT_LIVE_TOKEN or GITHUB_TOKEN to run the live GitHub smoke")
    }

    fn parse_github_owner_repo(remote_url: &str) -> Option<(String, String)> {
        let trimmed = remote_url
            .trim()
            .trim_end_matches('/')
            .trim_end_matches(".git");
        let path = trimmed
            .strip_prefix("https://github.com/")
            .or_else(|| trimmed.strip_prefix("http://github.com/"))
            .or_else(|| trimmed.strip_prefix("git@github.com:"))?;
        let (owner, repo) = path.split_once('/')?;
        (!owner.is_empty() && !repo.is_empty()).then(|| (owner.to_string(), repo.to_string()))
    }

    fn clone_live_remote(remote_url: &str, base: &str, token: &str, dir: &Path) {
        let auth_header = git_smart_http_authorization(token);
        let output = Command::new("git")
            .args([
                "-c",
                &format!("http.extraHeader=Authorization: {auth_header}"),
                "clone",
                "--depth",
                "1",
                "--single-branch",
                "--branch",
                base,
                "--quiet",
                remote_url,
            ])
            .arg(dir)
            .output()
            .expect("spawn git clone for live smoke");
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr)
                .replace(token, "<redacted>")
                .replace(&auth_header, "<redacted-auth>");
            panic!("live git clone failed: {}", stderr.trim());
        }
    }

    // Ignored live W2 oracle: prove the mock-covered GitHub-out path against a
    // real remote. Required env:
    //
    // - RUSTYRED_GIT_LIVE_REMOTE_URL, e.g. https://github.com/OWNER/REPO.git
    // - RUSTYRED_GIT_LIVE_TOKEN or GITHUB_TOKEN
    //
    // Optional env:
    //
    // - RUSTYRED_GIT_LIVE_OWNER / RUSTYRED_GIT_LIVE_REPO (parsed from github.com URL if absent)
    // - RUSTYRED_GIT_LIVE_BASE (default: main)
    // - RUSTYRED_GIT_LIVE_API_BASE (default: https://api.github.com)
    //
    // This creates a unique branch and a draft PR, so it is intentionally
    // ignored and should be run only against a disposable or expected test repo.
    #[test]
    #[ignore = "requires a real GitHub remote/token and creates a unique branch plus draft PR"]
    fn live_github_smart_http_push_opens_draft_pr() {
        let remote_url = live_env("RUSTYRED_GIT_LIVE_REMOTE_URL");
        let token = live_token();
        let base = std::env::var("RUSTYRED_GIT_LIVE_BASE").unwrap_or_else(|_| "main".to_string());
        let (parsed_owner, parsed_repo) =
            parse_github_owner_repo(&remote_url).unwrap_or_else(|| {
                (
                    live_env("RUSTYRED_GIT_LIVE_OWNER"),
                    live_env("RUSTYRED_GIT_LIVE_REPO"),
                )
            });
        let owner = std::env::var("RUSTYRED_GIT_LIVE_OWNER").unwrap_or(parsed_owner);
        let repo_name = std::env::var("RUSTYRED_GIT_LIVE_REPO").unwrap_or(parsed_repo);
        let branch = format!(
            "rustyred-live-smoke-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_secs()
        );

        let clone_dir = unique_temp_dir("live-github-push");
        clone_live_remote(&remote_url, &base, &token, &clone_dir);
        let git = GixWorkspaceRepo::open(&clone_dir);
        git.create_branch(&branch)
            .expect("create live smoke branch");
        let marker_dir = clone_dir.join(".rustyred-live-smoke");
        std::fs::create_dir_all(&marker_dir).expect("mkdir marker");
        std::fs::write(
            marker_dir.join(format!("{branch}.txt")),
            format!("RustyRed W2 live GitHub smoke for {branch}\n"),
        )
        .expect("write marker");
        let pushed_head = git
            .commit_all_from_worktree("rustyred live GitHub smoke")
            .expect("commit live smoke marker");
        let push = git
            .push_to_http_receive_pack(&remote_url, &token, &branch)
            .expect("live smart HTTP push");
        assert_eq!(push.branch, branch);
        assert_eq!(push.old_head, None);
        assert_eq!(push.new_head, pushed_head);
        assert!(push.status.contains(&"unpack ok".to_string()));

        let mut client = GitHubClient::new(&token);
        if let Ok(api_base) = std::env::var("RUSTYRED_GIT_LIVE_API_BASE") {
            client = client.with_base_url(api_base);
        }
        let pr = client
            .create_pull_request(
                &owner,
                &repo_name,
                &PullRequestInput::new(
                    format!("RustyRed W2 live smoke {branch}"),
                    &branch,
                    &base,
                )
                .body("Autogenerated ignored live smoke for the RustyRed code workspace W2 GitHub push/PR path.")
                .draft(true)
                .maintainer_can_modify(true),
            )
            .expect("open live draft PR");
        assert!(pr.number > 0);
        assert!(
            pr.html_url.contains(&format!("{owner}/{repo_name}/pull/")),
            "PR URL points at target repo: {}",
            pr.html_url
        );

        let _ = std::fs::remove_dir_all(&clone_dir);
    }

    // W2 acceptance #3: PR-open goes through the documented GitHub REST shape
    // and can be proven against a mock endpoint without a live token.
    #[test]
    fn github_pull_request_open_uses_documented_rest_shape() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("addr");
        let (tx, rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            tx.send(read_one_http_request(listener))
                .expect("send capture");
        });

        let receipt = GitHubClient::new("token-123")
            .with_base_url(format!("http://{addr}"))
            .with_user_agent("rustyred-git-test")
            .create_pull_request(
                "Travis-Gilbert",
                "Theorem",
                &PullRequestInput::new("Spec completion", "feature/spec", "main")
                    .body("Please pull the RustyRed code workspace slice.")
                    .draft(true)
                    .maintainer_can_modify(true),
            )
            .expect("create PR");
        assert_eq!(
            receipt,
            PullRequestReceipt {
                number: 42,
                html_url: "https://github.com/Travis-Gilbert/Theorem/pull/42".to_string(),
            }
        );

        server.join().expect("server join");
        let captured = rx.recv().expect("captured request");
        assert!(
            captured
                .request
                .starts_with("POST /repos/Travis-Gilbert/Theorem/pulls HTTP/1.1"),
            "path/method must match GitHub REST endpoint: {}",
            captured.request
        );
        assert!(
            captured
                .request
                .contains("accept: application/vnd.github+json")
                || captured
                    .request
                    .contains("Accept: application/vnd.github+json"),
            "GitHub media type header present: {}",
            captured.request
        );
        assert!(
            captured.request.contains("authorization: Bearer token-123")
                || captured.request.contains("Authorization: Bearer token-123"),
            "bearer auth header present: {}",
            captured.request
        );
        assert!(
            captured
                .request
                .contains("x-github-api-version: 2026-03-10")
                || captured
                    .request
                    .contains("X-GitHub-Api-Version: 2026-03-10"),
            "GitHub API version header present: {}",
            captured.request
        );
        assert!(
            captured.body.contains("\"title\":\"Spec completion\"")
                && captured.body.contains("\"head\":\"feature/spec\"")
                && captured.body.contains("\"base\":\"main\"")
                && captured.body.contains("\"draft\":true")
                && captured.body.contains("\"maintainer_can_modify\":true"),
            "request JSON carries PR input: {}",
            captured.body
        );
    }

    // W2 acceptance #4: git versions code; graph-version versions graph state.
    // They can reference the same logical workspace moment, but their histories
    // are not derivable from each other and must stay complementary stores.
    #[test]
    fn git_commit_and_graph_version_commit_are_distinct_histories() {
        use rustyred_thg_core::{
            compile_graph_pack, update_graph_ref_cas, GraphCompileOptions, GraphSnapshot,
            GraphVersionRepository, NodeRecord,
        };
        use serde_json::json;

        let root = unique_temp_dir("two-vcs");
        let git = WorkspaceRepo::init(&root).expect("init git");
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        std::fs::write(root.join("src/lib.rs"), b"pub fn answer() -> u8 { 42 }\n")
            .expect("write code");
        let git_commit = git.commit_all("code commit").expect("git commit");

        let snapshot = GraphSnapshot {
            version: 7,
            nodes: vec![NodeRecord::new(
                "file:src/lib.rs",
                ["File"],
                json!({
                    "path": "src/lib.rs",
                    "git_commit": git_commit,
                    "semantic_role": "code"
                }),
            )],
            edges: Vec::new(),
        };
        let pack = compile_graph_pack(
            &snapshot,
            GraphCompileOptions {
                branch: Some("main".to_string()),
                author: Some("rustyred-git-test".to_string()),
                message: Some("graph commit for code workspace".to_string()),
                timestamp_unix_ms: Some(1),
                ..GraphCompileOptions::default()
            },
        );
        let graph_commit = pack.commit.commit_hash.clone();
        let graph_update = update_graph_ref_cas(
            GraphVersionRepository::default(),
            pack.clone(),
            Some("main".to_string()),
            None,
            Some(1),
        )
        .expect("graph ref update");

        assert_eq!(git_commit.len(), 40, "git commit is a git object id");
        assert_ne!(
            git_commit, graph_commit,
            "graph-version commit hash is not the git commit hash"
        );
        assert_eq!(
            pack.commit.graph_version, 7,
            "graph-version carries graph version metadata git cannot replay"
        );
        assert_eq!(
            graph_update.reference.commit_hash, graph_commit,
            "graph ref points to the graph commit"
        );
        assert_eq!(
            git.head().expect("git head").as_deref(),
            Some(git_commit.as_str()),
            "git HEAD remains the code commit"
        );
        assert_eq!(
            git.read_file_at_head("src/lib.rs")
                .expect("read code")
                .as_deref(),
            Some(&b"pub fn answer() -> u8 { 42 }\n"[..]),
            "git reads code bytes from the committed tree"
        );
        assert_eq!(
            graph_update.repository.commits[0].tree_hash, pack.tree.root_hash,
            "graph history points at a Prolly-tree root, not a git tree"
        );
        assert!(
            graph_update
                .repository
                .objects
                .values()
                .any(|object| object.key == "node/file:src/lib.rs"),
            "graph history stores semantic graph objects"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
