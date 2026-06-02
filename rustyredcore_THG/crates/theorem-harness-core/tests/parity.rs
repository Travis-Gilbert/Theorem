use serde::Deserialize;
use serde_json::Value;
use theorem_harness_core::{
    apply_transition, empty_state_hash, HarnessError, RunState, TransitionInput,
};

const FIXTURES_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../docs/plans/harness-rust-port/parity/fixtures.json"
));

#[derive(Debug, Deserialize)]
struct Corpus {
    anchors: Anchors,
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
struct Anchors {
    empty_state_hash: String,
}

#[derive(Debug, Deserialize)]
struct Scenario {
    name: String,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
struct Step {
    input: TransitionInput,
    expect: String,
    #[serde(default)]
    state_hash_before: String,
    #[serde(default)]
    state_hash_after: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    seq: u64,
    #[serde(default)]
    guard_code: String,
    #[serde(default)]
    guard_message: String,
}

#[test]
fn python_reference_fixtures_replay_in_rust() {
    let corpus: Corpus = serde_json::from_str(FIXTURES_JSON).expect("parity fixtures should parse");
    assert_eq!(empty_state_hash(), corpus.anchors.empty_state_hash);

    for scenario in corpus.scenarios {
        let mut state: Option<RunState> = None;
        for (index, step) in scenario.steps.into_iter().enumerate() {
            let result = apply_transition(state.clone(), step.input);
            match (step.expect.as_str(), result) {
                ("ok", Ok(output)) => {
                    assert_eq!(
                        output.state_hash_before, step.state_hash_before,
                        "{} step {} state_hash_before",
                        scenario.name, index
                    );
                    assert_eq!(
                        output.state_hash_after, step.state_hash_after,
                        "{} step {} state_hash_after",
                        scenario.name, index
                    );
                    assert_eq!(
                        output.run.status, step.status,
                        "{} step {} status",
                        scenario.name, index
                    );
                    assert_eq!(
                        output.run.last_event_seq, step.seq,
                        "{} step {} seq",
                        scenario.name, index
                    );
                    state = Some(output.run);
                }
                ("guard", Err(HarnessError::Guard(violation))) => {
                    assert_eq!(
                        violation.code, step.guard_code,
                        "{} step {} guard_code: {}",
                        scenario.name, index, step.guard_message
                    );
                    break;
                }
                ("ok", Err(error)) => {
                    panic!(
                        "{} step {} expected ok, got {error:?}",
                        scenario.name, index
                    );
                }
                ("guard", Ok(output)) => {
                    panic!(
                        "{} step {} expected guard {}, got ok status {}",
                        scenario.name, index, step.guard_code, output.run.status
                    );
                }
                (other, _) => panic!(
                    "{} step {} unknown expectation {other}",
                    scenario.name, index
                ),
            }
        }
    }
}

#[test]
fn fixture_inputs_are_transition_inputs() {
    let corpus: Value = serde_json::from_str(FIXTURES_JSON).expect("parity fixtures should parse");
    let scenarios = corpus
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios should be an array");
    assert!(!scenarios.is_empty());
}
