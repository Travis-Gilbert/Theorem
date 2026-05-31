use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::atoms::{CoordinateSpace, SceneAtom, SceneRelation};
use crate::capabilities::{ChromeCapability, ProjectionCapability};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Goal {
    Compare,
    Locate,
    ExplainProcess,
    InspectEvidence,
    Rank,
    FindPattern,
    TellStory,
    Summarize,
    Navigate,
    Simulate,
}

impl Goal {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Compare => "compare",
            Self::Locate => "locate",
            Self::ExplainProcess => "explain-process",
            Self::InspectEvidence => "inspect-evidence",
            Self::Rank => "rank",
            Self::FindPattern => "find-pattern",
            Self::TellStory => "tell-story",
            Self::Summarize => "summarize",
            Self::Navigate => "navigate",
            Self::Simulate => "simulate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalDetection {
    pub goal: Goal,
    pub confidence: f64,
    pub fallbacks: Vec<Goal>,
    pub rationale: String,
}

pub fn classify_goal(
    question: &str,
    retrieved_summary: Option<&BTreeMap<String, String>>,
) -> GoalDetection {
    let text = question.to_lowercase();
    let text = text.trim();

    if text.is_empty() {
        return goal_detection(Goal::InspectEvidence, 0.4, "empty-question");
    }

    for (goal, keywords) in goal_keywords() {
        for keyword in &keywords {
            if text.contains(keyword) {
                let base = 0.55 + (keyword.len() as f64 / 80.0).min(0.15);
                let hits = keywords
                    .iter()
                    .filter(|candidate| text.contains(**candidate))
                    .count();
                let confidence = (base + 0.1 * hits.saturating_sub(1) as f64).min(0.95);
                return GoalDetection {
                    goal,
                    confidence,
                    fallbacks: goal_fallbacks(goal),
                    rationale: format!("keyword:{keyword}"),
                };
            }
        }
    }

    if let Some(summary) = retrieved_summary {
        match summary.get("dominant_shape").map(String::as_str) {
            Some("geo_set") => {
                return GoalDetection {
                    goal: Goal::Locate,
                    confidence: 0.45,
                    fallbacks: goal_fallbacks(Goal::Locate),
                    rationale: "summary:dominant_shape=geo_set".to_string(),
                };
            }
            Some("network" | "graph" | "process_graph") => {
                return GoalDetection {
                    goal: Goal::ExplainProcess,
                    confidence: 0.45,
                    fallbacks: goal_fallbacks(Goal::ExplainProcess),
                    rationale: "summary:dominant_shape=network".to_string(),
                };
            }
            Some("comparison_matrix") => {
                return GoalDetection {
                    goal: Goal::Compare,
                    confidence: 0.45,
                    fallbacks: goal_fallbacks(Goal::Compare),
                    rationale: "summary:dominant_shape=comparison_matrix".to_string(),
                };
            }
            _ => {}
        }
    }

    goal_detection(Goal::InspectEvidence, 0.35, "default")
}

fn goal_detection(goal: Goal, confidence: f64, rationale: &str) -> GoalDetection {
    GoalDetection {
        goal,
        confidence,
        fallbacks: goal_fallbacks(goal),
        rationale: rationale.to_string(),
    }
}

fn goal_keywords() -> Vec<(Goal, Vec<&'static str>)> {
    vec![
        (
            Goal::Compare,
            vec![
                "compare",
                "versus",
                "vs.",
                "vs ",
                "trade-off",
                "tradeoff",
                "which is better",
            ],
        ),
        (
            Goal::Locate,
            vec![
                "where is",
                "where are",
                "locate",
                "near",
                "nearby",
                "in which city",
                "in which country",
            ],
        ),
        (
            Goal::ExplainProcess,
            vec![
                "how does",
                "how do",
                "how is",
                "explain how",
                "what is the process",
                "walk me through",
            ],
        ),
        (
            Goal::Rank,
            vec![
                "rank ",
                "top ",
                "best ",
                "worst ",
                "highest ",
                "lowest ",
                "leaderboard",
            ],
        ),
        (
            Goal::FindPattern,
            vec!["pattern", "trend", "anomaly", "outlier", "cluster"],
        ),
        (
            Goal::TellStory,
            vec![
                "story of",
                "history of",
                "timeline",
                "what happened",
                "trajectory",
            ],
        ),
        (
            Goal::Summarize,
            vec!["summarize", "summary", "tldr", "tl;dr", "in short"],
        ),
        (
            Goal::Navigate,
            vec!["show me ", "open the ", "go to ", "navigate"],
        ),
        (
            Goal::Simulate,
            vec!["simulate", "what if", "counterfactual", "could we have"],
        ),
        (
            Goal::InspectEvidence,
            vec![
                "what evidence",
                "which sources",
                "show evidence",
                "show sources",
                "provenance",
            ],
        ),
    ]
}

fn goal_fallbacks(goal: Goal) -> Vec<Goal> {
    match goal {
        Goal::Compare => vec![Goal::Rank, Goal::InspectEvidence],
        Goal::Locate => vec![Goal::InspectEvidence],
        Goal::ExplainProcess => vec![Goal::Summarize, Goal::InspectEvidence],
        Goal::InspectEvidence => Vec::new(),
        Goal::Rank => vec![Goal::Compare, Goal::InspectEvidence],
        Goal::FindPattern => vec![Goal::InspectEvidence],
        Goal::TellStory => vec![Goal::Summarize, Goal::InspectEvidence],
        Goal::Summarize => vec![Goal::InspectEvidence],
        Goal::Navigate => vec![Goal::InspectEvidence],
        Goal::Simulate => vec![Goal::ExplainProcess, Goal::InspectEvidence],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataShape {
    GeoPointed,
    GeoRegioned,
    Timeline,
    Dag,
    Tree,
    MatrixRowsCols,
    ImageSet,
    DocumentSet,
    NumericSeries,
    CategoricalSet,
    Mixed,
}

impl DataShape {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GeoPointed => "geo-pointed",
            Self::GeoRegioned => "geo-regioned",
            Self::Timeline => "timeline",
            Self::Dag => "dag",
            Self::Tree => "tree",
            Self::MatrixRowsCols => "matrix-rows-cols",
            Self::ImageSet => "image-set",
            Self::DocumentSet => "document-set",
            Self::NumericSeries => "numeric-series",
            Self::CategoricalSet => "categorical-set",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataShapeDetection {
    pub shape: DataShape,
    pub confidence: f64,
    pub required_fields: Vec<String>,
    pub fallbacks: Vec<DataShape>,
    pub rationale: String,
}

pub fn detect_shape(atoms: &[SceneAtom], relations: &[SceneRelation]) -> DataShapeDetection {
    if atoms.is_empty() {
        return DataShapeDetection {
            shape: DataShape::Mixed,
            confidence: 0.0,
            required_fields: Vec::new(),
            fallbacks: Vec::new(),
            rationale: "empty".to_string(),
        };
    }

    let total = atoms.len();
    let geo_count = count_space(atoms, CoordinateSpace::Geo);
    let timeline_count = count_space(atoms, CoordinateSpace::Timeline);
    let matrix_count = count_space(atoms, CoordinateSpace::Matrix);
    let diagram_count = count_space(atoms, CoordinateSpace::Diagram);
    let image_kind = atoms
        .iter()
        .filter(|atom| atom.kind.to_lowercase().contains("image"))
        .count();
    let document_kind = atoms
        .iter()
        .filter(|atom| {
            matches!(
                atom.kind.to_lowercase().as_str(),
                "document" | "source" | "evidence"
            )
        })
        .count();

    if geo_count > 0 && ratio(geo_count, total) >= 0.6 {
        let regioned = atoms
            .iter()
            .filter(|atom| atom.metadata.contains_key("geometry"))
            .count();
        if regioned > 0 && ratio(regioned, geo_count) >= 0.5 {
            return shape_detection(
                DataShape::GeoRegioned,
                0.85,
                &format!("geo_regioned:{regioned}/{geo_count}"),
            );
        }
        return shape_detection(
            DataShape::GeoPointed,
            0.85,
            &format!("geo_pointed:{geo_count}/{total}"),
        );
    }

    if timeline_count > 0 && ratio(timeline_count, total) >= 0.6 {
        return shape_detection(
            DataShape::Timeline,
            0.8,
            &format!("timeline:{timeline_count}/{total}"),
        );
    }

    if matrix_count > 0 && ratio(matrix_count, total) >= 0.6 {
        return shape_detection(
            DataShape::MatrixRowsCols,
            0.8,
            &format!("matrix:{matrix_count}/{total}"),
        );
    }

    if !relations.is_empty() && diagram_count > 0 && ratio(diagram_count, total) >= 0.5 {
        if is_tree(atoms, relations) {
            return shape_detection(
                DataShape::Tree,
                0.7,
                &format!("tree:{}-edges", relations.len()),
            );
        }
        return shape_detection(
            DataShape::Dag,
            0.7,
            &format!("dag:{}-edges", relations.len()),
        );
    }

    if image_kind > 0 && ratio(image_kind, total) >= 0.5 {
        return shape_detection(
            DataShape::ImageSet,
            0.75,
            &format!("image_set:{image_kind}/{total}"),
        );
    }

    if document_kind > 0 && ratio(document_kind, total) >= 0.5 {
        return shape_detection(
            DataShape::DocumentSet,
            0.7,
            &format!("document_set:{document_kind}/{total}"),
        );
    }

    if !relations.is_empty() && relations.len() > total / 2 {
        return shape_detection(
            DataShape::Dag,
            0.55,
            &format!("network_dag:{}-edges", relations.len()),
        );
    }

    let weight_count = atoms.iter().filter(|atom| atom.weight.is_some()).count();
    if weight_count == total {
        return shape_detection(
            DataShape::NumericSeries,
            0.6,
            &format!("numeric_series:{weight_count}/{total}"),
        );
    }

    shape_detection(
        DataShape::CategoricalSet,
        0.5,
        &format!("categorical_default:{total}"),
    )
}

fn count_space(atoms: &[SceneAtom], space: CoordinateSpace) -> usize {
    atoms
        .iter()
        .filter(|atom| atom.position.as_ref().map(|position| position.space) == Some(space))
        .count()
}

fn ratio(part: usize, total: usize) -> f64 {
    part as f64 / total.max(1) as f64
}

fn is_tree(atoms: &[SceneAtom], relations: &[SceneRelation]) -> bool {
    let mut in_degree: BTreeMap<&str, usize> = atoms
        .iter()
        .map(|atom| (atom.id.as_str(), 0usize))
        .collect();
    for relation in relations {
        if let Some(degree) = in_degree.get_mut(relation.target_id.as_str()) {
            *degree += 1;
        }
    }
    let roots = in_degree.values().filter(|degree| **degree == 0).count();
    let multi_parent = in_degree.values().filter(|degree| **degree > 1).count();
    roots == 1 && multi_parent == 0
}

fn shape_detection(shape: DataShape, confidence: f64, rationale: &str) -> DataShapeDetection {
    DataShapeDetection {
        shape,
        confidence,
        required_fields: shape_required_fields(shape),
        fallbacks: shape_fallbacks(shape),
        rationale: rationale.to_string(),
    }
}

fn shape_required_fields(shape: DataShape) -> Vec<String> {
    match shape {
        DataShape::GeoPointed => strings(&["position.x", "position.y"]),
        DataShape::GeoRegioned => strings(&["metadata.geometry"]),
        DataShape::Timeline => strings(&["position.x"]),
        DataShape::Dag => strings(&["kind"]),
        DataShape::Tree => strings(&["kind", "metadata.parent_id"]),
        DataShape::MatrixRowsCols => strings(&["position.x", "position.y"]),
        DataShape::ImageSet => strings(&["source_refs"]),
        DataShape::DocumentSet => strings(&["source_refs"]),
        DataShape::NumericSeries => strings(&["weight"]),
        DataShape::CategoricalSet => strings(&["kind"]),
        DataShape::Mixed => Vec::new(),
    }
}

fn shape_fallbacks(shape: DataShape) -> Vec<DataShape> {
    match shape {
        DataShape::GeoPointed => vec![DataShape::CategoricalSet, DataShape::DocumentSet],
        DataShape::GeoRegioned => vec![DataShape::GeoPointed, DataShape::CategoricalSet],
        DataShape::Timeline => vec![DataShape::NumericSeries, DataShape::CategoricalSet],
        DataShape::Dag => vec![DataShape::Tree, DataShape::CategoricalSet],
        DataShape::Tree => vec![DataShape::Dag, DataShape::CategoricalSet],
        DataShape::MatrixRowsCols => vec![DataShape::CategoricalSet],
        DataShape::ImageSet => vec![DataShape::DocumentSet, DataShape::CategoricalSet],
        DataShape::DocumentSet => vec![DataShape::CategoricalSet],
        DataShape::NumericSeries => vec![DataShape::CategoricalSet],
        DataShape::CategoricalSet => Vec::new(),
        DataShape::Mixed => vec![DataShape::CategoricalSet, DataShape::DocumentSet],
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionSelection {
    pub projection_id: String,
    pub fallbacks: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionSelectionRefusal {
    pub code: String,
    pub message: String,
}

pub fn select_projection(
    goal: Goal,
    shape: DataShape,
    catalog: &[ProjectionCapability],
) -> Result<ProjectionSelection, ProjectionSelectionRefusal> {
    if catalog.is_empty() {
        return Err(ProjectionSelectionRefusal {
            code: "empty_catalog".to_string(),
            message: "No projections registered in the catalog.".to_string(),
        });
    }

    let allowed_spaces = shape_allowed_spaces(shape);
    let preferences = goal_shape_preferences(goal, shape).unwrap_or_else(|| allowed_spaces.clone());
    let mut hard_passers: Vec<&ProjectionCapability> = catalog
        .iter()
        .filter(|projection| allowed_spaces.contains(&projection.coordinate_space))
        .collect();

    if hard_passers.is_empty() {
        return Err(ProjectionSelectionRefusal {
            code: "no_compatible_projection".to_string(),
            message: format!(
                "No projection in the catalog accepts shape {:?} (allowed spaces: {:?}).",
                shape, allowed_spaces
            ),
        });
    }

    hard_passers
        .sort_by_key(|projection| preference_rank(&preferences, projection.coordinate_space));
    let finalist = hard_passers[0];
    let fallbacks = hard_passers[1..]
        .iter()
        .map(|projection| projection.id.clone())
        .collect();

    Ok(ProjectionSelection {
        projection_id: finalist.id.clone(),
        fallbacks,
        rationale: format!(
            "goal={}; shape={}; preferences={:?}; finalist_space={}",
            goal.as_str(),
            shape.as_str(),
            preferences
                .iter()
                .map(|space| space.as_str())
                .collect::<Vec<_>>(),
            finalist.coordinate_space.as_str()
        ),
    })
}

fn shape_allowed_spaces(shape: DataShape) -> Vec<CoordinateSpace> {
    match shape {
        DataShape::GeoPointed => vec![CoordinateSpace::Geo, CoordinateSpace::Graph],
        DataShape::GeoRegioned => vec![CoordinateSpace::Geo],
        DataShape::Timeline => vec![CoordinateSpace::Timeline, CoordinateSpace::Frame],
        DataShape::Dag => vec![CoordinateSpace::Diagram, CoordinateSpace::Graph],
        DataShape::Tree => vec![CoordinateSpace::Diagram, CoordinateSpace::Graph],
        DataShape::MatrixRowsCols => vec![CoordinateSpace::Matrix, CoordinateSpace::Graph],
        DataShape::ImageSet => vec![CoordinateSpace::Gallery, CoordinateSpace::Freeform],
        DataShape::DocumentSet => vec![
            CoordinateSpace::Gallery,
            CoordinateSpace::Graph,
            CoordinateSpace::Freeform,
        ],
        DataShape::NumericSeries => {
            vec![
                CoordinateSpace::Rank,
                CoordinateSpace::Timeline,
                CoordinateSpace::Matrix,
            ]
        }
        DataShape::CategoricalSet => vec![
            CoordinateSpace::Graph,
            CoordinateSpace::Rank,
            CoordinateSpace::Matrix,
            CoordinateSpace::Freeform,
        ],
        DataShape::Mixed => vec![CoordinateSpace::Graph, CoordinateSpace::Freeform],
    }
}

fn goal_shape_preferences(goal: Goal, shape: DataShape) -> Option<Vec<CoordinateSpace>> {
    match (goal, shape) {
        (Goal::Locate, DataShape::GeoPointed | DataShape::GeoRegioned) => {
            Some(vec![CoordinateSpace::Geo])
        }
        (Goal::Compare, DataShape::MatrixRowsCols) => Some(vec![CoordinateSpace::Matrix]),
        (Goal::Compare, DataShape::NumericSeries) => {
            Some(vec![CoordinateSpace::Matrix, CoordinateSpace::Rank])
        }
        (Goal::Compare, DataShape::CategoricalSet) => {
            Some(vec![CoordinateSpace::Matrix, CoordinateSpace::Graph])
        }
        (Goal::Rank, DataShape::NumericSeries | DataShape::CategoricalSet) => {
            Some(vec![CoordinateSpace::Rank, CoordinateSpace::Matrix])
        }
        (Goal::ExplainProcess, DataShape::Dag | DataShape::Tree) => {
            Some(vec![CoordinateSpace::Diagram, CoordinateSpace::Graph])
        }
        (Goal::ExplainProcess, DataShape::Timeline) => {
            Some(vec![CoordinateSpace::Frame, CoordinateSpace::Timeline])
        }
        (Goal::TellStory, DataShape::Timeline) => {
            Some(vec![CoordinateSpace::Frame, CoordinateSpace::Timeline])
        }
        (Goal::TellStory, DataShape::Dag) => {
            Some(vec![CoordinateSpace::Frame, CoordinateSpace::Diagram])
        }
        (Goal::Summarize, DataShape::Dag) => Some(vec![CoordinateSpace::Diagram]),
        (Goal::Summarize, DataShape::DocumentSet) => {
            Some(vec![CoordinateSpace::Gallery, CoordinateSpace::Graph])
        }
        (Goal::InspectEvidence, DataShape::ImageSet) => Some(vec![CoordinateSpace::Gallery]),
        (Goal::InspectEvidence, DataShape::DocumentSet) => {
            Some(vec![CoordinateSpace::Graph, CoordinateSpace::Gallery])
        }
        (Goal::InspectEvidence, DataShape::Mixed) => Some(vec![CoordinateSpace::Graph]),
        (Goal::Navigate, DataShape::GeoPointed) => Some(vec![CoordinateSpace::Geo]),
        (Goal::FindPattern, DataShape::NumericSeries) => {
            Some(vec![CoordinateSpace::Matrix, CoordinateSpace::Rank])
        }
        (Goal::Simulate, DataShape::Dag) => {
            Some(vec![CoordinateSpace::Diagram, CoordinateSpace::Graph])
        }
        _ => None,
    }
}

fn preference_rank(preferences: &[CoordinateSpace], space: CoordinateSpace) -> usize {
    preferences
        .iter()
        .position(|preference| *preference == space)
        .unwrap_or(preferences.len() + 1)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChromeSelection {
    pub chrome_id: String,
    pub fallbacks: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChromeSelectionRefusal {
    pub code: String,
    pub message: String,
}

pub fn select_chrome(
    goal: Goal,
    projection: &ProjectionCapability,
    catalog: &[ChromeCapability],
) -> Result<ChromeSelection, ChromeSelectionRefusal> {
    if catalog.is_empty() {
        return Err(ChromeSelectionRefusal {
            code: "empty_chrome_catalog".to_string(),
            message: "No chrome shells registered.".to_string(),
        });
    }

    let preferred = goal_chrome_affordances(goal);
    let mut candidates: Vec<&ChromeCapability> = catalog
        .iter()
        .filter(|chrome| {
            chrome.pairs_with_projections.is_empty()
                || chrome
                    .pairs_with_projections
                    .iter()
                    .any(|id| id == &projection.id)
        })
        .collect();

    if candidates.is_empty() {
        return Err(ChromeSelectionRefusal {
            code: "no_compatible_chrome".to_string(),
            message: format!(
                "No chrome in the catalog pairs with projection {:?}.",
                projection.id
            ),
        });
    }

    candidates.sort_by_key(|chrome| chrome_rank(&preferred, chrome));
    let finalist = candidates[0];
    let fallbacks = candidates[1..]
        .iter()
        .map(|chrome| chrome.id.clone())
        .collect();

    Ok(ChromeSelection {
        chrome_id: finalist.id.clone(),
        fallbacks,
        rationale: format!(
            "goal={}; projection={}; preferred_affordances={preferred:?}; finalist_affordances={:?}",
            goal.as_str(),
            projection.id,
            finalist.affordances
        ),
    })
}

fn goal_chrome_affordances(goal: Goal) -> Vec<&'static str> {
    match goal {
        Goal::Compare => vec!["compare-toolbar"],
        Goal::Locate => vec!["exploration-palette", "evidence-drawer"],
        Goal::ExplainProcess => vec!["player", "narration"],
        Goal::InspectEvidence => vec!["gallery-rail", "evidence-drawer", "exploration-palette"],
        Goal::Rank => vec!["compare-toolbar"],
        Goal::FindPattern => vec!["compare-toolbar", "exploration-palette"],
        Goal::TellStory => vec!["narration", "player"],
        Goal::Summarize => vec!["narration", "document-rail"],
        Goal::Navigate => vec!["exploration-palette"],
        Goal::Simulate => vec!["player", "narration"],
    }
}

fn chrome_rank(preferred: &[&str], chrome: &ChromeCapability) -> usize {
    for (idx, affordance) in preferred.iter().enumerate() {
        if chrome
            .affordances
            .iter()
            .any(|candidate| candidate == affordance)
        {
            return idx;
        }
    }
    preferred.len() + 1
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atoms::{AtomLifecycle, AtomPosition};
    use crate::catalogs::{production_chrome_catalog, production_projection_catalog};

    fn atom(id: &str, position_space: Option<CoordinateSpace>) -> SceneAtom {
        SceneAtom {
            id: id.to_string(),
            kind: "concept".to_string(),
            label: Some(id.to_string()),
            position: position_space.map(|space| AtomPosition {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                space,
            }),
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

    #[test]
    fn detects_dag_from_dense_relations() {
        let atoms = vec![atom("a", None), atom("b", None), atom("c", None)];
        let relations = vec![relation("a", "b"), relation("a", "c"), relation("b", "c")];
        let detection = detect_shape(&atoms, &relations);
        assert_eq!(detection.shape, DataShape::Dag);
        assert_eq!(detection.required_fields, vec!["kind"]);
    }

    #[test]
    fn lida_selection_matches_python_catalog_order_for_explain_dag() {
        let projections = production_projection_catalog();
        let chromes = production_chrome_catalog();
        let projection =
            select_projection(Goal::ExplainProcess, DataShape::Dag, &projections).unwrap();
        assert_eq!(projection.projection_id, "patent_diagram");
        assert!(projection.fallbacks.contains(&"tree_hierarchy".to_string()));

        let selected_projection = projections
            .iter()
            .find(|candidate| candidate.id == projection.projection_id)
            .expect("selected projection exists");
        let chrome = select_chrome(Goal::ExplainProcess, selected_projection, &chromes).unwrap();
        assert_eq!(chrome.chrome_id, "patent_plate_shell");
    }

    fn relation(source: &str, target: &str) -> SceneRelation {
        SceneRelation {
            id: format!("{source}->{target}"),
            source_id: source.to_string(),
            target_id: target.to_string(),
            kind: "related".to_string(),
            weight: None,
            color: None,
            opacity: None,
            glyph: None,
            lifecycle: AtomLifecycle::Present,
            metadata: BTreeMap::new(),
            source_refs: Vec::new(),
        }
    }
}
