//! Single-head (any configured head) live provider smoke.
//!
//! The tracer goal is "pick a configured head, type a task, a real turn comes
//! back." This test proves the provider path directly through
//! `RealHeadInvoker`/`ProviderHeadInvoker` and the live provider HTTP profiles
//! in `head_invoker/api.rs` (anthropic, deepseek, mistral, minimax, zhipu,
//! openai, ai21, gemma).
//!
//! It lowers the bar from the three-key
//! `composed_agent::tests::live_provider_invoker_runs_three_head_binding_when_enabled`
//! (which requires deepseek + mistral + minimax) to whatever head(s) you have a
//! key for - one is enough. The composed-agent consensus loop still requires at
//! least two reasoning heads; this smoke is only the live-provider wiring proof.
//!
//! Run it (Mistral):
//!   THEOREM_LIVE_PROVIDER_TEST=1 THEOREM_AGENT_HEADS=mistral MISTRAL_API_KEY=... \
//!   cargo test -p theorem-harness-runtime --test live_single_head_smoke -- --ignored --nocapture
//!
//! DeepSeek:      THEOREM_AGENT_HEADS=deepseek DEEPSEEK_API_KEY=...
//! Two heads:     THEOREM_AGENT_HEADS=mistral,deepseek (also lets the >=2-head
//!                consensus gate publish, not just answer).
//! Local Gemma:   THEOREM_AGENT_HEADS=gemma THEOREM_LOCAL_OPENAI_URL=http://127.0.0.1:8080/v1/chat/completions
//!                (THEOREM_AGENT_HEAD_GEMMA_TRANSPORT=local, no key)
//!
//! Without THEOREM_LIVE_PROVIDER_TEST=1 it self-skips, so the default offline
//! suite never makes a network call.

use theorem_harness_core::{
    AgentHeadRegistry, GroundedClaim, HeadInvocationKind, HeadInvocationRequest, HeadInvoker,
    HeadKind,
};
use theorem_harness_runtime::{default_theorem_binding, ProviderHeadInvoker};

#[test]
#[ignore = "requires THEOREM_LIVE_PROVIDER_TEST=1, THEOREM_AGENT_HEADS, and a real provider key"]
fn live_single_head_turn_returns_real_text() {
    if std::env::var("THEOREM_LIVE_PROVIDER_TEST").ok().as_deref() != Some("1") {
        eprintln!(
            "skipped: set THEOREM_LIVE_PROVIDER_TEST=1, THEOREM_AGENT_HEADS=<head> (e.g. mistral), \
             and the matching <PROVIDER>_API_KEY to run a real turn"
        );
        return;
    }

    let heads = std::env::var("THEOREM_AGENT_HEADS")
        .expect("set THEOREM_AGENT_HEADS to at least one head, e.g. mistral");
    assert!(
        !heads.trim().is_empty(),
        "THEOREM_AGENT_HEADS must name at least one head"
    );

    let binding = default_theorem_binding("agent:live-single-head-smoke")
        .expect("resolve configured binding");
    let registry = AgentHeadRegistry::from_binding(&binding).expect("build head registry");
    let head = registry
        .active_resolved_heads()
        .into_iter()
        .find(|head| head.kind != HeadKind::SkillPlugin)
        .expect("configured binding should include at least one reasoning head");
    let invoker = ProviderHeadInvoker::from_env().expect("build provider invoker from env");
    let task = "Reply with one short grounded sentence confirming this live single-head smoke ran.";
    let receipt = invoker
        .invoke(HeadInvocationRequest::new(
            head.clone(),
            HeadInvocationKind::Proposal,
            task,
            binding.working_memory_scope.scratchpad.version,
            Vec::new(),
            vec![GroundedClaim::new(task, "test:live_single_head_smoke")],
            "2026-06-25T00:00:00Z",
        ))
        .expect("single configured head should return real provider text");

    assert_eq!(
        receipt.head_id, head.head_id,
        "receipt should come from the configured live head"
    );

    // The receipt carries real model text (not an empty/fake completion).
    let any_real_text = receipt
        .payload
        .get("text")
        .and_then(|value| value.as_str())
        .map(|text| !text.trim().is_empty())
        .unwrap_or(false);
    assert!(
        any_real_text,
        "expected the head receipt to carry non-empty model text; receipt: {:#?}",
        receipt
    );

    eprintln!(
        "live single-head smoke OK: head={} provider={} model={} heads=[{}]",
        receipt.head_id,
        head.provider,
        head.model,
        heads.trim()
    );
}
