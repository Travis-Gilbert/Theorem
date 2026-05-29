use serde_json::{json, Value};

use rustyred_thg_core::{GraphMutation, GraphMutationBatch, ThgError, ThgResult};

use crate::fitness::find_adapter_by_id;
use crate::routing::adapter_training_centroid;
use crate::types::{
    edge_with_adapter_provenance, object_node_id, thg_error_from_store, AdapterGraphStore,
    AdapterUpsertResult, LoraAdapter, DERIVED_FROM, TRAINED_ON,
};

pub fn upsert_adapter<S: AdapterGraphStore>(
    store: &mut S,
    adapter: LoraAdapter,
    derived_from_adapter_id: Option<&str>,
    actor: Option<&str>,
) -> ThgResult<AdapterUpsertResult> {
    let adapter = adapter.normalized();
    adapter.validate()?;

    let existing = store
        .get_node(&adapter.node_id())
        .map_err(thg_error_from_store)?;
    let mut extra_properties = preserved_adapter_properties(existing.as_ref());
    if !extra_properties.get("fitness_half_life_days").is_some() {
        extra_properties["fitness_half_life_days"] = json!(adapter
            .tenant_id
            .eq_ignore_ascii_case("theseus")
            .then_some(14.0)
            .unwrap_or(30.0));
    }
    if let Some(embedding) = adapter_training_centroid(store, &adapter.training_object_ids)? {
        extra_properties["embedding"] = json!(embedding);
    }

    let adapter_node = adapter.to_node_record(actor, extra_properties);
    let mut mutations = vec![GraphMutation::NodeUpsert(adapter_node)];

    for object_pk in &adapter.training_object_ids {
        let object_node = object_node_id(*object_pk);
        ensure_node_exists(store, &object_node)?;
        mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            trained_on_edge_id(&adapter.node_id(), &object_node),
            adapter.node_id(),
            TRAINED_ON,
            object_node,
            json!({
                "training_object_id": object_pk,
                "tenant_id": adapter.tenant_id,
            }),
            actor,
        )));
    }

    if let Some(derived_from_adapter_id) = derived_from_adapter_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let prior = find_adapter_by_id(store, derived_from_adapter_id)?.ok_or_else(|| {
            ThgError::new("adapter_not_found", "derived_from_adapter_id not found")
        })?;
        if prior.tenant_id != adapter.tenant_id {
            return Err(ThgError::new(
                "tenant_scope_violation",
                "derived_from_adapter_id belongs to a different tenant",
            ));
        }
        mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            derived_from_edge_id(&adapter.node_id(), &prior.node_id()),
            adapter.node_id(),
            DERIVED_FROM,
            prior.node_id(),
            json!({
                "tenant_id": adapter.tenant_id,
                "derived_from_adapter_id": prior.adapter_id,
            }),
            actor,
        )));
    }

    let edge_count = mutations.len().saturating_sub(1);
    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;
    Ok(AdapterUpsertResult {
        node_id: adapter.node_id(),
        adapter,
        edge_count,
        transaction,
    })
}

fn ensure_node_exists<S: AdapterGraphStore>(store: &S, node_id: &str) -> ThgResult<()> {
    if store
        .get_node(node_id)
        .map_err(thg_error_from_store)?
        .is_some()
    {
        Ok(())
    } else {
        Err(ThgError::new(
            "missing_graph_endpoint",
            format!("training Object node {node_id} does not exist"),
        ))
    }
}

fn preserved_adapter_properties(existing: Option<&rustyred_thg_core::NodeRecord>) -> Value {
    let mut preserved = json!({});
    let Some(node) = existing else {
        return preserved;
    };
    for key in [
        "fitness_updated_at_ms",
        "fitness_half_life_days",
        "archived",
        "archived_at_ms",
        "embedding",
    ] {
        if let Some(value) = node.properties.get(key) {
            preserved[key] = value.clone();
        }
    }
    preserved
}

pub fn trained_on_edge_id(adapter_node_id: &str, object_node_id: &str) -> String {
    format!("edge:{adapter_node_id}:trained_on:{object_node_id}")
}

pub fn derived_from_edge_id(adapter_node_id: &str, prior_adapter_node_id: &str) -> String {
    format!("edge:{adapter_node_id}:derived_from:{prior_adapter_node_id}")
}
