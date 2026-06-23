use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::atoms::SceneScene;
use crate::catalogs::{
    mobile_projection_catalog, production_chrome_catalog, production_projection_catalog,
};
use crate::package::{ChromeBinding, ProjectionBinding, ScenePackageV2, SCENE_PACKAGE_V2_VERSION};
use crate::select::{
    classify_goal, detect_shape, select_chrome, select_projection, ChromeSelection, DataShape,
    DataShapeDetection, Goal, GoalDetection, ProjectionSelection,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneCompileInput {
    pub query: String,
    #[serde(default)]
    pub answer_type: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    pub scene: SceneScene,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub manifest_ref: Option<String>,
    #[serde(default)]
    pub provenance: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneCompileError {
    pub code: String,
    pub message: String,
}

impl SceneCompileError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for SceneCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SceneCompileError {}

pub fn compile_scene_package(
    input: SceneCompileInput,
) -> Result<ScenePackageV2, SceneCompileError> {
    if input.query.trim().is_empty() {
        return Err(SceneCompileError::new("empty_query", "query is required"));
    }

    let goal_detection = classify_goal(&input.query, None);
    let shape_detection = detect_shape(&input.scene.atoms, &input.scene.relations);
    let (goal_for_select, shape_for_select) = selector_overrides(
        input.answer_type.as_deref(),
        goal_detection.goal,
        shape_detection.shape,
    );

    let projection_catalog = projection_catalog_for_answer_type(input.answer_type.as_deref());
    let projection_selection =
        if let Some(projection_id) = explicit_projection_hint(input.answer_type.as_deref()) {
            explicit_projection_selection(projection_id, &projection_catalog)?
        } else {
            select_projection(goal_for_select, shape_for_select, &projection_catalog).map_err(
                |refusal| {
                    SceneCompileError::new(
                        "projection_select_failed",
                        format!("{}: {}", refusal.code, refusal.message),
                    )
                },
            )?
        };
    let selected_projection = projection_catalog
        .iter()
        .find(|projection| projection.id == projection_selection.projection_id)
        .ok_or_else(|| {
            SceneCompileError::new(
                "projection_missing",
                format!(
                    "selected projection {} was not present in catalog",
                    projection_selection.projection_id
                ),
            )
        })?;

    let chrome_catalog = production_chrome_catalog();
    let chrome_selection = select_chrome(goal_for_select, selected_projection, &chrome_catalog)
        .map_err(|refusal| {
            SceneCompileError::new(
                "chrome_select_failed",
                format!("{}: {}", refusal.code, refusal.message),
            )
        })?;

    let mut provenance = input.provenance;
    provenance.insert(
        "compileTrace".to_string(),
        serde_json::to_value(CompileTrace {
            goal: goal_detection,
            shape: shape_detection,
            projection_select: projection_selection.clone(),
            chrome_select: chrome_selection.clone(),
            selected_goal: goal_for_select,
            selected_shape: shape_for_select,
        })
        .expect("compile trace serializes"),
    );
    provenance.insert("director".to_string(), json!("scene-os-core"));

    let package_id = input.trace_id.as_ref().map_or_else(
        || {
            format!(
                "scene-{}-{}-{}",
                projection_selection.projection_id,
                input.scene.atoms.len(),
                input.scene.relations.len()
            )
        },
        |trace_id| format!("scene-{trace_id}"),
    );

    Ok(ScenePackageV2 {
        version: SCENE_PACKAGE_V2_VERSION.to_string(),
        id: package_id,
        manifest_ref: input.manifest_ref.unwrap_or_else(|| {
            input.trace_id.as_ref().map_or_else(
                || "theorem-rust-scene".to_string(),
                |trace_id| format!("manifest-{trace_id}"),
            )
        }),
        atoms: input.scene.atoms,
        relations: input.scene.relations,
        projection: ProjectionBinding {
            id: projection_selection.projection_id,
            params: BTreeMap::from([
                (
                    "coordinateSpace".to_string(),
                    json!(selected_projection.coordinate_space.as_str()),
                ),
                ("fit".to_string(), json!("content")),
            ]),
        },
        chrome: ChromeBinding {
            id: chrome_selection.chrome_id,
            params: BTreeMap::new(),
        },
        actions: Vec::new(),
        transitions: None,
        terminal_state: None,
        provenance,
    })
}

fn selector_overrides(
    answer_type: Option<&str>,
    goal: Goal,
    shape: DataShape,
) -> (Goal, DataShape) {
    match answer_type {
        Some("patent_diagram") | Some("patent_scene") => (Goal::ExplainProcess, DataShape::Dag),
        Some("tree_hierarchy") | Some("reconstruction_node_tree") => {
            (Goal::ExplainProcess, DataShape::Tree)
        }
        Some("numeric_series") => (Goal::Rank, DataShape::NumericSeries),
        Some("categorical_set") => (Goal::Summarize, DataShape::CategoricalSet),
        Some("flow_layered") | Some("sankey_flow") => (Goal::ExplainProcess, DataShape::Dag),
        Some("force_graph") | Some("fractal_expansion") => {
            (Goal::InspectEvidence, DataShape::DocumentSet)
        }
        Some("radial_rings") => (Goal::InspectEvidence, DataShape::DocumentSet),
        Some("tree_layout") => (Goal::ExplainProcess, DataShape::Tree),
        _ => (goal, shape),
    }
}

fn projection_catalog_for_answer_type(
    answer_type: Option<&str>,
) -> Vec<crate::capabilities::ProjectionCapability> {
    let mut catalog = production_projection_catalog();
    if matches!(
        answer_type,
        Some("force_graph" | "radial_rings" | "tree_layout" | "fractal_expansion")
    ) {
        catalog.extend(mobile_projection_catalog());
    }
    catalog
}

fn explicit_projection_hint(answer_type: Option<&str>) -> Option<&str> {
    match answer_type {
        Some(
            id @ ("patent_diagram" | "tree_hierarchy" | "numeric_series" | "categorical_set"
            | "flow_layered" | "sankey_flow" | "force_graph" | "radial_rings" | "tree_layout"
            | "fractal_expansion"),
        ) => Some(id),
        Some("patent_scene") => Some("patent_diagram"),
        Some("reconstruction_node_tree") => Some("tree_hierarchy"),
        _ => None,
    }
}

fn explicit_projection_selection(
    projection_id: &str,
    catalog: &[crate::capabilities::ProjectionCapability],
) -> Result<ProjectionSelection, SceneCompileError> {
    if !catalog
        .iter()
        .any(|projection| projection.id == projection_id)
    {
        return Err(SceneCompileError::new(
            "projection_hint_missing",
            format!("answer_type requested unknown projection {projection_id:?}"),
        ));
    }

    Ok(ProjectionSelection {
        projection_id: projection_id.to_string(),
        fallbacks: catalog
            .iter()
            .filter(|projection| projection.id != projection_id)
            .map(|projection| projection.id.clone())
            .collect(),
        rationale: format!("answer_type={projection_id}; explicit projection hint"),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompileTrace {
    goal: GoalDetection,
    shape: DataShapeDetection,
    projection_select: ProjectionSelection,
    chrome_select: ChromeSelection,
    selected_goal: Goal,
    selected_shape: DataShape,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atoms::{AtomLifecycle, SceneAtom, SceneRelation};

    #[test]
    fn compiles_patent_scene_package_with_trace() {
        let scene = SceneScene {
            atoms: vec![atom("a"), atom("b")],
            relations: vec![relation("a", "b")],
        };
        let package = compile_scene_package(SceneCompileInput {
            query: "How does the browser scene work?".to_string(),
            answer_type: Some("patent_diagram".to_string()),
            title: Some("Browser Scene".to_string()),
            scene,
            trace_id: Some("trace-1".to_string()),
            manifest_ref: None,
            provenance: BTreeMap::new(),
        })
        .expect("compile patent scene");

        assert_eq!(package.version, SCENE_PACKAGE_V2_VERSION);
        assert_eq!(package.id, "scene-trace-1");
        assert_eq!(package.manifest_ref, "manifest-trace-1");
        assert_eq!(package.projection.id, "patent_diagram");
        assert_eq!(package.chrome.id, "patent_plate_shell");
        assert!(package.provenance.get("compileTrace").is_some());
        assert_eq!(package.atoms.len(), 2);

        let value = serde_json::to_value(package).expect("serialize package");
        assert_eq!(value["manifestRef"], "manifest-trace-1");
        assert_eq!(value["projection"]["params"]["coordinateSpace"], "diagram");
        assert_eq!(
            value["provenance"]["compileTrace"]["selectedGoal"],
            "explain-process"
        );
    }

    #[test]
    fn explicit_tree_answer_type_selects_tree_projection_and_document_rail() {
        let scene = SceneScene {
            atoms: vec![atom("root"), atom("child")],
            relations: vec![relation("root", "child")],
        };
        let package = compile_scene_package(SceneCompileInput {
            query: "Show me the browser substrate scene.".to_string(),
            answer_type: Some("tree_hierarchy".to_string()),
            title: None,
            scene,
            trace_id: Some("tree-1".to_string()),
            manifest_ref: None,
            provenance: BTreeMap::new(),
        })
        .expect("compile tree hierarchy scene");

        assert_eq!(package.projection.id, "tree_hierarchy");
        assert_eq!(package.chrome.id, "document_rail");

        let value = serde_json::to_value(package).expect("serialize package");
        assert_eq!(value["provenance"]["compileTrace"]["selectedShape"], "tree");
        assert!(
            value["provenance"]["compileTrace"]["projectionSelect"]["rationale"]
                .as_str()
                .unwrap()
                .contains("explicit projection hint")
        );
    }

    #[test]
    fn explicit_mobile_projection_selects_dynamic_island_shell() {
        let scene = SceneScene {
            atoms: vec![atom("root"), atom("child")],
            relations: vec![relation("root", "child")],
        };
        let package = compile_scene_package(SceneCompileInput {
            query: "Find the substrate center.".to_string(),
            answer_type: Some("force_graph".to_string()),
            title: None,
            scene,
            trace_id: Some("force-1".to_string()),
            manifest_ref: None,
            provenance: BTreeMap::new(),
        })
        .expect("compile mobile force graph scene");

        assert_eq!(package.projection.id, "force_graph");
        assert_eq!(package.chrome.id, "dynamic_island_shell");
        assert_eq!(package.projection.params["coordinateSpace"], "graph");
    }

    fn atom(id: &str) -> SceneAtom {
        SceneAtom {
            id: id.to_string(),
            kind: "evidence".to_string(),
            label: Some(id.to_string()),
            position: None,
            weight: None,
            color: None,
            opacity: None,
            glyph: None,
            scale: None,
            lifecycle: AtomLifecycle::Present,
            metadata: BTreeMap::new(),
            source_refs: Vec::new(),
        }
    }

    fn relation(source: &str, target: &str) -> SceneRelation {
        SceneRelation {
            id: format!("{source}->{target}"),
            source_id: source.to_string(),
            target_id: target.to_string(),
            kind: "related".to_string(),
            weight: Some(1.0),
            color: None,
            opacity: None,
            glyph: None,
            lifecycle: AtomLifecycle::Present,
            metadata: BTreeMap::new(),
            source_refs: Vec::new(),
        }
    }
}
