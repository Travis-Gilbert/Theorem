//! CA-1: URL -> local clone for code-graph ingestion.
//!
//! Lets the code graph ingest a remote repository by URL without the caller
//! grepping, cloning, or otherwise touching the repo by hand. The tree is
//! shallow-cloned (no history) into a quarantined, deterministically-named
//! tempdir, size-capped, and removed when the returned handle drops.
//!
//! Discipline: we never EXECUTE the cloned tree. `git clone` writes files; the
//! parser then reads them. Untrusted remote code is data here, never a build.

use std::path::{Path, PathBuf};
use std::process::Command;

use rustyred_thg_core::stable_hash;

/// Caps on what a single fetch may pull onto local disk.
#[derive(Clone, Debug)]
pub struct RepoFetchCaps {
    /// Reject (and clean up) a clone whose working tree exceeds this many bytes.
    pub max_total_bytes: u64,
}

impl Default for RepoFetchCaps {
    fn default() -> Self {
        // 512 MiB: large enough for real repositories, small enough to bound a
        // hostile or runaway clone on a constrained runtime.
        Self {
            max_total_bytes: 512 * 1024 * 1024,
        }
    }
}

/// A cloned repository on local disk. Removed on drop unless `keep()` is taken.
#[derive(Debug)]
pub struct FetchedRepo {
    path: PathBuf,
    keep: bool,
}

impl FetchedRepo {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Hand ownership of the on-disk tree to the caller (no auto-cleanup).
    pub fn keep(mut self) -> PathBuf {
        self.keep = true;
        self.path.clone()
    }
}

impl Drop for FetchedRepo {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// A repo-fetch failure. Stringly-typed on purpose: the caller maps it into its
/// own error type (e.g. `CodeIndexError`) at the boundary.
#[derive(Clone, Debug)]
pub struct RepoFetchError {
    pub code: &'static str,
    pub message: String,
}

impl RepoFetchError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RepoFetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RepoFetchError {}

/// Schemes we are willing to hand to `git clone`. Everything else is rejected so
/// a caller cannot smuggle a `git` flag or a local-path traversal through the
/// URL field.
fn is_allowed_url(url: &str) -> bool {
    // A leading '-' would be parsed by git as an option, not a URL.
    if url.starts_with('-') {
        return false;
    }
    const ALLOWED: [&str; 5] = ["https://", "http://", "git://", "ssh://", "file://"];
    ALLOWED.iter().any(|scheme| url.starts_with(scheme)) || url.starts_with("git@")
}

/// Shallow-clone `url` into a quarantined tempdir and return a handle that
/// removes the tree on drop. Caps the on-disk size; never executes the tree.
pub fn fetch_repo(url: &str, caps: &RepoFetchCaps) -> Result<FetchedRepo, RepoFetchError> {
    let url = url.trim();
    if url.is_empty() {
        return Err(RepoFetchError::new("empty_url", "repo url is empty"));
    }
    if !is_allowed_url(url) {
        return Err(RepoFetchError::new(
            "unsupported_url",
            format!("refusing to clone unsupported or unsafe url: {url}"),
        ));
    }

    // Deterministic, collision-stable quarantine dir (no clock/random in the
    // name, so re-fetching the same url reuses the slot rather than leaking).
    let dir = std::env::temp_dir().join(format!("rustyred-code-clone-{}", stable_hash(url)));
    // Always start from a clean slot.
    let _ = std::fs::remove_dir_all(&dir);

    let output = Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--single-branch",
            "--no-tags",
            "--quiet",
            url,
        ])
        .arg(&dir)
        .output()
        .map_err(|err| {
            RepoFetchError::new("git_spawn_failed", format!("could not run git: {err}"))
        })?;

    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&dir);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RepoFetchError::new(
            "git_clone_failed",
            format!("git clone failed for {url}: {}", stderr.trim()),
        ));
    }

    // Handle owns cleanup from here, including the oversize bail-out below.
    let fetched = FetchedRepo {
        path: dir,
        keep: false,
    };

    let size = dir_size(fetched.path());
    if size > caps.max_total_bytes {
        // `fetched` drops here and removes the oversized tree.
        return Err(RepoFetchError::new(
            "clone_too_large",
            format!(
                "cloned tree is {size} bytes, exceeds cap {}",
                caps.max_total_bytes
            ),
        ));
    }

    Ok(fetched)
}

/// Sum of regular-file sizes under `root` (best-effort; ignores unreadable
/// entries). Skips the `.git` dir since only the working tree is ever parsed.
fn dir_size(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let meta = match entry.metadata() {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            if meta.is_dir() {
                if entry.file_name().to_str() == Some(".git") {
                    continue;
                }
                stack.push(entry.path());
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}
