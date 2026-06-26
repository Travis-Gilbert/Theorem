//! Theorem's first gRPC server.
//!
//! Serves theseus_search.v1.SearchService over the RustyRed substrate. gRPC is
//! merged with tiny HTTP `/health` and `/ready` routes on the same listener so
//! deploys can distinguish "process is alive" from "RedCore is still
//! recovering". Binds [::]:$PORT (IPv6 dual-stack) so Railway's private network
//! reaches it via theorem-grpc.railway.internal, and IPv4 healthchecks work too.
//! The civic-atlas-server dials this by setting THEOREM_SEARCH_URL (or the
//! legacy THESEUS_BRIDGE_URL).

mod app_affordance;
mod code_index;
mod code_kg;
mod code_service;
mod engine;
mod pb;
mod service;
mod session_delta;
mod valkey_cache;

use std::net::SocketAddr;
use std::sync::Arc;

use app_affordance::TheoremAppAffordanceService;
use axum::{extract::State, http::StatusCode, routing::get, Json};
use code_index::CodeIndexRuntime;
use code_service::TheoremCodeCrawlerService;
use engine::Engine;
use pb::{AppAffordanceServiceServer, CodeCrawlerServiceServer, SearchServiceServer};
use serde_json::{json, Value};
use service::TheoremSearchService;
use tokio::net::TcpListener;
use valkey_cache::ValkeyCache;

#[derive(Clone)]
struct ReadinessState {
    code_index: CodeIndexRuntime,
    app_affordance: TheoremAppAffordanceService,
    valkey_cache: ValkeyCache,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    // Railway injects PORT. Default 50071 for local dev (a free gRPC-ish port).
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(50071);

    // Bind [::] (IPv6 dual-stack) so Railway's IPv6 private network reaches it
    // via the railway.internal domain; it also accepts IPv4 for healthchecks.
    let addr: SocketAddr = format!("[::]:{port}").parse()?;

    // Build the engine (empty substrate is the honest slice-1 default) and wrap
    // it in an Arc so the owned store outlives every borrowing handler call.
    let engine = Arc::new(Engine::new());
    let valkey_cache = ValkeyCache::from_env();
    match valkey_cache.ping() {
        Ok(Some(pong)) => tracing::info!("THEOREM_GRPC_VALKEY_READY {}", pong),
        Ok(None) => tracing::info!("THEOREM_GRPC_VALKEY_DISABLED"),
        Err(error) => tracing::warn!("THEOREM_GRPC_VALKEY_UNREACHABLE {}", error),
    }
    // ONE code store for the whole service. It starts in "recovering" mode so
    // the socket can bind before RedCore replays /data. Code calls return
    // UNAVAILABLE until the background recovery swaps in the durable store.
    let code_index = CodeIndexRuntime::recovering().map_err(std::io::Error::other)?;
    let app_affordance = TheoremAppAffordanceService::recovering_with_code_index_and_cache(
        code_index.clone(),
        valkey_cache.clone(),
    )
    .map_err(std::io::Error::other)?;
    let readiness = ReadinessState {
        code_index: code_index.clone(),
        app_affordance: app_affordance.clone(),
        valkey_cache: valkey_cache.clone(),
    };
    let search_svc =
        SearchServiceServer::new(TheoremSearchService::new(engine, valkey_cache.clone()));
    let code_svc =
        CodeCrawlerServiceServer::new(TheoremCodeCrawlerService::new(code_index.clone()));
    let app_affordance_svc = AppAffordanceServiceServer::new(app_affordance);

    let grpc = tonic::transport::Server::builder()
        .add_service(search_svc)
        .add_service(code_svc)
        .add_service(app_affordance_svc);
    #[allow(deprecated)]
    let grpc = grpc.into_router();
    let app = axum::Router::new()
        .route(
            "/.well-known/theorems-harness/doctor.json",
            get(doctor_manifest),
        )
        .route("/diagnostics/dependencies", get(dependency_diagnostics))
        .route("/diagnostics/queue", get(queue_diagnostics))
        .route("/diagnostics/tenants", get(tenant_diagnostics))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .with_state(readiness)
        .merge(grpc);

    let listener = TcpListener::bind(addr).await?;
    tracing::info!("THEOREM_GRPC_BOUND {}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("theorem-grpc server stopped");
    Ok(())
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "status": "alive" }))
}

async fn doctor_manifest() -> Json<Value> {
    Json(doctor_manifest_json())
}

fn doctor_manifest_json() -> Value {
    json!({
        "schema_version": 1,
        "product": "Theorems-Harness",
        "service": "theorem-grpc",
        "description": "Remote doctor contract for theorem-grpc liveness, readiness, dependency isolation, async work, and tenant guardrails.",
        "endpoints": {
            "health": "/health",
            "ready": "/ready",
            "queue": "/diagnostics/queue",
            "dependencies": "/diagnostics/dependencies",
            "tenants": "/diagnostics/tenants",
        },
        "scope": {
            "owns": [
                "code_indexing",
                "code_search_affordances",
                "search_grpc",
                "app_affordance_grpc",
            ],
            "reports_gaps_for": [
                "agent_runs",
                "recall_hydration",
                "graph_compilation",
                "provider_calls",
                "tenant_product_policy",
            ],
        },
    })
}

async fn ready(State(state): State<ReadinessState>) -> (StatusCode, Json<Value>) {
    let code_index = state.code_index.diagnostics();
    let app_affordance = state.app_affordance.recovery_snapshot();
    let code_phase = code_index
        .as_ref()
        .map(|diagnostics| diagnostics.recovery.phase.as_str())
        .unwrap_or("failed");
    let app_phase = app_affordance.phase.as_str();
    let ready = code_phase == "ready" && app_phase == "ready";
    let failed = code_phase == "failed" || app_phase == "failed" || code_index.is_err();
    let status = if ready {
        "ready"
    } else if failed {
        "failed"
    } else {
        "recovering"
    };
    let status_code = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let code_index_json = match code_index {
        Ok(diagnostics) => diagnostics.to_json(),
        Err(error) => json!({
            "recovery": {
                "phase": "failed",
                "error": error.to_string(),
            }
        }),
    };
    (
        status_code,
        Json(json!({
            "ok": ready,
            "status": status,
            "code_index": code_index_json,
            "app_affordance": app_affordance.to_json(),
        })),
    )
}

async fn queue_diagnostics(State(state): State<ReadinessState>) -> Json<Value> {
    Json(queue_diagnostics_json(&state))
}

fn queue_diagnostics_json(state: &ReadinessState) -> Value {
    let code_index = state
        .code_index
        .diagnostics()
        .map(|diagnostics| diagnostics.to_json())
        .unwrap_or_else(|error| {
            json!({
                "recovery": {
                    "phase": "failed",
                    "error": error.to_string(),
                }
            })
        });
    json!({
        "schema_version": 1,
        "service": "theorem-grpc",
        "categories": {
            "agent_runs": queue_gap_category(
                "theorem-harness-server / theorem-dispatch",
                "not owned by theorem-grpc"
            ),
            "code_indexing": queue_category(json!({
                "async_default": true,
                "durable_queue": true,
                "durable": true,
                "leases": false,
                "heartbeats": false,
                "retries": true,
                "reaper": true,
                "public_contract": "202_job_id",
                "status": "durable_without_leases",
                "owned_by": "theorem-grpc",
                "details": {
                    "job_id_returned_immediately": true,
                    "heavy_phase_off_request_path": true,
                    "durable_mirror": "CodeIngestJob nodes in RedCore",
                    "interrupted_jobs_recovered": true,
                    "lease_model": "single local worker; no explicit lease/heartbeat fields yet",
                    "diagnostics": code_index,
                }
            })),
            "recall_hydration": queue_gap_category(
                "theorems-harness recall runtime",
                "not owned by theorem-grpc"
            ),
            "graph_compilation": queue_gap_category(
                "rustyred-thg-core / compiler runtime",
                "not owned by theorem-grpc"
            ),
            "provider_calls": queue_category(json!({
                "async_default": false,
                "durable_queue": false,
                "durable": false,
                "leases": false,
                "heartbeats": false,
                "retries": false,
                "reaper": false,
                "public_contract": "structured_timeout",
                "status": "structured_timeout_only",
                "owned_by": "theorem-grpc app affordance adapter",
                "details": {
                    "degrades_by_feature": true,
                    "next_fix": "move provider calls through a durable worker queue before remote doctor should treat this as ready"
                }
            })),
        },
    })
}

fn queue_category(payload: Value) -> Value {
    payload
}

fn queue_gap_category(owner_hint: &str, reason: &str) -> Value {
    json!({
        "async_default": false,
        "durable_queue": false,
        "durable": false,
        "leases": false,
        "heartbeats": false,
        "retries": false,
        "reaper": false,
        "public_contract": "not_exposed_by_this_service",
        "status": "not_owned_by_theorem_grpc",
        "owner_hint": owner_hint,
        "reason": reason,
    })
}

async fn dependency_diagnostics(State(state): State<ReadinessState>) -> Json<Value> {
    Json(dependency_diagnostics_json(&state))
}

fn dependency_diagnostics_json(state: &ReadinessState) -> Value {
    let code_index = state.code_index.diagnostics();
    let app_affordance = state.app_affordance.recovery_snapshot();
    let code_phase = code_index
        .as_ref()
        .map(|diagnostics| diagnostics.recovery.phase.as_str())
        .unwrap_or("failed");
    let app_phase = app_affordance.phase.as_str();
    let rustyred_status = if code_phase == "ready" && app_phase == "ready" {
        "ready"
    } else if code_phase == "failed" || app_phase == "failed" || code_index.is_err() {
        "degraded"
    } else {
        "recovering"
    };
    let code_index_json = code_index
        .map(|diagnostics| diagnostics.to_json())
        .unwrap_or_else(|error| {
            json!({
                "recovery": {
                    "phase": "failed",
                    "error": error.to_string(),
                }
            })
        });
    let valkey_metrics = state.valkey_cache.metrics();
    let valkey_status = if !state.valkey_cache.is_enabled() {
        "disabled"
    } else if valkey_metrics.errors > 0 {
        "degraded"
    } else {
        "ok"
    };

    json!({
        "schema_version": 1,
        "service": "theorem-grpc",
        "dependencies": {
            "deepseek": {
                "status": secret_dependency_status(&["DEEPSEEK_API_KEY", "DEEPSEEK_API_BASE"]),
                "isolated": true,
                "blast_radius": "feature_only",
                "required_for_core_liveness": false,
            },
            "valkey": {
                "status": valkey_status,
                "isolated": true,
                "blast_radius": "feature_only",
                "required_for_core_liveness": false,
                "enabled": state.valkey_cache.is_enabled(),
                "key_prefix": state.valkey_cache.key_prefix(),
                "ttl_seconds": state.valkey_cache.ttl_seconds(),
                "metrics": {
                    "hits": valkey_metrics.hits,
                    "misses": valkey_metrics.misses,
                    "writes": valkey_metrics.writes,
                    "errors": valkey_metrics.errors,
                },
                "note": "diagnostics avoid live network ping so dependency checks cannot stall the service"
            },
            "rustyred": {
                "status": rustyred_status,
                "isolated": true,
                "blast_radius": "readiness_only",
                "required_for_core_liveness": false,
                "recovery": {
                    "code_index": code_index_json,
                    "app_affordance": app_affordance.to_json(),
                }
            },
            "recall_index": {
                "status": "disabled",
                "isolated": true,
                "blast_radius": "feature_only",
                "required_for_core_liveness": false,
                "reason": "theorem-grpc does not own recall hydration/index readiness"
            }
        },
    })
}

fn secret_dependency_status(names: &[&str]) -> &'static str {
    if names.iter().any(|name| {
        std::env::var(name)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
    }) {
        "ok"
    } else {
        "missing_token"
    }
}

async fn tenant_diagnostics() -> Json<Value> {
    Json(tenant_diagnostics_json())
}

fn tenant_diagnostics_json() -> Value {
    tenant_diagnostics_json_with_flags(
        env_flag("THEOREM_GRPC_TENANT_QUOTAS_READY"),
        env_flag("THEOREM_GRPC_TENANT_CONCURRENCY_LIMITS_READY"),
        env_flag("THEOREM_GRPC_TENANT_RATE_LIMITS_READY"),
        env_flag("THEOREM_GRPC_NOISY_NEIGHBOR_PROTECTION_READY"),
    )
}

fn tenant_diagnostics_json_with_flags(
    quotas: bool,
    concurrency_limits: bool,
    rate_limits: bool,
    noisy_neighbor_protection: bool,
) -> Value {
    json!({
        "schema_version": 1,
        "service": "theorem-grpc",
        "default_policy": {
            "quotas": quotas,
            "concurrency_limits": concurrency_limits,
            "queue_isolation": true,
            "rate_limits": rate_limits,
            "storage_namespaces": true,
            "noisy_neighbor_protection": noisy_neighbor_protection,
        },
        "implemented": {
            "tenant_id_required_for_code_affordances": true,
            "tenant_guarded_job_status_and_watch": true,
            "tenant_id_persisted_on_code_nodes": true,
            "strict_per_tenant_quotas": quotas,
            "strict_per_tenant_concurrency_limits": concurrency_limits,
            "strict_per_tenant_rate_limits": rate_limits,
            "strict_noisy_neighbor_reaper": noisy_neighbor_protection,
        },
        "policy_source": "runtime_env_and_code_contract",
        "env_flags": {
            "quotas": "THEOREM_GRPC_TENANT_QUOTAS_READY",
            "concurrency_limits": "THEOREM_GRPC_TENANT_CONCURRENCY_LIMITS_READY",
            "rate_limits": "THEOREM_GRPC_TENANT_RATE_LIMITS_READY",
            "noisy_neighbor_protection": "THEOREM_GRPC_NOISY_NEIGHBOR_PROTECTION_READY",
        },
    })
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

/// Wait for SIGTERM (production / Docker / Railway) or Ctrl-C (dev). First
/// signal to fire wins; both are clean shutdown paths. Copied from
/// rustyred-thg-server/src/main.rs for clean Railway restarts.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_manifest_lists_remote_doctor_probe_paths() {
        let manifest = doctor_manifest_json();
        let endpoints = &manifest["endpoints"];
        assert_eq!(endpoints["health"], "/health");
        assert_eq!(endpoints["ready"], "/ready");
        assert_eq!(endpoints["queue"], "/diagnostics/queue");
        assert_eq!(endpoints["dependencies"], "/diagnostics/dependencies");
        assert_eq!(endpoints["tenants"], "/diagnostics/tenants");
    }

    #[test]
    fn tenant_diagnostics_do_not_claim_strict_guards_by_default() {
        let diagnostics = tenant_diagnostics_json_with_flags(false, false, false, false);
        let policy = &diagnostics["default_policy"];
        assert_eq!(policy["storage_namespaces"], true);
        assert_eq!(policy["queue_isolation"], true);
        assert_eq!(policy["quotas"], false);
        assert_eq!(policy["concurrency_limits"], false);
        assert_eq!(policy["rate_limits"], false);
        assert_eq!(policy["noisy_neighbor_protection"], false);
    }
}
