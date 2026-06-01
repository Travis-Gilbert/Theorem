//! SceneOS core contracts for Theorem.
//!
//! This crate is the Rust-native director seam for the browser: atoms and
//! relations use the same JSON contract as Index-API/Theseus-UI, catalogs
//! describe trusted projections/chromes, and the compiler selects a package
//! without crossing a Python API boundary.

pub mod atoms;
pub mod capabilities;
pub mod catalogs;
pub mod compile;
pub mod mobile;
pub mod package;
pub mod patent;
pub mod select;

pub use atoms::{
    AtomLifecycle, AtomPosition, CoordinateSpace, SceneAtom, SceneRelation, SceneScene, SourceRef,
};
pub use capabilities::{
    ChromeCapability, ProjectionAttributes, ProjectionBudgets, ProjectionCapability,
    ProjectionRequirements,
};
pub use catalogs::{
    mobile_projection_catalog, production_chrome_catalog, production_projection_catalog,
};
pub use compile::{compile_scene_package, SceneCompileError, SceneCompileInput};
pub use mobile::{
    available_projections, available_projections_for_package, center_node_id,
    center_node_id_for_package, reproject, reproject_package, CentralityMode,
    ProjectedAtomPosition, ProjectionAvailability, ReprojectResult,
};
pub use package::{
    ActionDescriptor, ChromeBinding, ProjectionBinding, ScenePackageV2, TerminalStateArtifact,
    TransitionDescriptor,
};
pub use patent::{lift_patent_scene_payload, PatentSceneLiftInput};
pub use select::{
    classify_goal, detect_shape, select_chrome, select_projection, ChromeSelection,
    ChromeSelectionRefusal, DataShape, DataShapeDetection, Goal, GoalDetection,
    ProjectionSelection, ProjectionSelectionRefusal,
};
