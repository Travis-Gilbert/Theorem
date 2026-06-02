use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::f64::consts::{PI, TAU};

use serde::{Deserialize, Serialize};

use crate::atoms::{CoordinateSpace, SceneAtom, SceneRelation};
use crate::catalogs::mobile_projection_catalog;
use crate::package::ScenePackageV2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionAvailability {
    pub id: String,
    pub label: String,
    pub available: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CentralityMode {
    PprMass,
    Degree,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedAtomPosition {
    pub id: String,
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub z: f64,
    pub space: CoordinateSpace,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReprojectResult {
    pub projection_id: String,
    pub positions: Vec<ProjectedAtomPosition>,
}

pub fn available_projections(
    scene_package_json: &str,
) -> Result<Vec<ProjectionAvailability>, String> {
    let package = parse_package(scene_package_json)?;
    Ok(available_projections_for_package(&package))
}

pub fn available_projections_for_package(package: &ScenePackageV2) -> Vec<ProjectionAvailability> {
    mobile_projection_catalog()
        .into_iter()
        .map(|projection| {
            let (available, reason) =
                projection_available(projection.id.as_str(), &package.atoms, &package.relations);
            ProjectionAvailability {
                id: projection.id,
                label: projection.label,
                available,
                reason,
            }
        })
        .collect()
}

pub fn center_node_id(scene_package_json: &str, mode: CentralityMode) -> Result<String, String> {
    let package = parse_package(scene_package_json)?;
    center_node_id_for_package(&package, mode).ok_or_else(|| "scene has no atoms".to_string())
}

pub fn center_node_id_for_package(
    package: &ScenePackageV2,
    mode: CentralityMode,
) -> Option<String> {
    match mode {
        CentralityMode::PprMass => package
            .atoms
            .iter()
            .max_by(compare_atoms_by_ppr_mass)
            .map(|atom| atom.id.clone()),
        CentralityMode::Degree => {
            let degree = relation_degree(&package.relations);
            package
                .atoms
                .iter()
                .max_by(|left, right| {
                    let left_degree = degree.get(left.id.as_str()).copied().unwrap_or_default();
                    let right_degree = degree.get(right.id.as_str()).copied().unwrap_or_default();
                    left_degree
                        .cmp(&right_degree)
                        .then_with(|| compare_atoms_by_ppr_mass(left, right))
                })
                .map(|atom| atom.id.clone())
        }
    }
}

pub fn reproject(scene_package_json: &str, projection_id: &str) -> Result<ReprojectResult, String> {
    let package = parse_package(scene_package_json)?;
    reproject_package(&package, projection_id)
}

pub fn reproject_package(
    package: &ScenePackageV2,
    projection_id: &str,
) -> Result<ReprojectResult, String> {
    let (available, reason) =
        projection_available(projection_id, &package.atoms, &package.relations);
    if !available {
        return Err(reason);
    }

    let positions = match projection_id {
        "force_graph" => force_positions(&package.atoms),
        "radial_rings" => radial_positions(&package.atoms),
        "tree_layout" => tree_positions(&package.atoms, &package.relations)?,
        "fractal_expansion" => fractal_positions(&package.atoms),
        _ => return Err(format!("unknown mobile projection {projection_id:?}")),
    };

    Ok(ReprojectResult {
        projection_id: projection_id.to_string(),
        positions,
    })
}

fn parse_package(scene_package_json: &str) -> Result<ScenePackageV2, String> {
    serde_json::from_str(scene_package_json)
        .map_err(|error| format!("invalid ScenePackageV2 JSON: {error}"))
}

fn projection_available(
    projection_id: &str,
    atoms: &[SceneAtom],
    relations: &[SceneRelation],
) -> (bool, String) {
    match projection_id {
        "force_graph" => {
            if atoms.len() < 2 {
                return (false, "force graph needs at least two atoms".to_string());
            }
            if known_relations(atoms, relations).is_empty() {
                return (false, "force graph needs at least one relation".to_string());
            }
            (true, "relations define a graph neighborhood".to_string())
        }
        "radial_rings" => {
            let missing = atoms
                .iter()
                .filter(|atom| atom_ring(atom).is_none())
                .count();
            if missing > 0 {
                return (
                    false,
                    format!("{missing} atom(s) lack a substrate ring value"),
                );
            }
            (true, "atoms carry substrate ring values".to_string())
        }
        "tree_layout" => match tree_report(atoms, relations) {
            Ok(_) => (
                true,
                "links form a rooted tree from the PPR center".to_string(),
            ),
            Err(reason) => (false, reason),
        },
        "fractal_expansion" => {
            if known_relations(atoms, relations).is_empty() {
                return (false, "fractal expansion needs graph relations".to_string());
            }
            let seeds = atoms
                .iter()
                .filter(|atom| atom_ring(atom) == Some(0))
                .count();
            if seeds == 0 {
                return (
                    false,
                    "fractal expansion needs at least one ring-0 match seed".to_string(),
                );
            }
            (
                true,
                "ring-0 seeds and relations can replay push PPR".to_string(),
            )
        }
        _ => (
            false,
            format!("unknown mobile projection {projection_id:?}"),
        ),
    }
}

fn force_positions(atoms: &[SceneAtom]) -> Vec<ProjectedAtomPosition> {
    let mut ranked = ranked_atoms(atoms);
    ranked
        .drain(..)
        .enumerate()
        .map(|(idx, atom)| {
            let radius = 36.0 + (idx as f64).sqrt() * 28.0;
            let angle = idx as f64 * PI * (3.0 - 5.0_f64.sqrt());
            projected(
                atom,
                radius * angle.cos(),
                radius * angle.sin(),
                CoordinateSpace::Graph,
            )
        })
        .collect()
}

fn radial_positions(atoms: &[SceneAtom]) -> Vec<ProjectedAtomPosition> {
    let mut by_ring: BTreeMap<usize, Vec<&SceneAtom>> = BTreeMap::new();
    for atom in atoms {
        by_ring
            .entry(atom_ring(atom).unwrap_or_default())
            .or_default()
            .push(atom);
    }

    let mut out = Vec::with_capacity(atoms.len());
    for (ring, mut ring_atoms) in by_ring {
        ring_atoms.sort_by(compare_atoms_by_ppr_mass);
        ring_atoms.reverse();
        let radius = if ring == 0 { 0.0 } else { ring as f64 * 96.0 };
        let count = ring_atoms.len().max(1);
        for (idx, atom) in ring_atoms.into_iter().enumerate() {
            let angle = if radius == 0.0 {
                0.0
            } else {
                idx as f64 / count as f64 * TAU
            };
            out.push(projected(
                atom,
                radius * angle.cos(),
                radius * angle.sin(),
                CoordinateSpace::Graph,
            ));
        }
    }
    out
}

fn tree_positions(
    atoms: &[SceneAtom],
    relations: &[SceneRelation],
) -> Result<Vec<ProjectedAtomPosition>, String> {
    let report = tree_report(atoms, relations)?;
    let by_id: BTreeMap<&str, &SceneAtom> =
        atoms.iter().map(|atom| (atom.id.as_str(), atom)).collect();
    let mut out = Vec::with_capacity(atoms.len());

    for (depth, ids) in report.levels.iter().enumerate() {
        let count = ids.len().max(1);
        let start_x = -((count.saturating_sub(1)) as f64) * 58.0;
        for (idx, id) in ids.iter().enumerate() {
            let atom = by_id
                .get(id.as_str())
                .ok_or_else(|| format!("tree references missing atom {id:?}"))?;
            out.push(projected(
                atom,
                start_x + idx as f64 * 116.0,
                depth as f64 * 104.0,
                CoordinateSpace::Diagram,
            ));
        }
    }

    Ok(out)
}

fn fractal_positions(atoms: &[SceneAtom]) -> Vec<ProjectedAtomPosition> {
    let mut positions = radial_positions(atoms);
    let rank_by_id: BTreeMap<String, usize> = ranked_atoms(atoms)
        .into_iter()
        .enumerate()
        .map(|(rank, atom)| (atom.id.clone(), rank))
        .collect();
    for position in &mut positions {
        position.z = rank_by_id
            .get(position.id.as_str())
            .copied()
            .unwrap_or_default() as f64;
    }
    positions
}

fn projected(atom: &SceneAtom, x: f64, y: f64, space: CoordinateSpace) -> ProjectedAtomPosition {
    ProjectedAtomPosition {
        id: atom.id.clone(),
        x,
        y,
        z: 0.0,
        space,
    }
}

#[derive(Debug, Clone, PartialEq)]
struct TreeReport {
    levels: Vec<Vec<String>>,
}

fn tree_report(atoms: &[SceneAtom], relations: &[SceneRelation]) -> Result<TreeReport, String> {
    if atoms.is_empty() {
        return Err("tree layout needs at least one atom".to_string());
    }
    if atoms.len() == 1 {
        return Ok(TreeReport {
            levels: vec![vec![atoms[0].id.clone()]],
        });
    }

    let root = center_atom(atoms).ok_or_else(|| "tree layout needs a root atom".to_string())?;
    let ids: BTreeSet<&str> = atoms.iter().map(|atom| atom.id.as_str()).collect();
    let relations = known_relations(atoms, relations);
    if relations.len() != atoms.len().saturating_sub(1) {
        return Err(format!(
            "tree layout needs exactly n-1 links; got {} link(s) for {} atom(s)",
            relations.len(),
            atoms.len()
        ));
    }

    let mut parent_count: BTreeMap<&str, usize> =
        ids.iter().copied().map(|id| (id, 0usize)).collect();
    let mut children: BTreeMap<&str, Vec<&str>> =
        ids.iter().copied().map(|id| (id, Vec::new())).collect();
    for relation in relations {
        *parent_count.entry(relation.target_id.as_str()).or_default() += 1;
        children
            .entry(relation.source_id.as_str())
            .or_default()
            .push(relation.target_id.as_str());
    }

    if parent_count
        .get(root.id.as_str())
        .copied()
        .unwrap_or_default()
        != 0
    {
        return Err("PPR center has an incoming link, so it is not a rooted tree".to_string());
    }
    if let Some((id, _)) = parent_count
        .iter()
        .find(|(id, count)| **id != root.id.as_str() && **count != 1)
    {
        return Err(format!("{id} does not have exactly one parent"));
    }

    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([(root.id.as_str(), 0usize)]);
    let mut levels: Vec<Vec<String>> = Vec::new();
    while let Some((id, depth)) = queue.pop_front() {
        if !seen.insert(id) {
            return Err("links form a cycle or cross-edge".to_string());
        }
        if levels.len() <= depth {
            levels.push(Vec::new());
        }
        levels[depth].push(id.to_string());
        for child in children.get(id).into_iter().flatten() {
            queue.push_back((child, depth + 1));
        }
    }

    if seen.len() != atoms.len() {
        return Err("not every atom is reachable from the PPR center".to_string());
    }

    Ok(TreeReport { levels })
}

fn known_relations<'a>(
    atoms: &'a [SceneAtom],
    relations: &'a [SceneRelation],
) -> Vec<&'a SceneRelation> {
    let ids: BTreeSet<&str> = atoms.iter().map(|atom| atom.id.as_str()).collect();
    relations
        .iter()
        .filter(|relation| {
            ids.contains(relation.source_id.as_str()) && ids.contains(relation.target_id.as_str())
        })
        .collect()
}

fn relation_degree(relations: &[SceneRelation]) -> BTreeMap<&str, usize> {
    let mut degree = BTreeMap::new();
    for relation in relations {
        *degree.entry(relation.source_id.as_str()).or_default() += 1;
        *degree.entry(relation.target_id.as_str()).or_default() += 1;
    }
    degree
}

fn ranked_atoms(atoms: &[SceneAtom]) -> Vec<&SceneAtom> {
    let mut ranked: Vec<&SceneAtom> = atoms.iter().collect();
    ranked.sort_by(|left, right| compare_atoms_by_ppr_mass(left, right));
    ranked.reverse();
    ranked
}

fn center_atom(atoms: &[SceneAtom]) -> Option<&SceneAtom> {
    atoms.iter().max_by(compare_atoms_by_ppr_mass)
}

fn compare_atoms_by_ppr_mass(left: &&SceneAtom, right: &&SceneAtom) -> Ordering {
    atom_ppr_mass(left)
        .partial_cmp(&atom_ppr_mass(right))
        .unwrap_or(Ordering::Equal)
        .then_with(|| right.id.cmp(&left.id))
}

fn atom_ppr_mass(atom: &SceneAtom) -> f64 {
    atom.metadata
        .get("matchScore")
        .and_then(|value| value.as_f64())
        .or(atom.weight)
        .unwrap_or_default()
}

fn atom_ring(atom: &SceneAtom) -> Option<usize> {
    atom.metadata
        .get("ring")
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
}

fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::atoms::{AtomLifecycle, SceneAtom, SceneRelation};
    use crate::package::{ChromeBinding, ProjectionBinding, SCENE_PACKAGE_V2_VERSION};

    #[test]
    fn search_tree_scene_supports_all_ios_v1_projections() {
        let package = package(
            vec![atom("root", 0, 0.9), atom("child", 1, 0.3)],
            vec![relation("root", "child")],
        );

        let availability = available_projections_for_package(&package);

        assert_eq!(availability.len(), 4);
        assert!(availability.iter().all(|projection| projection.available));
    }

    #[test]
    fn tree_layout_rejects_cycles_instead_of_fabricating_a_hierarchy() {
        let package = package(
            vec![atom("a", 0, 0.9), atom("b", 1, 0.4), atom("c", 1, 0.3)],
            vec![relation("a", "b"), relation("b", "c"), relation("c", "a")],
        );

        let tree = available_projections_for_package(&package)
            .into_iter()
            .find(|projection| projection.id == "tree_layout")
            .expect("tree availability exists");

        assert!(!tree.available);
        assert!(tree.reason.contains("n-1") || tree.reason.contains("incoming"));
        assert!(reproject_package(&package, "tree_layout").is_err());
    }

    #[test]
    fn center_node_uses_match_score_over_degree_for_ppr_mode() {
        let package = package(
            vec![
                atom("dense", 1, 0.2),
                atom("center", 0, 0.95),
                atom("leaf", 1, 0.1),
            ],
            vec![relation("dense", "leaf"), relation("center", "leaf")],
        );

        assert_eq!(
            center_node_id_for_package(&package, CentralityMode::PprMass),
            Some("center".to_string())
        );
    }

    #[test]
    fn radial_reprojection_places_atoms_for_swift_canvas() {
        let package = package(
            vec![atom("root", 0, 0.9), atom("child", 1, 0.3)],
            vec![relation("root", "child")],
        );

        let result = reproject_package(&package, "radial_rings").expect("radial layout");

        assert_eq!(result.projection_id, "radial_rings");
        assert_eq!(result.positions.len(), 2);
        assert!(result
            .positions
            .iter()
            .all(|position| position.space == CoordinateSpace::Graph));
    }

    fn package(atoms: Vec<SceneAtom>, relations: Vec<SceneRelation>) -> ScenePackageV2 {
        ScenePackageV2 {
            version: SCENE_PACKAGE_V2_VERSION.to_string(),
            id: "scene-ios-test".to_string(),
            manifest_ref: "manifest-ios-test".to_string(),
            atoms,
            relations,
            projection: ProjectionBinding {
                id: "force_graph".to_string(),
                params: BTreeMap::new(),
            },
            chrome: ChromeBinding {
                id: "dynamic_island_shell".to_string(),
                params: BTreeMap::new(),
            },
            actions: Vec::new(),
            transitions: None,
            terminal_state: None,
            provenance: BTreeMap::new(),
        }
    }

    fn atom(id: &str, ring: usize, match_score: f64) -> SceneAtom {
        SceneAtom {
            id: id.to_string(),
            kind: "concept".to_string(),
            label: Some(id.to_string()),
            position: None,
            weight: Some(match_score),
            color: None,
            opacity: None,
            glyph: None,
            scale: None,
            lifecycle: AtomLifecycle::Present,
            metadata: BTreeMap::from([
                ("ring".to_string(), json!(ring)),
                ("matchScore".to_string(), json!(match_score)),
            ]),
            source_refs: Vec::new(),
        }
    }

    fn relation(source: &str, target: &str) -> SceneRelation {
        SceneRelation {
            id: format!("{source}->{target}:links_to"),
            source_id: source.to_string(),
            target_id: target.to_string(),
            kind: "links_to".to_string(),
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
