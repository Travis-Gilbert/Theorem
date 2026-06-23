use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::atoms::{SceneAtom, SceneRelation};

pub const SCENE_PACKAGE_V2_VERSION: &str = "scene-package-v2";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionBinding {
    pub id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChromeBinding {
    pub id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransitionDescriptor {
    #[serde(rename = "from", skip_serializing_if = "Option::is_none")]
    pub from_package_id: Option<String>,
    pub choreography: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalStateArtifact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub svg: Option<String>,
    #[serde(rename = "json", skip_serializing_if = "Option::is_none")]
    pub json_payload: Option<BTreeMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionDescriptor {
    pub id: String,
    pub label: String,
    pub action_type: String,
    pub interaction: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub payload: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_confirmation: bool,
    #[serde(default = "default_true")]
    pub proposal_only: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenePackageV2 {
    #[serde(default = "default_version")]
    pub version: String,
    pub id: String,
    pub manifest_ref: String,
    pub atoms: Vec<SceneAtom>,
    pub relations: Vec<SceneRelation>,
    pub projection: ProjectionBinding,
    pub chrome: ChromeBinding,
    #[serde(default)]
    pub actions: Vec<ActionDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transitions: Option<TransitionDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_state: Option<TerminalStateArtifact>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provenance: BTreeMap<String, Value>,
}

fn default_version() -> String {
    SCENE_PACKAGE_V2_VERSION.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atoms::{AtomLifecycle, SceneAtom};

    #[test]
    fn package_serializes_to_scene_package_v2_wire_shape() {
        let package = ScenePackageV2 {
            version: SCENE_PACKAGE_V2_VERSION.to_string(),
            id: "pkg-1".to_string(),
            manifest_ref: "manifest-1".to_string(),
            atoms: vec![SceneAtom {
                id: "a".to_string(),
                kind: "evidence".to_string(),
                label: None,
                position: None,
                weight: None,
                color: None,
                opacity: None,
                glyph: None,
                scale: None,
                lifecycle: AtomLifecycle::Present,
                metadata: BTreeMap::new(),
                source_refs: Vec::new(),
            }],
            relations: Vec::new(),
            projection: ProjectionBinding {
                id: "patent_diagram".to_string(),
                params: BTreeMap::new(),
            },
            chrome: ChromeBinding {
                id: "patent_plate_shell".to_string(),
                params: BTreeMap::new(),
            },
            actions: Vec::new(),
            transitions: None,
            terminal_state: None,
            provenance: BTreeMap::new(),
        };

        let value = serde_json::to_value(package).expect("serialize package");
        assert_eq!(value["version"], SCENE_PACKAGE_V2_VERSION);
        assert_eq!(value["manifestRef"], "manifest-1");
        assert_eq!(value["projection"]["id"], "patent_diagram");
        assert!(value.get("terminalState").is_none());
    }
}
