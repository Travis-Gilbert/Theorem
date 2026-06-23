use crate::atoms::CoordinateSpace;
use crate::capabilities::{
    ChromeCapability, ProjectionAttributes, ProjectionBudgets, ProjectionCapability,
    ProjectionRequirements,
};

pub fn production_projection_catalog() -> Vec<ProjectionCapability> {
    vec![
        ProjectionCapability {
            id: "patent_diagram".to_string(),
            label: "Patent Diagram".to_string(),
            coordinate_space: CoordinateSpace::Diagram,
            requires: ProjectionRequirements {
                atom_fields: strings(&["id", "label"]),
                relation_fields: strings(&["id", "source_id", "target_id"]),
                min_atoms: Some(2),
                max_atoms: Some(60),
                source_shape: Some("patent_scene".to_string()),
            },
            attributes: ProjectionAttributes {
                drives: strings(&["position", "glyph", "opacity"]),
            },
            interactions: strings(&[
                "select",
                "hover",
                "annotate",
                "open-evidence",
                "save",
                "ask-follow-up",
            ]),
            patch_support: strings(&["full-replace", "atom-update", "state-update"]),
            budgets: ProjectionBudgets {
                max_atoms: Some(60),
                max_relations: Some(120),
                max_payload_bytes: Some(64_000),
                expected_fps: Some(30),
                ..ProjectionBudgets::default()
            },
            fallback_projection: None,
            emits_terminal_state: true,
        },
        ProjectionCapability {
            id: "tree_hierarchy".to_string(),
            label: "Tree Hierarchy".to_string(),
            coordinate_space: CoordinateSpace::Diagram,
            requires: ProjectionRequirements {
                atom_fields: strings(&["id", "label", "kind"]),
                relation_fields: strings(&["id", "source_id", "target_id"]),
                min_atoms: Some(1),
                max_atoms: Some(400),
                source_shape: Some("reconstruction_node_tree".to_string()),
            },
            attributes: ProjectionAttributes {
                drives: strings(&["position"]),
            },
            interactions: strings(&["select", "hover", "zoom", "open-evidence", "save"]),
            patch_support: strings(&["full-replace", "atom-update", "state-update"]),
            budgets: ProjectionBudgets {
                max_atoms: Some(400),
                max_relations: Some(800),
                max_payload_bytes: Some(128_000),
                expected_fps: Some(30),
                ..ProjectionBudgets::default()
            },
            fallback_projection: None,
            emits_terminal_state: true,
        },
        ProjectionCapability {
            id: "numeric_series".to_string(),
            label: "Numeric Series".to_string(),
            coordinate_space: CoordinateSpace::Rank,
            requires: ProjectionRequirements {
                atom_fields: strings(&["id"]),
                relation_fields: Vec::new(),
                min_atoms: Some(1),
                max_atoms: Some(1_000),
                source_shape: Some("numeric_series".to_string()),
            },
            attributes: ProjectionAttributes {
                drives: strings(&["position"]),
            },
            interactions: strings(&[
                "select",
                "hover",
                "zoom",
                "compare",
                "open-evidence",
                "save",
            ]),
            patch_support: strings(&["full-replace", "atom-update", "state-update"]),
            budgets: ProjectionBudgets {
                max_atoms: Some(1_000),
                max_relations: Some(1_000),
                max_payload_bytes: Some(128_000),
                expected_fps: Some(30),
                ..ProjectionBudgets::default()
            },
            fallback_projection: None,
            emits_terminal_state: true,
        },
        ProjectionCapability {
            id: "categorical_set".to_string(),
            label: "Categorical Set".to_string(),
            coordinate_space: CoordinateSpace::Matrix,
            requires: ProjectionRequirements {
                atom_fields: strings(&["id", "kind"]),
                relation_fields: Vec::new(),
                min_atoms: Some(1),
                max_atoms: Some(1_000),
                source_shape: Some("categorical_set".to_string()),
            },
            attributes: ProjectionAttributes {
                drives: strings(&["position"]),
            },
            interactions: strings(&["select", "hover", "filter", "open-evidence", "save"]),
            patch_support: strings(&["full-replace", "atom-update", "state-update"]),
            budgets: ProjectionBudgets {
                max_atoms: Some(1_000),
                max_relations: Some(1_000),
                max_payload_bytes: Some(128_000),
                expected_fps: Some(30),
                ..ProjectionBudgets::default()
            },
            fallback_projection: None,
            emits_terminal_state: true,
        },
        ProjectionCapability {
            id: "flow_layered".to_string(),
            label: "Layered Flow".to_string(),
            coordinate_space: CoordinateSpace::Diagram,
            requires: ProjectionRequirements {
                atom_fields: strings(&["id", "label"]),
                relation_fields: strings(&["id", "source_id", "target_id"]),
                min_atoms: Some(1),
                max_atoms: Some(400),
                source_shape: Some("flow_dag".to_string()),
            },
            attributes: ProjectionAttributes {
                drives: strings(&["position"]),
            },
            interactions: strings(&["select", "hover", "zoom", "open-evidence", "save"]),
            patch_support: strings(&["full-replace", "atom-update", "state-update"]),
            budgets: ProjectionBudgets {
                max_atoms: Some(400),
                max_relations: Some(1_200),
                max_payload_bytes: Some(128_000),
                expected_fps: Some(30),
                ..ProjectionBudgets::default()
            },
            fallback_projection: None,
            emits_terminal_state: true,
        },
        ProjectionCapability {
            id: "sankey_flow".to_string(),
            label: "Sankey Flow".to_string(),
            coordinate_space: CoordinateSpace::Diagram,
            requires: ProjectionRequirements {
                atom_fields: strings(&["id", "label"]),
                relation_fields: strings(&["id", "source_id", "target_id"]),
                min_atoms: Some(2),
                max_atoms: Some(400),
                source_shape: Some("weighted_flow".to_string()),
            },
            attributes: ProjectionAttributes {
                drives: strings(&["position"]),
            },
            interactions: strings(&["select", "hover", "zoom", "open-evidence", "save"]),
            patch_support: strings(&["full-replace", "atom-update", "state-update"]),
            budgets: ProjectionBudgets {
                max_atoms: Some(400),
                max_relations: Some(1_200),
                max_payload_bytes: Some(128_000),
                expected_fps: Some(30),
                ..ProjectionBudgets::default()
            },
            fallback_projection: None,
            emits_terminal_state: true,
        },
    ]
}

pub fn mobile_projection_catalog() -> Vec<ProjectionCapability> {
    vec![
        ProjectionCapability {
            id: "force_graph".to_string(),
            label: "Force Graph".to_string(),
            coordinate_space: CoordinateSpace::Graph,
            requires: ProjectionRequirements {
                atom_fields: strings(&["id", "label", "weight", "metadata.matchScore"]),
                relation_fields: strings(&["id", "source_id", "target_id"]),
                min_atoms: Some(2),
                max_atoms: Some(400),
                source_shape: Some("web_neighborhood".to_string()),
            },
            attributes: ProjectionAttributes {
                drives: strings(&["position", "scale", "opacity"]),
            },
            interactions: strings(&[
                "select",
                "hover",
                "drag",
                "zoom",
                "open-evidence",
                "save",
                "ask-follow-up",
            ]),
            patch_support: strings(&["full-replace", "atom-update", "state-update"]),
            budgets: ProjectionBudgets {
                max_atoms: Some(400),
                max_relations: Some(1_200),
                max_payload_bytes: Some(160_000),
                expected_fps: Some(60),
                ..ProjectionBudgets::default()
            },
            fallback_projection: None,
            emits_terminal_state: false,
        },
        ProjectionCapability {
            id: "radial_rings".to_string(),
            label: "Radial Rings".to_string(),
            coordinate_space: CoordinateSpace::Graph,
            requires: ProjectionRequirements {
                atom_fields: strings(&["id", "metadata.ring", "metadata.matchScore"]),
                relation_fields: strings(&["id", "source_id", "target_id"]),
                min_atoms: Some(1),
                max_atoms: Some(400),
                source_shape: Some("web_neighborhood_rings".to_string()),
            },
            attributes: ProjectionAttributes {
                drives: strings(&["position", "scale"]),
            },
            interactions: strings(&["select", "hover", "zoom", "open-evidence", "save"]),
            patch_support: strings(&["full-replace", "atom-update", "state-update"]),
            budgets: ProjectionBudgets {
                max_atoms: Some(400),
                max_relations: Some(1_200),
                max_payload_bytes: Some(160_000),
                expected_fps: Some(60),
                ..ProjectionBudgets::default()
            },
            fallback_projection: Some("force_graph".to_string()),
            emits_terminal_state: false,
        },
        ProjectionCapability {
            id: "tree_layout".to_string(),
            label: "Tree Layout".to_string(),
            coordinate_space: CoordinateSpace::Diagram,
            requires: ProjectionRequirements {
                atom_fields: strings(&["id", "metadata.matchScore"]),
                relation_fields: strings(&["id", "source_id", "target_id"]),
                min_atoms: Some(1),
                max_atoms: Some(400),
                source_shape: Some("rooted_tree".to_string()),
            },
            attributes: ProjectionAttributes {
                drives: strings(&["position"]),
            },
            interactions: strings(&["select", "hover", "zoom", "open-evidence", "save"]),
            patch_support: strings(&["full-replace", "atom-update", "state-update"]),
            budgets: ProjectionBudgets {
                max_atoms: Some(400),
                max_relations: Some(399),
                max_payload_bytes: Some(128_000),
                expected_fps: Some(60),
                ..ProjectionBudgets::default()
            },
            fallback_projection: Some("radial_rings".to_string()),
            emits_terminal_state: false,
        },
        ProjectionCapability {
            id: "fractal_expansion".to_string(),
            label: "Fractal Expansion".to_string(),
            coordinate_space: CoordinateSpace::Graph,
            requires: ProjectionRequirements {
                atom_fields: strings(&["id", "metadata.ring", "metadata.matchScore"]),
                relation_fields: strings(&["id", "source_id", "target_id"]),
                min_atoms: Some(2),
                max_atoms: Some(400),
                source_shape: Some("web_neighborhood_push_trace".to_string()),
            },
            attributes: ProjectionAttributes {
                drives: strings(&["position", "opacity", "scale"]),
            },
            interactions: strings(&[
                "select",
                "hover",
                "replay",
                "open-evidence",
                "save",
                "ask-follow-up",
            ]),
            patch_support: strings(&["full-replace", "atom-update", "state-update"]),
            budgets: ProjectionBudgets {
                max_atoms: Some(400),
                max_relations: Some(1_200),
                max_payload_bytes: Some(192_000),
                expected_fps: Some(60),
                ..ProjectionBudgets::default()
            },
            fallback_projection: Some("force_graph".to_string()),
            emits_terminal_state: false,
        },
    ]
}

pub fn production_chrome_catalog() -> Vec<ChromeCapability> {
    vec![
        ChromeCapability {
            id: "patent_plate_shell".to_string(),
            label: "Patent Plate Shell".to_string(),
            affordances: strings(&["narration", "document-rail"]),
            reserves_screen_regions: Vec::new(),
            pairs_with_projections: strings(&["patent_diagram"]),
            patch_support: strings(&["full-replace", "state-update"]),
        },
        ChromeCapability {
            id: "document_rail".to_string(),
            label: "Document Rail".to_string(),
            affordances: strings(&["document-rail"]),
            reserves_screen_regions: Vec::new(),
            pairs_with_projections: strings(&["tree_hierarchy"]),
            patch_support: strings(&["full-replace", "state-update"]),
        },
        ChromeCapability {
            id: "dynamic_island_shell".to_string(),
            label: "Dynamic Island Shell".to_string(),
            affordances: strings(&["search", "ask", "projection-switcher", "node-detail"]),
            reserves_screen_regions: strings(&["top-floating-island"]),
            pairs_with_projections: strings(&[
                "force_graph",
                "radial_rings",
                "tree_layout",
                "fractal_expansion",
            ]),
            patch_support: strings(&["full-replace", "atom-update", "state-update"]),
        },
    ]
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_catalogs_match_the_sceneos_lane_a_plan() {
        let projections = production_projection_catalog();
        let chromes = production_chrome_catalog();
        assert_eq!(projections.len(), 6);
        assert_eq!(chromes.len(), 3);
        assert_eq!(projections[0].id, "patent_diagram");
        assert_eq!(projections[5].id, "sankey_flow");
        assert_eq!(chromes[0].id, "patent_plate_shell");
        assert_eq!(chromes[1].pairs_with_projections, vec!["tree_hierarchy"]);
    }

    #[test]
    fn mobile_catalog_registers_the_ios_v1_algorithm_set() {
        let projections = mobile_projection_catalog();
        assert_eq!(
            projections
                .iter()
                .map(|projection| projection.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "force_graph",
                "radial_rings",
                "tree_layout",
                "fractal_expansion"
            ]
        );
        assert!(projections
            .iter()
            .all(|projection| projection.budgets.expected_fps == Some(60)));
    }
}
