use serde::Deserialize;
use serde_json::{Map, Value};
use theorem_harness_core::{catalog_as_dicts, compile_task_toolkit, Payload};

const FIXTURES_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../docs/plans/harness-rust-port/parity-toolgraph/toolkit_fixtures.json"
));

#[derive(Debug, Deserialize)]
struct Corpus {
    catalog: Value,
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
struct Scenario {
    name: String,
    input: ScenarioInput,
    expected: Value,
}

#[derive(Debug, Deserialize)]
struct ScenarioInput {
    task_type: String,
    permissions: Option<Value>,
    #[serde(default)]
    scope: Payload,
}

#[test]
fn python_toolgraph_fixtures_compile_in_rust() {
    let corpus: Corpus = serde_json::from_str(FIXTURES_JSON).expect("fixtures should parse");
    for scenario in corpus.scenarios {
        let permissions = scenario.input.permissions.map(permissions_from_value);
        let actual = compile_task_toolkit(
            &scenario.input.task_type,
            permissions,
            Some(scenario.input.scope),
        );
        let actual_value =
            serde_json::to_value(actual).expect("CompiledToolkit should serialize cleanly");
        assert_eq!(actual_value, scenario.expected, "{}", scenario.name);
    }
}

#[test]
fn python_toolgraph_catalog_matches_rust_catalog() {
    let corpus: Corpus = serde_json::from_str(FIXTURES_JSON).expect("fixtures should parse");
    let actual =
        serde_json::to_value(catalog_as_dicts()).expect("catalog should serialize cleanly");
    assert_eq!(actual, corpus.catalog);
}

fn permissions_from_value(value: Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        Value::String(text) => {
            if text.contains(',') {
                text.split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_string)
                    .collect()
            } else if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![text.trim().to_string()]
            }
        }
        Value::Object(map) => map_value_keys(map),
        Value::Null => Vec::new(),
        other => vec![other.to_string()],
    }
}

fn map_value_keys(map: Map<String, Value>) -> Vec<String> {
    map.into_iter().map(|(key, _)| key).collect()
}
