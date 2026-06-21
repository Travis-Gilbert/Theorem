use crate::skill_pack::{get_skill_pack, SkillPackGetInput, SkillPackGraphStore};
use prose_check::{check, pack_hash, Register, StyleReceipt, PACK_ID};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use theorem_harness_core::{BindingTransitionInput, Payload, RunState, TransitionInput};

pub const STYLE_RECEIPTS_FIELD: &str = "style_receipts";
pub const STYLE_FITNESS_FIELD: &str = "writing_engineering_fitness";
pub const WRITING_ENGINEERING_STATUS_FIELD: &str = "writing_engineering_status";

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

pub fn prepare_run_transition<S: SkillPackGraphStore>(
    store: &mut S,
    state: Option<&RunState>,
    mut transition: TransitionInput,
) -> TransitionInput {
    if transition.event_type == "RUN.CREATED" {
        stamp_run_created_status(store, &mut transition.payload);
    } else if run_boundary_text(&transition.event_type, &transition.payload).is_some() {
        carry_status_from_run_scope(state, &mut transition.payload);
    }
    transition
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

fn stamp_run_created_status<S: SkillPackGraphStore>(store: &mut S, payload: &mut Payload) {
    let scope = payload
        .entry("scope".to_string())
        .or_insert_with(|| json!({}));
    if !scope.is_object() {
        *scope = json!({});
    }
    let Some(scope) = scope.as_object_mut() else {
        return;
    };
    let tenant = first_text(
        scope,
        &[
            "tenant_slug",
            "tenantSlug",
            "tenant",
            "tenant_id",
            "tenantId",
        ],
    )
    .unwrap_or_else(|| "default".to_string());
    // Resolve the published prose pack for this tenant. The engineering corpus is
    // published advisory but nothing seeds it at server boot, so a fresh tenant
    // first resolves nothing and enforcement silently degrades to shadow. Lazily
    // publish the corpus (idempotent by content hash) for this tenant and retry
    // before falling back -- this brings every tenant up on its first RUN.CREATED.
    let mut resolved = get_skill_pack(
        store,
        SkillPackGetInput {
            tenant_slug: tenant.clone(),
            pack_id: PACK_ID.to_string(),
            pack_content_hash: pack_hash(),
        },
    );
    if resolved.is_err() {
        let _ = crate::engineering_packs::publish_engineering_packs(store, &tenant, "system");
        resolved = get_skill_pack(
            store,
            SkillPackGetInput {
                tenant_slug: tenant.clone(),
                pack_id: PACK_ID.to_string(),
                pack_content_hash: pack_hash(),
            },
        );
    }
    match resolved {
        Ok(pack) => {
            scope.insert(
                WRITING_ENGINEERING_STATUS_FIELD.to_string(),
                Value::String(normalize_pack_status(&pack.status)),
            );
            scope.insert(
                "writing_engineering_status_source".to_string(),
                Value::String("registry".to_string()),
            );
            scope.insert(
                "writing_engineering_pack_id".to_string(),
                Value::String(pack.pack_id),
            );
            scope.insert(
                "writing_engineering_pack_hash".to_string(),
                Value::String(pack.pack_content_hash),
            );
            scope.insert(
                "writing_engineering_origin_tenant".to_string(),
                Value::String(if pack.origin_tenant_slug.trim().is_empty() {
                    pack.tenant_slug
                } else {
                    pack.origin_tenant_slug
                }),
            );
        }
        Err(error) => {
            scope.insert(
                WRITING_ENGINEERING_STATUS_FIELD.to_string(),
                Value::String("shadow".to_string()),
            );
            scope.insert(
                "writing_engineering_status_source".to_string(),
                Value::String("fallback".to_string()),
            );
            scope.insert(
                "writing_engineering_pack_id".to_string(),
                Value::String(PACK_ID.to_string()),
            );
            scope.insert(
                "writing_engineering_pack_hash".to_string(),
                Value::String(pack_hash()),
            );
            scope.insert(
                "writing_engineering_status_error".to_string(),
                Value::String(error.to_string()),
            );
        }
    }
}

fn carry_status_from_run_scope(state: Option<&RunState>, payload: &mut Payload) {
    let Some(state) = state else {
        return;
    };
    for key in [
        WRITING_ENGINEERING_STATUS_FIELD,
        "writing_engineering_status_source",
        "writing_engineering_pack_id",
        "writing_engineering_pack_hash",
        "writing_engineering_origin_tenant",
        "writing_engineering_status_error",
    ] {
        if payload.contains_key(key) {
            continue;
        }
        if let Some(value) = state.scope.get(key) {
            payload.insert(key.to_string(), value.clone());
        }
    }
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

pub fn summarize_style_receipts_for_fitness(receipts: &[Value]) -> WritingStyleFitnessSummary {
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
    fn run_created_lazily_brings_up_corpus_and_resolves_advisory() {
        // A fresh tenant store has no published packs. RUN.CREATED must lazily
        // publish the engineering corpus and resolve the prose pack to advisory
        // from the registry, instead of falling back to shadow.
        let mut store = InMemoryGraphStore::new();
        let transition = TransitionInput::new(
            "RUN.CREATED",
            json!({ "scope": { "tenant_slug": "acme" } })
                .as_object()
                .unwrap()
                .clone(),
        );
        let prepared = prepare_run_transition(&mut store, None, transition);
        let scope = prepared.payload["scope"].as_object().unwrap();
        assert_eq!(
            scope["writing_engineering_status"],
            json!("advisory"),
            "fresh tenant should resolve advisory after lazy bring-up"
        );
        assert_eq!(scope["writing_engineering_status_source"], json!("registry"));
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
