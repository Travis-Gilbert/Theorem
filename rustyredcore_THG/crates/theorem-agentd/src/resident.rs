//! Proxy-resident capabilities for the local Anthropic Messages proxy.
//!
//! This module owns the cache-stable tool injection and the pure request/response
//! rewrites. The network loop stays in `proxy.rs` so credentials and upstream
//! topology remain in one place.

use std::path::Path;

use rustyred_thg_affordances::Affordance;
use rustyred_thg_offload::{
    plan_from_json, CalibrationSample, CascadeRouter, COMPUTE_OFFLOAD_ENGINE_ID,
    COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use theorem_harness_core::{affordance_by_id, BindingCapabilityScope};

pub const TOOL_SEARCH: &str = "tool_search";
pub const DESCRIBE: &str = "describe";
pub const INVOKE: &str = "invoke";
pub const DIRECT_COMPUTE_OFFLOAD_ROUTE: &str =
    "theorem_affordance__compute_offload__route_operation";
pub const RESIDENT_TOOL_NAMES: [&str; 4] =
    [TOOL_SEARCH, DESCRIBE, INVOKE, DIRECT_COMPUTE_OFFLOAD_ROUTE];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResidentToolUse {
    pub id: String,
    pub name: String,
    pub input: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CascadeCalibrationFile {
    pub source: String,
    pub samples: Vec<CalibrationSample>,
    #[serde(default = "default_cascade_quality_floor")]
    pub quality_floor: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CascadeRouteDecision {
    pub selected: CascadeRouteTarget,
    pub calibrated_confidence: f64,
    pub quality_floor: f64,
    pub calibration_source: String,
    pub raw_score: f64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CascadeRouteTarget {
    Local,
    Upstream,
    CalibrationRequired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VerificationClaim {
    pub claim: String,
    pub contradicted_by: String,
    pub basis: String,
    #[serde(default)]
    pub checker: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct VerificationFinding {
    pub claim: String,
    pub contradicted_by: String,
    pub basis: String,
    pub checker: String,
}

pub fn resident_enabled_from_env() -> bool {
    std::env::var("THEOREM_PROXY_RESIDENT_CAPABILITIES")
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(true)
}

pub fn inject_resident_tools(request: &mut Value) {
    let existing = request
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tools = request
        .as_object_mut()
        .expect("messages request is an object")
        .entry("tools")
        .or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(tools) = tools else {
        return;
    };
    for definition in resident_tool_definitions() {
        let Some(name) = definition.get("name").and_then(Value::as_str) else {
            continue;
        };
        if existing.iter().any(|existing| existing == name) {
            continue;
        }
        tools.push(definition);
    }
}

pub fn resident_tool_definitions() -> Vec<Value> {
    let mut definitions = vec![
        json!({
            "name": TOOL_SEARCH,
            "description": "Search tenant-scoped Theorem harness affordances through the proxy-resident connector gateway.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "query": { "type": "string" },
                    "task_type": { "type": "string" },
                    "k": { "type": "integer", "default": 10 },
                    "limit": { "type": "integer", "default": 10 },
                    "allow_affordance_ids": { "type": "array", "items": { "type": "string" } },
                    "allow_servers": { "type": "array", "items": { "type": "string" } },
                    "allow_families": { "type": "array", "items": { "type": "string" } },
                    "allow_tags": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": DESCRIBE,
            "description": "Materialize one Theorem harness affordance schema on demand by affordance_id.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "affordance_id": { "type": "string" }
                },
                "required": ["affordance_id"]
            }
        }),
        json!({
            "name": INVOKE,
            "description": "Invoke a selected Theorem harness affordance. Tier-two and tier-three actions require human authorization before execution.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "affordance_id": { "type": "string" },
                    "arguments": { "type": "object" },
                    "tool_arguments": { "type": "object" },
                    "task_type": { "type": "string" },
                    "candidate_affordance_ids": { "type": "array", "items": { "type": "string" } },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "dry_run": { "type": "boolean", "default": false },
                    "action_tier": { "type": "string" },
                    "human_authorized": { "type": "boolean", "default": false }
                },
                "required": ["affordance_id", "arguments"]
            }
        }),
    ];
    definitions.extend(selected_affordance_tool_definitions());
    definitions
}

pub fn selected_affordance_tool_definitions() -> Vec<Value> {
    let Some(contract) = affordance_by_id(COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID) else {
        return Vec::new();
    };
    let affordance = Affordance::from_contract(&contract, "Travis-Gilbert");
    vec![json!({
        "name": DIRECT_COMPUTE_OFFLOAD_ROUTE,
        "description": format!(
            "Theorem harness affordance {} from rustyred-thg-affordances. {}",
            affordance.affordance_id, affordance.description
        ),
        "input_schema": compute_offload_schema()
    })]
}

pub fn resident_tool_uses(response: &Value) -> Vec<ResidentToolUse> {
    response
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter_map(|block| {
            let name = block.get("name").and_then(Value::as_str)?;
            if !RESIDENT_TOOL_NAMES
                .iter()
                .any(|candidate| candidate == &name)
            {
                return None;
            }
            Some(ResidentToolUse {
                id: block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("resident-tool-use")
                    .to_string(),
                name: name.to_string(),
                input: block.get("input").cloned().unwrap_or_else(|| json!({})),
            })
        })
        .collect()
}

pub fn append_tool_results(request: &mut Value, assistant_response: &Value, results: Vec<Value>) {
    let Some(messages) = request.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    messages.push(json!({
        "role": "assistant",
        "content": assistant_response.get("content").cloned().unwrap_or_else(|| json!([]))
    }));
    messages.push(json!({
        "role": "user",
        "content": results
    }));
}

pub fn tool_result_block(tool_use_id: &str, payload: Value, is_error: bool) -> Value {
    json!({
        "type": "tool_result",
        "tool_use_id": tool_use_id,
        "is_error": is_error,
        "content": serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
    })
}

pub fn approval_required_payload(tool_use: &ResidentToolUse) -> Option<Value> {
    if tool_use.name != INVOKE && resident_tool_name_to_affordance_id(&tool_use.name).is_none() {
        return None;
    }
    let tier = action_tier(&tool_use.input)?;
    let scope = BindingCapabilityScope::for_agent("theorem-proxy");
    let requires = scope
        .action_tiers
        .iter()
        .find(|policy| policy.tier_id == tier)
        .map(|policy| policy.requires_human_authorization)
        .unwrap_or(false);
    if !requires || human_authorized(&tool_use.input) {
        return None;
    }
    let approval_id = format!(
        "proxy-approval:{}",
        stable_hash_hex(&json!({"tool_use_id": tool_use.id, "input": tool_use.input}).to_string())
    );
    Some(json!({
        "status": "approval_required",
        "approval_id": approval_id,
        "action_tier": tier,
        "message": "This affordance is held because the binding action tier requires explicit human authorization.",
        "phone_authorization_surface": "/v1/runs/:id/approve",
        "executed": false
    }))
}

pub fn fallback_tool_result(name: &str, input: &Value, tenant_slug: &str) -> Option<Value> {
    match name {
        TOOL_SEARCH => Some(fallback_search(input, tenant_slug)),
        DESCRIBE => fallback_describe(input, tenant_slug),
        INVOKE => fallback_invoke(input, tenant_slug),
        DIRECT_COMPUTE_OFFLOAD_ROUTE => fallback_invoke(
            &json!({
                "affordance_id": COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID,
                "arguments": input
            }),
            tenant_slug,
        ),
        _ => None,
    }
}

pub fn gateway_call_for_tool_use(tool_use: &ResidentToolUse, tenant_slug: &str) -> (String, Value) {
    if let Some(affordance_id) = resident_tool_name_to_affordance_id(&tool_use.name) {
        return (
            INVOKE.to_string(),
            json!({
                "tenant_slug": tenant_slug,
                "affordance_id": affordance_id,
                "arguments": tool_use.input
            }),
        );
    }
    let mut arguments = match tool_use.input.clone() {
        Value::Object(_) => tool_use.input.clone(),
        _ => json!({}),
    };
    if arguments.get("tenant_slug").is_none() && arguments.get("tenant").is_none() {
        arguments["tenant_slug"] = json!(tenant_slug);
    }
    (tool_use.name.clone(), arguments)
}

pub fn resident_tool_name_to_affordance_id(name: &str) -> Option<&'static str> {
    match name {
        DIRECT_COMPUTE_OFFLOAD_ROUTE => Some(COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID),
        _ => None,
    }
}

pub fn route_with_calibration_file(
    path: Option<&Path>,
    latest_user_text: &str,
    local_upstream_configured: bool,
) -> CascadeRouteDecision {
    let Some(path) = path else {
        return CascadeRouteDecision {
            selected: CascadeRouteTarget::CalibrationRequired,
            calibrated_confidence: 0.0,
            quality_floor: default_cascade_quality_floor(),
            calibration_source: "missing THEOREM_PROXY_CASCADE_CALIBRATION".to_string(),
            raw_score: raw_local_competence_score(latest_user_text),
        };
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return CascadeRouteDecision {
            selected: CascadeRouteTarget::CalibrationRequired,
            calibrated_confidence: 0.0,
            quality_floor: default_cascade_quality_floor(),
            calibration_source: format!("unreadable calibration file: {}", path.display()),
            raw_score: raw_local_competence_score(latest_user_text),
        };
    };
    let Ok(calibration) = serde_json::from_str::<CascadeCalibrationFile>(&text) else {
        return CascadeRouteDecision {
            selected: CascadeRouteTarget::CalibrationRequired,
            calibrated_confidence: 0.0,
            quality_floor: default_cascade_quality_floor(),
            calibration_source: format!("invalid calibration file: {}", path.display()),
            raw_score: raw_local_competence_score(latest_user_text),
        };
    };
    if calibration.samples.is_empty()
        || !calibration
            .source
            .contains("SPEC-BEHAVIOR-CORPUS.md deliverables 3 and 5")
    {
        return CascadeRouteDecision {
            selected: CascadeRouteTarget::CalibrationRequired,
            calibrated_confidence: 0.0,
            quality_floor: calibration.quality_floor,
            calibration_source: calibration.source,
            raw_score: raw_local_competence_score(latest_user_text),
        };
    }
    let raw_score = raw_local_competence_score(latest_user_text);
    let router = CascadeRouter {
        calibrator: rustyred_thg_offload::IsotonicCalibrator::fit(&calibration.samples),
        ..CascadeRouter::default()
    };
    let calibrated = router.calibrator.predict(raw_score);
    let selected = if local_upstream_configured && calibrated >= calibration.quality_floor {
        CascadeRouteTarget::Local
    } else {
        CascadeRouteTarget::Upstream
    };
    CascadeRouteDecision {
        selected,
        calibrated_confidence: calibrated,
        quality_floor: calibration.quality_floor,
        calibration_source: calibration.source,
        raw_score,
    }
}

pub fn raw_local_competence_score(text: &str) -> f64 {
    let len_penalty = (text.len() as f64 / 4000.0).min(0.45);
    let lower = text.to_ascii_lowercase();
    let hard_terms = [
        "deploy",
        "merge",
        "release",
        "security",
        "legal",
        "financial",
        "medical",
        "production",
        "irreversible",
    ];
    let hard_penalty = hard_terms
        .iter()
        .filter(|term| lower.contains(**term))
        .count() as f64
        * 0.06;
    (0.86 - len_penalty - hard_penalty).clamp(0.0, 1.0)
}

pub fn load_verification_claims(path: Option<&Path>, data_dir: &Path) -> Vec<VerificationClaim> {
    let path = path
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| data_dir.join("verification_claims.json"));
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    claims_from_json_text(&text)
}

pub fn claims_from_json_text(text: &str) -> Vec<VerificationClaim> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    if let Ok(claims) = serde_json::from_value::<Vec<VerificationClaim>>(value.clone()) {
        return clean_claims(claims);
    }
    serde_json::from_value::<Vec<VerificationClaim>>(
        value
            .get("claims")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    )
    .map(clean_claims)
    .unwrap_or_default()
}

pub fn verification_findings(text: &str, claims: &[VerificationClaim]) -> Vec<VerificationFinding> {
    let lower = text.to_ascii_lowercase();
    claims
        .iter()
        .filter(|claim| lower.contains(&claim.claim.to_ascii_lowercase()))
        .map(|claim| VerificationFinding {
            claim: claim.claim.clone(),
            contradicted_by: claim.contradicted_by.clone(),
            basis: claim.basis.clone(),
            checker: if claim.checker.trim().is_empty() {
                "proxy-resident-graph-consistency".to_string()
            } else {
                claim.checker.clone()
            },
        })
        .collect()
}

pub fn append_verification_advisory(
    request: &mut Value,
    assistant_response: &Value,
    findings: &[VerificationFinding],
) {
    let Some(messages) = request.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    messages.push(json!({
        "role": "assistant",
        "content": assistant_response.get("content").cloned().unwrap_or_else(|| json!([]))
    }));
    messages.push(json!({
        "role": "user",
        "content": [{
            "type": "text",
            "text": advisory_text(findings)
        }]
    }));
}

pub fn advisory_text(findings: &[VerificationFinding]) -> String {
    let body = serde_json::to_string_pretty(findings).unwrap_or_else(|_| "[]".to_string());
    format!(
        "<theorem_verification_advisory>\nThe substrate found advisory verification findings. This is not a blocking gate; revise if the basis is relevant.\n{body}\n</theorem_verification_advisory>"
    )
}

pub fn assistant_text(response: &Value) -> String {
    response
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn anthropic_sse_from_message(message: &Value) -> String {
    let mut out = String::new();
    let mut start = message.clone();
    start["content"] = json!([]);
    push_sse(
        &mut out,
        "message_start",
        json!({ "type": "message_start", "message": start }),
    );
    for (index, block) in message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                push_sse(
                    &mut out,
                    "content_block_start",
                    json!({
                        "type": "content_block_start",
                        "index": index,
                        "content_block": { "type": "text", "text": "" }
                    }),
                );
                push_sse(
                    &mut out,
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {
                            "type": "text_delta",
                            "text": block.get("text").and_then(Value::as_str).unwrap_or("")
                        }
                    }),
                );
                push_sse(
                    &mut out,
                    "content_block_stop",
                    json!({ "type": "content_block_stop", "index": index }),
                );
            }
            Some("tool_use") => {
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                let mut start_block = block.clone();
                start_block["input"] = json!({});
                push_sse(
                    &mut out,
                    "content_block_start",
                    json!({
                        "type": "content_block_start",
                        "index": index,
                        "content_block": start_block
                    }),
                );
                push_sse(
                    &mut out,
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {
                            "type": "input_json_delta",
                            "partial_json": input.to_string()
                        }
                    }),
                );
                push_sse(
                    &mut out,
                    "content_block_stop",
                    json!({ "type": "content_block_stop", "index": index }),
                );
            }
            _ => {}
        }
    }
    push_sse(
        &mut out,
        "message_delta",
        json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": message.get("stop_reason").cloned().unwrap_or(Value::Null),
                "stop_sequence": message.get("stop_sequence").cloned().unwrap_or(Value::Null)
            },
            "usage": message.get("usage").cloned().unwrap_or_else(|| json!({}))
        }),
    );
    push_sse(&mut out, "message_stop", json!({ "type": "message_stop" }));
    out
}

fn fallback_search(input: &Value, tenant_slug: &str) -> Value {
    let query = input
        .get("query")
        .or_else(|| input.get("task_type"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let lower = query.to_ascii_lowercase();
    let mut results = Vec::new();
    if lower.contains("compute") || lower.contains("offload") || lower.contains("verification") {
        results.push(compact_compute_offload_affordance());
    }
    json!({
        "tenant": tenant_slug,
        "query": query,
        "task_type": input.get("task_type").and_then(Value::as_str).unwrap_or(query),
        "results": results,
        "candidate_affordance_ids": results.iter().filter_map(|item| item.get("affordance_id").cloned()).collect::<Vec<_>>(),
        "ranking": "proxy-resident-fallback",
        "catalog_spend": "compact-only",
        "fallback": true
    })
}

fn fallback_describe(input: &Value, tenant_slug: &str) -> Option<Value> {
    let affordance_id = input.get("affordance_id").and_then(Value::as_str)?;
    if affordance_id != COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID {
        return None;
    }
    Some(json!({
        "tenant": tenant_slug,
        "affordance_id": COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID,
        "server_id": COMPUTE_OFFLOAD_ENGINE_ID,
        "tool_name": COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID,
        "name": "Compute Offload Route Operation",
        "description": "Plan CPU, cache, verification, and model executor routing for an operation list.",
        "input_schema": compute_offload_schema(),
        "permissions": ["read"],
        "writeback_policy": "read-only",
        "tags": ["compute_offload", "verification", "cascade"],
        "fallback": true
    }))
}

fn fallback_invoke(input: &Value, tenant_slug: &str) -> Option<Value> {
    let affordance_id = input.get("affordance_id").and_then(Value::as_str)?;
    if affordance_id != COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID {
        return None;
    }
    let arguments = input
        .get("arguments")
        .or_else(|| input.get("tool_arguments"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let plan = match plan_from_json(&arguments, 0) {
        Ok(plan) => json!(plan),
        Err(error) => {
            return Some(json!({
                "tenant": tenant_slug,
                "fired": false,
                "dry_run": true,
                "error": "compute_offload_invalid_arguments",
                "message": error,
                "fallback": true
            }))
        }
    };
    Some(json!({
        "tenant": tenant_slug,
        "task_type": input.get("task_type").and_then(Value::as_str).unwrap_or("compute offload"),
        "planned": {
            "affordance_id": COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID,
            "server_id": COMPUTE_OFFLOAD_ENGINE_ID,
            "tool_name": COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID,
            "writeback_policy": "read-only"
        },
        "fired": !input.get("dry_run").and_then(Value::as_bool).unwrap_or(false),
        "dry_run": input.get("dry_run").and_then(Value::as_bool).unwrap_or(false),
        "outcome": {
            "is_error": false,
            "text": "compute-offload plan generated by proxy-resident fallback"
        },
        "offload_plan": plan,
        "fallback": true
    }))
}

fn compact_compute_offload_affordance() -> Value {
    json!({
        "affordance_id": COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID,
        "name": "Compute Offload Route Operation",
        "one_line_description": "Plan CPU, cache, verification, and model executor routing for an operation list.",
        "server_id": COMPUTE_OFFLOAD_ENGINE_ID,
        "tool_name": COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID,
        "family": "compute_offload",
        "writeback_policy": "read-only",
        "score": 1.0,
        "fitness": 1.0
    })
}

fn compute_offload_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "operations": {
                "type": "array",
                "items": { "type": "object" }
            },
            "cost_weights": { "type": "object" },
            "graph_version": { "type": "integer" }
        },
        "required": ["operations"]
    })
}

fn action_tier(input: &Value) -> Option<String> {
    input
        .get("action_tier")
        .or_else(|| input.pointer("/arguments/action_tier"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn human_authorized(input: &Value) -> bool {
    input
        .get("human_authorized")
        .or_else(|| input.pointer("/arguments/human_authorized"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn clean_claims(claims: Vec<VerificationClaim>) -> Vec<VerificationClaim> {
    claims
        .into_iter()
        .filter(|claim| {
            !claim.claim.trim().is_empty()
                && !claim.contradicted_by.trim().is_empty()
                && !claim.basis.trim().is_empty()
        })
        .collect()
}

fn push_sse(out: &mut String, event: &str, data: Value) {
    out.push_str("event: ");
    out.push_str(event);
    out.push('\n');
    out.push_str("data: ");
    out.push_str(&data.to_string());
    out.push_str("\n\n");
}

fn stable_hash_hex(value: &str) -> String {
    format!("{:016x}", stable_hash(value.as_bytes()))
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn default_cascade_quality_floor() -> f64 {
    0.72
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_gateway_meta_tools_without_clobbering_existing_tools() {
        let mut request = json!({
            "tools": [{ "name": "existing", "input_schema": { "type": "object" } }],
            "messages": []
        });
        inject_resident_tools(&mut request);
        let names = request["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"existing"));
        assert!(names.contains(&TOOL_SEARCH));
        assert!(names.contains(&DESCRIBE));
        assert!(names.contains(&INVOKE));
        assert!(names.contains(&DIRECT_COMPUTE_OFFLOAD_ROUTE));
    }

    #[test]
    fn extracts_only_resident_tool_uses() {
        let response = json!({
            "content": [
                { "type": "tool_use", "id": "a", "name": TOOL_SEARCH, "input": { "query": "compute" } },
                { "type": "tool_use", "id": "c", "name": DIRECT_COMPUTE_OFFLOAD_ROUTE, "input": { "operations": [] } },
                { "type": "tool_use", "id": "b", "name": "client_tool", "input": {} }
            ]
        });
        let uses = resident_tool_uses(&response);
        assert_eq!(uses.len(), 2);
        assert_eq!(uses[0].id, "a");
        assert_eq!(uses[1].id, "c");
    }

    #[test]
    fn tier_three_invoke_requires_authorization_before_execution() {
        let tool_use = ResidentToolUse {
            id: "toolu_1".to_string(),
            name: INVOKE.to_string(),
            input: json!({
                "affordance_id": "external.publish",
                "arguments": { "action_tier": "tier_three" }
            }),
        };
        let hold = approval_required_payload(&tool_use).expect("hold");
        assert_eq!(hold["status"], "approval_required");
        assert_eq!(hold["executed"], false);
    }

    #[test]
    fn authorized_tier_three_invoke_is_not_held() {
        let tool_use = ResidentToolUse {
            id: "toolu_1".to_string(),
            name: INVOKE.to_string(),
            input: json!({
                "affordance_id": "external.publish",
                "arguments": { "action_tier": "tier_three", "human_authorized": true }
            }),
        };
        assert!(approval_required_payload(&tool_use).is_none());
    }

    #[test]
    fn fallback_compute_offload_invocation_runs_planner() {
        let result = fallback_tool_result(
            INVOKE,
            &json!({
                "affordance_id": COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID,
                "arguments": {
                    "operations": [{
                        "operation_id": "verify",
                        "kind": "verification_check",
                        "quality_floor": 0.9
                    }]
                }
            }),
            "Travis-Gilbert",
        )
        .expect("fallback");
        assert_eq!(
            result["planned"]["affordance_id"],
            COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID
        );
        assert_eq!(
            result["offload_plan"]["steps"][0]["operation"]["operation_id"],
            "verify"
        );
    }

    #[test]
    fn direct_affordance_tool_maps_back_to_gateway_invoke() {
        let tool_use = ResidentToolUse {
            id: "toolu_direct".to_string(),
            name: DIRECT_COMPUTE_OFFLOAD_ROUTE.to_string(),
            input: json!({
                "operations": [{
                    "operation_id": "verify",
                    "kind": "verification_check"
                }]
            }),
        };
        let (name, args) = gateway_call_for_tool_use(&tool_use, "Travis-Gilbert");
        assert_eq!(name, INVOKE);
        assert_eq!(args["tenant_slug"], "Travis-Gilbert");
        assert_eq!(args["affordance_id"], COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID);
        assert_eq!(args["arguments"]["operations"][0]["operation_id"], "verify");
    }

    #[test]
    fn cascade_requires_behavior_corpus_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("calibration.json");
        std::fs::write(
            &path,
            serde_json::to_string(&CascadeCalibrationFile {
                source: "hand tuned".to_string(),
                samples: vec![CalibrationSample {
                    raw_score: 0.9,
                    observed_success: true,
                    weight: 1.0,
                }],
                quality_floor: 0.7,
            })
            .unwrap(),
        )
        .unwrap();
        let decision = route_with_calibration_file(Some(&path), "small local task", true);
        assert_eq!(decision.selected, CascadeRouteTarget::CalibrationRequired);
    }

    #[test]
    fn cascade_routes_local_when_calibrated_by_behavior_corpus() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("calibration.json");
        std::fs::write(
            &path,
            serde_json::to_string(&CascadeCalibrationFile {
                source: "SPEC-BEHAVIOR-CORPUS.md deliverables 3 and 5".to_string(),
                samples: vec![
                    CalibrationSample {
                        raw_score: 0.2,
                        observed_success: false,
                        weight: 1.0,
                    },
                    CalibrationSample {
                        raw_score: 0.9,
                        observed_success: true,
                        weight: 1.0,
                    },
                ],
                quality_floor: 0.7,
            })
            .unwrap(),
        )
        .unwrap();
        let decision = route_with_calibration_file(Some(&path), "summarize this note", true);
        assert_eq!(decision.selected, CascadeRouteTarget::Local);
    }

    #[test]
    fn verification_advisory_is_non_blocking_context() {
        let claims = claims_from_json_text(
            r#"{"claims":[{"claim":"the graph is acyclic","contradicted_by":"cycle edge c -> a","basis":"fixture graph has a->b->c->a"}]}"#,
        );
        let findings = verification_findings("The graph is acyclic.", &claims);
        assert_eq!(findings.len(), 1);
        let text = advisory_text(&findings);
        assert!(text.contains("not a blocking gate"));
        assert!(text.contains("fixture graph"));
    }
}
