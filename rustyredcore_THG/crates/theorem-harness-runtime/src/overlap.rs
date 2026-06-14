use std::collections::{BTreeMap, BTreeSet};

use rustyred_thg_core::{GraphSnapshot, GraphStore, HarnessInstantKg, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::coordination::{
    stable_coordination_record_id, write_record, CoordinationResult, WriteRecordInput,
};

const CODE_FILE_LABEL: &str = "CodeFile";
const CODE_SYMBOL_LABEL: &str = "CodeSymbol";
const DECLARES_SYMBOL: &str = "DECLARES_SYMBOL";
const CALLS_SYMBOL: &str = "CALLS_SYMBOL";
const DEPENDS_ON_SYMBOL: &str = "DEPENDS_ON_SYMBOL";
const IMPORTS: &str = "IMPORTS";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Footprint {
    pub actor: String,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Neighborhood {
    pub actor: String,
    pub symbols: BTreeSet<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Overlap {
    pub a: String,
    pub b: String,
    pub shared_symbols: Vec<String>,
}

pub fn neighborhood_of(kg: &HarnessInstantKg, fp: &Footprint, hops: usize) -> Neighborhood {
    let snapshot = kg.merged_snapshot();
    let index = SnapshotIndex::new(snapshot);
    let mut symbols = BTreeSet::new();
    let file_set = fp
        .files
        .iter()
        .map(|path| normalize_path(path))
        .collect::<BTreeSet<_>>();

    for node in index.nodes.values() {
        if node.labels.iter().any(|label| label == CODE_SYMBOL_LABEL)
            && property_string(&node.properties, "file_path")
                .map(|path| file_set.contains(&normalize_path(&path)))
                .unwrap_or(false)
        {
            symbols.insert(node.id.clone());
        }
        if node.labels.iter().any(|label| label == CODE_FILE_LABEL)
            && property_string(&node.properties, "path")
                .or_else(|| property_string(&node.properties, "file_path"))
                .map(|path| file_set.contains(&normalize_path(&path)))
                .unwrap_or(false)
        {
            symbols.extend(index.declared_symbols(&node.id));
        }
    }

    let mut frontier = symbols.iter().cloned().collect::<Vec<_>>();
    let mut depth = 0usize;
    while depth < hops {
        let mut next = Vec::new();
        for symbol in frontier {
            for neighbor in index.semantic_neighbors(&symbol) {
                if symbols.insert(neighbor.clone()) {
                    next.push(neighbor);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
        depth += 1;
    }

    Neighborhood {
        actor: fp.actor.clone(),
        symbols,
    }
}

pub fn detect_overlaps(neighborhoods: &[Neighborhood]) -> Vec<Overlap> {
    let mut overlaps = Vec::new();
    for left_idx in 0..neighborhoods.len() {
        for right_idx in (left_idx + 1)..neighborhoods.len() {
            let left = &neighborhoods[left_idx];
            let right = &neighborhoods[right_idx];
            let shared = left
                .symbols
                .intersection(&right.symbols)
                .cloned()
                .collect::<Vec<_>>();
            if shared.is_empty() {
                continue;
            }
            overlaps.push(Overlap {
                a: left.actor.clone(),
                b: right.actor.clone(),
                shared_symbols: shared,
            });
        }
    }
    overlaps
}

pub fn emit_overlap_tension<S: GraphStore>(
    store: &mut S,
    tenant_slug: &str,
    room_id: &str,
    overlap: &Overlap,
) -> CoordinationResult<()> {
    let summary = format!(
        "Semantic overlap between {} and {} on {} symbol(s)",
        overlap.a,
        overlap.b,
        overlap.shared_symbols.len()
    );
    let body = format!(
        "Shared code graph symbols: {}",
        overlap.shared_symbols.join(", ")
    );
    let actor_id = "semantic-overlap-detector";
    let record_id =
        stable_coordination_record_id(tenant_slug, room_id, "tension", actor_id, &summary, "");
    let mut metadata = Map::new();
    metadata.insert("overlap".to_string(), json!(overlap));
    write_record(
        store,
        WriteRecordInput {
            tenant_slug: tenant_slug.to_string(),
            room_id: room_id.to_string(),
            actor_id: actor_id.to_string(),
            record_id,
            record_type: "tension".to_string(),
            title: "Semantic footprint overlap".to_string(),
            summary,
            body,
            metadata,
            created_at: String::new(),
        },
    )?;
    Ok(())
}

pub fn detect_and_emit_overlap_tensions<S: GraphStore>(
    store: &mut S,
    tenant_slug: &str,
    room_id: &str,
    kg: &HarnessInstantKg,
    footprints: &[Footprint],
    hops: usize,
) -> Vec<Overlap> {
    let neighborhoods = footprints
        .iter()
        .map(|footprint| neighborhood_of(kg, footprint, hops))
        .collect::<Vec<_>>();
    let overlaps = detect_overlaps(&neighborhoods);
    for overlap in &overlaps {
        let _ = emit_overlap_tension(store, tenant_slug, room_id, overlap);
    }
    overlaps
}

struct SnapshotIndex {
    nodes: BTreeMap<String, NodeRecord>,
    out: BTreeMap<String, Vec<(String, String)>>,
    incoming: BTreeMap<String, Vec<(String, String)>>,
}

impl SnapshotIndex {
    fn new(snapshot: GraphSnapshot) -> Self {
        let nodes = snapshot
            .nodes
            .into_iter()
            .filter(|node| !node.tombstone)
            .map(|node| (node.id.clone(), node))
            .collect::<BTreeMap<_, _>>();
        let mut out: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        let mut incoming: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for edge in snapshot.edges.into_iter().filter(|edge| !edge.tombstone) {
            if !nodes.contains_key(&edge.from_id) || !nodes.contains_key(&edge.to_id) {
                continue;
            }
            out.entry(edge.from_id.clone())
                .or_default()
                .push((edge.edge_type.clone(), edge.to_id.clone()));
            incoming
                .entry(edge.to_id)
                .or_default()
                .push((edge.edge_type, edge.from_id));
        }
        Self {
            nodes,
            out,
            incoming,
        }
    }

    fn declared_symbols(&self, file_id: &str) -> Vec<String> {
        self.out
            .get(file_id)
            .into_iter()
            .flatten()
            .filter(|(edge_type, _)| edge_type == DECLARES_SYMBOL)
            .map(|(_, to_id)| to_id.clone())
            .collect()
    }

    fn semantic_neighbors(&self, symbol_id: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (edge_type, node_id) in self
            .out
            .get(symbol_id)
            .into_iter()
            .flatten()
            .chain(self.incoming.get(symbol_id).into_iter().flatten())
        {
            if matches!(
                edge_type.as_str(),
                CALLS_SYMBOL | DEPENDS_ON_SYMBOL | DECLARES_SYMBOL | IMPORTS
            ) && self
                .nodes
                .get(node_id)
                .map(|node| node.labels.iter().any(|label| label == CODE_SYMBOL_LABEL))
                .unwrap_or(false)
            {
                out.push(node_id.clone());
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

fn property_string(properties: &Value, key: &str) -> Option<String> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn normalize_path(path: &str) -> String {
    path.trim().trim_start_matches("./").to_string()
}

#[cfg(test)]
mod tests {
    use rustyred_thg_core::{
        Direction, EdgeRecord, GraphSnapshot, InMemoryGraphStore, NeighborQuery, NodeRecord,
        SessionDelta,
    };
    use serde_json::json;

    use super::*;
    use crate::coordination::read_records_for_room;

    fn code_node(id: &str, label: &str, props: Value) -> NodeRecord {
        NodeRecord::new(id, [label], props)
    }

    fn fixture_kg(shared: bool) -> HarnessInstantKg {
        let mut nodes = vec![
            code_node(
                "file:a",
                CODE_FILE_LABEL,
                json!({ "file_path": "src/a.rs" }),
            ),
            code_node(
                "file:b",
                CODE_FILE_LABEL,
                json!({ "file_path": "src/b.rs" }),
            ),
            code_node(
                "sym:a",
                CODE_SYMBOL_LABEL,
                json!({ "file_path": "src/a.rs" }),
            ),
            code_node(
                "sym:b",
                CODE_SYMBOL_LABEL,
                json!({ "file_path": "src/b.rs" }),
            ),
        ];
        let mut edges = vec![
            EdgeRecord::new("decl:a", "file:a", DECLARES_SYMBOL, "sym:a", json!({})),
            EdgeRecord::new("decl:b", "file:b", DECLARES_SYMBOL, "sym:b", json!({})),
        ];
        if shared {
            nodes.push(code_node(
                "sym:shared",
                CODE_SYMBOL_LABEL,
                json!({ "file_path": "src/shared.rs" }),
            ));
            edges.push(EdgeRecord::new(
                "call:a",
                "sym:a",
                CALLS_SYMBOL,
                "sym:shared",
                json!({}),
            ));
            edges.push(EdgeRecord::new(
                "call:b",
                "sym:b",
                CALLS_SYMBOL,
                "sym:shared",
                json!({}),
            ));
        }
        HarnessInstantKg::new(
            GraphSnapshot {
                version: 1,
                nodes,
                edges,
            },
            None,
            SessionDelta::default(),
        )
    }

    #[test]
    fn disjoint_footprints_produce_no_overlap() {
        let kg = fixture_kg(false);
        let neighborhoods = [
            neighborhood_of(
                &kg,
                &Footprint {
                    actor: "codex".to_string(),
                    files: vec!["src/a.rs".to_string()],
                },
                1,
            ),
            neighborhood_of(
                &kg,
                &Footprint {
                    actor: "claude-code".to_string(),
                    files: vec!["src/b.rs".to_string()],
                },
                1,
            ),
        ];

        assert!(detect_overlaps(&neighborhoods).is_empty());
    }

    #[test]
    fn shared_call_neighborhood_produces_one_overlap() {
        let kg = fixture_kg(true);
        let neighborhoods = [
            neighborhood_of(
                &kg,
                &Footprint {
                    actor: "codex".to_string(),
                    files: vec!["src/a.rs".to_string()],
                },
                1,
            ),
            neighborhood_of(
                &kg,
                &Footprint {
                    actor: "claude-code".to_string(),
                    files: vec!["src/b.rs".to_string()],
                },
                1,
            ),
        ];

        let overlaps = detect_overlaps(&neighborhoods);

        assert_eq!(overlaps.len(), 1);
        assert_eq!(overlaps[0].shared_symbols, vec!["sym:shared".to_string()]);
    }

    #[test]
    fn overlap_tension_is_room_readable() {
        let mut store = InMemoryGraphStore::new();
        emit_overlap_tension(
            &mut store,
            "Travis-Gilbert",
            "room",
            &Overlap {
                a: "codex".to_string(),
                b: "claude-code".to_string(),
                shared_symbols: vec!["sym:shared".to_string()],
            },
        )
        .unwrap();

        let records = read_records_for_room(
            &store,
            "travis-gilbert",
            "room",
            &["tension".to_string()],
            10,
        )
        .unwrap();

        assert_eq!(records.len(), 1);
        assert!(records[0].body.contains("sym:shared"));
    }

    #[test]
    fn detector_does_not_block_when_tension_write_is_unavailable() {
        let mut store = InMemoryGraphStore::new();
        let kg = fixture_kg(true);
        let overlaps = detect_and_emit_overlap_tensions(
            &mut store,
            "",
            "",
            &kg,
            &[
                Footprint {
                    actor: "codex".to_string(),
                    files: vec!["src/a.rs".to_string()],
                },
                Footprint {
                    actor: "claude-code".to_string(),
                    files: vec!["src/b.rs".to_string()],
                },
            ],
            1,
        );

        assert_eq!(overlaps.len(), 1);
    }

    #[test]
    fn neighbor_query_import_remains_available_for_future_hook() {
        let query = NeighborQuery::out("sym:a").with_edge_type(CALLS_SYMBOL);
        assert_eq!(query.edge_type.as_deref(), Some(CALLS_SYMBOL));
        assert_eq!(query.direction, Direction::Out);
    }
}
