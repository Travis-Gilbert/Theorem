//! Pure BGI JSON/hash contracts shared by native callers and PyO3 wrappers.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn parse_json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|err| format!("expected valid JSON: {err}"))
}

fn canonical_json(value: &Value) -> Result<String, String> {
    serde_json::to_string(value).map_err(|err| format!("could not serialize JSON: {err}"))
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

fn array_len(value: Option<&Value>) -> usize {
    value.and_then(Value::as_array).map_or(0, Vec::len)
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn stable_hash_json(payload_json: &str) -> Result<String, String> {
    let payload = parse_json(payload_json)?;
    Ok(sha256_hex(&canonical_json(&payload)?))
}

pub fn fact_pack_hash_rows_json(rows_json: &str) -> Result<String, String> {
    let payload = parse_json(rows_json)?;
    let mut rows = payload
        .as_array()
        .cloned()
        .ok_or_else(|| "expected a JSON array".to_string())?;
    rows.sort_by(|left, right| {
        (
            object_field(left, "source_artifact_id"),
            object_field(left, "view_type"),
            object_field(left, "view_hash"),
        )
            .cmp(&(
                object_field(right, "source_artifact_id"),
                object_field(right, "view_type"),
                object_field(right, "view_hash"),
            ))
    });
    Ok(sha256_hex(&canonical_json(&Value::Array(rows))?))
}

pub fn egraph_receipt_summary_json(receipt_json: &str) -> Result<String, String> {
    let receipt = parse_json(receipt_json)?;
    let summary = json!({
        "domain": object_field(&receipt, "domain"),
        "engine": object_field(&receipt, "engine"),
        "equivalent": receipt.get("equivalent").and_then(Value::as_bool).unwrap_or(false),
        "extracted_cost": receipt.get("extracted_cost").and_then(Value::as_f64).unwrap_or(0.0),
        "input_hash": object_field(&receipt, "input_hash"),
        "native_backend": object_field(&receipt, "native_backend"),
        "original_cost": receipt.get("original_cost").and_then(Value::as_f64).unwrap_or(0.0),
        "output_hash": object_field(&receipt, "output_hash"),
        "rewrite_count": array_len(receipt.get("rewrite_trace")),
    });
    canonical_json(&summary)
}

pub fn datalog_receipt_summary_json(receipt_json: &str) -> Result<String, String> {
    let receipt = parse_json(receipt_json)?;
    let summary = json!({
        "derived_count": receipt.get("derived_count").and_then(Value::as_u64).unwrap_or(0),
        "engine": object_field(&receipt, "engine"),
        "fact_pack_hash": object_field(&receipt, "fact_pack_hash"),
        "rule_ids": string_array(receipt.get("rule_ids")),
        "warning_count": array_len(receipt.get("warnings")),
        "writeback_policy": object_field(&receipt, "writeback_policy"),
    });
    canonical_json(&summary)
}

pub fn compact_receipts_json(receipts_json: &str) -> Result<String, String> {
    let payload = parse_json(receipts_json)?;
    let receipts = payload
        .as_array()
        .cloned()
        .ok_or_else(|| "expected a JSON array".to_string())?;
    let mut status_counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut receipt_hashes: Vec<String> = Vec::new();

    for receipt in &receipts {
        let status = object_field(receipt, "status");
        if !status.is_empty() {
            *status_counts.entry(status.to_string()).or_insert(0) += 1;
        }
        for key in [
            "receipt_hash",
            "payload_hash",
            "formula_hash",
            "input_hash",
            "output_hash",
            "fact_pack_hash",
        ] {
            let value = object_field(receipt, key);
            if !value.is_empty() {
                receipt_hashes.push(value.to_string());
                break;
            }
        }
    }
    receipt_hashes.sort();
    receipt_hashes.dedup();

    let canonical_payload = canonical_json(&Value::Array(receipts))?;
    let status_value: Map<String, Value> = status_counts
        .into_iter()
        .map(|(key, value)| (key, Value::from(value)))
        .collect();
    let summary = json!({
        "count": payload.as_array().map_or(0, Vec::len),
        "payload_hash": sha256_hex(&canonical_payload),
        "receipt_hashes": receipt_hashes,
        "status_counts": Value::Object(status_value),
    });
    canonical_json(&summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_hash_json_matches_python_reference_style() {
        assert_eq!(
            stable_hash_json(r#"{"b":2,"a":1}"#).unwrap(),
            "43258cff783fe7036d8a43033f830adfc60ec037382473548ac742b888292777"
        );
    }

    #[test]
    fn fact_pack_hash_rows_sorts_before_hashing() {
        let rows = r#"[
            {"source_artifact_id":"artifact-2","view_type":"claim","view_hash":"h2"},
            {"source_artifact_id":"artifact-1","view_type":"text","view_hash":"h1"}
        ]"#;
        assert_eq!(
            fact_pack_hash_rows_json(rows).unwrap(),
            "335424fed5c7977efa5a9e6d68ade0cbd510de7c544039177321cc0c8ebbdc44"
        );
    }

    #[test]
    fn receipt_summaries_keep_the_contract_fields_only() {
        let egraph = egraph_receipt_summary_json(
            r#"{
                "domain":"context_pack",
                "engine":"egraph-theorem",
                "equivalent":true,
                "extracted_cost":1.25,
                "input_hash":"in",
                "native_backend":"rust-egg-context-pack",
                "original_cost":2.5,
                "output_hash":"out",
                "rewrite_trace":[{"rule_id":"drop_empty_optional"}],
                "ignored":"field"
            }"#,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&egraph).unwrap(),
            json!({
                "domain": "context_pack",
                "engine": "egraph-theorem",
                "equivalent": true,
                "extracted_cost": 1.25,
                "input_hash": "in",
                "native_backend": "rust-egg-context-pack",
                "original_cost": 2.5,
                "output_hash": "out",
                "rewrite_count": 1,
            })
        );

        let datalog = datalog_receipt_summary_json(
            r#"{
                "derived_count":3,
                "engine":"python-reference-datalog",
                "fact_pack_hash":"pack",
                "rule_ids":["unsupported_claim"],
                "warnings":["w"],
                "writeback_policy":"proposal_only"
            }"#,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&datalog).unwrap(),
            json!({
                "derived_count": 3,
                "engine": "python-reference-datalog",
                "fact_pack_hash": "pack",
                "rule_ids": ["unsupported_claim"],
                "warning_count": 1,
                "writeback_policy": "proposal_only",
            })
        );
    }

    #[test]
    fn compact_receipts_hashes_payload_and_counts_statuses() {
        let summary = compact_receipts_json(
            r#"[
                {"status":"accepted","payload_hash":"hash-b"},
                {"status":"rejected","receipt_hash":"hash-a"},
                {"status":"accepted","output_hash":"hash-c"}
            ]"#,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&summary).unwrap(),
            json!({
                "count": 3,
                "payload_hash": "a5f7e712f8baad04f72692918c60754678a849a20a93244a6ff9e3f05079170f",
                "receipt_hashes": ["hash-a", "hash-b", "hash-c"],
                "status_counts": {"accepted": 2, "rejected": 1},
            })
        );
    }
}
