//! Programmable Harness bridge: expose approved declarative skills and WASM
//! plugin exports as ordinary affordances.

use rustyred_plugin::{DeclarativeSkillDefinition, PluginExportSpec, WasmPluginSpec};
use serde_json::{json, Value};

use crate::registry::register_connector_with_target;
use crate::types::{
    AffordanceGraphStore, ConnectorManifest, ConnectorRegisterResult, ToolManifest,
};
use rustyred_thg_core::ThgResult;

pub const PROGRAMMABLE_PLUGIN_TRANSPORT: &str = "rustyred_plugin";
pub const DECLARATIVE_SKILL_TRANSPORT: &str = "rustyred_declarative_skill";
pub const PROGRAMMABLE_PLUGIN_FAMILY: &str = "wasm_plugin";
pub const DECLARATIVE_SKILL_FAMILY: &str = "declarative_skill";

pub fn register_wasm_plugin_exports<S: AffordanceGraphStore>(
    store: &mut S,
    spec: &WasmPluginSpec,
    actor: Option<&str>,
) -> ThgResult<ConnectorRegisterResult> {
    let spec = spec.clone().normalized();
    let manifest = ConnectorManifest {
        tenant_id: spec.tenant_id.clone(),
        server_id: plugin_server_id(&spec.plugin_id),
        label: format!("WASM plugin {}", spec.plugin_id),
        tools: spec
            .exports
            .iter()
            .map(|export| tool_from_plugin_export(export, PROGRAMMABLE_PLUGIN_FAMILY))
            .collect(),
    };
    let connection_target = json!({
        "transport": PROGRAMMABLE_PLUGIN_TRANSPORT,
        "plugin_id": spec.plugin_id,
        "tenant_id": spec.tenant_id,
        "source": spec.source,
        "exports": spec.exports,
        "source_hash": spec.source.content_hash().unwrap_or_default(),
        "limits": spec.limits,
        "grants": spec.grants,
        "provenance": spec.provenance,
    });
    register_connector_with_target(store, manifest, Some(connection_target), actor)
}

pub fn register_declarative_skill_plugin<S: AffordanceGraphStore>(
    store: &mut S,
    definition: &DeclarativeSkillDefinition,
    actor: Option<&str>,
) -> ThgResult<ConnectorRegisterResult> {
    let manifest = ConnectorManifest {
        tenant_id: definition.tenant_id.trim().to_string(),
        server_id: declarative_skill_server_id(&definition.skill_id),
        label: format!("Declarative skill {}", definition.skill_id),
        tools: vec![ToolManifest {
            name: "invoke".to_string(),
            label: definition.title.clone(),
            description: definition.description.clone(),
            family: DECLARATIVE_SKILL_FAMILY.to_string(),
            input_schema: if definition.parameters_schema.is_null() {
                json!({})
            } else {
                definition.parameters_schema.clone()
            },
            permissions: vec!["affordance.invoke".to_string()],
            cost: json!({
                "step_count": definition.steps.len(),
                "provenance": definition.provenance,
            }),
            writeback_policy: "delegated".to_string(),
            tags: vec!["programmable".to_string(), "declarative_skill".to_string()],
            description_embedding: None,
        }],
    };
    let connection_target = json!({
        "transport": DECLARATIVE_SKILL_TRANSPORT,
        "skill_id": definition.skill_id,
        "tenant_id": definition.tenant_id,
        "title": definition.title,
        "description": definition.description,
        "parameters_schema": definition.parameters_schema,
        "steps": definition.steps,
        "provenance": definition.provenance,
    });
    register_connector_with_target(store, manifest, Some(connection_target), actor)
}

fn tool_from_plugin_export(export: &PluginExportSpec, family: &str) -> ToolManifest {
    let export = export.clone().normalized();
    let mut tags = export.tags;
    tags.push("programmable".to_string());
    tags.push("wasm_plugin".to_string());
    tags.sort();
    tags.dedup();
    ToolManifest {
        name: export.name,
        label: export.label,
        description: export.description,
        family: family.to_string(),
        input_schema: normalize_schema(export.input_schema),
        permissions: export.permissions,
        cost: json!({
            "runtime": "extism",
            "sandbox": "wasm",
        }),
        writeback_policy: export.writeback_policy,
        tags,
        description_embedding: None,
    }
}

fn normalize_schema(value: Value) -> Value {
    if value.is_null() {
        json!({})
    } else {
        value
    }
}

fn plugin_server_id(plugin_id: &str) -> String {
    format!("wasm_plugin:{}", plugin_id.trim())
}

fn declarative_skill_server_id(skill_id: &str) -> String {
    format!("declarative_skill:{}", skill_id.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::affordance_nodes;
    use crate::registry::connector_connection_target;
    use rustyred_plugin::{
        CapabilityProvenance, HostFunctionGrant, PluginLimits, WasmPluginSource,
    };
    use rustyred_thg_core::InMemoryGraphStore;

    #[test]
    fn wasm_plugin_exports_register_as_discoverable_affordances() {
        let mut store = InMemoryGraphStore::default();
        let spec = WasmPluginSpec {
            plugin_id: "acme.echo".to_string(),
            tenant_id: "tenant".to_string(),
            source: WasmPluginSource::Wat("(module)".to_string()),
            exports: vec![PluginExportSpec {
                name: "summarize".to_string(),
                label: "Summarize".to_string(),
                description: "Summarize a graph neighborhood.".to_string(),
                input_schema: json!({"type": "object"}),
                permissions: vec!["graph.read".to_string()],
                writeback_policy: "read-only".to_string(),
                tags: vec!["summary".to_string()],
            }],
            grants: vec![HostFunctionGrant::GraphRead],
            limits: PluginLimits::default(),
            declared_tests: vec![],
            provenance: CapabilityProvenance::default(),
        };

        let receipt = register_wasm_plugin_exports(&mut store, &spec, Some("test")).unwrap();
        assert_eq!(receipt.affordance_node_ids.len(), 1);

        let nodes = affordance_nodes(&store).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(
            nodes[0].properties["affordance_id"],
            "wasm_plugin:acme.echo.summarize"
        );
        assert_eq!(nodes[0].properties["family"], PROGRAMMABLE_PLUGIN_FAMILY);

        let target =
            connector_connection_target(&store, "tenant", "wasm_plugin:acme.echo").unwrap();
        assert_eq!(target.unwrap()["transport"], PROGRAMMABLE_PLUGIN_TRANSPORT);
    }

    #[test]
    fn declarative_skill_registers_invoke_affordance_with_steps() {
        let mut store = InMemoryGraphStore::default();
        let definition = DeclarativeSkillDefinition {
            skill_id: "skill.graph-note".to_string(),
            tenant_id: "tenant".to_string(),
            title: "Graph note".to_string(),
            description: "read then write".to_string(),
            parameters_schema: json!({"type": "object"}),
            steps: vec![
                rustyred_plugin::DeclarativeSkillStep {
                    affordance_id: "graph.read".to_string(),
                    arguments: json!({}),
                },
                rustyred_plugin::DeclarativeSkillStep {
                    affordance_id: "fact.write".to_string(),
                    arguments: json!({}),
                },
            ],
            declared_tests: vec![],
            provenance: CapabilityProvenance::default(),
        };

        let receipt =
            register_declarative_skill_plugin(&mut store, &definition, Some("test")).unwrap();
        assert_eq!(receipt.affordance_node_ids.len(), 1);
        let nodes = affordance_nodes(&store).unwrap();
        assert_eq!(nodes[0].properties["family"], DECLARATIVE_SKILL_FAMILY);
        assert_eq!(nodes[0].properties["tool_name"], "invoke");
        let target =
            connector_connection_target(&store, "tenant", "declarative_skill:skill.graph-note")
                .unwrap()
                .unwrap();
        assert_eq!(target["transport"], DECLARATIVE_SKILL_TRANSPORT);
        assert_eq!(target["tenant_id"], "tenant");
        assert_eq!(target["skill_id"], "skill.graph-note");
        assert_eq!(target["steps"].as_array().unwrap().len(), 2);
    }
}
