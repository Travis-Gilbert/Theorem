//! theorem_grpc.AppAffordanceService implementation.
//!
//! This is the live gRPC boundary for the Theseus app affordance metadata that
//! `rustyred-thg-affordances` registers as `theorem_grpc.*` tools. The service
//! owns a graph-backed runtime: it dispatches concrete local handlers, records
//! invocation outcomes into the affordance graph, and returns the same
//! content-addressed receipt envelope the harness already understands.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustyred_thg_affordances::{
    record_invocation, register_theseus_app_affordances, select_affordances,
    theseus_app_affordances, Affordance, AffordanceGraphStore, CapabilityScope,
    InvocationRecordRequest, InvocationRecordResult, SelectionRequest, THEOREM_GRPC_TIMEOUT_MS,
};
use rustyred_thg_core::{stable_hash, RedCoreDurability, RedCoreGraphStore, RedCoreOptions};
use serde_json::{json, Map, Value};
use theorem_harness_core::{AffordanceReceipt, ProviderHeadExecutionContext};
use tonic::{Request, Response, Status};

use crate::code_index::{
    is_fetchable_repo_url, CodeContextInput, CodeIndexRuntime, ExplainCodeInput, ExploreCodeInput,
    IngestCodebaseInput, RecognizeCodeInput, RecordUseReceiptInput, RepoFetchCaps, SearchCodeInput,
};
use crate::pb;

#[derive(Clone)]
pub struct TheoremAppAffordanceService {
    runtime: AppAffordanceRuntime,
}

impl TheoremAppAffordanceService {
    pub fn try_new() -> Result<Self, String> {
        Self::try_new_with_code_index(CodeIndexRuntime::try_new().map_err(|err| err.to_string())?)
    }

    pub fn try_new_with_code_index(code_index: CodeIndexRuntime) -> Result<Self, String> {
        Ok(Self {
            runtime: AppAffordanceRuntime::try_new(code_index)?,
        })
    }

    pub fn new() -> Self {
        Self::try_new().expect("theorem_grpc RedCore app affordance runtime must open")
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
    store: Arc<Mutex<RedCoreGraphStore>>,
    adapter: TheseusAppAdapter,
    code_index: CodeIndexRuntime,
}

impl AppAffordanceRuntime {
    fn try_new(code_index: CodeIndexRuntime) -> Result<Self, String> {
        Self::try_new_at(
            redcore_data_dir(),
            redcore_options(),
            TheseusAppAdapter::from_env(),
            code_index,
        )
    }

    fn try_new_at(
        data_dir: impl AsRef<Path>,
        options: RedCoreOptions,
        adapter: TheseusAppAdapter,
        code_index: CodeIndexRuntime,
    ) -> Result<Self, String> {
        let mut store = RedCoreGraphStore::open(data_dir.as_ref(), options)
            .map_err(|err| format!("open theorem_grpc RedCore store failed: {err:?}"))?;
        register_theseus_app_affordances(&mut store, "theorem", Some("theorem-grpc"))
            .map_err(|err| format!("register theorem_grpc affordances failed: {err:?}"))?;
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            adapter,
            code_index,
        })
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
        Ok(invoke_registered_affordance(
            &mut *store,
            &self.adapter,
            &self.code_index,
            req,
            started,
        ))
    }
}

fn invoke_registered_affordance<S: AffordanceGraphStore>(
    store: &mut S,
    adapter: &TheseusAppAdapter,
    code_index: &CodeIndexRuntime,
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

    let outcome = handle_affordance(
        &affordance,
        &request_value,
        req.confirmed,
        timeout_ms,
        adapter,
        code_index,
        &tenant_id,
        actor.as_str(),
    );
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
    adapter: &TheseusAppAdapter,
    code_index: &CodeIndexRuntime,
    tenant_id: &str,
    actor: &str,
) -> HandlerOutcome {
    let request_hash = stable_hash(request.clone());
    let provider_context = ProviderHeadExecutionContext::from_request_head(
        "theorem_grpc.AppAffordanceService",
        affordance.tool_name.clone(),
        request.get("head"),
    );
    if uses_live_theseus_adapter(affordance) {
        return adapter.invoke(TheseusAppCall {
            tenant_id: tenant_id.to_string(),
            actor: actor.to_string(),
            affordance: affordance.clone(),
            request: request.clone(),
            confirmed,
            timeout_ms,
            provider_context,
        });
    }

    let base = json!({
        "handler": affordance.tool_name,
        "request_hash": request_hash,
        "timeout_ms": timeout_ms,
        "provider_head_adapter": provider_context.to_payload(),
    });

    let output = match affordance.tool_name.as_str() {
        "code_search.ingest" => {
            return code_ingest_handler(base, code_index, request, tenant_id, actor, false);
        }
        "code_search.reindex" => {
            return code_ingest_handler(base, code_index, request, tenant_id, actor, true);
        }
        "code_search.search" => {
            return code_search_handler(base, code_index, request, tenant_id);
        }
        "code_search.context" => {
            return code_context_handler(base, code_index, request, tenant_id);
        }
        "code_search.recognize" => {
            return code_recognize_handler(base, code_index, request, tenant_id);
        }
        "code_search.explore" => {
            return code_explore_handler(base, code_index, request, tenant_id);
        }
        "code_search.explain" => {
            return code_explain_handler(base, code_index, request, tenant_id);
        }
        "code_search.record_use_receipt" => {
            return code_record_use_handler(base, code_index, request, tenant_id, actor);
        }
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

fn code_ingest_handler(
    base: Value,
    code_index: &CodeIndexRuntime,
    request: &Value,
    tenant_id: &str,
    actor: &str,
    reindex: bool,
) -> HandlerOutcome {
    let raw_repo_path =
        request_string(request, &["repo_path", "repoPath", "path"]).unwrap_or_default();
    let explicit_repo_url =
        request_string(request, &["repo_url", "repoUrl", "url"]).unwrap_or_default();
    let repo_url = if !explicit_repo_url.trim().is_empty() {
        explicit_repo_url
    } else if is_fetchable_repo_url(&raw_repo_path) {
        raw_repo_path.clone()
    } else {
        String::new()
    };
    let input = IngestCodebaseInput {
        tenant_id: tenant_id.to_string(),
        repo_path: if repo_url.is_empty() {
            raw_repo_path
        } else {
            String::new()
        },
        repo_id: request_string(request, &["repo_id", "repoId"]).unwrap_or_default(),
        include_extensions: request_string_array(request, "include_extensions"),
        exclude_dirs: request_string_array(request, "exclude_dirs"),
        max_files: request_u64(request, "max_files").unwrap_or_default(),
        max_file_bytes: request_u64(request, "max_file_bytes").unwrap_or_default(),
        actor: actor.to_string(),
    };
    let result = if !repo_url.is_empty() && reindex {
        code_index.reindex_codebase_from_url(&repo_url, input, &RepoFetchCaps::default())
    } else if !repo_url.is_empty() {
        code_index.ingest_codebase_from_url(&repo_url, input, &RepoFetchCaps::default())
    } else if reindex {
        code_index.reindex_codebase(input)
    } else {
        code_index.ingest_codebase(input)
    };
    match result {
        Ok(output) => HandlerOutcome {
            status: "ok".to_string(),
            executed: true,
            output: merge_json(base, output.to_json()),
            error_code: String::new(),
            message: "native code index wrote codebase data into RedCore".to_string(),
            outcome_value: 1.0,
            outcome_label: if reindex {
                "code_reindex_ok".to_string()
            } else {
                "code_ingest_ok".to_string()
            },
        },
        Err(err) => HandlerOutcome {
            status: "failed".to_string(),
            executed: false,
            output: merge_json(base, json!({ "code_index_error": err.message })),
            error_code: err.code,
            message: "native code index failed to ingest codebase".to_string(),
            outcome_value: 0.0,
            outcome_label: "code_ingest_failed".to_string(),
        },
    }
}

fn code_search_handler(
    base: Value,
    code_index: &CodeIndexRuntime,
    request: &Value,
    tenant_id: &str,
) -> HandlerOutcome {
    let result = code_index.search_code(SearchCodeInput {
        tenant_id: tenant_id.to_string(),
        query: request_string(request, &["query", "text", "symbol"]).unwrap_or_default(),
        repo_id: request_string(request, &["repo_id"]).unwrap_or_default(),
        path_prefix: request_string(request, &["path_prefix", "file_prefix"]).unwrap_or_default(),
        kinds: request_string_array(request, "kinds"),
        limit: request_u64(request, "limit").unwrap_or_default(),
    });
    match result {
        Ok(output) => HandlerOutcome {
            status: "ok".to_string(),
            executed: true,
            output: merge_json(base, output.to_json()),
            error_code: String::new(),
            message: "native code search completed over RedCore".to_string(),
            outcome_value: 1.0,
            outcome_label: "code_search_ok".to_string(),
        },
        Err(err) => HandlerOutcome {
            status: "failed".to_string(),
            executed: false,
            output: merge_json(base, json!({ "code_index_error": err.message })),
            error_code: err.code,
            message: "native code search failed".to_string(),
            outcome_value: 0.0,
            outcome_label: "code_search_failed".to_string(),
        },
    }
}

fn code_context_handler(
    base: Value,
    code_index: &CodeIndexRuntime,
    request: &Value,
    tenant_id: &str,
) -> HandlerOutcome {
    let result = code_index.code_context(CodeContextInput {
        tenant_id: tenant_id.to_string(),
        node_id: request_string(request, &["node_id", "symbol_id", "file_id"]).unwrap_or_default(),
        repo_id: request_string(request, &["repo_id"]).unwrap_or_default(),
        file_path: request_string(request, &["file_path", "path"]).unwrap_or_default(),
        before_lines: request_u64(request, "before_lines").unwrap_or_default(),
        after_lines: request_u64(request, "after_lines").unwrap_or_default(),
        max_chars: request_u64(request, "max_chars").unwrap_or_default(),
    });
    match result {
        Ok(output) => HandlerOutcome {
            status: "ok".to_string(),
            executed: true,
            output: merge_json(base, output.to_json()),
            error_code: String::new(),
            message: "native code context completed over RedCore".to_string(),
            outcome_value: 1.0,
            outcome_label: "code_context_ok".to_string(),
        },
        Err(err) => HandlerOutcome {
            status: "failed".to_string(),
            executed: false,
            output: merge_json(base, json!({ "code_index_error": err.message })),
            error_code: err.code,
            message: "native code context failed".to_string(),
            outcome_value: 0.0,
            outcome_label: "code_context_failed".to_string(),
        },
    }
}

fn code_recognize_handler(
    base: Value,
    code_index: &CodeIndexRuntime,
    request: &Value,
    tenant_id: &str,
) -> HandlerOutcome {
    let result = code_index.recognize_code(RecognizeCodeInput {
        tenant_id: tenant_id.to_string(),
        repo_id: request_string(request, &["repo_id"]).unwrap_or_default(),
        file_path: request_string(request, &["file_path", "path"]).unwrap_or_default(),
        text: request_string(request, &["text", "source"]).unwrap_or_default(),
        limit: request_u64(request, "limit").unwrap_or_default(),
    });
    match result {
        Ok(output) => HandlerOutcome {
            status: "ok".to_string(),
            executed: true,
            output: merge_json(base, output.to_json()),
            error_code: String::new(),
            message: "native code recognition completed over RedCore".to_string(),
            outcome_value: 1.0,
            outcome_label: "code_recognize_ok".to_string(),
        },
        Err(err) => HandlerOutcome {
            status: "failed".to_string(),
            executed: false,
            output: merge_json(base, json!({ "code_index_error": err.message })),
            error_code: err.code,
            message: "native code recognition failed".to_string(),
            outcome_value: 0.0,
            outcome_label: "code_recognize_failed".to_string(),
        },
    }
}

fn code_explore_handler(
    base: Value,
    code_index: &CodeIndexRuntime,
    request: &Value,
    tenant_id: &str,
) -> HandlerOutcome {
    let result = code_index.explore_code(ExploreCodeInput {
        tenant_id: tenant_id.to_string(),
        node_id: request_string(request, &["node_id", "symbol_id"]).unwrap_or_default(),
        query: request_string(request, &["query", "text", "symbol"]).unwrap_or_default(),
        repo_id: request_string(request, &["repo_id"]).unwrap_or_default(),
        max_depth: request_u64(request, "max_depth").unwrap_or_default(),
        limit: request_u64(request, "limit").unwrap_or_default(),
    });
    match result {
        Ok(output) => HandlerOutcome {
            status: "ok".to_string(),
            executed: true,
            output: merge_json(base, output.to_json()),
            error_code: String::new(),
            message: "native code exploration completed over RedCore".to_string(),
            outcome_value: 1.0,
            outcome_label: "code_explore_ok".to_string(),
        },
        Err(err) => HandlerOutcome {
            status: "failed".to_string(),
            executed: false,
            output: merge_json(base, json!({ "code_index_error": err.message })),
            error_code: err.code,
            message: "native code exploration failed".to_string(),
            outcome_value: 0.0,
            outcome_label: "code_explore_failed".to_string(),
        },
    }
}

fn code_explain_handler(
    base: Value,
    code_index: &CodeIndexRuntime,
    request: &Value,
    tenant_id: &str,
) -> HandlerOutcome {
    let result = code_index.explain_code(ExplainCodeInput {
        tenant_id: tenant_id.to_string(),
        node_id: request_string(request, &["node_id", "symbol_id"]).unwrap_or_default(),
        query: request_string(request, &["query", "text", "symbol"]).unwrap_or_default(),
        repo_id: request_string(request, &["repo_id"]).unwrap_or_default(),
        max_chars: request_u64(request, "max_chars").unwrap_or_default(),
    });
    match result {
        Ok(output) => HandlerOutcome {
            status: "ok".to_string(),
            executed: true,
            output: merge_json(base, output.to_json()),
            error_code: String::new(),
            message: "native code explanation completed over RedCore".to_string(),
            outcome_value: 1.0,
            outcome_label: "code_explain_ok".to_string(),
        },
        Err(err) => HandlerOutcome {
            status: "failed".to_string(),
            executed: false,
            output: merge_json(base, json!({ "code_index_error": err.message })),
            error_code: err.code,
            message: "native code explanation failed".to_string(),
            outcome_value: 0.0,
            outcome_label: "code_explain_failed".to_string(),
        },
    }
}

fn code_record_use_handler(
    base: Value,
    code_index: &CodeIndexRuntime,
    request: &Value,
    tenant_id: &str,
    actor: &str,
) -> HandlerOutcome {
    let result = code_index.record_use_receipt(RecordUseReceiptInput {
        tenant_id: tenant_id.to_string(),
        node_id: request_string(request, &["node_id", "symbol_id"]).unwrap_or_default(),
        repo_id: request_string(request, &["repo_id"]).unwrap_or_default(),
        query: request_string(request, &["query", "text"]).unwrap_or_default(),
        action: request_string(request, &["action", "use_action"]).unwrap_or_default(),
        outcome: request_string(request, &["outcome", "result"]).unwrap_or_default(),
        actor: request_string(request, &["actor", "actor_id"]).unwrap_or_else(|| actor.to_string()),
        use_json: request
            .get("use")
            .or_else(|| request.get("metadata"))
            .map(Value::to_string)
            .unwrap_or_default(),
    });
    match result {
        Ok(output) => HandlerOutcome {
            status: "ok".to_string(),
            executed: true,
            output: merge_json(base, output.to_json()),
            error_code: String::new(),
            message: "native code use receipt recorded in RedCore".to_string(),
            outcome_value: 1.0,
            outcome_label: "code_use_receipt_ok".to_string(),
        },
        Err(err) => HandlerOutcome {
            status: "failed".to_string(),
            executed: false,
            output: merge_json(base, json!({ "code_index_error": err.message })),
            error_code: err.code,
            message: "native code use receipt failed".to_string(),
            outcome_value: 0.0,
            outcome_label: "code_use_receipt_failed".to_string(),
        },
    }
}

#[derive(Clone)]
struct TheseusAppAdapter {
    endpoint: Option<String>,
    bearer_token: Option<String>,
    client: reqwest::blocking::Client,
}

impl TheseusAppAdapter {
    fn from_env() -> Self {
        Self::new(
            std::env::var("THESEUS_APP_ADAPTER_ENDPOINT")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    std::env::var("THESEUS_APP_BASE_URL")
                        .ok()
                        .filter(|value| !value.trim().is_empty())
                        .map(|base| {
                            format!(
                                "{}/api/v2/theorem/app-affordances/invoke/",
                                base.trim_end_matches('/')
                            )
                        })
                }),
            std::env::var("THESEUS_APP_ADAPTER_TOKEN")
                .ok()
                .filter(|value| !value.trim().is_empty()),
        )
    }

    fn new(endpoint: Option<String>, bearer_token: Option<String>) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(THEOREM_GRPC_TIMEOUT_MS))
            .build()
            .expect("reqwest blocking client should build");
        Self {
            endpoint,
            bearer_token,
            client,
        }
    }

    fn invoke(&self, call: TheseusAppCall) -> HandlerOutcome {
        let Some(endpoint) = self.endpoint.as_deref() else {
            return HandlerOutcome {
                status: "failed".to_string(),
                executed: false,
                output: json!({
                    "handler": call.affordance.tool_name,
                    "request_hash": stable_hash(call.request),
                    "timeout_ms": call.timeout_ms,
                    "provider_head_adapter": call.provider_context.to_payload(),
                    "theseus_app_adapter": {
                        "configured": false,
                        "env": "THESEUS_APP_ADAPTER_ENDPOINT",
                    }
                }),
                error_code: "THESEUS_APP_ADAPTER_UNCONFIGURED".to_string(),
                message: "confirmed side-effecting affordance requires a configured Theseus app adapter endpoint".to_string(),
                outcome_value: 0.0,
                outcome_label: "theseus_adapter_unconfigured".to_string(),
            };
        };

        let payload = json!({
            "tenant_id": call.tenant_id,
            "actor": call.actor,
            "affordance_id": call.affordance.affordance_id,
            "tool_name": call.affordance.tool_name,
            "family": call.affordance.family,
            "writeback_policy": call.affordance.writeback_policy,
            "request": call.request,
            "confirmed": call.confirmed,
            "timeout_ms": call.timeout_ms,
            "provider_head_adapter": call.provider_context.to_payload(),
        });

        let mut request = self
            .client
            .post(endpoint)
            .timeout(Duration::from_millis(call.timeout_ms))
            .json(&payload);
        if let Some(token) = self.bearer_token.as_deref() {
            request = request.bearer_auth(token);
        }
        let response = match request.send() {
            Ok(response) => response,
            Err(err) => {
                return HandlerOutcome {
                    status: "failed".to_string(),
                    executed: false,
                    output: json!({
                        "theseus_app_adapter": {
                            "configured": true,
                            "endpoint": endpoint,
                            "request_hash": stable_hash(payload),
                            "error": err.to_string(),
                        }
                    }),
                    error_code: "THESEUS_APP_ADAPTER_SEND_FAILED".to_string(),
                    message: "Theseus app adapter request failed before a response was received"
                        .to_string(),
                    outcome_value: 0.0,
                    outcome_label: "theseus_adapter_send_failed".to_string(),
                };
            }
        };

        let status_code = response.status().as_u16();
        let response_json = response
            .json::<Value>()
            .unwrap_or_else(|err| json!({ "decode_error": err.to_string() }));
        let status = response_json
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or(if status_code < 400 { "ok" } else { "failed" })
            .to_string();
        let executed = response_json
            .get("executed")
            .and_then(Value::as_bool)
            .unwrap_or(status_code < 400);
        let error_code = response_json
            .get("error_code")
            .and_then(Value::as_str)
            .unwrap_or(if status_code < 400 {
                ""
            } else {
                "THESEUS_APP_ADAPTER_HTTP_FAILED"
            })
            .to_string();
        let message = response_json
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Theseus app adapter responded")
            .to_string();
        let ok = status_code < 400 && status == "ok" && executed;

        HandlerOutcome {
            status,
            executed,
            output: json!({
                "theseus_app_adapter": {
                    "configured": true,
                    "endpoint": endpoint,
                    "http_status": status_code,
                    "response": response_json,
                }
            }),
            error_code,
            message,
            outcome_value: if ok { 1.0 } else { 0.0 },
            outcome_label: if ok {
                "theseus_adapter_ok".to_string()
            } else {
                "theseus_adapter_failed".to_string()
            },
        }
    }
}

struct TheseusAppCall {
    tenant_id: String,
    actor: String,
    affordance: Affordance,
    request: Value,
    confirmed: bool,
    timeout_ms: u64,
    provider_context: ProviderHeadExecutionContext,
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

fn redcore_data_dir() -> PathBuf {
    std::env::var("THEOREM_GRPC_REDCORE_DIR")
        .or_else(|_| std::env::var("THEOREM_GRPC_DATA_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/theorem-grpc/redcore"))
}

fn redcore_options() -> RedCoreOptions {
    let mut options = RedCoreOptions::default();
    if let Ok(raw) = std::env::var("THEOREM_GRPC_REDCORE_DURABILITY") {
        options.durability = RedCoreDurability::parse(&raw);
    }
    if let Ok(raw) = std::env::var("THEOREM_GRPC_REDCORE_SNAPSHOT_INTERVAL") {
        if let Ok(value) = raw.parse::<u64>() {
            options.snapshot_interval_writes = value;
        }
    }
    if let Ok(raw) = std::env::var("THEOREM_GRPC_REDCORE_STRICT_ACID") {
        options.strict_acid = matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        );
    }
    options
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

fn uses_live_theseus_adapter(affordance: &Affordance) -> bool {
    if affordance.tool_name.starts_with("code_search.") {
        return false;
    }
    requires_confirmation(affordance)
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

fn request_string_array(request: &Value, key: &str) -> Vec<String> {
    request
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn request_u64(request: &Value, key: &str) -> Option<u64> {
    request.get(key).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_i64().and_then(|number| number.try_into().ok()))
            .or_else(|| value.as_str().and_then(|raw| raw.parse::<u64>().ok()))
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rustyred_thg_affordances::INVOCATION_RECEIPT_LABEL;
    use rustyred_thg_core::NodeQuery;

    use super::*;

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_options() -> RedCoreOptions {
        RedCoreOptions {
            durability: RedCoreDurability::AofAlways,
            snapshot_interval_writes: 100,
            strict_acid: true,
        }
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let counter = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "theorem-grpc-{name}-{}-{nanos}-{counter}",
            std::process::id()
        ))
    }

    fn runtime_with_adapter(adapter: TheseusAppAdapter) -> (AppAffordanceRuntime, PathBuf) {
        let data_dir = unique_test_dir("redcore");
        let code_dir = data_dir.join("code-index");
        let code_index = CodeIndexRuntime::try_new_at(&code_dir, test_options()).unwrap();
        let runtime =
            AppAffordanceRuntime::try_new_at(&data_dir, test_options(), adapter, code_index)
                .unwrap();
        (runtime, data_dir)
    }

    fn invoke(
        req: pb::InvokeAffordanceRequest,
    ) -> (AppAffordanceRuntime, pb::InvokeAffordanceResponse, PathBuf) {
        let (runtime, data_dir) = runtime_with_adapter(TheseusAppAdapter::new(None, None));
        let response = runtime.invoke(req, Instant::now()).unwrap();
        (runtime, response, data_dir)
    }

    fn fixture_code_repo() -> PathBuf {
        let repo_dir = unique_test_dir("code-repo");
        std::fs::create_dir_all(repo_dir.join("src")).unwrap();
        std::fs::write(
            repo_dir.join("src/lib.rs"),
            "pub fn native_code_helper(query: &str) -> usize {\n    query.len()\n}\n\npub fn native_code_search(query: &str) -> usize {\n    native_code_helper(query)\n}\n",
        )
        .unwrap();
        repo_dir
    }

    fn fixture_go_repo() -> PathBuf {
        let repo_dir = unique_test_dir("go-code-repo");
        std::fs::create_dir_all(repo_dir.join("internal")).unwrap();
        std::fs::write(
            repo_dir.join("go.mod"),
            "module example.com/boltbrowser\n\ngo 1.22\n",
        )
        .unwrap();
        std::fs::write(repo_dir.join("README.md"), "# boltbrowser fixture\n").unwrap();
        std::fs::write(
            repo_dir.join("main.go"),
            "package main\n\nconst AppName = \"boltbrowser\"\n\ntype Browser struct {\n    title string\n}\n\nfunc main() {\n    browser := Browser{title: AppName}\n    _ = browser.Draw()\n}\n\nfunc (browser Browser) Draw() string {\n    return browser.title\n}\n",
        )
        .unwrap();
        std::fs::write(
            repo_dir.join("internal/store.go"),
            "package internal\n\ntype BoltStore struct {}\n\nfunc OpenStore(path string) BoltStore {\n    return BoltStore{}\n}\n",
        )
        .unwrap();
        repo_dir
    }

    fn init_git_fixture(dir: &Path) -> bool {
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false)
        };
        run(&["init", "--quiet"])
            && run(&["config", "user.email", "fixture@example.com"])
            && run(&["config", "user.name", "Fixture"])
            && run(&["add", "."])
            && run(&["commit", "--quiet", "-m", "fixture"])
    }

    #[test]
    fn dry_run_returns_registered_affordance_receipt() {
        let (runtime, response, data_dir) = invoke(pb::InvokeAffordanceRequest {
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
        drop(runtime);
        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn external_write_requires_confirmation_and_records_failure() {
        let (runtime, response, data_dir) = invoke(pb::InvokeAffordanceRequest {
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
        let receipts = store
            .query_nodes(NodeQuery::label(INVOCATION_RECEIPT_LABEL))
            .unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(
            receipts[0].properties["outcome_label"],
            "confirmation_required"
        );
        drop(store);
        drop(runtime);
        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn read_only_affordance_runs_local_handler_and_records_feedback() {
        let (runtime, response, data_dir) = invoke(pb::InvokeAffordanceRequest {
            tenant_id: "theorem".to_string(),
            affordance_id: "theorem_grpc.observability.read_trace".to_string(),
            actor: "test".to_string(),
            request_json:
                r#"{"run_id":"run:1","head":{"provider":"fake-provider","model":"fake-model"}}"#
                    .to_string(),
            dry_run: false,
            confirmed: true,
            timeout_ms: 42_000,
        });

        assert_eq!(response.status, "ok");
        assert!(response.executed);
        assert_eq!(response.error_code, "");
        assert!(response.output_json.contains("\"empty_trace_local\""));
        assert!(response.output_json.contains("\"timeout_ms\":30000"));
        assert!(response
            .output_json
            .contains("\"theorem_grpc.AppAffordanceService\""));
        assert!(response.output_json.contains("\"fake-provider\""));
        assert!(response.output_json.contains("\"capability_selection\""));
        drop(runtime);
        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn code_search_affordance_ingests_and_searches_redcore_index() {
        let repo_dir = fixture_code_repo();
        let (runtime, data_dir) = runtime_with_adapter(TheseusAppAdapter::new(None, None));
        let ingest = runtime
            .invoke(
                pb::InvokeAffordanceRequest {
                    tenant_id: "theorem".to_string(),
                    affordance_id: "theorem_grpc.code_search.ingest".to_string(),
                    actor: "test".to_string(),
                    request_json: json!({
                        "repo_path": repo_dir.display().to_string(),
                        "include_extensions": ["rs"],
                    })
                    .to_string(),
                    dry_run: false,
                    confirmed: true,
                    timeout_ms: 0,
                },
                Instant::now(),
            )
            .unwrap();
        assert_eq!(ingest.status, "ok");
        assert!(ingest.output_json.contains("\"files_indexed\":1"));
        assert!(ingest.output_json.contains("\"graph_invocation\""));

        let search = runtime
            .invoke(
                pb::InvokeAffordanceRequest {
                    tenant_id: "theorem".to_string(),
                    affordance_id: "theorem_grpc.code_search.search".to_string(),
                    actor: "test".to_string(),
                    request_json: r#"{"query":"native_code_search"}"#.to_string(),
                    dry_run: false,
                    confirmed: false,
                    timeout_ms: 0,
                },
                Instant::now(),
            )
            .unwrap();
        assert_eq!(search.status, "ok");
        assert!(search.output_json.contains("\"native_code_search\""));
        assert!(search.output_json.contains("\"trust_tier\":\"advisory\""));
        assert!(search.output_json.contains("\"capability_selection\""));
        let search_output: Value = serde_json::from_str(&search.output_json).unwrap();
        let node_id = search_output["hits"][0]["node_id"]
            .as_str()
            .unwrap()
            .to_string();

        let recognize = runtime
            .invoke(
                pb::InvokeAffordanceRequest {
                    tenant_id: "theorem".to_string(),
                    affordance_id: "theorem_grpc.code_search.recognize".to_string(),
                    actor: "test".to_string(),
                    request_json:
                        r#"{"file_path":"src/inline.rs","text":"pub fn inline_affordance() {}"}"#
                            .to_string(),
                    dry_run: false,
                    confirmed: false,
                    timeout_ms: 0,
                },
                Instant::now(),
            )
            .unwrap();
        assert_eq!(recognize.status, "ok");
        assert!(recognize.output_json.contains("\"inline_affordance\""));

        let explore = runtime
            .invoke(
                pb::InvokeAffordanceRequest {
                    tenant_id: "theorem".to_string(),
                    affordance_id: "theorem_grpc.code_search.explore".to_string(),
                    actor: "test".to_string(),
                    request_json: json!({ "node_id": node_id, "max_depth": 1 }).to_string(),
                    dry_run: false,
                    confirmed: false,
                    timeout_ms: 0,
                },
                Instant::now(),
            )
            .unwrap();
        assert_eq!(explore.status, "ok");
        assert!(explore.output_json.contains("\"native_code_helper\""));
        assert!(explore.output_json.contains("\"CALLS_SYMBOL\""));

        let explain = runtime
            .invoke(
                pb::InvokeAffordanceRequest {
                    tenant_id: "theorem".to_string(),
                    affordance_id: "theorem_grpc.code_search.explain".to_string(),
                    actor: "test".to_string(),
                    request_json: r#"{"query":"native_code_search"}"#.to_string(),
                    dry_run: false,
                    confirmed: false,
                    timeout_ms: 0,
                },
                Instant::now(),
            )
            .unwrap();
        assert_eq!(explain.status, "ok");
        let explain_output: Value = serde_json::from_str(&explain.output_json).unwrap();
        let summary = explain_output["summary"].as_str().unwrap_or_default();
        assert!(summary.contains("Trust tier: advisory"), "{summary}");

        let use_receipt = runtime
            .invoke(
                pb::InvokeAffordanceRequest {
                    tenant_id: "theorem".to_string(),
                    affordance_id: "theorem_grpc.code_search.record_use_receipt".to_string(),
                    actor: "test".to_string(),
                    request_json: json!({
                        "node_id": node_id,
                        "query": "native_code_search",
                        "action": "explain",
                        "outcome": "useful",
                        "use": { "selected": true }
                    })
                    .to_string(),
                    dry_run: false,
                    confirmed: false,
                    timeout_ms: 0,
                },
                Instant::now(),
            )
            .unwrap();
        assert_eq!(use_receipt.status, "ok");
        assert!(use_receipt.output_json.contains("\"code_use_receipt_ok\""));

        drop(runtime);
        std::fs::remove_dir_all(repo_dir).ok();
        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn code_search_affordance_treats_repo_path_url_as_clone_target() {
        let repo_dir = fixture_code_repo();
        if !init_git_fixture(&repo_dir) {
            std::fs::remove_dir_all(repo_dir).ok();
            return;
        }
        let repo_url = format!("file://{}", repo_dir.display());
        let (runtime, data_dir) = runtime_with_adapter(TheseusAppAdapter::new(None, None));
        let ingest = runtime
            .invoke(
                pb::InvokeAffordanceRequest {
                    tenant_id: "theorem".to_string(),
                    affordance_id: "theorem_grpc.code_search.ingest".to_string(),
                    actor: "test".to_string(),
                    request_json: json!({
                        "repo_path": repo_url,
                        "include_extensions": ["rs"],
                    })
                    .to_string(),
                    dry_run: false,
                    confirmed: true,
                    timeout_ms: 0,
                },
                Instant::now(),
            )
            .unwrap();
        assert_eq!(ingest.status, "ok");
        let output: Value = serde_json::from_str(&ingest.output_json).unwrap();
        assert_eq!(output["files_indexed"], json!(1));
        assert!(output["repo_id"]
            .as_str()
            .unwrap_or_default()
            .starts_with("repo:theorem-grpc-code-repo-"));
        assert!(output["repo_root"]
            .as_str()
            .unwrap_or_default()
            .contains("rustyred-code-clone-"));

        drop(runtime);
        std::fs::remove_dir_all(repo_dir).ok();
        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn code_search_affordance_indexes_go_url_and_searches_main() {
        let repo_dir = fixture_go_repo();
        if !init_git_fixture(&repo_dir) {
            std::fs::remove_dir_all(repo_dir).ok();
            return;
        }
        let repo_url = format!("file://{}", repo_dir.display());
        let (runtime, data_dir) = runtime_with_adapter(TheseusAppAdapter::new(None, None));
        let ingest = runtime
            .invoke(
                pb::InvokeAffordanceRequest {
                    tenant_id: "theorem".to_string(),
                    affordance_id: "theorem_grpc.code_search.ingest".to_string(),
                    actor: "test".to_string(),
                    request_json: json!({
                        "repo_path": repo_url,
                        "repo_id": "repo:boltbrowser-affordance",
                    })
                    .to_string(),
                    dry_run: false,
                    confirmed: true,
                    timeout_ms: 0,
                },
                Instant::now(),
            )
            .unwrap();
        assert_eq!(ingest.status, "ok");
        let ingest_output: Value = serde_json::from_str(&ingest.output_json).unwrap();
        assert!(
            ingest_output["files_indexed"].as_u64().unwrap_or_default() >= 3,
            "{ingest_output}"
        );
        assert!(
            ingest_output["symbols_indexed"]
                .as_u64()
                .unwrap_or_default()
                >= 4,
            "{ingest_output}"
        );

        let search = runtime
            .invoke(
                pb::InvokeAffordanceRequest {
                    tenant_id: "theorem".to_string(),
                    affordance_id: "theorem_grpc.code_search.search".to_string(),
                    actor: "test".to_string(),
                    request_json: json!({
                        "repo_id": "repo:boltbrowser-affordance",
                        "query": "main",
                        "limit": 5,
                    })
                    .to_string(),
                    dry_run: false,
                    confirmed: false,
                    timeout_ms: 0,
                },
                Instant::now(),
            )
            .unwrap();
        assert_eq!(search.status, "ok");
        let search_output: Value = serde_json::from_str(&search.output_json).unwrap();
        assert_eq!(search_output["hits"][0]["name"], json!("main"));
        assert_eq!(search_output["hits"][0]["language"], json!("go"));

        drop(runtime);
        std::fs::remove_dir_all(repo_dir).ok();
        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn confirmed_side_effecting_affordance_requires_live_adapter_configuration() {
        let (runtime, response, data_dir) = invoke(pb::InvokeAffordanceRequest {
            tenant_id: "theorem".to_string(),
            affordance_id: "theorem_grpc.publisher.publish".to_string(),
            actor: "test".to_string(),
            request_json: r#"{"artifact_id":"a1"}"#.to_string(),
            dry_run: false,
            confirmed: true,
            timeout_ms: 0,
        });

        assert_eq!(response.status, "failed");
        assert_eq!(response.error_code, "THESEUS_APP_ADAPTER_UNCONFIGURED");
        assert!(!response.executed);
        assert!(response
            .output_json
            .contains("\"theseus_adapter_unconfigured\""));
        drop(runtime);
        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn confirmed_side_effecting_affordance_calls_configured_theseus_adapter() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 4096];
            let read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]);
            assert!(request.contains("theorem_grpc.publisher.publish"));
            let body = r#"{"status":"ok","executed":true,"message":"published via test adapter","output":{"publication_id":"pub:test"}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });

        let (runtime, data_dir) =
            runtime_with_adapter(TheseusAppAdapter::new(Some(endpoint), None));
        let response = runtime
            .invoke(
                pb::InvokeAffordanceRequest {
                    tenant_id: "theorem".to_string(),
                    affordance_id: "theorem_grpc.publisher.publish".to_string(),
                    actor: "test".to_string(),
                    request_json: r#"{"artifact_id":"a1"}"#.to_string(),
                    dry_run: false,
                    confirmed: true,
                    timeout_ms: 0,
                },
                Instant::now(),
            )
            .unwrap();

        assert_eq!(response.status, "ok");
        assert!(response.executed);
        assert!(response
            .output_json
            .contains("\"publication_id\":\"pub:test\""));
        assert!(response.output_json.contains("\"theseus_adapter_ok\""));
        handle.join().unwrap();
        drop(runtime);
        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn invalid_json_is_receipted_as_failure_and_graph_outcome() {
        let (runtime, response, data_dir) = invoke(pb::InvokeAffordanceRequest {
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
        let receipts = store
            .query_nodes(NodeQuery::label(INVOCATION_RECEIPT_LABEL))
            .unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].properties["outcome_value"], 0.0);
        drop(store);
        drop(runtime);
        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn redcore_runtime_recovers_invocation_receipts() {
        let data_dir = unique_test_dir("recover");
        {
            let code_index =
                CodeIndexRuntime::try_new_at(data_dir.join("code-index"), test_options()).unwrap();
            let runtime = AppAffordanceRuntime::try_new_at(
                &data_dir,
                test_options(),
                TheseusAppAdapter::new(None, None),
                code_index,
            )
            .unwrap();
            let response = runtime
                .invoke(
                    pb::InvokeAffordanceRequest {
                        tenant_id: "theorem".to_string(),
                        affordance_id: "theorem_grpc.observability.read_trace".to_string(),
                        actor: "test".to_string(),
                        request_json: r#"{"run_id":"run:1"}"#.to_string(),
                        dry_run: false,
                        confirmed: false,
                        timeout_ms: 0,
                    },
                    Instant::now(),
                )
                .unwrap();
            assert_eq!(response.status, "ok");
        }

        let recovered = RedCoreGraphStore::open(&data_dir, test_options()).unwrap();
        let receipts = recovered
            .query_nodes(NodeQuery::label(INVOCATION_RECEIPT_LABEL))
            .unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].properties["outcome_label"], "handler_ok");
        std::fs::remove_dir_all(data_dir).ok();
    }
}
