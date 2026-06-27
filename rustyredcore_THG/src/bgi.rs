use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

fn parse_json(text: &str, surface: &str) -> PyResult<Value> {
    serde_json::from_str(text)
        .map_err(|err| PyValueError::new_err(format!("{surface} expected valid JSON: {err}")))
}

fn canonical_json(value: &Value, surface: &str) -> PyResult<String> {
    serde_json::to_string(value)
        .map_err(|err| PyValueError::new_err(format!("{surface} could not serialize JSON: {err}")))
}

fn symbolic_err(surface: &str, err: String) -> PyErr {
    PyValueError::new_err(format!("{surface}: {err}"))
}

fn sha256_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn object_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn item_text(value: &Value) -> String {
    let text = object_field(value, "text");
    if !text.is_empty() {
        return text.to_string();
    }
    object_field(value, "summary").to_string()
}

fn item_key(value: &Value) -> String {
    let channel = object_field(value, "channel");
    let obligation = [
        object_field(value, "obligation_id"),
        object_field(value, "evidence_id"),
        object_field(value, "id"),
    ]
    .into_iter()
    .find(|candidate| !candidate.is_empty())
    .unwrap_or("");
    let semantic = [
        object_field(value, "semantic_hash"),
        object_field(value, "content_hash"),
        object_field(value, "text"),
    ]
    .into_iter()
    .find(|candidate| !candidate.is_empty())
    .unwrap_or("");
    format!("{channel}\u{1f}{obligation}\u{1f}{semantic}")
}

fn item_cost(value: &Value, item_weight: f64) -> f64 {
    let tokens = value
        .as_object()
        .and_then(|object| object.get("tokens"))
        .and_then(Value::as_f64)
        .unwrap_or_else(|| {
            let text = item_text(value);
            if text.is_empty() {
                1.0
            } else {
                text.len() as f64 / 4.0
            }
        });
    tokens + item_weight
}

fn expression_hash(
    expression_id: &str,
    domain: &str,
    items: &[Value],
    metadata: &Value,
) -> PyResult<String> {
    let payload = json!({
        "expression_id": expression_id,
        "domain": domain,
        "items": items,
        "metadata": metadata,
    });
    Ok(sha256_hex(&canonical_json(
        &payload,
        "bgi_expression_hash",
    )?))
}

fn egg_probe() -> Value {
    let expression: Result<egg::RecExpr<egg::SymbolLang>, _> = "(+ context 0)".parse();
    if let Ok(expr) = expression {
        let rules: Vec<egg::Rewrite<egg::SymbolLang, ()>> =
            vec![egg::rewrite!("bgi-add-zero"; "(+ ?a 0)" => "?a")];
        let runner = egg::Runner::default().with_expr(&expr).run(&rules);
        if let Some(root) = runner.roots.first() {
            let extractor = egg::Extractor::new(&runner.egraph, egg::AstSize);
            let (cost, best) = extractor.find_best(*root);
            return json!({"engine": "egg", "probe_cost": cost, "probe_best": best.to_string()});
        }
    }
    json!({"engine": "egg", "probe_error": "unavailable"})
}

#[pyfunction]
pub fn bgi_stable_hash_json(payload_json: &str) -> PyResult<String> {
    theorem_harness_core::bgi::stable_hash_json(payload_json)
        .map_err(|err| symbolic_err("bgi_stable_hash_json", err))
}

#[pyfunction]
pub fn bgi_fact_pack_hash_rows_json(rows_json: &str) -> PyResult<String> {
    theorem_harness_core::bgi::fact_pack_hash_rows_json(rows_json)
        .map_err(|err| symbolic_err("bgi_fact_pack_hash_rows_json", err))
}

#[pyfunction]
pub fn bgi_egraph_receipt_summary_json(receipt_json: &str) -> PyResult<String> {
    theorem_harness_core::bgi::egraph_receipt_summary_json(receipt_json)
        .map_err(|err| symbolic_err("bgi_egraph_receipt_summary_json", err))
}

#[pyfunction]
pub fn bgi_egraph_extract_context_pack_json(payload_json: &str) -> PyResult<String> {
    let payload = parse_json(payload_json, "bgi_egraph_extract_context_pack_json")?;
    let expression_id = object_field(&payload, "expression_id");
    let expression_id = if expression_id.is_empty() {
        "native-context-pack"
    } else {
        expression_id
    };
    let items = payload
        .as_object()
        .and_then(|object| object.get("items"))
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            PyValueError::new_err("bgi_egraph_extract_context_pack_json expected items array")
        })?;
    let cost_config = payload
        .as_object()
        .and_then(|object| object.get("cost_config"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let item_weight = cost_config
        .as_object()
        .and_then(|object| object.get("item_weight"))
        .and_then(Value::as_f64)
        .unwrap_or(0.05);
    let metadata = json!({"native_backend": "rust-egg-context-pack", "egg_probe": egg_probe()});

    let input_hash = expression_hash(expression_id, "context_pack", &items, &json!({}))?;
    let original_cost: f64 = items.iter().map(|item| item_cost(item, item_weight)).sum();
    let mut trace: Vec<Value> = Vec::new();

    let mut current = items.clone();
    let before_drop_hash = expression_hash(expression_id, "context_pack", &current, &json!({}))?;
    let before_drop_cost: f64 = current
        .iter()
        .map(|item| item_cost(item, item_weight))
        .sum();
    current = current
        .into_iter()
        .filter(|item| {
            item.as_object()
                .and_then(|object| object.get("hard_required"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || !item_text(item).trim().is_empty()
        })
        .collect();
    if current.len() != items.len() {
        let after_hash = expression_hash(expression_id, "context_pack", &current, &json!({}))?;
        let after_cost: f64 = current
            .iter()
            .map(|item| item_cost(item, item_weight))
            .sum();
        trace.push(json!({
            "rule_id": "drop_empty_optional",
            "before_hash": before_drop_hash,
            "after_hash": after_hash,
            "reason": "Removed optional empty context items without changing represented obligations.",
            "delta_cost": ((after_cost - before_drop_cost) * 1_000_000.0).round() / 1_000_000.0,
            "data": {"removed_count": items.len() - current.len()},
        }));
    }

    let before_dedupe = current.clone();
    let before_dedupe_hash =
        expression_hash(expression_id, "context_pack", &before_dedupe, &json!({}))?;
    let before_dedupe_cost: f64 = before_dedupe
        .iter()
        .map(|item| item_cost(item, item_weight))
        .sum();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut deduped: Vec<Value> = Vec::new();
    let mut removed_count = 0usize;
    for item in before_dedupe {
        let hard_required = item
            .as_object()
            .and_then(|object| object.get("hard_required"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let key = item_key(&item);
        if !hard_required && seen.contains(&key) {
            removed_count += 1;
            continue;
        }
        seen.insert(key);
        deduped.push(item);
    }
    current = deduped;
    if removed_count > 0 {
        let after_hash = expression_hash(expression_id, "context_pack", &current, &json!({}))?;
        let after_cost: f64 = current
            .iter()
            .map(|item| item_cost(item, item_weight))
            .sum();
        trace.push(json!({
            "rule_id": "dedupe_same_obligation",
            "before_hash": before_dedupe_hash,
            "after_hash": after_hash,
            "reason": "Removed duplicate non-required context items with the same obligation and channel.",
            "delta_cost": ((after_cost - before_dedupe_cost) * 1_000_000.0).round() / 1_000_000.0,
            "data": {"removed_count": removed_count},
        }));
    }

    let output_hash = expression_hash(expression_id, "context_pack", &current, &metadata)?;
    let extracted_cost: f64 = current
        .iter()
        .map(|item| item_cost(item, item_weight))
        .sum();
    let receipt = json!({
        "engine": "egraph-theorem",
        "native_backend": "rust-egg-context-pack",
        "input_hash": input_hash,
        "output_hash": output_hash,
        "domain": "context_pack",
        "equivalent": true,
        "original_cost": (original_cost * 1_000_000.0).round() / 1_000_000.0,
        "extracted_cost": (extracted_cost * 1_000_000.0).round() / 1_000_000.0,
        "rewrite_trace": trace,
        "extraction": {
            "expression_id": expression_id,
            "domain": "context_pack",
            "items": current,
            "metadata": metadata,
            "expression_hash": output_hash,
        },
    });
    canonical_json(&receipt, "bgi_egraph_extract_context_pack_json")
}

#[pyfunction]
pub fn bgi_datalog_receipt_summary_json(receipt_json: &str) -> PyResult<String> {
    theorem_harness_core::bgi::datalog_receipt_summary_json(receipt_json)
        .map_err(|err| symbolic_err("bgi_datalog_receipt_summary_json", err))
}

#[pyfunction]
pub fn bgi_datalog_verified_rule_ids_json() -> PyResult<String> {
    canonical_json(
        &json!(rustyred_thg_core::DATALOG_RULE_IDS),
        "bgi_datalog_verified_rule_ids_json",
    )
}

#[pyfunction]
pub fn bgi_datalog_derive_core_json(facts_json: &str) -> PyResult<String> {
    let receipt = rustyred_thg_core::derive_datalog_receipt_from_json(facts_json)
        .map_err(|err| symbolic_err("bgi_datalog_derive_core_json", err))?;
    canonical_json(&receipt, "bgi_datalog_derive_core_json")
}

#[pyfunction]
pub fn bgi_probabilistic_source_reliability_json(payload_json: &str) -> PyResult<String> {
    let receipt = rustyred_thg_core::probabilistic_source_reliability_from_json(payload_json)
        .map_err(|err| symbolic_err("bgi_probabilistic_source_reliability_json", err))?;
    canonical_json(&receipt, "bgi_probabilistic_source_reliability_json")
}

#[pyfunction]
pub fn bgi_probabilistic_expected_value_json(payload_json: &str) -> PyResult<String> {
    let receipt = rustyred_thg_core::probabilistic_expected_value_from_json(payload_json)
        .map_err(|err| symbolic_err("bgi_probabilistic_expected_value_json", err))?;
    canonical_json(&receipt, "bgi_probabilistic_expected_value_json")
}

#[pyfunction]
pub fn bgi_evolution_archive_json(payload_json: &str) -> PyResult<String> {
    let receipt = rustyred_thg_core::evolution_archive_from_json(payload_json)
        .map_err(|err| symbolic_err("bgi_evolution_archive_json", err))?;
    canonical_json(&receipt, "bgi_evolution_archive_json")
}

#[pyfunction]
pub fn bgi_compact_receipts_json(receipts_json: &str) -> PyResult<String> {
    theorem_harness_core::bgi::compact_receipts_json(receipts_json)
        .map_err(|err| symbolic_err("bgi_compact_receipts_json", err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgi_hash_wrappers_delegate_to_core_contracts() {
        let payload = r#"{"b":2,"a":1}"#;
        assert_eq!(
            bgi_stable_hash_json(payload).unwrap(),
            theorem_harness_core::bgi::stable_hash_json(payload).unwrap()
        );

        let rows = r#"[
            {"source_artifact_id":"artifact-2","view_type":"claim","view_hash":"h2"},
            {"source_artifact_id":"artifact-1","view_type":"text","view_hash":"h1"}
        ]"#;
        assert_eq!(
            bgi_fact_pack_hash_rows_json(rows).unwrap(),
            theorem_harness_core::bgi::fact_pack_hash_rows_json(rows).unwrap()
        );

        let egraph_receipt = r#"{
            "domain":"context_pack",
            "engine":"egraph-theorem",
            "equivalent":true,
            "extracted_cost":1.25,
            "input_hash":"in",
            "native_backend":"rust-egg-context-pack",
            "original_cost":2.5,
            "output_hash":"out",
            "rewrite_trace":[{"rule_id":"drop_empty_optional"}]
        }"#;
        assert_eq!(
            bgi_egraph_receipt_summary_json(egraph_receipt).unwrap(),
            theorem_harness_core::bgi::egraph_receipt_summary_json(egraph_receipt).unwrap()
        );

        let datalog_receipt = r#"{
            "derived_count":3,
            "engine":"python-reference-datalog",
            "fact_pack_hash":"pack",
            "rule_ids":["unsupported_claim"],
            "warnings":["w"],
            "writeback_policy":"proposal_only"
        }"#;
        assert_eq!(
            bgi_datalog_receipt_summary_json(datalog_receipt).unwrap(),
            theorem_harness_core::bgi::datalog_receipt_summary_json(datalog_receipt).unwrap()
        );

        let receipts = r#"[
            {"status":"accepted","payload_hash":"hash-b"},
            {"status":"rejected","receipt_hash":"hash-a"},
            {"status":"accepted","output_hash":"hash-c"}
        ]"#;
        assert_eq!(
            bgi_compact_receipts_json(receipts).unwrap(),
            theorem_harness_core::bgi::compact_receipts_json(receipts).unwrap()
        );
    }
}
