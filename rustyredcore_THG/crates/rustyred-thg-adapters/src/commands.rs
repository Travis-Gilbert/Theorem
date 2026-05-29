use serde::Deserialize;
use serde_json::{json, Value};

use rustyred_thg_core::{ThgCommand, ThgError, ThgResponse};

use crate::fitness::{find_adapter_by_id, list_adapters, record_fitness, supersede_adapter};
use crate::routing::find_adapters_for;
use crate::types::{
    AdapterFindRequest, AdapterFitnessRecordRequest, AdapterGraphStore, AdapterListRequest,
    LoraAdapter,
};
use crate::upsert::upsert_adapter;

pub type AdapterCommandResponse = ThgResponse;

#[derive(Debug, Deserialize)]
struct AdapterUpsertArgs {
    #[serde(default)]
    adapter: Option<LoraAdapter>,
    #[serde(default)]
    derived_from_adapter_id: Option<String>,
    #[serde(default)]
    actor: Option<String>,
    #[serde(flatten)]
    flattened_adapter: serde_json::Map<String, Value>,
}

#[derive(Debug, Deserialize)]
struct AdapterGetArgs {
    adapter_id: String,
}

#[derive(Debug, Deserialize)]
struct AdapterSupersedeArgs {
    old_adapter_id: String,
    new_adapter_id: String,
    #[serde(default)]
    archive_old: bool,
    #[serde(default)]
    actor: Option<String>,
}

pub fn execute_adapter_command<S: AdapterGraphStore>(
    store: &mut S,
    command_name: &str,
    args: Value,
    state_hash: impl Into<String>,
) -> AdapterCommandResponse {
    let state_hash = state_hash.into();
    let command = match ThgCommand::from_name(command_name) {
        Ok(command) => command,
        Err(error) => return ThgResponse::err(command_name, error, state_hash),
    };
    let result = match command {
        ThgCommand::AdaptersUpsert => adapter_upsert(store, args),
        ThgCommand::AdaptersFind => adapter_find(store, args),
        ThgCommand::AdaptersGet => adapter_get(store, args),
        ThgCommand::AdaptersFitnessRecord => adapter_fitness_record(store, args),
        ThgCommand::AdaptersList => adapter_list(store, args),
        ThgCommand::AdaptersSupersede => adapter_supersede(store, args),
        _ => Err(ThgError::unsupported_command(command.name())),
    };
    match result {
        Ok(payload) => ThgResponse::ok(command.name(), "ok", payload, state_hash),
        Err(error) => ThgResponse::err(command.name(), error, state_hash),
    }
}

fn adapter_upsert<S: AdapterGraphStore>(store: &mut S, args: Value) -> Result<Value, ThgError> {
    let parsed: AdapterUpsertArgs = serde_json::from_value(args.clone())
        .map_err(|error| ThgError::new("invalid_adapter_request", error.to_string()))?;
    let adapter = if let Some(adapter) = parsed.adapter {
        adapter
    } else {
        serde_json::from_value(Value::Object(parsed.flattened_adapter))
            .map_err(|error| ThgError::new("invalid_adapter_request", error.to_string()))?
    };
    let result = upsert_adapter(
        store,
        adapter,
        parsed.derived_from_adapter_id.as_deref(),
        parsed.actor.as_deref(),
    )?;
    Ok(json!({ "result": result }))
}

fn adapter_find<S: AdapterGraphStore>(store: &S, args: Value) -> Result<Value, ThgError> {
    let request = adapter_find_request_from_value(args)?;
    let refs = find_adapters_for(store, &request)?;
    Ok(json!({
        "adapters": refs,
        "stats": { "returned": refs.len() },
    }))
}

fn adapter_get<S: AdapterGraphStore>(store: &S, args: Value) -> Result<Value, ThgError> {
    let parsed: AdapterGetArgs = serde_json::from_value(args)
        .map_err(|error| ThgError::new("invalid_adapter_request", error.to_string()))?;
    Ok(json!({ "adapter": find_adapter_by_id(store, &parsed.adapter_id)? }))
}

fn adapter_fitness_record<S: AdapterGraphStore>(
    store: &mut S,
    args: Value,
) -> Result<Value, ThgError> {
    let actor = args
        .get("actor")
        .and_then(Value::as_str)
        .map(str::to_string);
    let request: AdapterFitnessRecordRequest = serde_json::from_value(args)
        .map_err(|error| ThgError::new("invalid_adapter_request", error.to_string()))?;
    let result = record_fitness(store, request, actor.as_deref())?;
    Ok(json!({ "result": result }))
}

fn adapter_list<S: AdapterGraphStore>(store: &S, args: Value) -> Result<Value, ThgError> {
    let request = adapter_list_request_from_value(args)?;
    let adapters = list_adapters(store, request)?;
    Ok(json!({
        "adapters": adapters,
        "stats": { "returned": adapters.len() },
    }))
}

fn adapter_supersede<S: AdapterGraphStore>(store: &mut S, args: Value) -> Result<Value, ThgError> {
    let parsed: AdapterSupersedeArgs = serde_json::from_value(args)
        .map_err(|error| ThgError::new("invalid_adapter_request", error.to_string()))?;
    let result = supersede_adapter(
        store,
        &parsed.old_adapter_id,
        &parsed.new_adapter_id,
        parsed.archive_old,
        parsed.actor.as_deref(),
    )?;
    Ok(json!({ "result": result }))
}

fn adapter_find_request_from_value(args: Value) -> Result<AdapterFindRequest, ThgError> {
    let tenant_id = required_string(&args, "tenant_id")?;
    let seed_node_ids = args
        .get("seed_node_ids")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(AdapterFindRequest {
        tenant_id,
        seed_node_ids,
        k: optional_u32(&args, "k").unwrap_or(10),
        base_model_sha: args
            .get("base_model_sha")
            .and_then(Value::as_str)
            .map(str::to_string),
        include_superseded: args
            .get("include_superseded")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        min_fitness: args
            .get("min_fitness")
            .and_then(Value::as_f64)
            .map(|value| value as f32),
        ppr_damping: args
            .get("ppr_damping")
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(crate::types::DEFAULT_PPR_DAMPING),
        ppr_max_iter: optional_u32(&args, "ppr_max_iter")
            .unwrap_or(crate::types::DEFAULT_PPR_MAX_PUSHES),
        shared_weight: args
            .get("shared_weight")
            .and_then(Value::as_f64)
            .map(|value| value as f32),
    })
}

fn adapter_list_request_from_value(args: Value) -> Result<AdapterListRequest, ThgError> {
    Ok(AdapterListRequest {
        tenant_id: required_string(&args, "tenant_id")?,
        base_model_sha: args
            .get("base_model_sha")
            .and_then(Value::as_str)
            .map(str::to_string),
        min_fitness: args
            .get("min_fitness")
            .and_then(Value::as_f64)
            .map(|value| value as f32),
        include_superseded: args
            .get("include_superseded")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn required_string(args: &Value, key: &str) -> Result<String, ThgError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ThgError::new("invalid_adapter_request", format!("{key} is required")))
}

fn optional_u32(args: &Value, key: &str) -> Option<u32> {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}
