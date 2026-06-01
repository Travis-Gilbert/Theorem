use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};

use crate::atoms::{AtomLifecycle, SceneAtom, SceneRelation, SourceRef};
use crate::package::{
    ChromeBinding, ProjectionBinding, ScenePackageV2, TerminalStateArtifact,
    SCENE_PACKAGE_V2_VERSION,
};

#[derive(Debug, Clone, PartialEq)]
pub struct PatentSceneLiftInput {
    pub query: String,
    pub patent_payload: Value,
    pub evidence_ids: Vec<String>,
    pub trace_id: Option<String>,
    pub degraded: bool,
    pub model_used: Option<String>,
}

pub fn lift_patent_scene_payload(input: PatentSceneLiftInput) -> ScenePackageV2 {
    let payload = normalize_payload(input.patent_payload);
    let source_refs = shared_source_refs(&input.evidence_ids);
    let (atoms, relations) = atoms_and_relations_from_payload(&payload, &source_refs);
    let title_block = object_field_or_empty(&payload, "title_block");
    let sheet_footer = object_field_or_empty(&payload, "sheet_footer");
    let legend = array_field_or_empty(&payload, "legend");
    let model_used = input
        .model_used
        .as_ref()
        .map_or(Value::Null, |model| json!(model));

    let projection = ProjectionBinding {
        id: "patent_diagram".to_string(),
        params: BTreeMap::from([
            ("payload".to_string(), payload.clone()),
            ("degraded".to_string(), json!(input.degraded)),
            ("model_used".to_string(), model_used.clone()),
        ]),
    };

    let chrome = ChromeBinding {
        id: "patent_plate_shell".to_string(),
        params: BTreeMap::from([
            ("title_block".to_string(), title_block),
            ("sheet_footer".to_string(), sheet_footer),
            ("legend".to_string(), legend),
            ("degraded".to_string(), json!(input.degraded)),
            ("model_used".to_string(), model_used.clone()),
        ]),
    };

    let terminal_state = TerminalStateArtifact {
        svg: None,
        json_payload: Some(BTreeMap::from([
            ("kind".to_string(), json!("patent_scene")),
            ("payload".to_string(), payload.clone()),
        ])),
        source_refs: terminal_source_refs(&source_refs),
    };

    let package_id = stable_patent_id(&input.query, input.trace_id.as_deref());

    ScenePackageV2 {
        version: SCENE_PACKAGE_V2_VERSION.to_string(),
        id: package_id.clone(),
        manifest_ref: format!("{package_id}.manifest"),
        atoms,
        relations,
        projection,
        chrome,
        actions: Vec::new(),
        transitions: None,
        terminal_state: Some(terminal_state),
        provenance: BTreeMap::from([
            ("resolver".to_string(), json!("patent_scene")),
            ("query".to_string(), json!(input.query)),
            (
                "trace_id".to_string(),
                input.trace_id.map_or(Value::Null, Value::String),
            ),
            ("degraded".to_string(), json!(input.degraded)),
            ("model_used".to_string(), model_used),
        ]),
    }
}

fn normalize_payload(payload: Value) -> Value {
    let Value::Object(mut output) = payload else {
        return Value::Object(Map::new());
    };

    if let Some(title_block) = output.get("titleBlock").cloned() {
        output.entry("title_block").or_insert(title_block);
    }
    if let Some(sheet_footer) = output.get("sheetFooter").cloned() {
        output.entry("sheet_footer").or_insert(sheet_footer);
    }

    Value::Object(output)
}

fn atoms_and_relations_from_payload(
    payload: &Value,
    source_refs: &[SourceRef],
) -> (Vec<SceneAtom>, Vec<SceneRelation>) {
    let figures = payload
        .get("figures")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let mut atoms = Vec::new();
    let mut relations = Vec::new();

    for (figure_index, figure) in figures.iter().enumerate() {
        let Some(figure) = figure.as_object() else {
            continue;
        };
        let figure_number = figure
            .get("number")
            .and_then(Value::as_i64)
            .unwrap_or((figure_index + 1) as i64);
        let Some(dot) = figure.get("dot").and_then(Value::as_str) else {
            continue;
        };
        let callout_by_node = callout_by_node(figure);

        for (node_id, label) in extract_dot_nodes(dot) {
            atoms.push(SceneAtom {
                id: format!("f{figure_number}.{node_id}"),
                kind: "patent-node".to_string(),
                label,
                position: None,
                weight: None,
                color: None,
                opacity: None,
                glyph: None,
                scale: None,
                lifecycle: AtomLifecycle::Present,
                metadata: BTreeMap::from([
                    ("figure_number".to_string(), json!(figure_number)),
                    ("dot_node_id".to_string(), json!(node_id)),
                    (
                        "callout_id".to_string(),
                        callout_by_node
                            .get(&node_id)
                            .cloned()
                            .unwrap_or(Value::Null),
                    ),
                ]),
                source_refs: source_refs.to_vec(),
            });
        }

        for (source, target) in extract_dot_edges(dot) {
            relations.push(SceneRelation {
                id: format!("f{figure_number}.{source}->{target}"),
                source_id: format!("f{figure_number}.{source}"),
                target_id: format!("f{figure_number}.{target}"),
                kind: "patent-edge".to_string(),
                weight: None,
                color: None,
                opacity: None,
                glyph: None,
                lifecycle: AtomLifecycle::Present,
                metadata: BTreeMap::from([("figure_number".to_string(), json!(figure_number))]),
                source_refs: source_refs.to_vec(),
            });
        }
    }

    (atoms, relations)
}

fn callout_by_node(figure: &Map<String, Value>) -> BTreeMap<String, Value> {
    let mut callouts = BTreeMap::new();
    let Some(items) = figure.get("callouts").and_then(Value::as_array) else {
        return callouts;
    };

    for item in items {
        let Some(item) = item.as_object() else {
            continue;
        };
        let target = item
            .get("target_node_id")
            .or_else(|| item.get("targetNodeId"))
            .and_then(Value::as_str);
        let id = item.get("id").cloned();
        if let (Some(target), Some(id)) = (target, id) {
            callouts.insert(target.to_string(), id);
        }
    }

    callouts
}

fn extract_dot_nodes(dot: &str) -> Vec<(String, Option<String>)> {
    let mut nodes = Vec::new();
    let mut seen = BTreeSet::new();

    for statement in dot.split(';') {
        if statement.contains("->") {
            continue;
        }
        let Some(open_bracket) = statement.find('[') else {
            continue;
        };
        let node_id = last_dot_identifier(&statement[..open_bracket]);
        if node_id.is_empty() || is_dot_keyword(&node_id) || !is_dot_identifier(&node_id) {
            continue;
        }
        if !seen.insert(node_id.clone()) {
            continue;
        }

        let attrs = statement[open_bracket + 1..]
            .split(']')
            .next()
            .unwrap_or_default();
        nodes.push((node_id, extract_dot_label(attrs)));
    }

    nodes
}

fn extract_dot_edges(dot: &str) -> Vec<(String, String)> {
    let mut edges = Vec::new();

    for statement in dot.split(';') {
        let Some(edge_at) = statement.find("->") else {
            continue;
        };
        let source = last_dot_identifier(&statement[..edge_at]);
        let target = first_dot_identifier(&statement[edge_at + 2..]);
        if is_dot_identifier(&source) && is_dot_identifier(&target) {
            edges.push((source, target));
        }
    }

    edges
}

fn extract_dot_label(attrs: &str) -> Option<String> {
    let label_start = attrs.find("label")?;
    let after_label = &attrs[label_start + "label".len()..];
    let equals_at = after_label.find('=')?;
    let mut value = after_label[equals_at + 1..].trim_start();
    if !value.starts_with('"') {
        return None;
    }
    value = &value[1..];

    let mut label = String::new();
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            if character == 'n' {
                label.push(' ');
            } else {
                label.push(character);
            }
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '"' => break,
            _ => label.push(character),
        }
    }

    let label = label.replace('\n', " ");
    let label = label.trim();
    (!label.is_empty()).then(|| label.to_string())
}

fn last_dot_identifier(input: &str) -> String {
    input
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .filter(|part| !part.is_empty())
        .last()
        .unwrap_or_default()
        .to_string()
}

fn first_dot_identifier(input: &str) -> String {
    input
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .find(|part| !part.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn is_dot_identifier(input: &str) -> bool {
    let mut characters = input.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn is_dot_keyword(input: &str) -> bool {
    matches!(
        input,
        "digraph"
            | "graph"
            | "subgraph"
            | "node"
            | "edge"
            | "rank"
            | "rankdir"
            | "splines"
            | "ranksep"
            | "nodesep"
            | "bgcolor"
    )
}

fn shared_source_refs(evidence_ids: &[String]) -> Vec<SourceRef> {
    evidence_ids
        .iter()
        .take(12)
        .map(|id| SourceRef {
            kind: "object".to_string(),
            id: id.clone(),
            label: None,
            metadata: BTreeMap::new(),
        })
        .collect()
}

fn terminal_source_refs(source_refs: &[SourceRef]) -> Vec<BTreeMap<String, Value>> {
    source_refs
        .iter()
        .map(|source_ref| {
            BTreeMap::from([
                ("kind".to_string(), json!(source_ref.kind)),
                ("id".to_string(), json!(source_ref.id)),
            ])
        })
        .collect()
}

fn object_field_or_empty(payload: &Value, key: &str) -> Value {
    payload
        .get(key)
        .and_then(Value::as_object)
        .map_or_else(|| json!({}), |object| Value::Object(object.clone()))
}

fn array_field_or_empty(payload: &Value, key: &str) -> Value {
    payload
        .get(key)
        .and_then(Value::as_array)
        .map_or_else(|| json!([]), |array| Value::Array(array.clone()))
}

fn stable_patent_id(query: &str, trace_id: Option<&str>) -> String {
    let query_prefix: String = query.chars().take(120).collect();
    let seed = format!("patent:{}:{query_prefix}", trace_id.unwrap_or_default());
    let mut hash = 0xcbf29ce484222325u64;
    for byte in seed.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("patent-{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lift_produces_patent_atoms_relations_and_register_payload() {
        let package = lift_patent_scene_payload(PatentSceneLiftInput {
            query: "what is the architecture of a knowledge graph".to_string(),
            patent_payload: sample_patent_payload(),
            evidence_ids: vec!["101".to_string(), "202".to_string(), "303".to_string()],
            trace_id: Some("test-trace".to_string()),
            degraded: false,
            model_used: Some("26b".to_string()),
        });

        assert_eq!(package.version, SCENE_PACKAGE_V2_VERSION);
        assert_eq!(package.atoms.len(), 3);
        assert_eq!(package.relations.len(), 2);
        assert_eq!(package.projection.id, "patent_diagram");
        assert_eq!(package.chrome.id, "patent_plate_shell");
        assert!(package.id.starts_with("patent-"));
        assert_eq!(package.manifest_ref, format!("{}.manifest", package.id));
        assert_eq!(
            package.projection.params["payload"]["title_block"]["title"],
            "KNOWLEDGE GRAPH ARCHITECTURE"
        );
        assert_eq!(package.chrome.params["legend"][0]["number"], 10);
        assert_eq!(
            package
                .terminal_state
                .as_ref()
                .and_then(|state| state.json_payload.as_ref())
                .expect("terminal state payload")["kind"],
            "patent_scene"
        );
    }

    #[test]
    fn lift_attaches_callouts_and_evidence_source_refs() {
        let package = lift_patent_scene_payload(PatentSceneLiftInput {
            query: "q".to_string(),
            patent_payload: sample_patent_payload(),
            evidence_ids: vec!["101".to_string(), "202".to_string(), "303".to_string()],
            trace_id: None,
            degraded: false,
            model_used: None,
        });

        let sources_atom = package
            .atoms
            .iter()
            .find(|atom| atom.id == "f1.sources")
            .expect("sources atom");
        assert_eq!(sources_atom.metadata["callout_id"], 10);
        assert_eq!(sources_atom.source_refs.len(), 3);
        assert_eq!(sources_atom.source_refs[0].kind, "object");
        assert_eq!(sources_atom.source_refs[0].id, "101");

        let engine_atom = package
            .atoms
            .iter()
            .find(|atom| atom.id == "f1.query_engine")
            .expect("query engine atom");
        assert_eq!(engine_atom.metadata["callout_id"], Value::Null);
    }

    #[test]
    fn lift_normalizes_camelcase_register_fields() {
        let mut payload = sample_patent_payload();
        let Value::Object(ref mut object) = payload else {
            panic!("sample payload should be an object");
        };
        let title_block = object.remove("title_block").expect("title block");
        let sheet_footer = object.remove("sheet_footer").expect("sheet footer");
        object.insert("titleBlock".to_string(), title_block);
        object.insert("sheetFooter".to_string(), sheet_footer);

        let package = lift_patent_scene_payload(PatentSceneLiftInput {
            query: "q".to_string(),
            patent_payload: payload,
            evidence_ids: Vec::new(),
            trace_id: None,
            degraded: true,
            model_used: Some("4b".to_string()),
        });

        assert_eq!(package.chrome.params["title_block"]["inventor"], "THESEUS");
        assert_eq!(
            package.chrome.params["sheet_footer"]["sheet_number"],
            "Sheet 1 of 1"
        );
        assert_eq!(package.chrome.params["degraded"], true);
        assert_eq!(package.chrome.params["model_used"], "4b");
    }

    #[test]
    fn lift_reuses_stable_scene_id_for_same_query_and_trace() {
        let first = lift_patent_scene_payload(PatentSceneLiftInput {
            query: "q".to_string(),
            patent_payload: sample_patent_payload(),
            evidence_ids: Vec::new(),
            trace_id: Some("abc".to_string()),
            degraded: false,
            model_used: None,
        });
        let second = lift_patent_scene_payload(PatentSceneLiftInput {
            query: "q".to_string(),
            patent_payload: sample_patent_payload(),
            evidence_ids: Vec::new(),
            trace_id: Some("abc".to_string()),
            degraded: false,
            model_used: None,
        });

        assert_eq!(first.id, second.id);
    }

    #[test]
    fn wire_shape_round_trips_through_scene_package_v2() {
        let package = lift_patent_scene_payload(PatentSceneLiftInput {
            query: "q".to_string(),
            patent_payload: sample_patent_payload(),
            evidence_ids: Vec::new(),
            trace_id: None,
            degraded: false,
            model_used: None,
        });
        let value = serde_json::to_value(&package).expect("serialize package");

        assert_eq!(
            value["projection"]["params"]["payload"]["figures"][0]["number"],
            1
        );
        assert_eq!(value["atoms"][0]["kind"], "patent-node");
        assert_eq!(value["relations"][0]["kind"], "patent-edge");
        assert_eq!(value["terminalState"]["json"]["kind"], "patent_scene");

        let restored: ScenePackageV2 =
            serde_json::from_value(value).expect("deserialize scene package");
        assert_eq!(restored.id, package.id);
        assert_eq!(restored.atoms.len(), package.atoms.len());
        assert_eq!(restored.projection.id, "patent_diagram");
    }

    fn sample_patent_payload() -> Value {
        json!({
            "title_block": {
                "date": "May 19, 2026",
                "inventor": "THESEUS",
                "patent_number": "T-2026-001",
                "title": "KNOWLEDGE GRAPH ARCHITECTURE",
                "filed_date": "Filed May 19, 2026"
            },
            "figures": [
                {
                    "number": 1,
                    "caption": "System architecture",
                    "dot": "digraph G { node [shape=box]; sources [label=\"Data Sources\"]; graph_db [label=\"Graph Database\"]; query_engine [label=\"Query Engine\"]; sources -> graph_db; graph_db -> query_engine; }",
                    "callouts": [
                        {"id": 10, "target_node_id": "sources", "description": "Data sources"},
                        {"id": 12, "target_node_id": "graph_db", "description": "Graph DB"}
                    ]
                }
            ],
            "legend": [
                {"number": 10, "description": "Data ingestion sources"},
                {"number": 12, "description": "In-memory graph store"}
            ],
            "sheet_footer": {"inventor": "THESEUS", "sheet_number": "Sheet 1 of 1"}
        })
    }
}
