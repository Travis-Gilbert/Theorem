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
pub mod package;
pub mod select;

pub use atoms::{
    AtomLifecycle, AtomPosition, CoordinateSpace, SceneAtom, SceneRelation, SceneScene, SourceRef,
};
pub use capabilities::{
    ChromeCapability, ProjectionAttributes, ProjectionBudgets, ProjectionCapability,
    ProjectionRequirements,
};
pub use catalogs::{production_chrome_catalog, production_projection_catalog};
pub use compile::{compile_scene_package, SceneCompileError, SceneCompileInput};
pub use package::{
    ActionDescriptor, ChromeBinding, ProjectionBinding, ScenePackageV2, TerminalStateArtifact,
    TransitionDescriptor,
};
pub use select::{
    classify_goal, detect_shape, select_chrome, select_projection, ChromeSelection,
    ChromeSelectionRefusal, DataShape, DataShapeDetection, Goal, GoalDetection,
    ProjectionSelection, ProjectionSelectionRefusal,
};
