//! Spawn planning: how the receiver launches a head as a child process.
//!
//! The plan is built as pure data ([`SpawnPlan`]) and only then turned into a
//! [`std::process::Command`], so the spawn shape (program, args, cwd, stripped
//! env) is unit-testable without executing anything.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::head::adapter_for;

/// The env var that must NEVER reach the child. An API key in the child env
/// silently wins precedence over the CLI's subscription login and bills metered
/// rates; the whole point of local spawn is to draw on the existing login.
pub const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";

/// Build the intent prompt handed to the spawned head. Verbatim contract from
/// the dispatch-queue HANDOFF: implement the spec, on branch `job/{job_id}`,
/// push + PR + call job_complete, no scope expansion.
pub fn build_intent(spec_ref: &str, job_id: &str) -> String {
    format!(
        "Implement {spec_ref} fully as written. This is {job_id}. Work on branch job/{job_id}. \
When done: push the branch, open a PR with the local gh login if present, and call job_complete \
with the outcome, pr_ref, and receipts. Do not expand scope beyond the spec."
    )
}

/// A fully-resolved plan for spawning a head, as pure data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnPlan {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Env vars to remove from the inherited environment before exec.
    pub strip_env: Vec<String>,
}

/// Build the spawn plan for a lane by dispatching to its head adapter. Returns
/// `None` for an unregistered lane. The per-head shape (program, args, env
/// policy) lives in `head.rs`:
///
///   - Claude lane: `claude -p "<intent>" --permission-mode acceptEdits`.
///   - Codex lane:  `codex exec "<intent>"`.
pub fn build_spawn_plan(lane: &str, intent: &str, worktree: &Path) -> Option<SpawnPlan> {
    adapter_for(lane).map(|adapter| adapter.spawn_plan(intent, worktree))
}

/// Turn a plan into a runnable [`Command`] (inherits the environment minus the
/// stripped keys).
pub fn command_from_plan(plan: &SpawnPlan) -> Command {
    let mut command = Command::new(&plan.program);
    command.args(&plan.args);
    command.current_dir(&plan.cwd);
    for key in &plan.strip_env {
        command.env_remove(key);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_carries_spec_ref_and_job_id_on_the_right_branch() {
        let intent = build_intent("docs/plans/theorem-desktop/HANDOFF.md", "job-001");
        assert!(intent.contains("docs/plans/theorem-desktop/HANDOFF.md"));
        assert!(intent.contains("job-001"));
        assert!(intent.contains("branch job/job-001"));
        assert!(intent.contains("call job_complete"));
        assert!(intent.contains("Do not expand scope"));
    }

    #[test]
    fn claude_plan_uses_print_mode_and_strips_api_key() {
        let plan = build_spawn_plan("claude", "do the thing", Path::new("/repos/theorem")).unwrap();
        assert_eq!(plan.program, "claude");
        assert_eq!(
            plan.args,
            vec![
                "-p".to_string(),
                "do the thing".to_string(),
                "--permission-mode".to_string(),
                "acceptEdits".to_string()
            ]
        );
        assert_eq!(plan.cwd, PathBuf::from("/repos/theorem"));
        assert!(plan.strip_env.contains(&ANTHROPIC_API_KEY.to_string()));
    }

    #[test]
    fn codex_plan_uses_exec() {
        let plan = build_spawn_plan("codex", "do the thing", Path::new("/repos/theorem")).unwrap();
        assert_eq!(plan.program, "codex");
        assert_eq!(plan.args, vec!["exec".to_string(), "do the thing".to_string()]);
        assert!(plan.strip_env.contains(&ANTHROPIC_API_KEY.to_string()));
    }

    #[test]
    fn unknown_lane_has_no_plan() {
        assert!(build_spawn_plan("gemini", "x", Path::new("/repos")).is_none());
    }

    #[test]
    fn command_strips_api_key_from_child_env() {
        let plan = build_spawn_plan("claude", "x", Path::new("/repos")).unwrap();
        let command = command_from_plan(&plan);
        // The child env must not carry ANTHROPIC_API_KEY even if the parent has it.
        let removed = command
            .get_envs()
            .any(|(key, value)| key == ANTHROPIC_API_KEY && value.is_none());
        assert!(removed, "ANTHROPIC_API_KEY must be explicitly removed from child env");
    }
}
