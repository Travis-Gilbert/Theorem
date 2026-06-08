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
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use rustyred_thg_core::stable_hash;

pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const ABSOLUTE_MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const DEFAULT_CLONE_TIMEOUT_MS: u64 = 20_000;
const MAX_TOTAL_BYTES_ENV: [&str; 2] = [
    "THEOREM_CODE_FETCH_MAX_TOTAL_BYTES",
    "RUSTYRED_THG_CODE_FETCH_MAX_TOTAL_BYTES",
];

/// Caps on what a single fetch may pull onto local disk.
#[derive(Clone, Debug)]
pub struct RepoFetchCaps {
    /// Reject (and clean up) a clone whose working tree exceeds this many bytes.
    pub max_total_bytes: u64,
    /// Kill `git clone` if it stalls past this many milliseconds.
    pub clone_timeout_ms: u64,
}

impl Default for RepoFetchCaps {
    fn default() -> Self {
        Self {
            max_total_bytes: configured_default_max_total_bytes(),
            clone_timeout_ms: DEFAULT_CLONE_TIMEOUT_MS,
        }
    }
}

impl RepoFetchCaps {
    pub fn from_requested(max_total_bytes: u64) -> Self {
        let mut caps = Self::default();
        if max_total_bytes > 0 {
            caps.max_total_bytes = max_total_bytes.min(ABSOLUTE_MAX_TOTAL_BYTES);
        }
        caps
    }
}

fn configured_default_max_total_bytes() -> u64 {
    MAX_TOTAL_BYTES_ENV
        .iter()
        .find_map(|key| {
            std::env::var(key)
                .ok()
                .and_then(|raw| raw.trim().parse::<u64>().ok())
                .filter(|bytes| *bytes > 0)
        })
        .unwrap_or(DEFAULT_MAX_TOTAL_BYTES)
        .min(ABSOLUTE_MAX_TOTAL_BYTES)
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

/// True when a caller-supplied repository location should be treated as a git
/// clone target instead of a local filesystem path.
pub fn is_fetchable_repo_url(url: &str) -> bool {
    is_allowed_url(url.trim())
}

/// Shallow-clone `url` into a quarantined tempdir and return a handle that
/// removes the tree on drop. Caps the on-disk size; never executes the tree.
pub fn fetch_repo(url: &str, caps: &RepoFetchCaps) -> Result<FetchedRepo, RepoFetchError> {
    fetch_repo_with_git(url, caps, Path::new("git"))
}

fn fetch_repo_with_git(
    url: &str,
    caps: &RepoFetchCaps,
    git_binary: &Path,
) -> Result<FetchedRepo, RepoFetchError> {
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

    let clone = match run_git_clone(git_binary, url, &dir, caps.clone_timeout_ms) {
        Ok(clone) => clone,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(err);
        }
    };
    if !clone.status.success() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(RepoFetchError::new(
            "git_clone_failed",
            format!("git clone failed for {url}: {}", clone.stderr.trim()),
        ));
    }

    if !dir.is_dir() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(RepoFetchError::new(
            "git_clone_failed",
            format!("git clone for {url} did not create {}", dir.display()),
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

struct GitCloneOutput {
    status: ExitStatus,
    stderr: String,
}

fn run_git_clone(
    git_binary: &Path,
    url: &str,
    dir: &Path,
    timeout_ms: u64,
) -> Result<GitCloneOutput, RepoFetchError> {
    let stderr_path = std::env::temp_dir().join(format!(
        "rustyred-code-clone-stderr-{}",
        stable_hash(json_safe_path(dir))
    ));
    let stdout_path = std::env::temp_dir().join(format!(
        "rustyred-code-clone-stdout-{}",
        stable_hash(json_safe_path(dir))
    ));
    let stderr_file = std::fs::File::create(&stderr_path).map_err(|err| {
        RepoFetchError::new(
            "git_spawn_failed",
            format!("could not create git stderr capture: {err}"),
        )
    })?;
    let stdout_file = std::fs::File::create(&stdout_path).map_err(|err| {
        RepoFetchError::new(
            "git_spawn_failed",
            format!("could not create git stdout capture: {err}"),
        )
    })?;

    let mut child = Command::new(git_binary)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "/bin/true")
        .env("SSH_ASKPASS", "/bin/true")
        .env(
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new",
        )
        .args([
            "-c",
            "credential.helper=",
            "-c",
            "http.lowSpeedLimit=1",
            "-c",
            "http.lowSpeedTime=10",
            "clone",
            "--depth",
            "1",
            "--single-branch",
            "--no-tags",
            "--quiet",
            url,
        ])
        .arg(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .map_err(|err| {
            RepoFetchError::new("git_spawn_failed", format!("could not run git: {err}"))
        })?;

    let timeout = Duration::from_millis(timeout_ms.max(1));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let stderr = read_lossy(&stderr_path);
                    let _ = std::fs::remove_file(&stderr_path);
                    let _ = std::fs::remove_file(&stdout_path);
                    return Err(RepoFetchError::new(
                        "git_clone_timeout",
                        format!(
                            "git clone timed out after {}ms for {url}: {}",
                            timeout.as_millis(),
                            stderr.trim()
                        ),
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&stderr_path);
                let _ = std::fs::remove_file(&stdout_path);
                return Err(RepoFetchError::new(
                    "git_wait_failed",
                    format!("could not wait for git clone: {err}"),
                ));
            }
        }
    };

    let stderr = read_lossy(&stderr_path);
    let _ = std::fs::remove_file(&stderr_path);
    let _ = std::fs::remove_file(&stdout_path);

    Ok(GitCloneOutput { status, stderr })
}

fn read_lossy(path: &Path) -> String {
    std::fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
        .unwrap_or_default()
}

fn json_safe_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
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

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn unique_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rustyred-fetch-test-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[cfg(unix)]
    fn write_fake_git(name: &str, body: &str) -> (PathBuf, PathBuf) {
        let dir = unique_dir(name);
        fs::create_dir_all(&dir).unwrap();
        let git = dir.join("git");
        fs::write(&git, body).unwrap();
        let mut perms = fs::metadata(&git).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&git, perms).unwrap();
        (dir, git)
    }

    #[test]
    #[cfg(unix)]
    fn fetch_repo_sets_noninteractive_git_environment() {
        let (script_dir, fake_git) = write_fake_git(
            "env",
            r#"#!/bin/sh
if [ "$GIT_TERMINAL_PROMPT" != "0" ]; then echo "missing GIT_TERMINAL_PROMPT" >&2; exit 11; fi
if [ "$GIT_ASKPASS" != "/bin/true" ]; then echo "missing GIT_ASKPASS" >&2; exit 12; fi
if [ "$SSH_ASKPASS" != "/bin/true" ]; then echo "missing SSH_ASKPASS" >&2; exit 13; fi
case " $* " in
  *" -c credential.helper= "*) ;;
  *) echo "missing credential helper reset: $*" >&2; exit 14 ;;
esac
case " $* " in
  *" clone "*) ;;
  *) echo "missing clone verb: $*" >&2; exit 15 ;;
esac
case " $* " in
  *" --depth 1 "*) ;;
  *) echo "missing shallow depth: $*" >&2; exit 16 ;;
esac
last=""
for arg in "$@"; do last="$arg"; done
mkdir -p "$last"
printf 'package main\nfunc main() {}\n' > "$last/main.go"
"#,
        );
        let fetched = fetch_repo_with_git(
            "https://github.com/example/tiny.git",
            &RepoFetchCaps {
                clone_timeout_ms: 1_000,
                ..RepoFetchCaps::default()
            },
            &fake_git,
        )
        .unwrap();

        assert!(fetched.path().join("main.go").is_file());
        drop(fetched);
        fs::remove_dir_all(script_dir).ok();
    }

    #[test]
    #[cfg(unix)]
    fn fetch_repo_times_out_hung_git_clone() {
        let (script_dir, fake_git) = write_fake_git(
            "timeout",
            r#"#!/bin/sh
echo "waiting forever" >&2
sleep 5
"#,
        );
        let url = "https://github.com/example/hung.git";
        let clone_dir =
            std::env::temp_dir().join(format!("rustyred-code-clone-{}", stable_hash(url)));
        let started = Instant::now();
        let err = fetch_repo_with_git(
            url,
            &RepoFetchCaps {
                clone_timeout_ms: 50,
                ..RepoFetchCaps::default()
            },
            &fake_git,
        )
        .unwrap_err();

        assert_eq!(err.code, "git_clone_timeout");
        assert!(started.elapsed() < Duration::from_secs(2), "{err}");
        assert!(!clone_dir.exists());
        fs::remove_dir_all(script_dir).ok();
    }
}
