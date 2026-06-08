use crate::skill_pack::{skill_pack_node_id, SkillPackState};
use crate::{HarnessRuntimeError, RuntimeResult};
use prose_check::{check, pack_hash, Register, StyleReceipt, PACK_ID};
use rustyred_thg_core::GraphStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use theorem_harness_core::{BindingTransitionInput, Payload, TransitionInput};

pub const STYLE_RECEIPTS_FIELD: &str = "style_receipts";
pub const STYLE_FITNESS_FIELD: &str = "writing_engineering_fitness";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoundaryStyleReceipt {
    pub boundary: String,
    pub pack_id: String,
    pub pack_status: String,
    pub action: String,
    pub hard_axis_failed: bool,
    pub receipt: StyleReceipt,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct WritingStyleFitnessSummary {
    pub pack_id: String,
    pub pack_hash: String,
    pub style_receipt_count: u64,
    pub fidelity_failures: u64,
    pub hard_axis_failures: u64,
    pub em_dash_failures: u64,
    pub total_reduction: f32,
    pub average_reduction: f32,
    pub last_register: String,
    pub last_hard_axis_failed: bool,
}

pub fn enrich_run_transition(mut transition: TransitionInput) -> TransitionInput {
    if let Some((boundary, text)) = run_boundary_text(&transition.event_type, &transition.payload) {
        let identifiers = source_identifiers_from_payload(&transition.payload);
        let pack_status = pack_status_from_payload(&transition.payload);
        push_style_receipt(
            &mut transition.payload,
            &boundary,
            &text,
            &identifiers,
            pack_status,
        );
    }
    transition
}

pub fn enrich_binding_transition(mut transition: BindingTransitionInput) -> BindingTransitionInput {
    if let Some((boundary, text)) =
        binding_boundary_text(&transition.event_type, &transition.payload)
    {
        let identifiers = source_identifiers_from_payload(&transition.payload);
        let pack_status = pack_status_from_payload(&transition.payload);
        push_style_receipt(
            &mut transition.payload,
            &boundary,
            &text,
            &identifiers,
            pack_status,
        );
    }
    transition
}

pub fn metadata_with_style_receipt(
    mut metadata: Map<String, Value>,
    boundary: &str,
    text: &str,
    source_identifiers: &[String],
) -> Map<String, Value> {
    let pack_status = pack_status_from_payload(&metadata);
    push_style_receipt(
        &mut metadata,
        boundary,
        text,
        source_identifiers,
        pack_status,
    );
    metadata
}

pub fn check_boundary_text(
    boundary: &str,
    text: &str,
    source_identifiers: &[String],
    pack_status: &str,
) -> BoundaryStyleReceipt {
    let receipt = check(text, register_for_boundary(boundary), source_identifiers);
    let hard_axis_failed = !receipt.fidelity.preserved || receipt.em_dash_count > 0;
    BoundaryStyleReceipt {
        boundary: boundary.to_string(),
        pack_id: PACK_ID.to_string(),
        pack_status: normalize_pack_status(pack_status),
        action: style_action(pack_status, hard_axis_failed).to_string(),
        hard_axis_failed,
        receipt,
    }
}

pub fn register_for_boundary(boundary: &str) -> Register {
    match boundary {
        "coordinate"
        | "coordination_intent"
        | "coordination_reflection"
        | "coordination_record"
        | "handoff_summary"
        | "mention" => Register::Wire,
        "spare" => Register::Spare,
        _ => Register::Plain,
    }
}

pub fn fold_style_receipts_into_pack_fitness<S: GraphStore>(
    store: &mut S,
    tenant_slug: &str,
    payload: &Payload,
) -> RuntimeResult<Option<WritingStyleFitnessSummary>> {
    let Some(receipts) = payload.get(STYLE_RECEIPTS_FIELD).and_then(Value::as_array) else {
        return Ok(None);
    };
    let current_summary = summarize_receipts(receipts);
    let Some(mut node) = store
        .get_node(&skill_pack_node_id(tenant_slug, &pack_hash()))
        .cloned()
    else {
        return Ok(Some(current_summary));
    };
    let mut state: SkillPackState = serde_json::from_value(node.properties.clone())
        .map_err(|error| HarnessRuntimeError::Deserialization(error.to_string()))?;
    let previous = state
        .metadata
        .get("fitness")
        .and_then(|fitness| fitness.get("writing_engineering"))
        .cloned()
        .and_then(|value| serde_json::from_value::<WritingStyleFitnessSummary>(value).ok())
        .unwrap_or_else(|| WritingStyleFitnessSummary {
            pack_id: PACK_ID.to_string(),
            pack_hash: pack_hash(),
            ..WritingStyleFitnessSummary::default()
        });
    let merged = merge_fitness(previous, current_summary);
    state.status = next_status(&state.status, &state.metadata, &merged);
    state.metadata.insert(
        "fitness".to_string(),
        merged_fitness_metadata(&state.metadata, &merged),
    );
    if merged.last_hard_axis_failed && state.status == "advisory" {
        state.metadata.insert(
            "last_tension".to_string(),
            json!({
                "kind": "writing_engineering_fidelity_regression",
                "pack_id": PACK_ID,
                "pack_hash": pack_hash(),
                "message": "A hard writing axis failed; canonical packs demote to advisory."
            }),
        );
    }
    node.properties = serde_json::to_value(&state)
        .map_err(|error| HarnessRuntimeError::Serialization(error.to_string()))?;
    store.upsert_node(node)?;
    Ok(Some(merged))
}

fn push_style_receipt(
    payload: &mut Map<String, Value>,
    boundary: &str,
    text: &str,
    source_identifiers: &[String],
    pack_status: String,
) {
    if text.trim().is_empty() {
        return;
    }
    let receipt = check_boundary_text(boundary, text, source_identifiers, &pack_status);
    let value = serde_json::to_value(receipt).expect("style receipt serializes");
    match payload.get_mut(STYLE_RECEIPTS_FIELD) {
        Some(Value::Array(receipts)) => receipts.push(value),
        _ => {
            payload.insert(STYLE_RECEIPTS_FIELD.to_string(), Value::Array(vec![value]));
        }
    }
}

fn run_boundary_text(event_type: &str, payload: &Payload) -> Option<(String, String)> {
    match event_type {
        "RUN.CLOSED" => text_path(payload, "summary").map(|text| ("report".to_string(), text)),
        "SESSION.EVENT_RECORDED" => {
            let subtype = text_path(payload, "event_subtype").unwrap_or_default();
            let boundary = if subtype.contains("coordinate") {
                "coordinate"
            } else if subtype.contains("synthesis") {
                "synthesis"
            } else if subtype.contains("report") {
                "report"
            } else {
                return None;
            };
            first_text(payload, &["content", "message", "summary"])
                .map(|text| (boundary.to_string(), text))
        }
        _ => None,
    }
}

fn binding_boundary_text(event_type: &str, payload: &Payload) -> Option<(String, String)> {
    match event_type {
        "DRAFTS.SYNTHESIZED" => first_text(
            payload,
            &[
                "synthesis_text",
                "draft",
                "content",
                "summary",
                "synthesis_id",
            ],
        )
        .map(|text| ("synthesis".to_string(), text)),
        "RUN.CLOSED" => text_path(payload, "summary").map(|text| ("report".to_string(), text)),
        _ => None,
    }
}

fn source_identifiers_from_payload(payload: &Payload) -> Vec<String> {
    payload
        .get("source_identifiers")
        .or_else(|| payload.get("sourceIdentifiers"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn pack_status_from_payload(payload: &Payload) -> String {
    first_text(
        payload,
        &[
            "writing_engineering_status",
            "pack_status",
            "metadata.writing_engineering_status",
            "metadata.pack_status",
        ],
    )
    .unwrap_or_else(|| "shadow".to_string())
}

fn normalize_pack_status(status: &str) -> String {
    match status.trim().to_ascii_lowercase().as_str() {
        "shadow" | "advisory" | "validated" | "canonical" => status.trim().to_ascii_lowercase(),
        _ => "shadow".to_string(),
    }
}

fn style_action(status: &str, hard_axis_failed: bool) -> &'static str {
    match (normalize_pack_status(status).as_str(), hard_axis_failed) {
        ("shadow", _) => "receipt_only",
        ("advisory", true) => "advisory_context",
        ("advisory", false) => "receipt_only",
        ("validated" | "canonical", true) => "revision_required",
        ("validated" | "canonical", false) => "emit",
        _ => "receipt_only",
    }
}

fn summarize_receipts(receipts: &[Value]) -> WritingStyleFitnessSummary {
    let mut summary = WritingStyleFitnessSummary {
        pack_id: PACK_ID.to_string(),
        pack_hash: pack_hash(),
        style_receipt_count: receipts.len() as u64,
        ..WritingStyleFitnessSummary::default()
    };
    for receipt in receipts {
        let hard_axis_failed = receipt
            .get("hard_axis_failed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let style = receipt.get("receipt").unwrap_or(receipt);
        let fidelity_failed = style
            .get("fidelity")
            .and_then(|value| value.get("preserved"))
            .and_then(Value::as_bool)
            .map(|preserved| !preserved)
            .unwrap_or(false);
        let em_dash_failed = style
            .get("em_dash_count")
            .and_then(Value::as_u64)
            .map(|count| count > 0)
            .unwrap_or(false);
        let reduction = style
            .get("reduction")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as f32;
        summary.fidelity_failures += u64::from(fidelity_failed);
        summary.em_dash_failures += u64::from(em_dash_failed);
        summary.hard_axis_failures +=
            u64::from(hard_axis_failed || fidelity_failed || em_dash_failed);
        summary.total_reduction += reduction;
        summary.last_hard_axis_failed = hard_axis_failed || fidelity_failed || em_dash_failed;
        summary.last_register = style
            .get("register")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    if summary.style_receipt_count > 0 {
        summary.average_reduction = summary.total_reduction / summary.style_receipt_count as f32;
    }
    summary
}

fn merge_fitness(
    mut previous: WritingStyleFitnessSummary,
    current: WritingStyleFitnessSummary,
) -> WritingStyleFitnessSummary {
    previous.pack_id = PACK_ID.to_string();
    previous.pack_hash = pack_hash();
    previous.style_receipt_count += current.style_receipt_count;
    previous.fidelity_failures += current.fidelity_failures;
    previous.hard_axis_failures += current.hard_axis_failures;
    previous.em_dash_failures += current.em_dash_failures;
    previous.total_reduction += current.total_reduction;
    previous.last_register = current.last_register;
    previous.last_hard_axis_failed = current.last_hard_axis_failed;
    if previous.style_receipt_count > 0 {
        previous.average_reduction = previous.total_reduction / previous.style_receipt_count as f32;
    }
    previous
}

fn next_status(
    current_status: &str,
    metadata: &Map<String, Value>,
    fitness: &WritingStyleFitnessSummary,
) -> String {
    if current_status == "canonical" && fitness.last_hard_axis_failed {
        return "advisory".to_string();
    }
    if current_status == "shadow"
        && metadata
            .get("benchmark_gate_passed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return "advisory".to_string();
    }
    if current_status == "advisory"
        && fitness.style_receipt_count >= 25
        && fitness.fidelity_failures == 0
    {
        return "validated".to_string();
    }
    current_status.to_string()
}

fn merged_fitness_metadata(
    metadata: &Map<String, Value>,
    summary: &WritingStyleFitnessSummary,
) -> Value {
    let mut fitness = metadata
        .get("fitness")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    fitness.insert(
        "writing_engineering".to_string(),
        serde_json::to_value(summary).expect("fitness summary serializes"),
    );
    Value::Object(fitness)
}

fn first_text(payload: &Payload, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| text_path(payload, path))
}

fn text_path(payload: &Payload, path: &str) -> Option<String> {
    let mut current = None;
    for (index, segment) in path.split('.').enumerate() {
        let segment = segment.trim();
        if segment.is_empty() {
            return None;
        }
        current = if index == 0 {
            payload.get(segment)
        } else {
            current.and_then(|value| value.get(segment))
        };
    }
    current
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill_pack::{publish_skill_pack, SkillPackPublishInput};
    use prose_check::writing_engineering_pack_payload;
    use rustyred_thg_core::InMemoryGraphStore;
    use serde_json::json;

    #[test]
    fn audience_routing_uses_wire_for_coordinate_and_plain_for_report() {
        let coordinate =
            check_boundary_text("coordinate", "Patch done. Tests pass.", &[], "shadow");
        let report = check_boundary_text("report", "Patch done. Tests pass.", &[], "shadow");

        assert_eq!(coordinate.receipt.register, Register::Wire);
        assert_eq!(report.receipt.register, Register::Plain);
    }

    #[test]
    fn run_close_payload_receives_report_style_receipt() {
        let transition = TransitionInput::new(
            "RUN.CLOSED",
            json!({ "summary": "Patch done. Tests pass.", "closed_by": "codex" })
                .as_object()
                .unwrap()
                .clone(),
        );
        let enriched = enrich_run_transition(transition);
        let receipts = enriched.payload[STYLE_RECEIPTS_FIELD].as_array().unwrap();

        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0]["boundary"], json!("report"));
        assert_eq!(receipts[0]["receipt"]["register"], json!("Plain"));
    }

    #[test]
    fn canonical_fidelity_failure_demotes_pack_and_records_negative_signal() {
        let mut store = InMemoryGraphStore::new();
        let mut pack = writing_engineering_pack_payload(None);
        pack["metadata"]["status"] = json!("canonical");
        publish_skill_pack(
            &mut store,
            SkillPackPublishInput {
                tenant_slug: "default".to_string(),
                pack_content_hash: pack_hash(),
                status: "canonical".to_string(),
                pack,
                ..SkillPackPublishInput::default()
            },
        )
        .unwrap();
        let transition = enrich_run_transition(TransitionInput::new(
            "RUN.CLOSED",
            json!({
                "summary": "The runtime module changed.",
                "closed_by": "codex",
                "source_identifiers": ["rustyred-web/src/lib.rs"],
                "writing_engineering_status": "canonical"
            })
            .as_object()
            .unwrap()
            .clone(),
        ));

        let summary =
            fold_style_receipts_into_pack_fitness(&mut store, "default", &transition.payload)
                .unwrap()
                .unwrap();
        assert_eq!(summary.style_receipt_count, 1);
        assert_eq!(summary.fidelity_failures, 1);

        let node = store
            .get_node(&skill_pack_node_id("default", &pack_hash()))
            .unwrap();
        let state: SkillPackState = serde_json::from_value(node.properties.clone()).unwrap();
        assert_eq!(state.status, "advisory");
        assert!(state.metadata.get("last_tension").is_some());
    }

    #[test]
    fn three_run_closes_aggregate_pack_fitness_counts() {
        let mut store = InMemoryGraphStore::new();
        publish_skill_pack(
            &mut store,
            SkillPackPublishInput {
                tenant_slug: "default".to_string(),
                pack_content_hash: pack_hash(),
                status: "shadow".to_string(),
                pack: writing_engineering_pack_payload(None),
                ..SkillPackPublishInput::default()
            },
        )
        .unwrap();

        for index in 0..3 {
            let transition = enrich_run_transition(TransitionInput::new(
                "RUN.CLOSED",
                json!({
                    "summary": format!("Run {index} closed with receipts."),
                    "closed_by": "codex"
                })
                .as_object()
                .unwrap()
                .clone(),
            ));
            fold_style_receipts_into_pack_fitness(&mut store, "default", &transition.payload)
                .unwrap();
        }

        let node = store
            .get_node(&skill_pack_node_id("default", &pack_hash()))
            .unwrap();
        let state: SkillPackState = serde_json::from_value(node.properties.clone()).unwrap();
        assert_eq!(
            state.metadata["fitness"]["writing_engineering"]["style_receipt_count"],
            json!(3)
        );
        assert_eq!(
            state.metadata["fitness"]["writing_engineering"]["fidelity_failures"],
            json!(0)
        );
    }

    #[test]
    fn destructive_operation_receipt_records_clarity_break() {
        let warning = "Warning: DROP TABLE users is irreversible. Confirm backup, tenant, and rollback before running.";
        let receipt = check_boundary_text("report", warning, &[], "shadow");

        assert!(!receipt.receipt.clarity_breaks.is_empty());
        assert!(receipt.receipt.clutter_hits.is_empty());
        assert_eq!(receipt.receipt.em_dash_count, 0);
    }

    #[test]
    fn fenced_code_and_commit_message_are_passthrough_spans() {
        let code = "```rust\nfn main() {\n    println!(\"ok\");\n}\n```";
        let commit = "fix(runtime): record style receipts";
        let text = format!("Report.\n\n{code}\n\nCommit message:\n{commit}\n\nDone.");
        let receipt = check_boundary_text("report", &text, &[], "shadow");
        let spans = receipt.receipt.code_spans;

        assert_eq!(spans.len(), 2);
        assert_eq!(&text[spans[0].start as usize..spans[0].end as usize], code);
        assert_eq!(
            &text[spans[1].start as usize..spans[1].end as usize],
            commit
        );
    }
}
