//! theorem_grpc.AppAffordanceService implementation.
//!
//! This is the live gRPC boundary for the Theseus app affordance metadata that
//! `rustyred-thg-affordances` registers as `theorem_grpc.*` tools. The service
//! owns a graph-backed runtime: it dispatches concrete local handlers, records
//! invocation outcomes into the affordance graph, and returns the same
//! content-addressed receipt envelope the harness already understands.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use rustyred_thg_affordances::{
    record_invocation, register_theseus_app_affordances, select_affordances,
    theseus_app_affordances, Affordance, AffordanceGraphStore, CapabilityScope,
    InvocationRecordRequest, InvocationRecordResult, SelectionRequest, THEOREM_GRPC_TIMEOUT_MS,
};
use rustyred_thg_core::{stable_hash, InMemoryGraphStore};
use serde_json::{json, Map, Value};
use theorem_harness_core::AffordanceReceipt;
use tonic::{Request, Response, Status};

use crate::pb;

#[derive(Clone)]
pub struct TheoremAppAffordanceService {
    runtime: AppAffordanceRuntime,
}

impl TheoremAppAffordanceService {
    pub fn new() -> Self {
        Self {
            runtime: AppAffordanceRuntime::new(),
        }
    }
}

impl Default for TheoremAppAffordanceService {
    fn default() -> Self {
        Self::new()
    }
}

#[tonic::async_trait]
impl pb::AppAffordanceService for TheoremAppAffordanceService {
    async fn invoke_affordance(
        &self,
        request: Request<pb::InvokeAffordanceRequest>,
    ) -> Result<Response<pb::InvokeAffordanceResponse>, Status> {
        let started = Instant::now();
        let req = request.into_inner();
        let response = self
            .runtime
            .invoke(req, started)
            .map_err(Status::internal)?;
        Ok(Response::new(response))
    }
}

#[derive(Clone)]
struct AppAffordanceRuntime {
    store: Arc<Mutex<InMemoryGraphStore>>,
}

impl AppAffordanceRuntime {
    fn new() -> Self {
        let mut store = InMemoryGraphStore::new();
        register_theseus_app_affordances(&mut store, "theorem", Some("theorem-grpc"))
            .expect("built-in theorem_grpc affordance registry must validate");
        Self {
            store: Arc::new(Mutex::new(store)),
        }
    }

    fn invoke(
        &self,
        req: pb::InvokeAffordanceRequest,
        started: Instant,
    ) -> Result<pb::InvokeAffordanceResponse, String> {
        let mut store = self
            .store
            .lock()
            .map_err(|_| "app affordance graph store lock poisoned".to_string())?;
        Ok(invoke_registered_affordance(&mut *store, req, started))
    }
}

fn invoke_registered_affordance<S: AffordanceGraphStore>(
    store: &mut S,
    req: pb::InvokeAffordanceRequest,
    started: Instant,
) -> pb::InvokeAffordanceResponse {
    let tenant_id = normalize_tenant(&req.tenant_id);
    let requested_id = req.affordance_id.trim().to_string();
    let timeout_ms = normalized_timeout(req.timeout_ms);
    let request_json = parse_request_json(&req.request_json);
    let actor = req.actor.trim().to_string();

    if let Err(err) =
        register_theseus_app_affordances(store, &tenant_id, nonempty_actor(actor.as_str()))
    {
        return response_with_receipt(ResponseParts {
            tenant_id,
            affordance_id: requested_id,
            server_id: "theorem_grpc".to_string(),
            tool_name: String::new(),
            status: "failed".to_string(),
            executed: false,
            output: json!({}),
            error_code: "AFFORDANCE_REGISTRY_FAILED".to_string(),
            message: format!("theorem_grpc affordance registry write failed: {err:?}"),
            actor,
            request: request_json.unwrap_or_else(|_| json!({})),
            dry_run: req.dry_run,
            confirmed: req.confirmed,
            timeout_ms,
            elapsed_ms: elapsed_ms(started),
            writeback_policy: "read-only".to_string(),
        });
    }

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
            actor,
            request: request_json.unwrap_or_else(|_| json!({})),
            dry_run: req.dry_run,
            confirmed: req.confirmed,
            timeout_ms,
            elapsed_ms: elapsed_ms(started),
            writeback_policy: "read-only".to_string(),
        });
    };

    let request_value = match request_json {
        Ok(value) => value,
        Err(message) => {
            let feedback = record_feedback(
                store,
                &tenant_id,
                actor.as_str(),
                &affordance,
                &json!({}),
                0.0,
                "invalid_request_json",
            );
            return response_with_receipt(ResponseParts {
                tenant_id,
                affordance_id: affordance.affordance_id.clone(),
                server_id: affordance.server_id.clone(),
                tool_name: affordance.tool_name.clone(),
                status: "failed".to_string(),
                executed: false,
                output: output_with_feedback(json!({}), feedback),
                error_code: "INVALID_REQUEST_JSON".to_string(),
                message,
                actor,
                request: json!({}),
                dry_run: req.dry_run,
                confirmed: req.confirmed,
                timeout_ms,
                elapsed_ms: elapsed_ms(started),
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
            actor,
            request: request_value,
            dry_run: true,
            confirmed: req.confirmed,
            timeout_ms,
            elapsed_ms: elapsed_ms(started),
            writeback_policy: affordance.writeback_policy.clone(),
        });
    }

    if requires_confirmation(&affordance) && !req.confirmed {
        let feedback = record_feedback(
            store,
            &tenant_id,
            actor.as_str(),
            &affordance,
            &request_value,
            0.0,
            "confirmation_required",
        );
        return response_with_receipt(ResponseParts {
            tenant_id,
            affordance_id: affordance.affordance_id.clone(),
            server_id: affordance.server_id.clone(),
            tool_name: affordance.tool_name.clone(),
            status: "denied".to_string(),
            executed: false,
            output: output_with_feedback(affordance_metadata(&affordance), feedback),
            error_code: "CONFIRMATION_REQUIRED".to_string(),
            message: "affordance requires confirmation before live execution".to_string(),
            actor,
            request: request_value,
            dry_run: false,
            confirmed: false,
            timeout_ms,
            elapsed_ms: elapsed_ms(started),
            writeback_policy: affordance.writeback_policy.clone(),
        });
    }

    let outcome = handle_affordance(&affordance, &request_value, req.confirmed, timeout_ms);
    let feedback = record_feedback(
        store,
        &tenant_id,
        actor.as_str(),
        &affordance,
        &request_value,
        outcome.outcome_value,
        &outcome.outcome_label,
    );

    response_with_receipt(ResponseParts {
        tenant_id,
        affordance_id: affordance.affordance_id.clone(),
        server_id: affordance.server_id.clone(),
        tool_name: affordance.tool_name.clone(),
        status: outcome.status,
        executed: outcome.executed,
        output: output_with_feedback(outcome.output, feedback),
        error_code: outcome.error_code,
        message: outcome.message,
        actor,
        request: request_value,
        dry_run: false,
        confirmed: req.confirmed,
        timeout_ms,
        elapsed_ms: elapsed_ms(started),
        writeback_policy: affordance.writeback_policy.clone(),
    })
}

struct HandlerOutcome {
    status: String,
    executed: bool,
    output: Value,
    error_code: String,
    message: String,
    outcome_value: f32,
    outcome_label: String,
}

fn handle_affordance(
    affordance: &Affordance,
    request: &Value,
    confirmed: bool,
    timeout_ms: u64,
) -> HandlerOutcome {
    let request_hash = stable_hash(request.clone());
    let adapter = AppAffordanceHeadAdapter::from_request(affordance, request);
    let base = json!({
        "handler": affordance.tool_name,
        "request_hash": request_hash,
        "timeout_ms": timeout_ms,
        "provider_head_adapter": adapter.execution_context(),
    });

    let output = match affordance.tool_name.as_str() {
        "anti_misinfo_algo.inspect_claim" => merge_json(
            base,
            json!({
                "claim": request_string(request, &["claim", "text", "query"]).unwrap_or_default(),
                "claim_id": stable_hash(json!({
                    "tool": affordance.tool_name,
                    "claim": request_string(request, &["claim", "text", "query"]).unwrap_or_default(),
                })),
                "inspection": {
                    "status": "needs_evidence",
                    "flags": [],
                    "confidence": 0.5
                }
            }),
        ),
        "corpus_surface.retrieve" => merge_json(
            base,
            json!({
                "query": request_string(request, &["query", "topic"]).unwrap_or_default(),
                "surfaces": [],
                "result_state": "empty_graph_local"
            }),
        ),
        "federation.sync" => merge_json(
            base,
            json!({
                "sync_id": stable_hash(json!({"federation": request})),
                "accepted": confirmed,
                "mutations": [],
                "result_state": "receipt_recorded"
            }),
        ),
        "epistemic_federation.merge" => merge_json(
            base,
            json!({
                "merge_id": stable_hash(json!({"epistemic_federation": request})),
                "accepted": confirmed,
                "merged_records": 0,
                "result_state": "receipt_recorded"
            }),
        ),
        "paper_trail.trace" => merge_json(
            base,
            json!({
                "trace_id": stable_hash(json!({"paper_trail": request})),
                "anchors": request_array_or_empty(request, "anchors"),
                "result_state": "trace_receipted"
            }),
        ),
        "public_verbs.execute" => merge_json(
            base,
            json!({
                "verb": request_string(request, &["verb", "action"]).unwrap_or_default(),
                "external_side_effect": "not_performed_by_local_handler",
                "result_state": "confirmed_receipt_recorded"
            }),
        ),
        "publisher.publish" => merge_json(
            base,
            json!({
                "artifact_id": request_string(request, &["artifact_id", "id"]).unwrap_or_default(),
                "publication_receipt_id": stable_hash(json!({"publish": request})),
                "external_side_effect": "not_performed_by_local_handler",
                "result_state": "confirmed_publication_receipt_recorded"
            }),
        ),
        "research.expand" => merge_json(
            base,
            json!({
                "query": request_string(request, &["query", "topic", "task"]).unwrap_or_default(),
                "frontier_id": stable_hash(json!({"research": request})),
                "frontier_delta": [],
                "result_state": "frontier_receipted"
            }),
        ),
        "user_model.update" => merge_json(
            base,
            json!({
                "patch_id": stable_hash(json!({"user_model": request})),
                "privacy_scope": "binding_private",
                "private_write": "receipt_only",
                "result_state": "private_patch_receipted"
            }),
        ),
        "memory_tensions.detect" => merge_json(
            base,
            json!({
                "tension_scan_id": stable_hash(json!({"memory_tensions": request})),
                "tensions": [],
                "result_state": "scan_receipted"
            }),
        ),
        "observability.read_trace" => merge_json(
            base,
            json!({
                "run_id": request_string(request, &["run_id", "trace_id"]).unwrap_or_default(),
                "events": [],
                "result_state": "empty_trace_local"
            }),
        ),
        _ => {
            return HandlerOutcome {
                status: "failed".to_string(),
                executed: false,
                output: base,
                error_code: "HANDLER_NOT_IMPLEMENTED".to_string(),
                message:
                    "gRPC affordance transport is wired; concrete app handler is not implemented yet"
                        .to_string(),
                outcome_value: 0.0,
                outcome_label: "handler_not_implemented".to_string(),
            };
        }
    };

    HandlerOutcome {
        status: "ok".to_string(),
        executed: true,
        output,
        error_code: String::new(),
        message: "theorem_grpc local app handler completed and recorded an invocation receipt"
            .to_string(),
        outcome_value: 1.0,
        outcome_label: "handler_ok".to_string(),
    }
}

#[derive(Clone, Debug)]
struct AppAffordanceHeadAdapter {
    head_id: String,
    provider: String,
    model: String,
    transport: String,
}

impl AppAffordanceHeadAdapter {
    fn from_request(affordance: &Affordance, request: &Value) -> Self {
        let head = request.get("head").and_then(Value::as_object);
        Self {
            head_id: head
                .and_then(|value| value.get("head_id"))
                .and_then(Value::as_str)
                .unwrap_or("theorem-grpc-local")
                .to_string(),
            provider: head
                .and_then(|value| value.get("provider"))
                .and_then(Value::as_str)
                .unwrap_or("theorem_grpc")
                .to_string(),
            model: head
                .and_then(|value| value.get("model"))
                .and_then(Value::as_str)
                .unwrap_or(affordance.tool_name.as_str())
                .to_string(),
            transport: head
                .and_then(|value| value.get("transport"))
                .and_then(Value::as_str)
                .unwrap_or("local")
                .to_string(),
        }
    }

    fn execution_context(&self) -> Value {
        json!({
            "adapter": "AppAffordanceHeadAdapter",
            "head_id": self.head_id,
            "provider": self.provider,
            "model": self.model,
            "transport": self.transport,
        })
    }
}

struct FeedbackRecord {
    recorded: InvocationRecordResult,
    candidates: Vec<String>,
    task_type: String,
    outcome_label: String,
    recommendations: Vec<Value>,
}

fn record_feedback<S: AffordanceGraphStore>(
    store: &mut S,
    tenant_id: &str,
    actor: &str,
    affordance: &Affordance,
    request: &Value,
    outcome_value: f32,
    outcome_label: &str,
) -> Result<FeedbackRecord, String> {
    let task_type = task_type_from_request(request, affordance);
    let scope = CapabilityScope {
        agent_id: "theorem-grpc-app-affordance".to_string(),
        allow_servers: vec![affordance.server_id.clone()],
        allow_families: vec![affordance.family.clone()],
        ..Default::default()
    };
    let selection = SelectionRequest {
        tenant_id: tenant_id.to_string(),
        task_type: task_type.clone(),
        k: 8,
        scope,
        min_fitness: Some(0.0),
        ppr_damping: 0.0,
        ppr_max_iter: 0,
    };
    let mut candidates = select_affordances(store, &selection)
        .map_err(|err| format!("capability selection failed: {err:?}"))?
        .into_iter()
        .map(|item| item.affordance.affordance_id)
        .collect::<Vec<_>>();
    if !candidates
        .iter()
        .any(|candidate| candidate == &affordance.affordance_id)
    {
        candidates.push(affordance.affordance_id.clone());
    }

    let recorded = record_invocation(
        store,
        InvocationRecordRequest {
            tenant_id: tenant_id.to_string(),
            task_type: task_type.clone(),
            candidate_affordance_ids: candidates.clone(),
            selected_affordance_id: affordance.affordance_id.clone(),
            outcome_value,
            outcome_weight: 1.0,
            outcome_label: outcome_label.to_string(),
            previous_affordance_id: previous_affordance_from_request(request),
            query_text: query_text_from_request(request),
            recorded_at_ms: None,
        },
        nonempty_actor(actor),
    )
    .map_err(|err| format!("record_invocation failed: {err:?}"))?;

    let recommendations = select_affordances(store, &selection)
        .map_err(|err| format!("post-record capability selection failed: {err:?}"))?
        .into_iter()
        .map(|item| {
            json!({
                "affordance_id": item.affordance.affordance_id,
                "server_id": item.affordance.server_id,
                "family": item.affordance.family,
                "score": item.score,
                "fitness": item.affordance.fitness,
            })
        })
        .collect::<Vec<_>>();

    Ok(FeedbackRecord {
        recorded,
        candidates,
        task_type,
        outcome_label: outcome_label.to_string(),
        recommendations,
    })
}

fn output_with_feedback(mut output: Value, feedback: Result<FeedbackRecord, String>) -> Value {
    if !output.is_object() {
        output = json!({ "value": output });
    }
    match feedback {
        Ok(feedback) => {
            output["graph_invocation"] = json!({
                "receipt_hash": feedback.recorded.receipt_hash,
                "receipt_node_id": feedback.recorded.receipt_node_id,
                "graph_version": feedback.recorded.graph_version,
                "effective_fitness": feedback.recorded.effective_fitness,
                "task_type": feedback.task_type,
                "outcome_label": feedback.outcome_label,
                "candidate_affordance_ids": feedback.candidates,
            });
            output["capability_selection"] = json!({
                "scope": "theorem_grpc.family",
                "recommendations": feedback.recommendations,
            });
        }
        Err(message) => {
            output["graph_invocation"] = json!({
                "recorded": false,
                "error": message,
            });
        }
    }
    output
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

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

fn nonempty_actor(actor: &str) -> Option<&str> {
    let actor = actor.trim();
    if actor.is_empty() {
        None
    } else {
        Some(actor)
    }
}

fn task_type_from_request(request: &Value, affordance: &Affordance) -> String {
    request
        .get("task_type")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(affordance.family.as_str())
        .to_string()
}

fn query_text_from_request(request: &Value) -> String {
    request_string(
        request,
        &[
            "query",
            "topic",
            "task",
            "claim",
            "text",
            "artifact_id",
            "run_id",
        ],
    )
    .unwrap_or_else(|| stable_hash(request.clone()))
}

fn previous_affordance_from_request(request: &Value) -> Option<String> {
    request_string(request, &["previous_affordance_id"]).filter(|value| !value.trim().is_empty())
}

fn request_string(request: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        request
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn request_array_or_empty(request: &Value, key: &str) -> Value {
    request
        .get(key)
        .and_then(Value::as_array)
        .map(|items| json!(items))
        .unwrap_or_else(|| json!([]))
}

fn merge_json(mut left: Value, right: Value) -> Value {
    let Some(left_object) = left.as_object_mut() else {
        return right;
    };
    if let Some(right_object) = right.as_object() {
        for (key, value) in right_object {
            left_object.insert(key.clone(), value.clone());
        }
    }
    left
}

#[cfg(test)]
mod tests {
    use rustyred_thg_affordances::INVOCATION_RECEIPT_LABEL;
    use rustyred_thg_core::NodeQuery;

    use super::*;

    fn invoke(
        req: pb::InvokeAffordanceRequest,
    ) -> (AppAffordanceRuntime, pb::InvokeAffordanceResponse) {
        let runtime = AppAffordanceRuntime::new();
        let response = runtime.invoke(req, Instant::now()).unwrap();
        (runtime, response)
    }

    #[test]
    fn dry_run_returns_registered_affordance_receipt() {
        let (_, response) = invoke(pb::InvokeAffordanceRequest {
            tenant_id: "theorem".to_string(),
            affordance_id: "theorem_grpc.publisher.publish".to_string(),
            actor: "test".to_string(),
            request_json: r#"{"artifact_id":"a1"}"#.to_string(),
            dry_run: true,
            confirmed: false,
            timeout_ms: 0,
        });

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
    fn external_write_requires_confirmation_and_records_failure() {
        let (runtime, response) = invoke(pb::InvokeAffordanceRequest {
            tenant_id: "theorem".to_string(),
            affordance_id: "theorem_grpc.publisher.publish".to_string(),
            actor: "test".to_string(),
            request_json: "{}".to_string(),
            dry_run: false,
            confirmed: false,
            timeout_ms: 0,
        });

        assert_eq!(response.status, "denied");
        assert_eq!(response.error_code, "CONFIRMATION_REQUIRED");
        assert!(!response.executed);
        assert!(response.output_json.contains("\"graph_invocation\""));

        let store = runtime.store.lock().unwrap();
        let receipts = store.query_nodes(NodeQuery::label(INVOCATION_RECEIPT_LABEL));
        assert_eq!(receipts.len(), 1);
        assert_eq!(
            receipts[0].properties["outcome_label"],
            "confirmation_required"
        );
    }

    #[test]
    fn confirmed_known_affordance_runs_concrete_handler_and_records_feedback() {
        let (_, response) = invoke(pb::InvokeAffordanceRequest {
            tenant_id: "theorem".to_string(),
            affordance_id: "theorem_grpc.research.expand".to_string(),
            actor: "test".to_string(),
            request_json: r#"{"query":"substrate browsers","head":{"provider":"fake-provider","model":"fake-model"}}"#
                .to_string(),
            dry_run: false,
            confirmed: true,
            timeout_ms: 42_000,
        });

        assert_eq!(response.status, "ok");
        assert!(response.executed);
        assert_eq!(response.error_code, "");
        assert!(response.output_json.contains("\"frontier_receipted\""));
        assert!(response.output_json.contains("\"timeout_ms\":30000"));
        assert!(response
            .output_json
            .contains("\"AppAffordanceHeadAdapter\""));
        assert!(response.output_json.contains("\"fake-provider\""));
        assert!(response.output_json.contains("\"capability_selection\""));
    }

    #[test]
    fn invalid_json_is_receipted_as_failure_and_graph_outcome() {
        let (runtime, response) = invoke(pb::InvokeAffordanceRequest {
            tenant_id: "theorem".to_string(),
            affordance_id: "theorem_grpc.research.expand".to_string(),
            actor: "test".to_string(),
            request_json: "{broken".to_string(),
            dry_run: false,
            confirmed: true,
            timeout_ms: 0,
        });

        assert_eq!(response.status, "failed");
        assert_eq!(response.error_code, "INVALID_REQUEST_JSON");
        assert!(!response.receipt_hash.is_empty());
        assert!(response.output_json.contains("\"invalid_request_json\""));

        let store = runtime.store.lock().unwrap();
        let receipts = store.query_nodes(NodeQuery::label(INVOCATION_RECEIPT_LABEL));
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].properties["outcome_value"], 0.0);
    }
}
