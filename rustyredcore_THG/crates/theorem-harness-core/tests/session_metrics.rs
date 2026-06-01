use serde_json::json;
use theorem_harness_core::{
    compare_modes, load_jsonl_metrics, summarize_pairformer_ab, SessionMetricsState,
};

#[test]
fn session_metrics_normalize_python_contract_defaults() {
    let state = SessionMetricsState::from_value(&json!({
        "total_input_tokens": -10,
        "total_output_tokens": "25",
        "total_tool_calls": -3,
        "task_completion": true,
        "pairformer_mode": "unknown",
        "task_category": "",
        "workstream_id": "ws",
        "session_id": "s1"
    }));

    assert_eq!(state.total_input_tokens, 0);
    assert_eq!(state.total_output_tokens, 25);
    assert_eq!(state.total_tool_calls, 0);
    assert_eq!(state.pairformer_mode, "off");
    assert_eq!(state.total_tokens, 25);
}

#[test]
fn jsonl_loader_skips_blank_lines() {
    let metrics = load_jsonl_metrics([
        r#"{"total_input_tokens": 100, "total_output_tokens": 25, "pairformer_mode": "gate"}"#,
        "",
        "   ",
        r#"{"total_input_tokens": 10, "total_output_tokens": 5, "pairformer_mode": "full"}"#,
    ])
    .expect("jsonl should parse");

    assert_eq!(metrics.len(), 2);
    assert_eq!(metrics[0].pairformer_mode, "gate");
    assert_eq!(metrics[1].total_tokens, 15);
}

#[test]
fn pairformer_summary_groups_completed_sessions() {
    let metrics = sample_metrics();
    let rows = summarize_pairformer_ab(&metrics);

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].task_category, "fix");
    assert_eq!(rows[0].pairformer_mode, "full");
    assert_eq!(rows[0].completed_sessions, 2);
    assert_eq!(rows[0].mean_tokens_per_completed_task, 850.0);
    assert_eq!(rows[0].median_tokens_per_completed_task, 850.0);
    assert_eq!(rows[0].mean_tool_calls, 2.5);

    assert_eq!(rows[1].task_category, "fix");
    assert_eq!(rows[1].pairformer_mode, "off");
    assert_eq!(rows[1].completed_sessions, 2);
    assert_eq!(rows[1].mean_tokens_per_completed_task, 1100.0);
}

#[test]
fn compare_modes_matches_python_shape() {
    let metrics = sample_metrics();
    assert_eq!(
        compare_modes(&metrics, Some("off"), Some("gate")),
        json!({"status": "insufficient_data"})
    );

    let comparison = compare_modes(&metrics, Some("off"), Some("full"));
    assert_eq!(comparison["status"], "ok");
    assert_eq!(comparison["baseline"], "off");
    assert_eq!(comparison["candidate"], "full");
    assert_eq!(comparison["baseline_n"], 2);
    assert_eq!(comparison["candidate_n"], 2);
    assert_eq!(comparison["baseline_mean"], 1100.0);
    assert_eq!(comparison["candidate_mean"], 850.0);
    assert_eq!(comparison["token_reduction"], 0.2273);
    assert_eq!(comparison["confidence_90_bar_met"], false);
    assert_eq!(comparison["z_score"], 2.2361);
}

fn sample_metrics() -> Vec<SessionMetricsState> {
    [
        json!({
            "total_input_tokens": 700,
            "total_output_tokens": 300,
            "total_tool_calls": 2,
            "task_completion": true,
            "pairformer_mode": "off",
            "task_category": "fix",
            "session_id": "off-1"
        }),
        json!({
            "total_input_tokens": 900,
            "total_output_tokens": 300,
            "total_tool_calls": 4,
            "task_completion": true,
            "pairformer_mode": "off",
            "task_category": "fix",
            "session_id": "off-2"
        }),
        json!({
            "total_input_tokens": 650,
            "total_output_tokens": 150,
            "total_tool_calls": 2,
            "task_completion": true,
            "pairformer_mode": "full",
            "task_category": "fix",
            "session_id": "full-1"
        }),
        json!({
            "total_input_tokens": 750,
            "total_output_tokens": 150,
            "total_tool_calls": 3,
            "task_completion": true,
            "pairformer_mode": "full",
            "task_category": "fix",
            "session_id": "full-2"
        }),
        json!({
            "total_input_tokens": 10,
            "total_output_tokens": 10,
            "total_tool_calls": 1,
            "task_completion": false,
            "pairformer_mode": "gate",
            "task_category": "fix",
            "session_id": "gate-incomplete"
        }),
    ]
    .iter()
    .map(SessionMetricsState::from_value)
    .collect()
}
