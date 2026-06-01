//! JSON/HTTP transport handlers over `theorem-harness-runtime`.
//!
//! The two read shapes the Theorem clients consume (see
//! docs/plans/harness-rust-port/ios-transport-handoff.md):
//!   - `runs_json`  -> { "runs":   [ <run node properties> ] }
//!   - `run_json`   -> { "run": <RunState>, "events": [ <EventState> ] }
//!
//! Kept as pure functions over `GraphStore` so they are unit-testable without a
//! live server; `main.rs` is a thin Axum shell that calls them.

use rustyred_thg_core::{GraphStore, NodeQuery};
use serde_json::{json, Value};
use theorem_harness_runtime::{load_events, load_run, HarnessRuntimeError};

/// Node label the runtime persists run state under (`event_log::run_node`).
pub const RUN_LABEL: &str = "HarnessRun";

/// List runs as the client list contract. Each run node's properties are the
/// `RunState` serde shape plus `state_hash`.
pub fn runs_json<S: GraphStore>(store: &S) -> Value {
    let query = NodeQuery {
        label: Some(RUN_LABEL.to_string()),
        properties: Default::default(),
        limit: None,
        include_expired: false,
    };
    let nodes = GraphStore::query_nodes(store, query);
    let runs: Vec<Value> = nodes.into_iter().map(|node| node.properties).collect();
    json!({ "runs": runs })
}

/// One run plus its ordered event log as the client detail contract. Returns
/// `None` when the run is unknown (the handler maps that to 404).
pub fn run_json<S: GraphStore>(
    store: &S,
    run_id: &str,
) -> Result<Option<Value>, HarnessRuntimeError> {
    match load_run(store, run_id)? {
        None => Ok(None),
        Some(run) => {
            let events = load_events(store, run_id)?;
            Ok(Some(json!({ "run": run, "events": events })))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::InMemoryGraphStore;
    use serde_json::Map;
    use theorem_harness_core::TransitionInput;
    use theorem_harness_runtime::append_transition_from_store;

    fn payload(pairs: &[(&str, Value)]) -> Map<String, Value> {
        let mut map = Map::new();
        for (key, value) in pairs {
            map.insert((*key).to_string(), value.clone());
        }
        map
    }

    fn seed_run(store: &mut InMemoryGraphStore) {
        let created = TransitionInput::new(
            "RUN.CREATED",
            payload(&[("task", json!("port harness")), ("actor", json!("claude-code"))]),
        )
        .with_run_id("run-http-test");
        append_transition_from_store(store, created).expect("RUN.CREATED");

        let observed = TransitionInput::new(
            "HOST.OBSERVED",
            payload(&[
                ("repo", json!("Theorem")),
                ("branch", json!("main")),
                ("commit_sha", json!("deadbeef")),
                ("cwd", json!("/repo")),
            ]),
        )
        .with_run_id("run-http-test");
        append_transition_from_store(store, observed).expect("HOST.OBSERVED");
    }

    #[test]
    fn serves_list_and_detail_in_client_contract() {
        let mut store = InMemoryGraphStore::default();
        seed_run(&mut store);

        let list = runs_json(&store);
        let runs = list["runs"].as_array().expect("runs array");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0]["run_id"], json!("run-http-test"));
        assert_eq!(runs[0]["task"], json!("port harness"));

        let detail = run_json(&store, "run-http-test")
            .expect("load ok")
            .expect("run present");
        assert_eq!(detail["run"]["run_id"], json!("run-http-test"));
        let events = detail["events"].as_array().expect("events array");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], json!("RUN.CREATED"));
        assert_eq!(events[1]["type"], json!("HOST.OBSERVED"));
        assert!(events[1]["state_hash_after"].as_str().is_some());

        assert!(run_json(&store, "unknown-run").expect("load ok").is_none());
    }
}
