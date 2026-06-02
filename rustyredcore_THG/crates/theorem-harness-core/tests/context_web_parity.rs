use serde::Deserialize;
use serde_json::Value;
use theorem_harness_core::{
    is_generated_artifact, normalize_context_web_node_id, ContextWebPackInput,
};

const FIXTURES_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../docs/plans/harness-rust-port/parity-context/context_fixtures.json"
));

#[derive(Debug, Deserialize)]
struct Corpus {
    pack_scenarios: Vec<PackScenario>,
    policy_cases: Vec<PolicyCase>,
}

#[derive(Debug, Deserialize)]
struct PackScenario {
    name: String,
    input: ContextWebPackInput,
    expected: Value,
}

#[derive(Debug, Deserialize)]
struct PolicyCase {
    input: PolicyInput,
    expected: PolicyExpected,
}

#[derive(Debug, Deserialize)]
struct PolicyInput {
    id: String,
    labels: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PolicyExpected {
    normalized: String,
    is_generated: bool,
}

#[test]
fn python_context_pack_fixtures_compile_in_rust() {
    let corpus: Corpus = serde_json::from_str(FIXTURES_JSON).expect("fixtures should parse");
    for scenario in corpus.pack_scenarios {
        let (pack, policy) = scenario.input.into_pack_and_policy();
        let actual = pack.bounded(Some(&policy));
        let actual_value =
            serde_json::to_value(actual).expect("ContextWebPack should serialize cleanly");
        assert_eq!(actual_value, scenario.expected, "{}", scenario.name);
    }
}

#[test]
fn python_context_policy_cases_match_rust_policy() {
    let corpus: Corpus = serde_json::from_str(FIXTURES_JSON).expect("fixtures should parse");
    for case in corpus.policy_cases {
        assert_eq!(
            normalize_context_web_node_id(&case.input.id),
            case.expected.normalized,
            "normalize {}",
            case.input.id
        );
        assert_eq!(
            is_generated_artifact(&case.input.id, &case.input.labels),
            case.expected.is_generated,
            "is_generated {}",
            case.input.id
        );
    }
}
