use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const PAIRFORMER_MODES: &[&str] = &["off", "gate", "full"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionMetricsState {
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_tool_calls: i64,
    pub task_completion: bool,
    pub pairformer_mode: String,
    pub task_category: String,
    pub workstream_id: String,
    pub session_id: String,
    pub total_tokens: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PairformerSummaryRow {
    pub task_category: String,
    pub pairformer_mode: String,
    pub completed_sessions: usize,
    pub mean_tokens_per_completed_task: f64,
    pub median_tokens_per_completed_task: f64,
    pub mean_tool_calls: f64,
}

impl SessionMetricsState {
    pub fn from_value(value: &Value) -> Self {
        let empty = Map::new();
        let data = value.as_object().unwrap_or(&empty);
        let pairformer_mode =
            normalize_pairformer_mode(string_field(data, "pairformer_mode", "off"));
        let total_input_tokens = non_negative_i64(data.get("total_input_tokens"));
        let total_output_tokens = non_negative_i64(data.get("total_output_tokens"));
        Self {
            total_input_tokens,
            total_output_tokens,
            total_tool_calls: non_negative_i64(data.get("total_tool_calls")),
            task_completion: data
                .get("task_completion")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            pairformer_mode,
            task_category: string_field(data, "task_category", "unknown"),
            workstream_id: string_field(data, "workstream_id", ""),
            session_id: string_field(data, "session_id", ""),
            total_tokens: total_input_tokens + total_output_tokens,
        }
    }

    pub fn from_json_str(line: &str) -> Result<Self, serde_json::Error> {
        let value = serde_json::from_str::<Value>(line)?;
        Ok(Self::from_value(&value))
    }
}

pub fn load_jsonl_metrics<I, S>(lines: I) -> Result<Vec<SessionMetricsState>, serde_json::Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut metrics = Vec::new();
    for line in lines {
        let line = line.as_ref();
        if line.trim().is_empty() {
            continue;
        }
        metrics.push(SessionMetricsState::from_json_str(line)?);
    }
    Ok(metrics)
}

pub fn summarize_pairformer_ab(metrics: &[SessionMetricsState]) -> Vec<PairformerSummaryRow> {
    let mut buckets: BTreeMap<(String, String), Vec<&SessionMetricsState>> = BTreeMap::new();
    for item in metrics.iter().filter(|item| item.task_completion) {
        buckets
            .entry((item.task_category.clone(), item.pairformer_mode.clone()))
            .or_default()
            .push(item);
    }

    buckets
        .into_iter()
        .map(|((category, mode), items)| {
            let totals = items
                .iter()
                .map(|item| item.total_tokens)
                .collect::<Vec<_>>();
            PairformerSummaryRow {
                task_category: category,
                pairformer_mode: mode,
                completed_sessions: items.len(),
                mean_tokens_per_completed_task: round2(mean_i64(&totals)),
                median_tokens_per_completed_task: median_i64(&totals),
                mean_tool_calls: round2(
                    items
                        .iter()
                        .map(|item| item.total_tool_calls as f64)
                        .sum::<f64>()
                        / items.len() as f64,
                ),
            }
        })
        .collect()
}

pub fn compare_modes(
    metrics: &[SessionMetricsState],
    baseline: Option<&str>,
    candidate: Option<&str>,
) -> Value {
    let baseline = baseline.unwrap_or("off");
    let candidate = candidate.unwrap_or("full");
    let completed = metrics
        .iter()
        .filter(|item| item.task_completion)
        .collect::<Vec<_>>();
    let base = completed
        .iter()
        .filter(|item| item.pairformer_mode == baseline)
        .map(|item| item.total_tokens)
        .collect::<Vec<_>>();
    let cand = completed
        .iter()
        .filter(|item| item.pairformer_mode == candidate)
        .map(|item| item.total_tokens)
        .collect::<Vec<_>>();
    if base.is_empty() || cand.is_empty() {
        return json!({ "status": "insufficient_data" });
    }

    let base_mean = mean_i64(&base);
    let cand_mean = mean_i64(&cand);
    let reduction = if base_mean == 0.0 {
        0.0
    } else {
        (base_mean - cand_mean) / base_mean
    };
    let z_score = welch_z(&base, &cand);
    json!({
        "status": "ok",
        "baseline": baseline,
        "candidate": candidate,
        "baseline_n": base.len(),
        "candidate_n": cand.len(),
        "baseline_mean": round2(base_mean),
        "candidate_mean": round2(cand_mean),
        "token_reduction": round4(reduction),
        "confidence_90_bar_met": base.len() >= 50
            && cand.len() >= 50
            && z_score >= 1.645
            && reduction > 0.0,
        "z_score": round4(z_score),
    })
}

fn normalize_pairformer_mode(value: String) -> String {
    let mode = value.trim().to_lowercase();
    if PAIRFORMER_MODES.contains(&mode.as_str()) {
        mode
    } else {
        "off".to_string()
    }
}

fn non_negative_i64(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Number(number)) => number.as_i64().unwrap_or(0).max(0),
        Some(Value::String(text)) => text.trim().parse::<i64>().unwrap_or(0).max(0),
        _ => 0,
    }
}

fn string_field(data: &Map<String, Value>, key: &str, fallback: &str) -> String {
    data.get(key)
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

fn mean_i64(values: &[i64]) -> f64 {
    values.iter().sum::<i64>() as f64 / values.len() as f64
}

fn median_i64(values: &[i64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let len = sorted.len();
    if len == 0 {
        return 0.0;
    }
    if len % 2 == 1 {
        sorted[len / 2] as f64
    } else {
        (sorted[len / 2 - 1] + sorted[len / 2]) as f64 / 2.0
    }
}

fn welch_z(base: &[i64], cand: &[i64]) -> f64 {
    if base.len() < 2 || cand.len() < 2 {
        return 0.0;
    }
    let base_mean = mean_i64(base);
    let cand_mean = mean_i64(cand);
    let base_var = sample_variance(base, base_mean);
    let cand_var = sample_variance(cand, cand_mean);
    let denom = (base_var / base.len() as f64 + cand_var / cand.len() as f64).sqrt();
    if denom <= 0.0 {
        if base_mean > cand_mean {
            f64::INFINITY
        } else {
            0.0
        }
    } else {
        (base_mean - cand_mean) / denom
    }
}

fn sample_variance(values: &[i64], mean: f64) -> f64 {
    values
        .iter()
        .map(|value| (*value as f64 - mean).powi(2))
        .sum::<f64>()
        / (values.len() - 1) as f64
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn round4(value: f64) -> f64 {
    if value.is_finite() {
        (value * 10_000.0).round() / 10_000.0
    } else {
        value
    }
}
