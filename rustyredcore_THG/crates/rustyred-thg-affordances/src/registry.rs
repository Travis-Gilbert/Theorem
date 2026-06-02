//! Connector registration: turn an MCP server's tool catalog into Affordance
//! graph nodes. This is the inverse of the MCP adapter: the adapter exposes the
//! core as tools; the registry ingests connectors' tools as affordance nodes
//! through the same store. Registration is idempotent on re-registration:
//! re-registering the same connector preserves accumulated fitness, embeddings,
//! and outcome history.

use serde_json::{json, Value};

use rustyred_thg_core::{GraphMutation, GraphMutationBatch, NodeRecord, ThgResult};
use theorem_harness_core::default_affordance_registry;

use crate::types::{
    connector_node_id, edge_with_affordance_provenance, normalize_tenant_id, tenant_node_id,
    thg_error_from_store, Affordance, AffordanceGraphStore, AffordanceUpsertResult,
    ConnectorManifest, ConnectorRegisterResult, ToolManifest, CONNECTOR_LABEL,
    DEFAULT_HALF_LIFE_DAYS, OFFERS, TENANT_LABEL, THG_AFFORDANCE_SOURCE,
};

/// Register an entire connector: one `Connector` node + one `Affordance` node
/// per tool + `OFFERS` edges, in a single transaction. Idempotent.
pub fn register_connector<S: AffordanceGraphStore>(
    store: &mut S,
    manifest: ConnectorManifest,
    actor: Option<&str>,
) -> ThgResult<ConnectorRegisterResult> {
    let tenant_id = normalize_tenant_id(&manifest.tenant_id);
    let server_id = manifest.server_id.trim().to_string();
    if server_id.is_empty() {
        return Err(rustyred_thg_core::ThgError::new(
            "invalid_connector",
            "server_id is required",
        ));
    }

    let connector_node = connector_node_id(&tenant_id, &server_id);
    let mut mutations = vec![
        GraphMutation::NodeUpsert(NodeRecord::new(
            tenant_node_id(&tenant_id),
            [TENANT_LABEL],
            json!({ "tenant_id": tenant_id, "source": THG_AFFORDANCE_SOURCE }),
        )),
        GraphMutation::NodeUpsert(NodeRecord::new(
            &connector_node,
            [CONNECTOR_LABEL],
            json!({
                "tenant_id": tenant_id,
                "server_id": server_id,
                "label": manifest.label,
                "tool_count": manifest.tools.len(),
                "source": THG_AFFORDANCE_SOURCE,
            }),
        )),
    ];

    let mut affordance_node_ids = Vec::with_capacity(manifest.tools.len());
    for tool in &manifest.tools {
        let affordance = affordance_from_tool(&tenant_id, &server_id, tool);
        affordance.validate()?;
        let node_id = affordance.node_id();
        let extra = preserved_affordance_properties(
            store.get_node(&node_id).map_err(thg_error_from_store)?.as_ref(),
            affordance.embedding.is_some(),
        );
        mutations.push(GraphMutation::NodeUpsert(affordance.to_node_record(actor, extra)));
        mutations.push(GraphMutation::EdgeUpsert(edge_with_affordance_provenance(
            offers_edge_id(&connector_node, &node_id),
            &connector_node,
            OFFERS,
            &node_id,
            json!({ "tenant_id": tenant_id }),
            actor,
        )));
        affordance_node_ids.push(node_id);
    }

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;

    Ok(ConnectorRegisterResult {
        tenant_id,
        server_id,
        connector_node_id: connector_node,
        affordance_node_ids,
        transaction,
    })
}

/// Upsert a single affordance (its node + owning connector + `OFFERS` edge),
/// preserving accumulated fitness/embedding on re-registration.
pub fn upsert_affordance<S: AffordanceGraphStore>(
    store: &mut S,
    affordance: Affordance,
    actor: Option<&str>,
) -> ThgResult<AffordanceUpsertResult> {
    let affordance = affordance.normalized();
    affordance.validate()?;
    let node_id = affordance.node_id();
    let connector_node = connector_node_id(&affordance.tenant_id, &affordance.server_id);

    let extra = preserved_affordance_properties(
        store.get_node(&node_id).map_err(thg_error_from_store)?.as_ref(),
        affordance.embedding.is_some(),
    );

    let mutations = vec![
        GraphMutation::NodeUpsert(NodeRecord::new(
            &connector_node,
            [CONNECTOR_LABEL],
            json!({
                "tenant_id": affordance.tenant_id,
                "server_id": affordance.server_id,
                "source": THG_AFFORDANCE_SOURCE,
            }),
        )),
        GraphMutation::NodeUpsert(affordance.to_node_record(actor, extra)),
        GraphMutation::EdgeUpsert(edge_with_affordance_provenance(
            offers_edge_id(&connector_node, &node_id),
            &connector_node,
            OFFERS,
            &node_id,
            json!({ "tenant_id": affordance.tenant_id }),
            actor,
        )),
    ];

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;
    Ok(AffordanceUpsertResult {
        node_id: affordance.node_id(),
        affordance,
        edge_count: 1,
        transaction,
    })
}

/// Project the built-in `theorem-harness-core` affordance registry (the 11
/// symbolic engines) into graph nodes, so the existing affordances are
/// first-class learning nodes too, not only newly connected MCP tools.
pub fn register_builtin_affordances<S: AffordanceGraphStore>(
    store: &mut S,
    tenant_id: &str,
    actor: Option<&str>,
) -> ThgResult<ConnectorRegisterResult> {
    let tenant_id = normalize_tenant_id(tenant_id);
    let mut mutations = vec![GraphMutation::NodeUpsert(NodeRecord::new(
        tenant_node_id(&tenant_id),
        [TENANT_LABEL],
        json!({ "tenant_id": tenant_id, "source": THG_AFFORDANCE_SOURCE }),
    ))];

    let mut affordance_node_ids = Vec::new();
    let mut connectors_seen = std::collections::BTreeSet::new();
    for contract in default_affordance_registry() {
        let affordance = Affordance::from_contract(&contract, &tenant_id);
        affordance.validate()?;
        let node_id = affordance.node_id();
        let connector_node = connector_node_id(&tenant_id, &affordance.server_id);
        if connectors_seen.insert(connector_node.clone()) {
            mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
                &connector_node,
                [CONNECTOR_LABEL],
                json!({
                    "tenant_id": tenant_id,
                    "server_id": affordance.server_id,
                    "label": "theorem-native",
                    "source": THG_AFFORDANCE_SOURCE,
                }),
            )));
        }
        let extra = preserved_affordance_properties(
            store.get_node(&node_id).map_err(thg_error_from_store)?.as_ref(),
            false,
        );
        mutations.push(GraphMutation::NodeUpsert(affordance.to_node_record(actor, extra)));
        mutations.push(GraphMutation::EdgeUpsert(edge_with_affordance_provenance(
            offers_edge_id(&connector_node, &node_id),
            &connector_node,
            OFFERS,
            &node_id,
            json!({ "tenant_id": tenant_id }),
            actor,
        )));
        affordance_node_ids.push(node_id);
    }

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;
    Ok(ConnectorRegisterResult {
        tenant_id,
        server_id: "theorem-native".to_string(),
        connector_node_id: String::new(),
        affordance_node_ids,
        transaction,
    })
}

fn affordance_from_tool(tenant_id: &str, server_id: &str, tool: &ToolManifest) -> Affordance {
    let tool_name = tool.name.trim().to_string();
    let affordance_id = format!("{server_id}.{tool_name}");
    Affordance {
        affordance_id,
        tenant_id: tenant_id.to_string(),
        server_id: server_id.to_string(),
        tool_name,
        family: "connector".to_string(),
        label: if tool.label.trim().is_empty() {
            tool.name.trim().to_string()
        } else {
            tool.label.trim().to_string()
        },
        description: tool.description.clone(),
        input_schema: if tool.input_schema.is_null() {
            json!({})
        } else {
            tool.input_schema.clone()
        },
        permissions: tool.permissions.clone(),
        cost: if tool.cost.is_null() {
            json!({})
        } else {
            tool.cost.clone()
        },
        writeback_policy: tool.writeback_policy.clone(),
        tags: tool.tags.clone(),
        embedding: tool.description_embedding.clone(),
        fitness: 0.0,
        version: 1,
        created_at_ms: 0,
        manifest_version: 1,
    }
    .normalized()
}

/// Preserve learned state across re-registration: fitness, fitness decay
/// metadata, original creation time, and the existing embedding unless the new
/// manifest supplies one. Defaults the fitness half-life if absent.
fn preserved_affordance_properties(
    existing: Option<&NodeRecord>,
    has_new_embedding: bool,
) -> Value {
    let mut preserved = json!({});
    if let Some(node) = existing {
        for key in ["fitness", "fitness_updated_at_ms", "fitness_half_life_days", "created_at_ms"] {
            if let Some(value) = node.properties.get(key) {
                preserved[key] = value.clone();
            }
        }
        if !has_new_embedding {
            if let Some(embedding) = node.properties.get("embedding") {
                preserved["embedding"] = embedding.clone();
            }
        }
    }
    if preserved.get("fitness_half_life_days").is_none() {
        preserved["fitness_half_life_days"] = json!(DEFAULT_HALF_LIFE_DAYS);
    }
    preserved
}

fn offers_edge_id(connector_node_id: &str, affordance_node_id: &str) -> String {
    format!("edge:{connector_node_id}:offers:{affordance_node_id}")
}
