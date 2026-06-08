//! Lane detection: which heads are installed on this machine.
//!
//! A lane is present iff its CLI is on `PATH` (the equivalent of `which claude` /
//! `which codex`). The receiver registers only what is present, so a machine
//! without `codex` never claims Codex-lane jobs (acceptance criterion 3).

use std::path::{Path, PathBuf};

use crate::head::head_adapters;

/// Detect installed lanes against the real `PATH` and filesystem.
pub fn detect_lanes() -> Vec<String> {
    let path_var = std::env::var("PATH").unwrap_or_default();
    detect_lanes_in(&path_var, is_executable)
}

/// Detect installed lanes against an explicit `PATH` value and an executable
/// predicate. Pure: the predicate is injected so this is unit-testable without a
/// real filesystem. Each registered head adapter decides whether it is installed
/// via its own `detect`; the result preserves registry (detect-priority) order.
pub fn detect_lanes_in(path_var: &str, exists: impl Fn(&Path) -> bool) -> Vec<String> {
    head_adapters()
        .iter()
        .filter(|adapter| adapter.detect(path_var, &exists))
        .map(|adapter| adapter.head_id().to_string())
        .collect()
}

/// Locate `program` on `path_var`, returning the first matching path the
/// predicate accepts.
pub fn which_in(path_var: &str, program: &str, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    for dir in std::env::split_paths(path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(program);
        if exists(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Real executable check: a regular file with an owner-execute bit (Unix) or any
/// existing file (non-Unix).
fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn predicate(present: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        let set: HashSet<PathBuf> = present.iter().map(PathBuf::from).collect();
        move |path: &Path| set.contains(path)
    }

    #[test]
    fn detects_both_lanes_when_present() {
        let path = "/usr/local/bin:/usr/bin";
        let exists = predicate(&["/usr/local/bin/claude", "/usr/local/bin/codex"]);
        let lanes = detect_lanes_in(path, exists);
        assert_eq!(lanes, vec!["claude".to_string(), "codex".to_string()]);
    }

    #[test]
    fn detects_only_claude_when_codex_absent() {
        let path = "/usr/local/bin:/usr/bin";
        let exists = predicate(&["/usr/local/bin/claude"]);
        let lanes = detect_lanes_in(path, exists);
        assert_eq!(lanes, vec!["claude".to_string()]);
        assert!(!lanes.contains(&"codex".to_string()));
    }

    #[test]
    fn detects_nothing_on_empty_path() {
        let lanes = detect_lanes_in("", predicate(&[]));
        assert!(lanes.is_empty());
    }

    #[test]
    fn searches_path_in_order() {
        let path = "/a:/b";
        let exists = predicate(&["/b/codex"]);
        assert_eq!(
            which_in(path, "codex", &exists),
            Some(PathBuf::from("/b/codex"))
        );
        assert_eq!(which_in(path, "claude", &exists), None);
    }
}
