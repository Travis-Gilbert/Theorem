use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;

use crate::auth::require_scope;
use crate::config::StorageMode;
use crate::state::AppState;

/// `GET /metrics` — Prometheus text exposition.
///
/// Returns counters in `# HELP / # TYPE / name value\n` form. The mime type
/// is the Prometheus 0.0.4 text exposition format.
pub async fn metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    require_scope(
        &headers,
        &state.config.api_tokens,
        "admin:read",
        state.config.require_auth,
    )?;
    let mut body = state.observability.render_prometheus();
    if let Ok(tenant_metrics) = state.render_tenant_engine_prometheus() {
        body.push_str(&tenant_metrics);
    }
    let mut resp = (StatusCode::OK, body).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    Ok(resp)
}

/// `GET /v1/diagnostics/slow_queries` — returns the slow-query ring buffer.
pub async fn slow_queries(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    require_scope(
        &headers,
        &state.config.api_tokens,
        "admin:read",
        state.config.require_auth,
    )?;
    let entries = state.observability.snapshot_slow_queries();
    let entries_json: Vec<Value> = entries
        .into_iter()
        .map(|e| {
            json!({
                "recorded_at_unix_ms": e.recorded_at_unix_ms.to_string(),
                "nanos": e.nanos,
                "kind": e.kind,
                "detail": e.detail,
                "nodes_visited": e.nodes_visited,
                "edges_touched": e.edges_touched,
            })
        })
        .collect();
    Ok(Json(json!({
        "entries": entries_json,
        "count": entries_json.len(),
    })))
}

/// `GET /v1/diagnostics/config` — exposes static configuration (previously
/// served by `/metrics`). Kept for backward compatibility with operators.
pub async fn diagnostics_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    require_scope(
        &headers,
        &state.config.api_tokens,
        "admin:read",
        state.config.require_auth,
    )?;
    let tenant_memory_quota_supported = matches!(
        state.config.storage_mode,
        StorageMode::Embedded | StorageMode::Memory
    );
    let tenant_override_detail = state
        .config
        .tenant_config_overrides
        .iter()
        .map(|(tenant, config)| {
            let mut detail = Map::new();
            if let Some(durability) = &config.durability {
                detail.insert(
                    "durability".to_string(),
                    Value::String(durability.as_str().to_string()),
                );
            }
            if let Some(snapshot_interval_writes) = config.snapshot_interval_writes {
                detail.insert(
                    "snapshot_interval_writes".to_string(),
                    json!(snapshot_interval_writes),
                );
            }
            if let Some(strict_acid) = config.strict_acid {
                detail.insert("strict_acid".to_string(), json!(strict_acid));
            }
            if let Some(tenant_memory_quota_bytes) = config.tenant_memory_quota_bytes {
                detail.insert(
                    "tenant_memory_quota_bytes".to_string(),
                    json!(tenant_memory_quota_bytes),
                );
            }
            if let Some(hybrid_scoring) = &config.hybrid_scoring {
                detail.insert(
                    "hybrid_scoring".to_string(),
                    json!({
                        "alpha": hybrid_scoring.alpha,
                        "confidence_weighted_graph_distance": hybrid_scoring.confidence_weighted_graph_distance,
                        "edge_type_weights": &hybrid_scoring.edge_type_weights,
                    }),
                );
            }
            (tenant.clone(), Value::Object(detail))
        })
        .collect::<Map<String, Value>>();
    // TTL-04: aggregate TTL counters across all materialized tenants
    // (sum ttl_active_count) plus sweep loop telemetry.
    let ttl_active_count_total: usize = state
        .iter_redcore_tenants()
        .map(|tenants| {
            tenants
                .iter()
                .map(|(_, executor)| executor.ttl_active_count().unwrap_or(0))
                .sum()
        })
        .unwrap_or(0);

    Ok(Json(json!({
        "service": state.config.service_name.as_str(),
        "status": "ok",
        "auth_required": state.config.require_auth,
        "configured_origins": state.config.allowed_origins.len(),
        "storage_mode": state.config.storage_mode.as_str(),
        "durability": state.config.durability.as_str(),
        "strict_acid": state.config.strict_acid,
        "tenant_memory_quota_bytes": state.config.tenant_memory_quota_bytes,
        "tenant_memory_quota_supported": tenant_memory_quota_supported,
        "tenant_memory_quota_enforced": tenant_memory_quota_supported
            && state.config.tenant_memory_quota_bytes > 0,
        "tenant_idle_ms": state.config.tenant_idle_ms,
        "tenant_warm_pool_size": state.config.tenant_warm_pool_size,
        "slow_query_threshold_nanos": state.config.slow_query_threshold_nanos,
        "slow_query_capacity": state.config.slow_query_capacity,
        "slow_query_log_enabled": state.config.slow_query_log.is_some(),
        "hybrid_scoring": {
            "alpha": state.config.hybrid_scoring.alpha,
            "confidence_weighted_graph_distance": state.config.hybrid_scoring.confidence_weighted_graph_distance,
            "edge_type_weights": &state.config.hybrid_scoring.edge_type_weights,
        },
        "tenant_config_overrides": state.config.tenant_config_overrides.len(),
        "tenant_config_runtime_mutation_supported": false,
        "tenant_config_tenants": state
            .config
            .tenant_config_overrides
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        "tenant_config_overrides_detail": tenant_override_detail,
        "ttl_sweep": {
            "interval_ms": state.config.ttl_sweep_ms,
            "active_count": ttl_active_count_total,
            "swept_total": state.ttl_sweep.swept_total(),
            "last_sweep_at_ms": state.ttl_sweep.last_sweep_at_ms(),
            "sweep_duration_p99_ms": state.ttl_sweep.sweep_duration_p99_ms(),
        },
    })))
}

/// `GET /v1/diagnostics/memory` — operator snapshot for process and tenant
/// graph memory. The tenant sections only include already materialized tenants;
/// reading this route does not lazily open on-disk graphs.
pub async fn diagnostics_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    require_scope(
        &headers,
        &state.config.api_tokens,
        "admin:read",
        state.config.require_auth,
    )?;

    let redcore_tenants = state
        .iter_redcore_tenants()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut tenant_versions = BTreeMap::new();
    let tenant_reports = redcore_tenants
        .iter()
        .map(|(tenant, executor)| match executor.cheap_stats() {
            Ok(stats) => {
                tenant_versions.insert(tenant.clone(), stats.version);
                json!({
                    "tenant": tenant,
                    "stats": stats,
                    "stats_memory_bytes_estimated": false,
                    "cached_edges": executor.cached_edges_diagnostics(),
                })
            }
            Err(error) => json!({
                "tenant": tenant,
                "error": {
                    "code": error.code,
                    "message": error.message,
                },
                "cached_edges": executor.cached_edges_diagnostics(),
            }),
        })
        .collect::<Vec<_>>();

    let graph_caches = state
        .iter_graph_caches()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let cache_reports = graph_caches
        .iter()
        .map(|(tenant, cache)| {
            let graph_version = tenant_versions.get(tenant).copied().unwrap_or(0);
            match cache.stats(graph_version) {
                Ok(stats) => json!({
                    "tenant": tenant,
                    "graph_version_source": if tenant_versions.contains_key(tenant) {
                        "materialized_redcore_tenant"
                    } else {
                        "default_zero"
                    },
                    "cache": stats,
                }),
                Err(error) => json!({
                    "tenant": tenant,
                    "error": {
                        "code": error.code,
                        "message": error.message,
                    },
                }),
            }
        })
        .collect::<Vec<_>>();
    let tenant_engines = state
        .tenant_engine_reports()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({
        "service": state.config.service_name.as_str(),
        "status": "ok",
        "storage_mode": state.config.storage_mode.as_str(),
        "process": process_memory_snapshot(),
        "ppr_cache": {
            "scoped_entries": rustyred_thg_core::scoped_ppr_cache_len(),
        },
        "redcore_tenant_count": tenant_reports.len(),
        "redcore_tenants": tenant_reports,
        "tenant_engine_count": tenant_engines.len(),
        "tenant_engines": tenant_engines,
        "graph_cache_tenant_count": cache_reports.len(),
        "graph_caches": cache_reports,
    })))
}

fn process_memory_snapshot() -> Value {
    match fs::read_to_string("/proc/self/status") {
        Ok(status) => json!({
            "source": "/proc/self/status",
            "available": true,
            "vm_rss_bytes": status_kib(&status, "VmRSS").map(kib_to_bytes),
            "vm_hwm_bytes": status_kib(&status, "VmHWM").map(kib_to_bytes),
            "vm_size_bytes": status_kib(&status, "VmSize").map(kib_to_bytes),
            "vm_data_bytes": status_kib(&status, "VmData").map(kib_to_bytes),
        }),
        Err(error) => json!({
            "source": "/proc/self/status",
            "available": false,
            "error": error.to_string(),
        }),
    }
}

fn status_kib(status: &str, key: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.trim_start_matches(':').trim();
        value.split_whitespace().next()?.parse::<u64>().ok()
    })
}

fn kib_to_bytes(kib: u64) -> u64 {
    kib.saturating_mul(1024)
}
