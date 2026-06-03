//! theorem_grpc.AppAffordanceService implementation.
//!
//! This is the live gRPC boundary for the Theseus app affordance metadata that
//! `rustyred-thg-affordances` registers as `theorem_grpc.*` tools. The first
//! slice proves transport, validation, confirmation gates, and receipt shape.
//! Concrete app-family handlers can be plugged in behind the same request path.

use std::time::Instant;

use rustyred_thg_affordances::{theseus_app_affordances, Affordance, THEOREM_GRPC_TIMEOUT_MS};
use rustyred_thg_core::stable_hash;
use serde_json::{json, Map, Value};
use theorem_harness_core::AffordanceReceipt;
use tonic::{Request, Response, Status};

use crate::pb;

#[derive(Clone, Debug, Default)]
pub struct TheoremAppAffordanceService;

#[tonic::async_trait]
impl pb::AppAffordanceService for TheoremAppAffordanceService {
    async fn invoke_affordance(
        &self,
        request: Request<pb::InvokeAffordanceRequest>,
    ) -> Result<Response<pb::InvokeAffordanceResponse>, Status> {
        let started = Instant::now();
        let req = request.into_inner();
        Ok(Response::new(invoke_registered_affordance(req, started)))
    }
}

fn invoke_registered_affordance(
    req: pb::InvokeAffordanceRequest,
    started: Instant,
) -> pb::InvokeAffordanceResponse {
    let tenant_id = normalize_tenant(&req.tenant_id);
    let requested_id = req.affordance_id.trim().to_string();
    let timeout_ms = normalized_timeout(req.timeout_ms);
    let request_json = parse_request_json(&req.request_json);

    let Some(affordance) = find_app_affordance(&tenant_id, &requested_id) else {
        return response_with_receipt(ResponseParts {
            tenant_id,
            affordance_id: requested_id,
            server_id: "theorem_grpc".to_string(),
            tool_name: String::new(),
            status: "failed".to_string(),
            executed: false,
            output: json!({}),
            error_code: "AFFORDANCE_NOT_FOUND".to_string(),
            message: "registered theorem_grpc affordance was not found".to_string(),
            actor: req.actor,
            request: request_json.unwrap_or_else(|_| json!({})),
            dry_run: req.dry_run,
            confirmed: req.confirmed,
            timeout_ms,
            elapsed_ms: started.elapsed().as_millis() as u64,
            writeback_policy: "read-only".to_string(),
        });
    };

    let request_value = match request_json {
        Ok(value) => value,
        Err(message) => {
            return response_with_receipt(ResponseParts {
                tenant_id,
                affordance_id: affordance.affordance_id.clone(),
                server_id: affordance.server_id.clone(),
                tool_name: affordance.tool_name.clone(),
                status: "failed".to_string(),
                executed: false,
                output: json!({}),
                error_code: "INVALID_REQUEST_JSON".to_string(),
                message,
                actor: req.actor,
                request: json!({}),
                dry_run: req.dry_run,
                confirmed: req.confirmed,
                timeout_ms,
                elapsed_ms: started.elapsed().as_millis() as u64,
                writeback_policy: affordance.writeback_policy.clone(),
            });
        }
    };

    if req.dry_run {
        return response_with_receipt(ResponseParts {
            tenant_id,
            affordance_id: affordance.affordance_id.clone(),
            server_id: affordance.server_id.clone(),
            tool_name: affordance.tool_name.clone(),
            status: "dry_run".to_string(),
            executed: false,
            output: affordance_metadata(&affordance),
            error_code: String::new(),
            message: "affordance is registered; dry_run skipped handler execution".to_string(),
            actor: req.actor,
            request: request_value,
            dry_run: true,
            confirmed: req.confirmed,
            timeout_ms,
            elapsed_ms: started.elapsed().as_millis() as u64,
            writeback_policy: affordance.writeback_policy.clone(),
        });
    }

    if requires_confirmation(&affordance) && !req.confirmed {
        return response_with_receipt(ResponseParts {
            tenant_id,
            affordance_id: affordance.affordance_id.clone(),
            server_id: affordance.server_id.clone(),
            tool_name: affordance.tool_name.clone(),
            status: "denied".to_string(),
            executed: false,
            output: affordance_metadata(&affordance),
            error_code: "CONFIRMATION_REQUIRED".to_string(),
            message: "affordance requires confirmation before live execution".to_string(),
            actor: req.actor,
            request: request_value,
            dry_run: false,
            confirmed: false,
            timeout_ms,
            elapsed_ms: started.elapsed().as_millis() as u64,
            writeback_policy: affordance.writeback_policy.clone(),
        });
    }

    response_with_receipt(ResponseParts {
        tenant_id,
        affordance_id: affordance.affordance_id.clone(),
        server_id: affordance.server_id.clone(),
        tool_name: affordance.tool_name.clone(),
        status: "failed".to_string(),
        executed: false,
        output: affordance_metadata(&affordance),
        error_code: "HANDLER_NOT_IMPLEMENTED".to_string(),
        message: "gRPC affordance transport is wired; concrete app handler is not implemented yet"
            .to_string(),
        actor: req.actor,
        request: request_value,
        dry_run: false,
        confirmed: req.confirmed,
        timeout_ms,
        elapsed_ms: started.elapsed().as_millis() as u64,
        writeback_policy: affordance.writeback_policy.clone(),
    })
}

struct ResponseParts {
    tenant_id: String,
    affordance_id: String,
    server_id: String,
    tool_name: String,
    status: String,
    executed: bool,
    output: Value,
    error_code: String,
    message: String,
    actor: String,
    request: Value,
    dry_run: bool,
    confirmed: bool,
    timeout_ms: u64,
    elapsed_ms: u64,
    writeback_policy: String,
}

fn response_with_receipt(parts: ResponseParts) -> pb::InvokeAffordanceResponse {
    let input_hash = stable_hash(json!({
        "tenant_id": parts.tenant_id,
        "affordance_id": parts.affordance_id,
        "actor": parts.actor,
        "request": parts.request,
        "dry_run": parts.dry_run,
        "confirmed": parts.confirmed,
        "timeout_ms": parts.timeout_ms,
    }));

    let mut payload = Map::new();
    payload.insert("tenant_id".to_string(), json!(parts.tenant_id));
    payload.insert("affordance_id".to_string(), json!(parts.affordance_id));
    payload.insert("server_id".to_string(), json!(parts.server_id));
    payload.insert("tool_name".to_string(), json!(parts.tool_name));
    payload.insert("status".to_string(), json!(parts.status));
    payload.insert("executed".to_string(), json!(parts.executed));
    payload.insert("output".to_string(), parts.output.clone());
    payload.insert("error_code".to_string(), json!(parts.error_code));
    payload.insert("message".to_string(), json!(parts.message));
    payload.insert("actor".to_string(), json!(parts.actor));
    payload.insert("dry_run".to_string(), json!(parts.dry_run));
    payload.insert("confirmed".to_string(), json!(parts.confirmed));
    payload.insert("timeout_ms".to_string(), json!(parts.timeout_ms));
    payload.insert("elapsed_ms".to_string(), json!(parts.elapsed_ms));

    let receipt = AffordanceReceipt::new(
        parts.server_id.clone(),
        parts.affordance_id.clone(),
        input_hash,
        payload,
    )
    .with_writeback_policy(parts.writeback_policy);
    let receipt_hash = receipt.receipt_hash.clone();
    let receipt_json = serde_json::to_string(&receipt).unwrap_or_else(|_| "{}".to_string());
    let output_json = serde_json::to_string(&parts.output).unwrap_or_else(|_| "{}".to_string());

    pb::InvokeAffordanceResponse {
        tenant_id: parts.tenant_id,
        affordance_id: parts.affordance_id,
        server_id: parts.server_id,
        tool_name: parts.tool_name,
        status: parts.status,
        executed: parts.executed,
        receipt_hash,
        receipt_json,
        output_json,
        error_code: parts.error_code,
        message: parts.message,
        elapsed_ms: parts.elapsed_ms,
    }
}

fn find_app_affordance(tenant_id: &str, requested_id: &str) -> Option<Affordance> {
    if requested_id.is_empty() {
        return None;
    }
    theseus_app_affordances(tenant_id)
        .into_iter()
        .find(|affordance| {
            affordance.affordance_id == requested_id || affordance.tool_name == requested_id
        })
}

fn parse_request_json(raw: &str) -> Result<Value, String> {
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(raw).map_err(|err| format!("request_json must be valid JSON: {err}"))
}

fn normalize_tenant(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        "theorem".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalized_timeout(raw: u64) -> u64 {
    if raw == 0 {
        THEOREM_GRPC_TIMEOUT_MS
    } else {
        raw.min(THEOREM_GRPC_TIMEOUT_MS)
    }
}

fn requires_confirmation(affordance: &Affordance) -> bool {
    if !matches!(
        affordance.writeback_policy.as_str(),
        "read-only" | "receipt-only"
    ) {
        return true;
    }
    affordance
        .permissions
        .iter()
        .chain(&affordance.tags)
        .any(|value| {
            matches!(
                value.as_str(),
                "external_action" | "private_write" | "write" | "writeback"
            )
        })
}

fn affordance_metadata(affordance: &Affordance) -> Value {
    json!({
        "family": affordance.family,
        "label": affordance.label,
        "permissions": affordance.permissions,
        "writeback_policy": affordance.writeback_policy,
        "cost": affordance.cost,
        "tags": affordance.tags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_returns_registered_affordance_receipt() {
        let response = invoke_registered_affordance(
            pb::InvokeAffordanceRequest {
                tenant_id: "theorem".to_string(),
                affordance_id: "theorem_grpc.publisher.publish".to_string(),
                actor: "test".to_string(),
                request_json: r#"{"artifact_id":"a1"}"#.to_string(),
                dry_run: true,
                confirmed: false,
                timeout_ms: 0,
            },
            Instant::now(),
        );

        assert_eq!(response.status, "dry_run");
        assert!(!response.executed);
        assert_eq!(response.server_id, "theorem_grpc");
        assert_eq!(response.tool_name, "publisher.publish");
        assert!(!response.receipt_hash.is_empty());
        assert!(response
            .receipt_json
            .contains("theorem_grpc.publisher.publish"));
    }

    #[test]
    fn external_write_requires_confirmation() {
        let response = invoke_registered_affordance(
            pb::InvokeAffordanceRequest {
                tenant_id: "theorem".to_string(),
                affordance_id: "theorem_grpc.publisher.publish".to_string(),
                actor: "test".to_string(),
                request_json: "{}".to_string(),
                dry_run: false,
                confirmed: false,
                timeout_ms: 0,
            },
            Instant::now(),
        );

        assert_eq!(response.status, "denied");
        assert_eq!(response.error_code, "CONFIRMATION_REQUIRED");
        assert!(!response.executed);
    }

    #[test]
    fn confirmed_known_affordance_returns_not_implemented_receipt() {
        let response = invoke_registered_affordance(
            pb::InvokeAffordanceRequest {
                tenant_id: "theorem".to_string(),
                affordance_id: "theorem_grpc.research.expand".to_string(),
                actor: "test".to_string(),
                request_json: "{}".to_string(),
                dry_run: false,
                confirmed: true,
                timeout_ms: 42_000,
            },
            Instant::now(),
        );

        assert_eq!(response.status, "failed");
        assert_eq!(response.error_code, "HANDLER_NOT_IMPLEMENTED");
        assert!(!response.executed);
        assert!(response.output_json.contains("\"timeout_ms\":30000"));
    }

    #[test]
    fn invalid_json_is_receipted_as_failure() {
        let response = invoke_registered_affordance(
            pb::InvokeAffordanceRequest {
                tenant_id: "theorem".to_string(),
                affordance_id: "theorem_grpc.research.expand".to_string(),
                actor: "test".to_string(),
                request_json: "{broken".to_string(),
                dry_run: false,
                confirmed: true,
                timeout_ms: 0,
            },
            Instant::now(),
        );

        assert_eq!(response.status, "failed");
        assert_eq!(response.error_code, "INVALID_REQUEST_JSON");
        assert!(!response.receipt_hash.is_empty());
    }
}
