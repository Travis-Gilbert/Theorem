use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CoordinateSpace {
    Graph,
    Geo,
    Timeline,
    Rank,
    Matrix,
    Diagram,
    Frame,
    Gallery,
    #[default]
    Freeform,
}

impl CoordinateSpace {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Graph => "graph",
            Self::Geo => "geo",
            Self::Timeline => "timeline",
            Self::Rank => "rank",
            Self::Matrix => "matrix",
            Self::Diagram => "diagram",
            Self::Frame => "frame",
            Self::Gallery => "gallery",
            Self::Freeform => "freeform",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AtomLifecycle {
    Entering,
    #[default]
    Present,
    Leaving,
    Terminal,
}

impl AtomLifecycle {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Entering => "entering",
            Self::Present => "present",
            Self::Leaving => "leaving",
            Self::Terminal => "terminal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtomPosition {
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub z: f64,
    #[serde(default)]
    pub space: CoordinateSpace,
}

fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub kind: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneAtom {
    pub id: String,
    #[serde(default = "default_atom_kind")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<AtomPosition>,
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
    #[serde(default)]
    pub lifecycle: AtomLifecycle,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<SourceRef>,
}

fn default_atom_kind() -> String {
    "evidence".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneRelation {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    #[serde(default = "default_relation_kind")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glyph: Option<String>,
    #[serde(default)]
    pub lifecycle: AtomLifecycle,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<SourceRef>,
}

fn default_relation_kind() -> String {
    "related".to_string()
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SceneScene {
    pub atoms: Vec<SceneAtom>,
    pub relations: Vec<SceneRelation>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn atom_relation_wire_contract_matches_sceneos_v2() {
        let atom = SceneAtom {
            id: "a".to_string(),
            kind: "evidence".to_string(),
            label: Some("Alpha".to_string()),
            position: None,
            weight: None,
            color: None,
            opacity: Some(0.8),
            glyph: None,
            scale: None,
            lifecycle: AtomLifecycle::Present,
            metadata: BTreeMap::new(),
            source_refs: vec![SourceRef {
                kind: "Object".to_string(),
                id: "42".to_string(),
                label: None,
                metadata: BTreeMap::from([("score".to_string(), json!(0.91))]),
            }],
        };
        let relation = SceneRelation {
            id: "a->b".to_string(),
            source_id: "a".to_string(),
            target_id: "b".to_string(),
            kind: "supports".to_string(),
            weight: Some(1.0),
            color: None,
            opacity: None,
            glyph: None,
            lifecycle: AtomLifecycle::Present,
            metadata: BTreeMap::new(),
            source_refs: Vec::new(),
        };

        let scene = SceneScene {
            atoms: vec![atom],
            relations: vec![relation],
        };
        let value = serde_json::to_value(&scene).expect("serialize scene");
        assert_eq!(value["atoms"][0]["lifecycle"], "present");
        assert!(value["atoms"][0].get("position").is_none());
        assert!(value["atoms"][0].get("sourceRefs").is_some());
        assert_eq!(value["relations"][0]["sourceId"], "a");
        assert_eq!(value["relations"][0]["targetId"], "b");

        let round_trip: SceneScene = serde_json::from_value(value).expect("round-trip scene");
        assert_eq!(round_trip, scene);
    }
}
