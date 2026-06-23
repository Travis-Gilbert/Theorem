//! SceneOS atom-substrate adapter — browser plan step 2.
//!
//! Wires the reconstruction engine's relational output into the SceneOS atom
//! substrate: a [`BlockSubgraph`] (the engine's `GraphNode`s + `GraphEdge`s)
//! becomes a [`SceneScene`] of atoms + relations matching the SceneOS wire
//! contract (`Index-API/apps/notebook/scene_os/atoms.py` and
//! `Theseus-UI/src/scene-os/atoms/types.ts`).
//!
//! This is the engine half of the plan's "the generative projection is the
//! reconstruction engine's `AssetGenerator` output feeding the SceneOS atom
//! substrate": the engine *generates the structure* (atoms + relations), SceneOS
//! *places* it (a projection assigns coordinates) and renders it. So the atoms
//! here carry no `position` — placement is SceneOS's job, by design.
//!
//! The mapping is domain-agnostic: it reads only the generic `GraphNode` /
//! `GraphEdge` fields, so it works for any `ReconstructionDomain` (buildings
//! today, browser domains tomorrow), not just the building reference impl.
//!
//! Wire format is camelCase (`sourceId` / `targetId`) to match the SceneOS
//! TypeScript types. Lifecycle is `"present"` (settled, driven by the active
//! projection) per `AtomLifecycle`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{BlockSubgraph, GraphEdge, GraphNode, PipelineOutput};

/// `AtomLifecycle.PRESENT` — settled, placed by the active projection.
const LIFECYCLE_PRESENT: &str = "present";

/// Placement of one atom in a coordinate space. The engine leaves this unset;
/// SceneOS's projection computes it. Defined here for round-trip fidelity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneAtomPosition {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub z: f64,
    pub space: String,
}

/// One atom in the SceneOS substrate (a generated structural element).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneAtom {
    pub id: String,
    pub kind: String,
    pub lifecycle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<SceneAtomPosition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glyph: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

/// One relation (edge) between two atoms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneRelation {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub kind: String,
    pub lifecycle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glyph: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

/// A scene: the atoms + relations a SceneOS projection places and renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneScene {
    pub atoms: Vec<SceneAtom>,
    pub relations: Vec<SceneRelation>,
}

fn atom_from_node<S>(node: &GraphNode<S>) -> SceneAtom {
    SceneAtom {
        id: node.node_id.clone(),
        kind: node.node_type.clone(),
        lifecycle: LIFECYCLE_PRESENT.to_string(),
        label: node.attributes.get("label").cloned(),
        position: None, // SceneOS's projection assigns placement.
        weight: None,
        color: None,
        opacity: None,
        glyph: None,
        scale: None,
        metadata: node.attributes.clone(),
    }
}

fn relation_from_edge(edge: &GraphEdge) -> SceneRelation {
    let mut metadata = edge.attributes.clone();
    if let Some(confidence) = edge.confidence {
        metadata.insert("confidence".to_string(), confidence.to_string());
    }
    if let Some(distance) = edge.distance_m {
        metadata.insert("distanceM".to_string(), distance.to_string());
    }
    if let Some(years) = edge.time_distance_years {
        metadata.insert("timeDistanceYears".to_string(), years.to_string());
    }
    SceneRelation {
        id: format!("{}->{}:{}", edge.source, edge.target, edge.edge_type),
        source_id: edge.source.clone(),
        target_id: edge.target.clone(),
        kind: edge.edge_type.clone(),
        lifecycle: LIFECYCLE_PRESENT.to_string(),
        weight: Some(edge.weight),
        color: None,
        opacity: None,
        glyph: None,
        metadata,
    }
}

/// Convert a reconstruction's relational subgraph into a SceneOS scene.
///
/// Each `GraphNode` becomes an atom, each `GraphEdge` a relation. Atoms carry no
/// position — SceneOS places them. Domain-agnostic: reads only generic fields.
pub fn scene_from_subgraph<S>(subgraph: &BlockSubgraph<S>) -> SceneScene {
    SceneScene {
        atoms: subgraph.nodes.iter().map(atom_from_node).collect(),
        relations: subgraph.edges.iter().map(relation_from_edge).collect(),
    }
}

/// Convenience: build a SceneOS scene from a full pipeline output (uses its
/// block subgraph — the relational structure the eight stages produced).
pub fn scene_from_pipeline<S>(output: &PipelineOutput<S>) -> SceneScene {
    scene_from_subgraph(&output.block_subgraph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn node(id: &str, kind: &str, label: Option<&str>) -> GraphNode<Value> {
        let mut attributes = BTreeMap::new();
        if let Some(l) = label {
            attributes.insert("label".to_string(), l.to_string());
        }
        GraphNode {
            node_id: id.to_string(),
            node_type: kind.to_string(),
            object: None,
            direct_spec: None,
            embedding: Vec::new(),
            missing_embedding: false,
            attributes,
        }
    }

    fn edge(source: &str, target: &str, kind: &str, confidence: Option<f64>) -> GraphEdge {
        GraphEdge {
            source: source.to_string(),
            target: target.to_string(),
            edge_type: kind.to_string(),
            weight: 1.0,
            distance_m: None,
            time_distance_years: None,
            confidence,
            attributes: BTreeMap::new(),
        }
    }

    fn subgraph() -> BlockSubgraph<Value> {
        BlockSubgraph {
            focus_node: "a".to_string(),
            nodes: vec![
                node("a", "concept", Some("Apple")),
                node("b", "concept", Some("Orchard")),
            ],
            edges: vec![edge("a", "b", "relates_to", Some(0.8))],
        }
    }

    #[test]
    fn nodes_become_atoms_edges_become_relations() {
        let scene = scene_from_subgraph(&subgraph());
        assert_eq!(scene.atoms.len(), 2);
        assert_eq!(scene.relations.len(), 1);

        let a = &scene.atoms[0];
        assert_eq!(a.id, "a");
        assert_eq!(a.kind, "concept");
        assert_eq!(a.lifecycle, "present");
        assert_eq!(a.label.as_deref(), Some("Apple"));
        assert!(a.position.is_none(), "engine emits structure; SceneOS places");

        let r = &scene.relations[0];
        assert_eq!(r.source_id, "a");
        assert_eq!(r.target_id, "b");
        assert_eq!(r.kind, "relates_to");
        assert_eq!(r.metadata.get("confidence").map(String::as_str), Some("0.8"));
    }

    #[test]
    fn serializes_to_the_sceneos_wire_contract() {
        let scene = scene_from_subgraph(&subgraph());
        let json = serde_json::to_string(&scene).expect("serialize");
        // camelCase relation keys match the SceneOS TS contract.
        assert!(json.contains("\"sourceId\":\"a\""), "sourceId (not source_id)");
        assert!(json.contains("\"targetId\":\"b\""), "targetId (not target_id)");
        assert!(json.contains("\"lifecycle\":\"present\""));
        // Atom without a position omits the key (projection will add it).
        assert!(!json.contains("\"position\":null"));

        // Re-parse confirms a stable round-trip.
        let back: SceneScene = serde_json::from_str(&json).expect("round-trip");
        assert_eq!(back, scene);
    }
}
