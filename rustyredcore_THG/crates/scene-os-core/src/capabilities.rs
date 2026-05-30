use serde::{Deserialize, Serialize};

use crate::atoms::CoordinateSpace;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProjectionRequirements {
    #[serde(default)]
    pub atom_fields: Vec<String>,
    #[serde(default)]
    pub relation_fields: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_atoms: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_atoms: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_shape: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProjectionAttributes {
    #[serde(default)]
    pub drives: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProjectionBudgets {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_atoms: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_relations: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_images: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_frames: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_payload_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_fps: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionCapability {
    pub id: String,
    pub label: String,
    pub coordinate_space: CoordinateSpace,
    #[serde(default)]
    pub requires: ProjectionRequirements,
    #[serde(default)]
    pub attributes: ProjectionAttributes,
    #[serde(default)]
    pub interactions: Vec<String>,
    #[serde(default = "default_full_replace")]
    pub patch_support: Vec<String>,
    #[serde(default)]
    pub budgets: ProjectionBudgets,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_projection: Option<String>,
    #[serde(default)]
    pub emits_terminal_state: bool,
}

fn default_full_replace() -> Vec<String> {
    vec!["full-replace".to_string()]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChromeCapability {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub affordances: Vec<String>,
    #[serde(default)]
    pub reserves_screen_regions: Vec<String>,
    #[serde(default)]
    pub pairs_with_projections: Vec<String>,
    #[serde(default = "default_full_replace")]
    pub patch_support: Vec<String>,
}
