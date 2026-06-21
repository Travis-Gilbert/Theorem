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

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Value};
use theorem_harness_core::{LANE_CLAUDE, LANE_CODEX};

use crate::config::{
    HeadRuntimeRecipe, ModelBackendConfig, ModelBackendKind, ProviderSeamConfig, ProviderWireMode,
};
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
    /// shared dispatch v2 intent (note receipts with job_note, archive with
    /// job_archive when done).
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadRuntimePlan {
    pub spawn: SpawnPlan,
    pub env: BTreeMap<String, String>,
    pub model_backend: String,
    pub backend_kind: ModelBackendKind,
    pub wire_mode: ProviderWireMode,
    pub sandbox: bool,
}

pub fn runtime_plan_from_recipe(
    recipe: &HeadRuntimeRecipe,
    backend: &ModelBackendConfig,
    provider_seam: &ProviderSeamConfig,
    intent: &str,
    worktree: &Path,
) -> HeadRuntimePlan {
    let wire_mode = recipe_wire_mode(recipe, backend);
    let env = runtime_env(recipe, backend, provider_seam, wire_mode);
    HeadRuntimePlan {
        spawn: SpawnPlan {
            program: recipe.runtime_binary.clone(),
            args: runtime_args(recipe, intent),
            cwd: worktree.to_path_buf(),
            strip_env: Vec::new(),
        },
        env,
        model_backend: recipe.model_backend.clone(),
        backend_kind: backend.kind.clone(),
        wire_mode,
        sandbox: recipe.sandbox,
    }
}

fn recipe_wire_mode(recipe: &HeadRuntimeRecipe, backend: &ModelBackendConfig) -> ProviderWireMode {
    if recipe.wire_mode == ProviderWireMode::ChatCompletions
        && backend.wire_mode != ProviderWireMode::ChatCompletions
    {
        backend.wire_mode
    } else {
        recipe.wire_mode
    }
}

fn runtime_args(recipe: &HeadRuntimeRecipe, intent: &str) -> Vec<String> {
    if !recipe.args.is_empty() {
        return recipe
            .args
            .iter()
            .map(|arg| arg.replace("{intent}", intent))
            .collect();
    }
    match recipe.runtime_binary.trim() {
        "claude" | "claude-code" => vec![
            "-p".to_string(),
            intent.to_string(),
            "--permission-mode".to_string(),
            "acceptEdits".to_string(),
        ],
        "codex" => vec!["exec".to_string(), intent.to_string()],
        "aider" => vec!["--message".to_string(), intent.to_string()],
        "openhands" => vec!["run".to_string(), intent.to_string()],
        _ => vec![intent.to_string()],
    }
}

fn runtime_env(
    recipe: &HeadRuntimeRecipe,
    backend: &ModelBackendConfig,
    provider_seam: &ProviderSeamConfig,
    wire_mode: ProviderWireMode,
) -> BTreeMap<String, String> {
    let mut env = recipe.env.clone();
    env.insert(
        "THEOREM_MODEL_BACKEND_KIND".to_string(),
        match backend.kind {
            ModelBackendKind::Single => "single",
            ModelBackendKind::Composed => "composed",
        }
        .to_string(),
    );
    env.insert(
        "THEOREM_PROVIDER_WIRE_MODE".to_string(),
        match wire_mode {
            ProviderWireMode::ChatCompletions => "chat_completions",
            ProviderWireMode::Responses => "responses",
            ProviderWireMode::Messages => "messages",
        }
        .to_string(),
    );
    if let Some(model) = backend
        .model
        .as_deref()
        .or_else(|| {
            backend
                .provider
                .as_deref()
                .and_then(|provider| provider_seam.models.get(provider).map(String::as_str))
        })
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        env.entry("OPENAI_MODEL".to_string())
            .or_insert_with(|| model.to_string());
    }
    if let Some(base_url) = backend
        .base_url
        .as_deref()
        .or(provider_seam.base_url.as_deref())
        .map(str::trim)
        .filter(|base_url| !base_url.is_empty())
    {
        env.entry("OPENAI_API_BASE".to_string())
            .or_insert_with(|| base_url.trim_end_matches('/').to_string());
    }
    if let Some(endpoint) = backend
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|base_url| !base_url.is_empty())
        .map(str::to_string)
        .or_else(|| provider_seam.endpoint_for_wire_mode(wire_mode))
    {
        env.entry("THEOREM_PROVIDER_ENDPOINT".to_string())
            .or_insert(endpoint);
    }
    if let Some(key_env) = backend
        .credential_env
        .as_deref()
        .or(provider_seam.api_key_env.as_deref())
        .map(str::trim)
        .filter(|key_env| !key_env.is_empty())
    {
        env.entry("OPENAI_API_KEY_ENV".to_string())
            .or_insert_with(|| key_env.to_string());
    }
    if let Some(binding_id) = backend
        .composed_binding_id
        .as_deref()
        .map(str::trim)
        .filter(|binding_id| !binding_id.is_empty())
    {
        env.entry("THEOREM_COMPOSED_AGENT_BINDING_ID".to_string())
            .or_insert_with(|| binding_id.to_string());
    }
    env
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
        assert_eq!(
            plan.args,
            vec!["exec".to_string(), "do the thing".to_string()]
        );
    }

    #[test]
    fn detect_uses_the_program_on_path() {
        let present: HashSet<PathBuf> = ["/usr/local/bin/claude"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let exists = |path: &Path| present.contains(path);
        assert!(adapter_for("claude")
            .unwrap()
            .detect("/usr/local/bin", &exists));
        assert!(!adapter_for("codex")
            .unwrap()
            .detect("/usr/local/bin", &exists));
    }

    #[test]
    fn default_parse_receipt_shape() {
        let receipt = adapter_for("claude")
            .unwrap()
            .parse_receipt(Some(1), "panic: boom");
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
        assert!(intent.contains("job-001"));
        assert!(intent.contains("job_note"));
        assert!(intent.contains("job_archive"));
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

    #[test]
    fn runtime_recipe_builds_sandbox_codex_plan_with_responses_seam() {
        let recipe = HeadRuntimeRecipe {
            runtime_binary: "codex".to_string(),
            model_backend: "codex_single".to_string(),
            wire_mode: ProviderWireMode::Responses,
            sandbox: true,
            env: BTreeMap::new(),
            args: Vec::new(),
        };
        let backend = ModelBackendConfig {
            kind: ModelBackendKind::Single,
            provider: Some("openai".to_string()),
            model: Some("gpt-4.1-mini".to_string()),
            credential_env: Some("OPENAI_API_KEY".to_string()),
            base_url: None,
            wire_mode: ProviderWireMode::Responses,
            composed_binding_id: None,
        };
        let seam = ProviderSeamConfig {
            base_url: Some("http://litellm.internal:4000".to_string()),
            api_key_env: None,
            ..ProviderSeamConfig::default()
        };

        let plan = runtime_plan_from_recipe(
            &recipe,
            &backend,
            &seam,
            "change the file",
            Path::new("/workspace/repo"),
        );

        assert_eq!(plan.spawn.program, "codex");
        assert_eq!(plan.spawn.args, vec!["exec", "change the file"]);
        assert!(plan.sandbox);
        assert_eq!(plan.wire_mode, ProviderWireMode::Responses);
        assert_eq!(
            plan.env["THEOREM_PROVIDER_ENDPOINT"],
            "http://litellm.internal:4000/v1/responses"
        );
        assert_eq!(plan.env["OPENAI_MODEL"], "gpt-4.1-mini");
        assert_eq!(plan.env["OPENAI_API_KEY_ENV"], "OPENAI_API_KEY");
    }

    #[test]
    fn runtime_recipe_can_target_composed_backend() {
        let recipe = HeadRuntimeRecipe {
            runtime_binary: "aider".to_string(),
            model_backend: "agent_consensus".to_string(),
            wire_mode: ProviderWireMode::ChatCompletions,
            sandbox: true,
            env: BTreeMap::new(),
            args: vec!["--message".to_string(), "{intent}".to_string()],
        };
        let backend = ModelBackendConfig {
            kind: ModelBackendKind::Composed,
            provider: None,
            model: None,
            credential_env: None,
            base_url: Some("http://receiver.local/composed-agent".to_string()),
            wire_mode: ProviderWireMode::ChatCompletions,
            composed_binding_id: Some("agent:theorem".to_string()),
        };

        let plan = runtime_plan_from_recipe(
            &recipe,
            &backend,
            &ProviderSeamConfig::default(),
            "repair tests",
            Path::new("/workspace/repo"),
        );

        assert_eq!(plan.backend_kind, ModelBackendKind::Composed);
        assert_eq!(plan.spawn.args, vec!["--message", "repair tests"]);
        assert_eq!(
            plan.env["THEOREM_COMPOSED_AGENT_BINDING_ID"],
            "agent:theorem"
        );
    }
}
