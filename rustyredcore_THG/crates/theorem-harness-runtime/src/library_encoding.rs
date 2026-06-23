use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use theorem_harness_core::stable_value_hash;

const PLAN_JSON: &str = include_str!("library_encoding/plan.json");

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryEncodingPlan {
    pub schema_version: u64,
    pub id: String,
    pub title: String,
    pub source_files: Vec<String>,
    pub verified_inventory_date: String,
    pub plugin_inventory: Vec<String>,
    pub infrastructure_not_packs: Vec<String>,
    pub retired_into_hooks: Vec<RetiredProcessPlugin>,
    pub packs: Vec<LibraryPackSpec>,
    pub keystone: LibraryKeystone,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetiredProcessPlugin {
    pub source: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryPackSpec {
    pub pack_id: String,
    pub tier: u64,
    pub gate_kind: String,
    pub checker_domain: bool,
    pub status_cap: String,
    pub sources: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryKeystone {
    pub pack_id: String,
    pub role: String,
    pub build_once: bool,
}

pub fn library_encoding_plan() -> LibraryEncodingPlan {
    serde_json::from_str(PLAN_JSON).expect("library encoding plan JSON is valid")
}

pub fn library_encoding_plan_value() -> Value {
    serde_json::from_str(PLAN_JSON).expect("library encoding plan JSON is valid")
}

pub fn library_encoding_plan_hash() -> String {
    format!(
        "sha256:{}",
        stable_value_hash(&library_encoding_plan_value())
    )
}

pub fn library_encoding_pack_payload(parent_hash: Option<&str>) -> Value {
    let plan = library_encoding_plan_value();
    json!({
        "kind": "skill_pack",
        "name": "library-encoding-plan",
        "title": "Library Encoding Plan",
        "description": "Content-addressed map for consolidating the claude-marketplace plugin library into gated, fitness-bearing harness packs.",
        "capabilities": ["library-encoding", "pack-inventory", "gate-spec"],
        "validators": [{
            "id": "library-plan-static",
            "kind": "static_inventory",
            "status": "passed",
            "message": "Inventory encodes plugin consolidation, gate tiers, hook retirements, and infrastructure exclusions."
        }],
        "metadata": {
            "plan_hash": library_encoding_plan_hash(),
            "parent_pack_content_hash": parent_hash.unwrap_or(""),
            "library_encoding_plan": plan
        }
    })
}

pub fn library_pack_by_source(source: &str) -> Option<LibraryPackSpec> {
    let source = source.trim();
    library_encoding_plan()
        .packs
        .into_iter()
        .find(|pack| pack.sources.iter().any(|item| item == source))
}

pub fn library_source_is_infrastructure(source: &str) -> bool {
    let source = source.trim();
    library_encoding_plan()
        .infrastructure_not_packs
        .iter()
        .any(|item| item == source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_reduces_twenty_plugins_to_consolidated_packs() {
        let plan = library_encoding_plan();
        assert_eq!(plan.plugin_inventory.len(), 20);
        assert!(plan.packs.len() < plan.plugin_inventory.len());
        assert_eq!(
            library_pack_by_source("django-design").unwrap().pack_id,
            "django"
        );
        assert_eq!(library_pack_by_source("app-forge").unwrap().pack_id, "app");
        assert_eq!(
            library_pack_by_source("vie-design").unwrap().pack_id,
            "design-engineering"
        );
    }

    #[test]
    fn process_and_infrastructure_plugins_do_not_become_packs() {
        let plan = library_encoding_plan();
        assert!(plan
            .retired_into_hooks
            .iter()
            .any(|item| item.source == "spec-compliance"));
        assert!(plan
            .retired_into_hooks
            .iter()
            .any(|item| item.source == "spec-guard"));
        assert!(library_source_is_infrastructure("theorems-harness"));
        assert!(library_source_is_infrastructure("plugin-server"));
    }

    #[test]
    fn theseus_pro_is_knowledge_not_checker_domain() {
        let theseus = library_pack_by_source("theseus-pro").unwrap();
        assert_eq!(theseus.pack_id, "theseus");
        assert_eq!(theseus.tier, 4);
        assert!(!theseus.checker_domain);
        assert_eq!(theseus.gate_kind, "retrieval-uplift");
    }

    #[test]
    fn payload_is_content_addressed_and_publishable() {
        let hash = library_encoding_plan_hash();
        assert!(hash.starts_with("sha256:"));
        let payload = library_encoding_pack_payload(Some("sha256:parent"));
        assert_eq!(payload["kind"], "skill_pack");
        assert_eq!(
            payload["metadata"]["parent_pack_content_hash"],
            "sha256:parent"
        );
        assert_eq!(payload["metadata"]["plan_hash"], hash);
    }
}
