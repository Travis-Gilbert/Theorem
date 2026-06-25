use std::{env, fs};

use rustyred_thg_code::{
    compiler_ambient_readout_in_store, import_runpod_burst_response_in_store,
    CodeRunPodBurstResponse,
};
use rustyred_thg_core::InMemoryGraphStore;
use serde_json::{json, Value};

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: cargo run -p rustyred-thg-code --example import_code_compiler_runpod_response -- response.json");
        std::process::exit(2);
    });
    let raw = fs::read_to_string(&path).unwrap_or_else(|error| {
        eprintln!("failed to read {path}: {error}");
        std::process::exit(2);
    });
    let value: Value = serde_json::from_str(&raw).unwrap_or_else(|error| {
        eprintln!("failed to parse JSON: {error}");
        std::process::exit(2);
    });
    let response_value = value
        .get("output")
        .cloned()
        .or_else(|| {
            value
                .get("response")
                .and_then(|inner| inner.get("output"))
                .cloned()
        })
        .unwrap_or(value);
    let response: CodeRunPodBurstResponse =
        serde_json::from_value(response_value).unwrap_or_else(|error| {
            eprintln!("failed to decode CodeRunPodBurstResponse: {error}");
            std::process::exit(2);
        });
    let tenant_id = response.tenant_id.clone();
    let repo_id = response.repo_id.clone();
    let mut store = InMemoryGraphStore::new();
    let report =
        import_runpod_burst_response_in_store(&mut store, response).unwrap_or_else(|error| {
            eprintln!("failed to import RunPod response: {}", error.message);
            std::process::exit(1);
        });
    let ambient = compiler_ambient_readout_in_store(&mut store, &tenant_id, &repo_id, "", "", 8)
        .unwrap_or_else(|error| {
            eprintln!("failed to read ambient compiler output: {}", error.message);
            std::process::exit(1);
        });
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "ok": true,
            "report": report,
            "ambient": ambient.to_json(),
        }))
        .unwrap()
    );
}
