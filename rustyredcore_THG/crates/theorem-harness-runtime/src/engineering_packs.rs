//! The three engineering skill packs, authored from the plugin pack prose and published
//! into the harness skill registry (North Star addendum T7: skill-corpus bring-up).
//!
//! The skill corpus is the same machine as the tool corpus: a pack is authored
//! (`skill_publish`), recorded on use (`skill_apply` -> `SkillPackUseReceipt`), compounded on
//! run-close by the Compound spine, and ranked by `ensemble_select`. This module supplies the
//! three packs that already exist as plugin prose so `skill_list` returns them instead of zero:
//!
//! - `writing-engineering` and `design-engineering` reuse the canonical, content-addressed
//!   payloads already encoded in-repo by the `prose-check` and `design-check` skill-pack
//!   crates, so there is one source of truth and the published hash matches the checker crate.
//! - `rust-engineering` has no in-repo checker crate, so it is authored here directly from the
//!   active plugin prose (`skills/rust-engineering/SKILL.md`), carrying that pack's own
//!   content-addressed hash so it stays joined to its corpus and provenance.
//!
//! Publishing rides the existing `publish_skill_pack`; no new ranking is introduced.

use crate::skill_pack::{
    publish_skill_pack, SkillPackGraphStore, SkillPackPublishInput, SkillPackPublishReceipt,
    SkillPackResult,
};
use serde_json::{json, Value};

/// Content hash of the rust-engineering pack, from the plugin prose provenance
/// (`skills/rust-engineering/SKILL.md`). Keeping the published hash equal to the corpus hash
/// keeps the runtime pack joined to its distilled source and held-out provenance.
pub const RUST_ENGINEERING_PACK_CONTENT_HASH: &str =
    "sha256:325ba9cbba248cadb5edc2c207f1b5071331d64e7e2191f8ebbfa3d2fa92cf43";

/// Source-corpus hash of the rust-engineering pack, from the plugin prose provenance.
pub const RUST_ENGINEERING_SOURCE_CONTENT_HASH: &str =
    "sha256:683af3877bc763fb5202ed7c0d6303b47685214408973c468a77af87c1019f96";

/// One engineering pack ready to publish: the `CapabilityPackSpec` payload, its
/// content-addressed hash, and the status it should land at in the registry.
#[derive(Clone, Debug)]
pub struct EngineeringPack {
    pub name: &'static str,
    pub pack: Value,
    pub pack_content_hash: String,
    pub status: &'static str,
}

/// The rust-engineering `CapabilityPackSpec`, authored faithfully from the plugin prose
/// (Core Posture, Standard Rust Workflow, Domain Map, Validation Defaults, Anti-Patterns,
/// Capabilities). The structured contract mirrors `prose-check`'s writing pack shape so the
/// registry, validators, and ensemble selector treat all three engineering packs uniformly.
pub fn rust_engineering_pack_payload(parent_hash: Option<&str>) -> Value {
    let mut metadata = json!({
        "status": "advisory",
        "promotion_state": "advisory",
        "pack_content_hash": RUST_ENGINEERING_PACK_CONTENT_HASH,
        "source_content_hash": RUST_ENGINEERING_SOURCE_CONTENT_HASH,
        "provenance": "source:rust-engineering-external-corpus-v0.4 (code_corpus_v1); scanned (compiled, not yet held-out validated)",
        "fitness": {
            "validator_pass_count": 0,
            "held_out_failures": 0,
            "promotion_gate": "held_out_pending"
        }
    });
    if let Some(parent_hash) = parent_hash.filter(|value| !value.trim().is_empty()) {
        metadata["parent_pack_content_hash"] = Value::String(parent_hash.to_string());
    }
    json!({
        "id": "rust-engineering",
        "name": "rust-engineering",
        "kind": "skill_pack",
        "title": "Rust Engineering",
        "description": "Encoded Rust pack for implementation, review, debugging, validator work, and the Rust corpus/encoding pipeline over Cargo workspaces, MCP/server crates, PyO3 bridges, and async services.",
        "directive": "Start from the live crate/workspace shape; prefer local crate patterns over generic Rust advice; keep ownership explicit and announce a coordination_intent before editing hot Rust files; treat compiler errors as design feedback and re-plan on a third workaround in one module; validate narrowly first (focused cargo test -p crate test_name), then widen.",
        "workflow": [
            "Observe the workspace: rg --files, Cargo.toml, Cargo.lock, crate-local README/AGENTS, nearby tests; classify member vs standalone vs generated vs bridge.",
            "Classify the Rust domain via the domain map to choose patterns, references, and validation gates.",
            "Edit in the local style: existing traits, newtypes, serde shapes, error enums, feature flags, module layout; add abstractions only to remove real repeated complexity.",
            "Validate the real seam: compile or test the smallest crate that owns the behavior; add targeted tests for behavior changes, graph contracts, validators, parser outputs, protocol schemas, and persistence edges.",
            "Feed the skill loop: encode reusable patterns, bugs, validators, and postmortems as skill-pack signals with outcome metadata."
        ],
        "domain_map": {
            "crate_workspace_plumbing": "Make the dependency edge explicit; avoid hidden cross-crate imports.",
            "async_server": "Test the handler/stream contract and auth/tenant scoping, not just parse (tokio, axum, tonic, hyper, streams).",
            "graph_storage_substrate": "Verify durable reopen or trait-vs-inherent method behavior (GraphStore, AOF/snapshot, indexes).",
            "parsers_macros": "Prefer AST APIs over string parsing; test representative syntax (syn, quote, proc_macro2, tree-sitter).",
            "ffi_bridges": "Preserve exported names and byte/parity contracts (pyo3, maturin, UniFFI, C ABI).",
            "validators_skills": "Keep raw source execution out of request paths; record validator mode (SkillPack, artifacts, receipts).",
            "systems_browser": "Isolate unsafe/platform assumptions; keep reproducible fixtures (Servo, OS kernels, low-level IO).",
            "ml_rust_data": "Pin shapes/dimensions; test small deterministic fixtures (candle, tensor runtimes, vector search)."
        },
        "validation_defaults": [
            "cargo test -p <crate> <test_name> for behavior or runtime contracts",
            "cargo test --manifest-path <path> for standalone crates",
            "cargo check -p <crate> when tests are too heavy but type contracts matter",
            "cargo clippy -p <crate> --all-targets --no-deps -- -D warnings when warning-clean and disk/time allow",
            "git diff --check before reporting or committing",
            "if disk is tight: CARGO_INCREMENTAL=0, reuse an existing CARGO_TARGET_DIR, or one crate at a time; report skipped broad gates honestly"
        ],
        "anti_patterns": [
            "Assuming a repo-level Cargo workspace when the project has standalone crates.",
            "Adding a dependency to code without adding the manifest edge that makes it compile outside the editor session.",
            "Replacing typed Rust APIs with ad hoc string parsing.",
            "Treating node --check, cargo fmt, or a successful grep as runtime proof.",
            "Turning one-off reasoning traces into public skills instead of encoding them as evidence for a broader Rust capability."
        ],
        "capabilities": [
            "checker_rule",
            "context_atom_template",
            "dependency_context_hint",
            "fallback_text_context",
            "native_validator_candidate",
            "source_file_context",
            "structure_decision_hint",
            "validator_contract"
        ],
        "validators": [
            {"id": "rust-directive", "kind": "required_field", "field": "directive"},
            {"id": "rust-domain-map", "kind": "required_field", "field": "domain_map"},
            {"id": "rust-validator-contract", "kind": "required_field", "field": "validators"}
        ],
        "spec": {
            "kind": "skill_pack",
            "domain": "rust-engineering",
            "pack_content_hash": RUST_ENGINEERING_PACK_CONTENT_HASH,
            "source_content_hash": RUST_ENGINEERING_SOURCE_CONTENT_HASH
        },
        "metadata": metadata
    })
}

/// The three engineering packs ready to publish into the harness registry, in a stable order
/// (rust, writing, design). Writing and design come from the canonical checker-crate payloads;
/// rust is authored here from the plugin prose.
pub fn engineering_capability_packs() -> Vec<EngineeringPack> {
    vec![
        EngineeringPack {
            name: "rust-engineering",
            pack: rust_engineering_pack_payload(None),
            pack_content_hash: RUST_ENGINEERING_PACK_CONTENT_HASH.to_string(),
            status: "advisory",
        },
        EngineeringPack {
            name: "writing-engineering",
            pack: prose_check::writing_engineering_pack_payload(None),
            pack_content_hash: prose_check::pack_hash(),
            status: "advisory",
        },
        EngineeringPack {
            name: "design-engineering",
            pack: design_check::design_engineering_pack_payload(None),
            pack_content_hash: design_check::pack_hash(),
            status: "advisory",
        },
    ]
}

/// Publish all three engineering packs into `tenant_slug`'s skill registry. Idempotent by
/// content hash (re-publishing the same pack upserts the same node), so it is safe to call on
/// every bring-up. Returns the publish receipts in pack order.
pub fn publish_engineering_packs<S: SkillPackGraphStore>(
    store: &mut S,
    tenant_slug: &str,
    actor_id: &str,
) -> SkillPackResult<Vec<SkillPackPublishReceipt>> {
    let mut receipts = Vec::new();
    for pack in engineering_capability_packs() {
        receipts.push(publish_skill_pack(
            store,
            SkillPackPublishInput {
                tenant_slug: tenant_slug.to_string(),
                actor_id: actor_id.to_string(),
                pack_content_hash: pack.pack_content_hash,
                status: pack.status.to_string(),
                pack: pack.pack,
                ..SkillPackPublishInput::default()
            },
        )?);
    }
    Ok(receipts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill_pack::{list_skill_packs, SkillPackListInput};
    use rustyred_thg_core::InMemoryGraphStore;

    #[test]
    fn publishes_the_three_engineering_packs_into_the_registry() {
        let mut store = InMemoryGraphStore::new();
        let receipts = publish_engineering_packs(&mut store, "Travis-Gilbert", "claude-code")
            .expect("publish engineering packs");
        assert_eq!(receipts.len(), 3);

        let packs = list_skill_packs(
            &store,
            SkillPackListInput {
                tenant_slug: "Travis-Gilbert".to_string(),
                limit: 50,
                ..SkillPackListInput::default()
            },
        )
        .expect("list skill packs");
        // skill_list returns the published packs rather than zero.
        let names: Vec<&str> = packs.iter().map(|pack| pack.pack_id.as_str()).collect();
        assert!(
            packs.len() >= 3,
            "expected at least 3 published packs, got {}: {names:?}",
            packs.len()
        );
        for expected in ["rust-engineering", "writing-engineering", "design-engineering"] {
            assert!(
                packs
                    .iter()
                    .any(|pack| pack.title.to_lowercase().contains(&expected.replace('-', " "))
                        || pack.pack.get("name").and_then(Value::as_str) == Some(expected)),
                "published registry is missing {expected}: {names:?}"
            );
        }
    }

    #[test]
    fn each_published_pack_carries_a_spec_so_it_is_a_capability_pack() {
        let mut store = InMemoryGraphStore::new();
        publish_engineering_packs(&mut store, "Travis-Gilbert", "claude-code").unwrap();
        let packs = list_skill_packs(
            &store,
            SkillPackListInput {
                tenant_slug: "Travis-Gilbert".to_string(),
                limit: 50,
                ..SkillPackListInput::default()
            },
        )
        .unwrap();
        for pack in &packs {
            assert!(
                !pack.spec.is_null(),
                "pack {} must carry a non-null spec to join the ensemble registry",
                pack.pack_id
            );
        }
    }
}
