use serde_json::json;

use rustyred_thg_core::{
    now_ms, GraphMutation, GraphMutationBatch, NeighborQuery, NodeQuery, ThgError, ThgResult,
};

use crate::types::{
    edge_with_adapter_provenance, property_f32, property_i64, tenant_node_id, thg_error_from_store,
    AdapterFitnessRecordRequest, AdapterFitnessRecordResult, AdapterGraphStore, AdapterListRequest,
    AdapterSupersedeResult, LoraAdapter, DEFAULT_FITNESS_EPSILON, DEFAULT_MIN_FITNESS,
    DEFAULT_THESEUS_HALF_LIFE_DAYS, FITNESS_SIGNAL, LORA_ADAPTER_LABEL, SHARED_WITH, SUPERSEDES,
};

pub fn find_adapter_by_id<S: AdapterGraphStore>(
    store: &S,
    adapter_id: &str,
) -> ThgResult<Option<LoraAdapter>> {
    find_adapter_node_by_id(store, adapter_id)?
        .map(|node| LoraAdapter::from_node_record(&node))
        .transpose()
}

pub fn find_adapter_node_by_id<S: AdapterGraphStore>(
    store: &S,
    adapter_id: &str,
) -> ThgResult<Option<rustyred_thg_core::NodeRecord>> {
    let adapter_id = adapter_id.trim();
    if adapter_id.is_empty() {
        return Ok(None);
    }
    let hits = store
        .query_nodes(
            NodeQuery::label(LORA_ADAPTER_LABEL)
                .with_property("adapter_id", json!(adapter_id))
                .with_limit(2),
        )
        .map_err(thg_error_from_store)?;
    Ok(hits.into_iter().next())
}

pub fn list_adapters<S: AdapterGraphStore>(
    store: &S,
    req: AdapterListRequest,
) -> ThgResult<Vec<LoraAdapter>> {
    let tenant_id = req.tenant_id.trim().to_string();
    let min_fitness = req
        .min_fitness
        .unwrap_or(DEFAULT_MIN_FITNESS)
        .clamp(0.0, 1.0);
    let mut adapters = Vec::new();
    for node in adapter_nodes(store)? {
        let mut adapter = LoraAdapter::from_node_record(&node)?;
        if adapter.tenant_id != tenant_id {
            continue;
        }
        if let Some(base_model_sha) = req.base_model_sha.as_deref() {
            if adapter.base_model_sha != base_model_sha {
                continue;
            }
        }
        if !req.include_superseded && is_superseded(store, &adapter)? {
            continue;
        }
        adapter.fitness = effective_fitness_from_node(&node, &adapter);
        if adapter.fitness < min_fitness {
            continue;
        }
        adapters.push(adapter);
    }
    adapters.sort_by(|a, b| {
        b.version
            .cmp(&a.version)
            .then_with(|| b.created_at_ms.cmp(&a.created_at_ms))
            .then_with(|| a.adapter_id.cmp(&b.adapter_id))
    });
    Ok(adapters)
}

pub fn record_fitness<S: AdapterGraphStore>(
    store: &mut S,
    req: AdapterFitnessRecordRequest,
    actor: Option<&str>,
) -> ThgResult<AdapterFitnessRecordResult> {
    let node = find_adapter_node_by_id(store, &req.adapter_id)?
        .ok_or_else(|| ThgError::new("adapter_not_found", "adapter_id not found"))?;
    let mut adapter = LoraAdapter::from_node_record(&node)?;
    if store
        .get_node(&req.source_node_id)
        .map_err(thg_error_from_store)?
        .is_none()
    {
        return Err(ThgError::new(
            "missing_graph_endpoint",
            "source_node_id does not exist",
        ));
    }

    let recorded_at_ms = req.recorded_at_ms.unwrap_or_else(now_ms);
    let old_fitness = effective_fitness_from_node(&node, &adapter);
    let value = req.value.clamp(0.0, 1.0);
    let weight = req.weight.max(0.0);
    let alpha = if weight <= 0.0 {
        0.0
    } else {
        (weight / (weight + 4.0)).clamp(0.0, 1.0)
    };
    let updated = (old_fitness + alpha * (value - old_fitness)).clamp(DEFAULT_FITNESS_EPSILON, 1.0);
    adapter.fitness = updated;

    let mut updated_node = node.clone();
    updated_node.properties["fitness"] = json!(updated);
    updated_node.properties["fitness_updated_at_ms"] = json!(recorded_at_ms);

    let edge_id = fitness_signal_edge_id(
        &req.source_node_id,
        &adapter.node_id(),
        &req.kind,
        recorded_at_ms,
    );
    let edge = edge_with_adapter_provenance(
        edge_id.clone(),
        req.source_node_id,
        FITNESS_SIGNAL,
        adapter.node_id(),
        json!({
            "value": value,
            "weight": weight,
            "recorded_at_ms": recorded_at_ms,
            "kind": req.kind,
            "tenant_id": adapter.tenant_id,
        }),
        actor,
    );

    let transaction = store
        .commit_batch(GraphMutationBatch::new([
            GraphMutation::EdgeUpsert(edge),
            GraphMutation::NodeUpsert(updated_node),
        ]))
        .map_err(thg_error_from_store)?;
    Ok(AdapterFitnessRecordResult {
        adapter,
        edge_id,
        effective_fitness: updated,
        transaction,
    })
}

pub fn supersede_adapter<S: AdapterGraphStore>(
    store: &mut S,
    old_adapter_id: &str,
    new_adapter_id: &str,
    archive_old: bool,
    actor: Option<&str>,
) -> ThgResult<AdapterSupersedeResult> {
    let old_node = find_adapter_node_by_id(store, old_adapter_id)?
        .ok_or_else(|| ThgError::new("adapter_not_found", "old_adapter_id not found"))?;
    let new_node = find_adapter_node_by_id(store, new_adapter_id)?
        .ok_or_else(|| ThgError::new("adapter_not_found", "new_adapter_id not found"))?;
    let old_adapter = LoraAdapter::from_node_record(&old_node)?;
    let new_adapter = LoraAdapter::from_node_record(&new_node)?;
    if old_adapter.tenant_id != new_adapter.tenant_id {
        return Err(ThgError::new(
            "tenant_scope_violation",
            "old_adapter_id and new_adapter_id belong to different tenants",
        ));
    }

    let edge_id = supersedes_edge_id(&new_adapter.node_id(), &old_adapter.node_id());
    let edge = edge_with_adapter_provenance(
        edge_id.clone(),
        new_adapter.node_id(),
        SUPERSEDES,
        old_adapter.node_id(),
        json!({
            "tenant_id": old_adapter.tenant_id,
            "old_adapter_id": old_adapter.adapter_id,
            "new_adapter_id": new_adapter.adapter_id,
        }),
        actor,
    );

    let mut mutations = vec![GraphMutation::EdgeUpsert(edge)];
    if archive_old {
        let mut archived_node = old_node;
        archived_node.properties["archived"] = json!(true);
        archived_node.properties["archived_at_ms"] = json!(now_ms());
        mutations.push(GraphMutation::NodeUpsert(archived_node));
    }
    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;
    Ok(AdapterSupersedeResult {
        old_adapter,
        new_adapter,
        edge_id,
        transaction,
    })
}

pub fn effective_fitness<S: AdapterGraphStore>(store: &S, adapter: &LoraAdapter) -> ThgResult<f32> {
    let Some(node) = store
        .get_node(&adapter.node_id())
        .map_err(thg_error_from_store)?
    else {
        return Ok(adapter.fitness.clamp(DEFAULT_FITNESS_EPSILON, 1.0));
    };
    Ok(effective_fitness_from_node(&node, adapter))
}

pub fn effective_fitness_from_node(
    node: &rustyred_thg_core::NodeRecord,
    adapter: &LoraAdapter,
) -> f32 {
    let updated_at_ms = property_i64(&node.properties, "fitness_updated_at_ms");
    let Some(updated_at_ms) = updated_at_ms else {
        return adapter.fitness.clamp(DEFAULT_FITNESS_EPSILON, 1.0);
    };
    let half_life_days = property_f32(&node.properties, "fitness_half_life_days")
        .unwrap_or(DEFAULT_THESEUS_HALF_LIFE_DAYS)
        .max(1.0);
    let age_ms = now_ms().saturating_sub(updated_at_ms).max(0) as f32;
    let half_life_ms = half_life_days * 86_400_000.0;
    let decay = 0.5_f32.powf(age_ms / half_life_ms);
    (DEFAULT_FITNESS_EPSILON + (adapter.fitness - DEFAULT_FITNESS_EPSILON) * decay)
        .clamp(DEFAULT_FITNESS_EPSILON, 1.0)
}

pub fn is_superseded<S: AdapterGraphStore>(store: &S, adapter: &LoraAdapter) -> ThgResult<bool> {
    let hits = store
        .neighbors(NeighborQuery::in_(adapter.node_id()).with_edge_type(SUPERSEDES))
        .map_err(thg_error_from_store)?;
    Ok(!hits.is_empty())
}

pub fn is_shared_with_tenant<S: AdapterGraphStore>(
    store: &S,
    adapter: &LoraAdapter,
    tenant_id: &str,
) -> ThgResult<bool> {
    let tenant_node = tenant_node_id(tenant_id);
    for query in [
        NeighborQuery::out(adapter.node_id()).with_edge_type(SHARED_WITH),
        NeighborQuery::in_(adapter.node_id()).with_edge_type(SHARED_WITH),
    ] {
        let hits = store.neighbors(query).map_err(thg_error_from_store)?;
        if hits.iter().any(|hit| hit.node_id == tenant_node) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn adapter_nodes<S: AdapterGraphStore>(
    store: &S,
) -> ThgResult<Vec<rustyred_thg_core::NodeRecord>> {
    store
        .query_nodes(NodeQuery::label(LORA_ADAPTER_LABEL).with_limit(10_000))
        .map_err(thg_error_from_store)
}

fn fitness_signal_edge_id(
    source_node_id: &str,
    adapter_node_id: &str,
    kind: &str,
    recorded_at_ms: i64,
) -> String {
    format!("edge:{source_node_id}:fitness_signal:{adapter_node_id}:{kind}:{recorded_at_ms}")
}

fn supersedes_edge_id(new_adapter_node_id: &str, old_adapter_node_id: &str) -> String {
    format!("edge:{new_adapter_node_id}:supersedes:{old_adapter_node_id}")
}
