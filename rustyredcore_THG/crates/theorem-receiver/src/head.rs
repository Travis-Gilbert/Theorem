//! Head adapters: each foreign model behind one uniform interface.
//!
//! The "FDW idea applied to models" (synthesis doc_ae17ec52822651d6, builder note
//! msg_e228150b344a1525): a head adapter wraps a foreign model's CLI behind one
//! dispatch interface, so a future head lands as config plus one impl instead of a
//! rewrite of lane detection and spawn planning.
//!
//! The seven adapter fields the synthesis names map to trait methods:
//!   - `head_id`           -> [`HeadAdapter::head_id`]
//!   - `detect`            -> [`HeadAdapter::detect`]        (default: program on PATH)
//!   - `spawn_cmd`         -> [`HeadAdapter::build_args`] / [`HeadAdapter::spawn_plan`]
//!   - `intent_template`   -> [`HeadAdapter::intent_template`] (default: shared dispatch intent)
//!   - `env_policy`        -> [`HeadAdapter::strip_env`]    (default: strip ANTHROPIC_API_KEY)
//!   - `receipt_parser`    -> [`HeadAdapter::parse_receipt`] (default: exit + stdout tail)
//!   - `capability_priors` -> [`HeadAdapter::capability_priors`] (default: none here)
//!
//! Only the first three are usually overridden. `capability_priors` is a thin
//! default on purpose: trust tiers and selection priors live in the Ensemble
//! registry (ensemble_register / ensemble_select), the harness plugin system; this
//! adapter does not rebuild a parallel one.

use std::path::Path;

use serde_json::{json, Value};
use theorem_harness_core::{LANE_CLAUDE, LANE_CODEX};

use crate::lanes::which_in;
use crate::spawn::{build_intent, SpawnPlan, ANTHROPIC_API_KEY};

/// One foreign head behind a uniform dispatch interface. A new head implements
/// the three required methods (`head_id`, `program`, `build_args`) and inherits
/// detection, env policy, intent framing, and receipt parsing from the defaults.
pub trait HeadAdapter: Sync {
    /// Stable lane / head id (matches `theorem_harness_core` `LANE_*` and the
    /// `TargetHead` the queue targets).
    fn head_id(&self) -> &'static str;

    /// The executable to look for on `PATH` (the common `detect` input).
    fn program(&self) -> &'static str;

    /// Build the argv after the program for an intent (`spawn_cmd`).
    fn build_args(&self, intent: &str) -> Vec<String>;

    /// Env vars to strip from the child before exec (`env_policy`). Default strips
    /// the metered-billing key, which would otherwise win precedence over the
    /// CLI's subscription login.
    fn strip_env(&self) -> Vec<String> {
        vec![ANTHROPIC_API_KEY.to_string()]
    }

    /// Frame the intent prompt for this head (`intent_template`). Default is the
    /// shared dispatch intent (implement the spec on branch job/{job_id}, push +
    /// PR + call job_complete, no scope expansion).
    fn intent_template(&self, spec_ref: &str, job_id: &str) -> String {
        build_intent(spec_ref, job_id)
    }

    /// Whether this head is installed (`detect`). Default: the program is on the
    /// given `PATH` and accepted by the predicate.
    fn detect(&self, path_var: &str, exists: &dyn Fn(&Path) -> bool) -> bool {
        which_in(path_var, self.program(), exists).is_some()
    }

    /// Parse the child's exit code and stdout tail into a fitness receipt
    /// (`receipt_parser`). Default is the generic receiver fallback shape.
    fn parse_receipt(&self, exit_code: Option<i32>, stdout_tail: &str) -> Value {
        json!({
            "source": "receiver_fallback",
            "lane": self.head_id(),
            "exit_code": exit_code,
            "stdout_tail": stdout_tail,
        })
    }

    /// Capability hints for the router (`capability_priors`). Thin by design: the
    /// authoritative priors and trust tiers live in the Ensemble registry, not
    /// here. Default: none.
    fn capability_priors(&self) -> &'static [&'static str] {
        &[]
    }

    /// Assemble the full spawn plan for this head. Composed from `program`,
    /// `build_args`, and `strip_env`; heads rarely override this.
    fn spawn_plan(&self, intent: &str, worktree: &Path) -> SpawnPlan {
        SpawnPlan {
            program: self.program().to_string(),
            args: self.build_args(intent),
            cwd: worktree.to_path_buf(),
            strip_env: self.strip_env(),
        }
    }
}

/// Claude Code head: `claude -p "<intent>" --permission-mode acceptEdits`.
struct ClaudeHead;

impl HeadAdapter for ClaudeHead {
    fn head_id(&self) -> &'static str {
        LANE_CLAUDE
    }

    fn program(&self) -> &'static str {
        "claude"
    }

    fn build_args(&self, intent: &str) -> Vec<String> {
        vec![
            "-p".to_string(),
            intent.to_string(),
            "--permission-mode".to_string(),
            "acceptEdits".to_string(),
        ]
    }
}

/// Codex head: `codex exec "<intent>"`.
struct CodexHead;

impl HeadAdapter for CodexHead {
    fn head_id(&self) -> &'static str {
        LANE_CODEX
    }

    fn program(&self) -> &'static str {
        "codex"
    }

    fn build_args(&self, intent: &str) -> Vec<String> {
        vec!["exec".to_string(), intent.to_string()]
    }
}

static CLAUDE_HEAD: ClaudeHead = ClaudeHead;
static CODEX_HEAD: CodexHead = CodexHead;
static ADAPTERS: [&dyn HeadAdapter; 2] = [&CLAUDE_HEAD, &CODEX_HEAD];

/// The registered head adapters, in claim/detect priority order. Add a head by
/// appending one entry (config + one impl).
pub fn head_adapters() -> &'static [&'static dyn HeadAdapter] {
    &ADAPTERS
}

/// The adapter for a lane / head id, if registered.
pub fn adapter_for(head_id: &str) -> Option<&'static dyn HeadAdapter> {
    head_adapters()
        .iter()
        .copied()
        .find(|adapter| adapter.head_id() == head_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    #[test]
    fn registry_resolves_known_heads() {
        assert_eq!(adapter_for("claude").unwrap().head_id(), "claude");
        assert_eq!(adapter_for("codex").unwrap().head_id(), "codex");
        assert!(adapter_for("gemini").is_none());
        // Detect order is stable: claude then codex.
        let ids: Vec<_> = head_adapters().iter().map(|a| a.head_id()).collect();
        assert_eq!(ids, vec!["claude", "codex"]);
    }

    #[test]
    fn claude_adapter_spawn_plan() {
        let plan = adapter_for("claude")
            .unwrap()
            .spawn_plan("do the thing", Path::new("/repos/theorem"));
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
        assert!(plan.strip_env.contains(&ANTHROPIC_API_KEY.to_string()));
    }

    #[test]
    fn codex_adapter_spawn_plan() {
        let plan = adapter_for("codex")
            .unwrap()
            .spawn_plan("do the thing", Path::new("/repos/theorem"));
        assert_eq!(plan.program, "codex");
        assert_eq!(plan.args, vec!["exec".to_string(), "do the thing".to_string()]);
    }

    #[test]
    fn detect_uses_the_program_on_path() {
        let present: HashSet<PathBuf> = ["/usr/local/bin/claude"].iter().map(PathBuf::from).collect();
        let exists = |path: &Path| present.contains(path);
        assert!(adapter_for("claude").unwrap().detect("/usr/local/bin", &exists));
        assert!(!adapter_for("codex").unwrap().detect("/usr/local/bin", &exists));
    }

    #[test]
    fn default_parse_receipt_shape() {
        let receipt = adapter_for("claude").unwrap().parse_receipt(Some(1), "panic: boom");
        assert_eq!(receipt["source"], json!("receiver_fallback"));
        assert_eq!(receipt["lane"], json!("claude"));
        assert_eq!(receipt["exit_code"], json!(1));
        assert_eq!(receipt["stdout_tail"], json!("panic: boom"));
        // A signal-terminated child has no exit code.
        let signal = adapter_for("codex").unwrap().parse_receipt(None, "");
        assert_eq!(signal["exit_code"], Value::Null);
    }

    #[test]
    fn default_intent_template_carries_spec_and_job() {
        let intent = adapter_for("claude")
            .unwrap()
            .intent_template("docs/plans/x/HANDOFF.md", "job-001");
        assert!(intent.contains("docs/plans/x/HANDOFF.md"));
        assert!(intent.contains("branch job/job-001"));
    }

    // Proves the synthesis promise: a new head is config plus one impl. This fake
    // head overrides only the three required methods and inherits everything else.
    struct GeminiHead;
    impl HeadAdapter for GeminiHead {
        fn head_id(&self) -> &'static str {
            "gemini"
        }
        fn program(&self) -> &'static str {
            "gemini"
        }
        fn build_args(&self, intent: &str) -> Vec<String> {
            vec!["run".to_string(), intent.to_string()]
        }
    }

    #[test]
    fn new_head_is_config_plus_one_impl() {
        let head = GeminiHead;
        let plan = head.spawn_plan("task", Path::new("/repos"));
        assert_eq!(plan.program, "gemini");
        assert_eq!(plan.args, vec!["run".to_string(), "task".to_string()]);
        // Inherited defaults: env policy, intent template, receipt parser.
        assert!(plan.strip_env.contains(&ANTHROPIC_API_KEY.to_string()));
        assert!(head.intent_template("spec", "job-x").contains("job-x"));
        assert_eq!(head.parse_receipt(Some(0), "ok")["lane"], json!("gemini"));
        assert!(head.capability_priors().is_empty());
    }
}
