use std::collections::{BTreeMap, BTreeSet};

use rustyred_thg_core::stable_hash;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Canonical runtime trace event observed from networking or state-machine layers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeTraceEvent {
    pub event_id: String,
    pub kind: RuntimeTraceEventKind,
}

/// Typed payload variants for trace events.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RuntimeTraceEventKind {
    HttpExchange(HttpExchangeTrace),
    StateTransition(ObservedStateTransition),
    Error(TraceErrorObservation),
}

/// Timing information for a single observed exchange.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimingRange {
    pub start_ms: u64,
    pub end_ms: u64,
}

impl TimingRange {
    fn normalized(mut self) -> Self {
        if self.end_ms < self.start_ms {
            std::mem::swap(&mut self.start_ms, &mut self.end_ms);
        }
        self
    }

    fn merge(self, other: TimingRange) -> Self {
        TimingRange {
            start_ms: self.start_ms.min(other.start_ms),
            end_ms: self.end_ms.max(other.end_ms),
        }
    }
}

/// Simple body-shape hint for trace-derived contract inference.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct BodyShapeHint {
    pub kind: String,
    pub fields: Vec<String>,
}

impl BodyShapeHint {
    fn normalized(&self) -> Self {
        let mut fields = self.fields.clone();
        fields.sort();
        fields.dedup();
        Self {
            kind: self.kind.trim().to_string(),
            fields,
        }
    }
}

/// HTTP exchange observation from runtime/network tracing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HttpExchangeTrace {
    pub exchange_id: String,
    pub tenant_id: String,
    pub repo_id: String,
    pub run_id: String,
    pub method: String,
    pub path: String,
    pub status_code: u16,
    pub request_content_type: Option<String>,
    pub response_content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_body_shape: Option<BodyShapeHint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_body_shape: Option<BodyShapeHint>,
    pub timing: TimingRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<TraceErrorObservation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_ids: Vec<String>,
}

/// Runtime state transition observed while serving a request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservedStateTransition {
    pub transition_id: String,
    pub tenant_id: String,
    pub repo_id: String,
    pub run_id: String,
    pub state_machine: String,
    pub from_state: String,
    pub to_state: String,
    pub observed_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_ids: Vec<String>,
}

/// Error observation captured from traces.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TraceErrorObservation {
    pub error_id: String,
    pub tenant_id: String,
    pub repo_id: String,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApiContractObservation {
    pub observation_id: String,
    pub tenant_ids: Vec<String>,
    pub repo_ids: Vec<String>,
    pub run_ids: Vec<String>,
    pub method: String,
    pub path: String,
    pub status_codes: Vec<u16>,
    pub status_shapes: Vec<String>,
    pub request_content_types: Vec<String>,
    pub response_content_types: Vec<String>,
    pub request_body_shape: Vec<BodyShapeHint>,
    pub response_body_shape: Vec<BodyShapeHint>,
    pub error_observations: Vec<TraceErrorObservation>,
    pub timing: TimingRange,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EndpointContract {
    pub endpoint_id: String,
    pub tenant_ids: Vec<String>,
    pub repo_ids: Vec<String>,
    pub run_ids: Vec<String>,
    pub method: String,
    pub path: String,
    pub status_codes: Vec<u16>,
    pub status_shapes: Vec<String>,
    pub request_content_types: Vec<String>,
    pub response_content_types: Vec<String>,
    pub request_body_shape: Vec<BodyShapeHint>,
    pub response_body_shape: Vec<BodyShapeHint>,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TraceValidatorSpec {
    pub validator_id: String,
    pub tenant_ids: Vec<String>,
    pub repo_ids: Vec<String>,
    pub run_ids: Vec<String>,
    pub target_id: String,
    pub validator_kind: String,
    pub rule: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TraceContractReport {
    pub tenant_ids: Vec<String>,
    pub repo_ids: Vec<String>,
    pub run_ids: Vec<String>,
    pub event_count: usize,
    pub endpoint_contracts: Vec<EndpointContract>,
    pub observations: Vec<ApiContractObservation>,
    pub state_transitions: Vec<ObservedStateTransition>,
    pub error_observations: Vec<TraceErrorObservation>,
    pub validator_specs: Vec<TraceValidatorSpec>,
    pub evidence_ids: Vec<String>,
    pub artifact_hash: String,
}

/// Builds API-contract-oriented observations from typed runtime/network traces.
pub fn compile_trace_contract(events: &[RuntimeTraceEvent]) -> TraceContractReport {
    let mut endpoint_map = BTreeMap::<(String, String), EndpointAccumulator>::new();
    let mut state_transitions = Vec::new();
    let mut error_observations = Vec::new();
    let mut tenant_ids = BTreeSet::new();
    let mut repo_ids = BTreeSet::new();
    let mut run_ids = BTreeSet::new();
    let mut evidence_ids = BTreeSet::new();

    for event in events {
        match &event.kind {
            RuntimeTraceEventKind::HttpExchange(exchange) => {
                let method = normalize_method(&exchange.method);
                let path = normalize_path(&exchange.path);
                let key = (method.clone(), path);
                let entry = endpoint_map.entry(key).or_default();
                entry.tenant_ids.insert(exchange.tenant_id.clone());
                entry.repo_ids.insert(exchange.repo_id.clone());
                entry.run_ids.insert(exchange.run_id.clone());

                entry.status_codes.insert(exchange.status_code);
                entry
                    .status_shapes
                    .insert(status_shape(exchange.status_code));
                if let Some(content_type) = &exchange.request_content_type {
                    if !content_type.trim().is_empty() {
                        entry
                            .request_content_types
                            .insert(content_type.trim().to_string());
                    }
                }
                if let Some(content_type) = &exchange.response_content_type {
                    if !content_type.trim().is_empty() {
                        entry
                            .response_content_types
                            .insert(content_type.trim().to_string());
                    }
                }
                if let Some(shape) = &exchange.request_body_shape {
                    let shape = shape.normalized();
                    entry
                        .request_body_shape
                        .entry(shape.kind)
                        .or_default()
                        .extend(shape.fields.into_iter());
                }
                if let Some(shape) = &exchange.response_body_shape {
                    let shape = shape.normalized();
                    entry
                        .response_body_shape
                        .entry(shape.kind)
                        .or_default()
                        .extend(shape.fields.into_iter());
                }
                for evidence in &exchange.evidence_ids {
                    entry.evidence_ids.insert(evidence.clone());
                    evidence_ids.insert(evidence.clone());
                }
                entry.timing = Some(
                    entry
                        .timing
                        .take()
                        .map_or(exchange.timing.normalized(), |timing| {
                            timing.merge(exchange.timing.normalized())
                        }),
                );

                tenant_ids.insert(exchange.tenant_id.clone());
                repo_ids.insert(exchange.repo_id.clone());
                run_ids.insert(exchange.run_id.clone());

                if let Some(error) = &exchange.error {
                    entry.error_observations.push(error.clone());
                    error_observations.push(error.clone());
                }
            }
            RuntimeTraceEventKind::StateTransition(transition) => {
                for evidence in &transition.evidence_ids {
                    evidence_ids.insert(evidence.clone());
                }
                tenant_ids.insert(transition.tenant_id.clone());
                repo_ids.insert(transition.repo_id.clone());
                run_ids.insert(transition.run_id.clone());
                state_transitions.push(transition.clone());
            }
            RuntimeTraceEventKind::Error(error) => {
                for evidence in &error.evidence_ids {
                    evidence_ids.insert(evidence.clone());
                }
                tenant_ids.insert(error.tenant_id.clone());
                repo_ids.insert(error.repo_id.clone());
                run_ids.insert(error.run_id.clone());
                error_observations.push(error.clone());
            }
        }
    }

    let mut endpoint_contracts = Vec::new();
    let mut observations = Vec::new();
    let mut validator_specs = Vec::new();

    for ((method, path), accumulator) in endpoint_map {
        let status_codes = to_vec(accumulator.status_codes);
        let status_shapes = to_vec(accumulator.status_shapes);
        let request_content_types = to_vec(accumulator.request_content_types);
        let response_content_types = to_vec(accumulator.response_content_types);
        let tenant_ids = to_vec(accumulator.tenant_ids);
        let repo_ids = to_vec(accumulator.repo_ids);
        let run_ids = to_vec(accumulator.run_ids);
        let request_body_shape = materialize_shape(accumulator.request_body_shape);
        let response_body_shape = materialize_shape(accumulator.response_body_shape);
        let evidence_ids = to_vec(accumulator.evidence_ids);
        let mut endpoint_errors = accumulator.error_observations;
        endpoint_errors.sort_by(|left, right| left.error_id.cmp(&right.error_id));
        let timing = accumulator.timing.unwrap_or(TimingRange {
            start_ms: 0,
            end_ms: 0,
        });

        let endpoint_signature = json!({
            "method": &method,
            "path": &path,
            "status_codes": &status_codes,
            "status_shapes": &status_shapes,
            "request_content_types": &request_content_types,
            "response_content_types": &response_content_types,
            "request_body_shape": &request_body_shape,
            "response_body_shape": &response_body_shape,
            "run_ids": &run_ids,
        });
        let endpoint_id = format!("trace:endpoint:{}", stable_hash(endpoint_signature));

        endpoint_contracts.push(EndpointContract {
            endpoint_id: endpoint_id.clone(),
            tenant_ids: tenant_ids.clone(),
            repo_ids: repo_ids.clone(),
            run_ids: run_ids.clone(),
            method: method.clone(),
            path: path.clone(),
            status_codes: status_codes.clone(),
            status_shapes: status_shapes.clone(),
            request_content_types: request_content_types.clone(),
            response_content_types: response_content_types.clone(),
            request_body_shape: request_body_shape.clone(),
            response_body_shape: response_body_shape.clone(),
            evidence_ids: evidence_ids.clone(),
        });

        observations.push(ApiContractObservation {
            observation_id: format!(
                "trace:observation:{}",
                stable_hash(json!({
                    "method": &method,
                    "path": &path,
                    "status_codes": &status_codes,
                    "status_shapes": &status_shapes,
                    "tenant_ids": &tenant_ids,
                    "repo_ids": &repo_ids,
                    "run_ids": &run_ids,
                })),
            ),
            tenant_ids: tenant_ids.clone(),
            repo_ids: repo_ids.clone(),
            run_ids: run_ids.clone(),
            method: method.clone(),
            path: path.clone(),
            status_codes: status_codes.clone(),
            status_shapes: status_shapes.clone(),
            request_content_types: request_content_types.clone(),
            response_content_types: response_content_types.clone(),
            request_body_shape: request_body_shape.clone(),
            response_body_shape: response_body_shape.clone(),
            error_observations: endpoint_errors,
            timing,
            evidence_ids: evidence_ids.clone(),
        });

        validator_specs.push(TraceValidatorSpec {
            validator_id: format!(
                "trace:validator:{}",
                stable_hash(json!({
                    "validator": "trace-contract-shape",
                    "target": &endpoint_id,
                    "request_content_types": &request_content_types,
                    "response_content_types": &response_content_types,
                    "request_body_shape": &request_body_shape,
                    "response_body_shape": &response_body_shape,
                    "tenant_ids": &tenant_ids,
                    "repo_ids": &repo_ids,
                    "run_ids": &run_ids,
                })),
            ),
            tenant_ids: tenant_ids.clone(),
            repo_ids: repo_ids.clone(),
            run_ids,
            target_id: endpoint_id,
            validator_kind: "trace-contract-shape".to_string(),
            rule: "Validate endpoint status, content-type, and body-shape signatures observed in runtime traces.".to_string(),
            evidence_ids,
        });
    }

    endpoint_contracts.sort_by(|left, right| {
        left.method
            .cmp(&right.method)
            .then_with(|| left.path.cmp(&right.path))
    });
    observations.sort_by(|left, right| {
        left.method
            .cmp(&right.method)
            .then_with(|| left.path.cmp(&right.path))
    });
    state_transitions.sort_by_key(|transition| transition.observed_ms);
    error_observations.sort_by(|left, right| left.error_id.cmp(&right.error_id));
    validator_specs.sort_by(|left, right| left.validator_id.cmp(&right.validator_id));

    let artifact_hash = stable_hash(json!({
        "tenant_ids": &tenant_ids,
        "repo_ids": &repo_ids,
        "run_ids": &run_ids,
        "event_count": events.len(),
        "endpoint_contracts": &endpoint_contracts,
        "observations": &observations,
        "state_transitions": &state_transitions,
        "error_observations": &error_observations,
        "validator_specs": &validator_specs,
    }));

    TraceContractReport {
        tenant_ids: to_vec(tenant_ids),
        repo_ids: to_vec(repo_ids),
        run_ids: to_vec(run_ids),
        event_count: events.len(),
        endpoint_contracts,
        observations,
        state_transitions,
        error_observations,
        validator_specs,
        evidence_ids: to_vec(evidence_ids),
        artifact_hash: format!("trace:contract:{}", artifact_hash),
    }
}

fn normalize_method(input: &str) -> String {
    input.trim().to_ascii_uppercase()
}

fn normalize_path(input: &str) -> String {
    let path = input.trim();
    if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    }
}

fn status_shape(status: u16) -> String {
    format!("{}xx", status / 100)
}

fn materialize_shape(shape_by_kind: BTreeMap<String, BTreeSet<String>>) -> Vec<BodyShapeHint> {
    let mut out = shape_by_kind
        .into_iter()
        .map(|(kind, fields)| {
            let mut fields = fields.into_iter().collect::<Vec<_>>();
            fields.sort();
            BodyShapeHint { kind, fields }
        })
        .collect::<Vec<_>>();
    out.sort();
    out
}

fn to_vec<T: Ord>(items: BTreeSet<T>) -> Vec<T> {
    items.into_iter().collect()
}

#[derive(Default)]
struct EndpointAccumulator {
    tenant_ids: BTreeSet<String>,
    repo_ids: BTreeSet<String>,
    run_ids: BTreeSet<String>,
    status_codes: BTreeSet<u16>,
    status_shapes: BTreeSet<String>,
    request_content_types: BTreeSet<String>,
    response_content_types: BTreeSet<String>,
    request_body_shape: BTreeMap<String, BTreeSet<String>>,
    response_body_shape: BTreeMap<String, BTreeSet<String>>,
    evidence_ids: BTreeSet<String>,
    error_observations: Vec<TraceErrorObservation>,
    timing: Option<TimingRange>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(
        exchange_id: &str,
        tenant_id: &str,
        repo_id: &str,
        run_id: &str,
        method: &str,
        path: &str,
        status_code: u16,
        request_content_type: Option<&str>,
        response_content_type: Option<&str>,
        request_fields: &[&str],
        response_fields: &[&str],
        evidence: &[&str],
        start_ms: u64,
        end_ms: u64,
    ) -> RuntimeTraceEvent {
        RuntimeTraceEvent {
            event_id: format!("event:{exchange_id}"),
            kind: RuntimeTraceEventKind::HttpExchange(HttpExchangeTrace {
                exchange_id: exchange_id.to_string(),
                tenant_id: tenant_id.to_string(),
                repo_id: repo_id.to_string(),
                run_id: run_id.to_string(),
                method: method.to_string(),
                path: path.to_string(),
                status_code,
                request_content_type: request_content_type.map(str::to_string),
                response_content_type: response_content_type.map(str::to_string),
                request_body_shape: Some(BodyShapeHint {
                    kind: "object".to_string(),
                    fields: request_fields
                        .iter()
                        .map(|field| field.to_string())
                        .collect(),
                }),
                response_body_shape: Some(BodyShapeHint {
                    kind: "object".to_string(),
                    fields: response_fields
                        .iter()
                        .map(|field| field.to_string())
                        .collect(),
                }),
                timing: TimingRange { start_ms, end_ms },
                error: None,
                evidence_ids: evidence.iter().map(|value| value.to_string()).collect(),
            }),
        }
    }

    #[test]
    fn endpoint_grouping_is_by_method_and_path() {
        let tenant_id = "Travis-Gilbert";
        let repo_id = "repo:compiler";
        let run_id = "run:main";

        let events = vec![
            exchange(
                "e1",
                tenant_id,
                repo_id,
                run_id,
                "GET",
                "/api/items",
                200,
                Some("application/json"),
                Some("application/json"),
                &["page"],
                &["items"],
                &["e1-a"],
                10,
                12,
            ),
            exchange(
                "e2",
                tenant_id,
                repo_id,
                run_id,
                "GET",
                "/api/items",
                201,
                Some("application/json"),
                Some("application/json"),
                &["page"],
                &["count"],
                &["e2-a"],
                15,
                18,
            ),
            exchange(
                "e3",
                tenant_id,
                repo_id,
                run_id,
                "POST",
                "/api/items",
                201,
                Some("application/json"),
                Some("application/json"),
                &["name"],
                &["created"],
                &["e3-a"],
                20,
                22,
            ),
        ];

        let report = compile_trace_contract(&events);

        assert_eq!(report.endpoint_contracts.len(), 2);
        let get_items = report
            .endpoint_contracts
            .iter()
            .find(|contract| contract.method == "GET" && contract.path == "/api/items")
            .expect("GET /api/items should be present");
        assert_eq!(get_items.status_codes, vec![200, 201]);

        let post_items = report
            .endpoint_contracts
            .iter()
            .find(|contract| contract.method == "POST" && contract.path == "/api/items")
            .expect("POST /api/items should be present");
        assert_eq!(post_items.status_codes, vec![201]);
        assert_eq!(report.tenant_ids, vec![tenant_id.to_string()]);
        assert_eq!(report.repo_ids, vec![repo_id.to_string()]);
        assert_eq!(report.run_ids, vec![run_id.to_string()]);
    }

    #[test]
    fn status_and_body_shape_aggregation_collects_fields_and_shapes() {
        let events = vec![
            exchange(
                "e1",
                "Travis-Gilbert",
                "repo:contract",
                "run:one",
                "GET",
                "/api/orders",
                200,
                Some("application/json"),
                Some("application/json"),
                &["page", "limit"],
                &["id", "total"],
                &["evidence-1"],
                20,
                26,
            ),
            exchange(
                "e2",
                "Travis-Gilbert",
                "repo:contract",
                "run:one",
                "GET",
                "/api/orders",
                404,
                Some("application/json"),
                Some("application/json"),
                &["page"],
                &["error", "detail"],
                &["evidence-2"],
                30,
                40,
            ),
        ];

        let report = compile_trace_contract(&events);
        let observation = &report.observations[0];

        assert_eq!(observation.method, "GET");
        assert_eq!(observation.path, "/api/orders");
        assert_eq!(observation.status_shapes, vec!["2xx", "4xx"]);
        assert_eq!(observation.request_content_types, vec!["application/json"]);
        assert_eq!(observation.response_content_types, vec!["application/json"]);

        let response_fields = observation
            .response_body_shape
            .iter()
            .find(|shape| shape.kind == "object")
            .expect("response body shape should exist");
        assert_eq!(
            response_fields.fields,
            vec!["detail", "error", "id", "total"]
        );
        assert_eq!(
            observation.timing,
            TimingRange {
                start_ms: 20,
                end_ms: 40
            }
        );
    }

    #[test]
    fn evidence_ids_are_preserved_for_http_endpoints() {
        let events = vec![
            exchange(
                "e1",
                "Travis-Gilbert",
                "repo:evidence",
                "run:alpha",
                "GET",
                "/api/evidence",
                200,
                Some("application/json"),
                Some("application/json"),
                &["q"],
                &["ok"],
                &["http-e1", "shared"],
                11,
                15,
            ),
            exchange(
                "e2",
                "Travis-Gilbert",
                "repo:evidence",
                "run:alpha",
                "GET",
                "/api/evidence",
                200,
                Some("application/json"),
                Some("application/json"),
                &["q"],
                &["ok"],
                &["http-e2", "shared"],
                20,
                23,
            ),
        ];

        let report = compile_trace_contract(&events);
        let evidence = &report.evidence_ids;

        assert_eq!(evidence, &vec!["http-e1", "http-e2", "shared"]);
        let contract = &report.endpoint_contracts[0];
        assert_eq!(contract.evidence_ids, vec!["http-e1", "http-e2", "shared"]);
    }

    #[test]
    fn state_transitions_are_captured_from_trace_events() {
        let events = vec![RuntimeTraceEvent {
            event_id: "state-1".to_string(),
            kind: RuntimeTraceEventKind::StateTransition(ObservedStateTransition {
                transition_id: "transition-1".to_string(),
                tenant_id: "Travis-Gilbert".to_string(),
                repo_id: "repo:state".to_string(),
                run_id: "run:state".to_string(),
                state_machine: "order".to_string(),
                from_state: "draft".to_string(),
                to_state: "submitted".to_string(),
                observed_ms: 100,
                evidence_ids: vec!["state-evidence".to_string()],
            }),
        }];

        let report = compile_trace_contract(&events);

        assert_eq!(report.state_transitions.len(), 1);
        assert_eq!(report.state_transitions[0].transition_id, "transition-1");
        assert_eq!(report.state_transitions[0].from_state, "draft");
        assert_eq!(report.state_transitions[0].to_state, "submitted");
        assert_eq!(report.evidence_ids, vec!["state-evidence"]);
    }

    #[test]
    fn empty_input_returns_empty_report_and_stable_hash() {
        let report = compile_trace_contract(&[]);

        assert_eq!(report.event_count, 0);
        assert!(report.endpoint_contracts.is_empty());
        assert!(report.observations.is_empty());
        assert!(report.state_transitions.is_empty());
        assert!(report.error_observations.is_empty());
        assert!(report.validator_specs.is_empty());
        assert!(report.evidence_ids.is_empty());
        assert!(report.tenant_ids.is_empty());
        assert!(report.repo_ids.is_empty());
        assert!(report.run_ids.is_empty());
        assert_eq!(
            report.artifact_hash,
            compile_trace_contract(&[]).artifact_hash,
            "empty input should be deterministic"
        );
    }

    #[test]
    fn endpoint_observations_only_include_endpoint_errors() {
        let first = exchange(
            "e1",
            "Travis-Gilbert",
            "repo:errors",
            "run:errors",
            "GET",
            "/api/ok",
            200,
            None,
            None,
            &[],
            &[],
            &["ok-evidence"],
            10,
            12,
        );
        let mut second = exchange(
            "e2",
            "Travis-Gilbert",
            "repo:errors",
            "run:errors",
            "GET",
            "/api/fails",
            500,
            None,
            None,
            &[],
            &[],
            &["fail-evidence"],
            20,
            24,
        );
        if let RuntimeTraceEventKind::HttpExchange(exchange) = &mut second.kind {
            exchange.error = Some(TraceErrorObservation {
                error_id: "error:fails".to_string(),
                tenant_id: "Travis-Gilbert".to_string(),
                repo_id: "repo:errors".to_string(),
                run_id: "run:errors".to_string(),
                source: Some("handler".to_string()),
                code: Some("500".to_string()),
                message: Some("observed failure".to_string()),
                evidence_ids: vec!["fail-evidence".to_string()],
            });
        }

        let report = compile_trace_contract(&[first, second]);
        let ok = report
            .observations
            .iter()
            .find(|observation| observation.path == "/api/ok")
            .expect("ok endpoint");
        let fails = report
            .observations
            .iter()
            .find(|observation| observation.path == "/api/fails")
            .expect("failing endpoint");

        assert!(ok.error_observations.is_empty());
        assert_eq!(fails.error_observations.len(), 1);
        assert_eq!(report.error_observations.len(), 1);
    }
}
