//! Commit-to-live pickup detection for programmable harness capability files.
//!
//! The notify watcher remains read-only on the working tree. This module turns a
//! settled change set into inspectable receipts that the capability registrar can
//! consume after git has carried the versioned file into the watched checkout.

use crate::{ChangeKind, ChangeSet};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgrammableCapabilityKind {
    DeclarativeSkill,
    WasmPlugin,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProgrammableCapabilityPickup {
    pub path: String,
    pub kind: ProgrammableCapabilityKind,
    pub change_kind: ChangeKind,
}

pub fn detect_programmable_capability_pickups(
    change_set: &ChangeSet,
) -> Vec<ProgrammableCapabilityPickup> {
    change_set
        .changes
        .iter()
        .filter_map(|change| {
            let kind = classify_capability_path(&change.path)?;
            Some(ProgrammableCapabilityPickup {
                path: change.path.to_string_lossy().replace('\\', "/"),
                kind,
                change_kind: change.kind,
            })
        })
        .collect()
}

fn classify_capability_path(path: &Path) -> Option<ProgrammableCapabilityKind> {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.contains(".theorem/capabilities/declarative/")
        && (normalized.ends_with(".json") || normalized.ends_with(".toml"))
    {
        return Some(ProgrammableCapabilityKind::DeclarativeSkill);
    }
    if normalized.contains(".theorem/capabilities/wasm/")
        && (normalized.ends_with(".wasm")
            || normalized.ends_with(".wat")
            || normalized.ends_with(".json")
            || normalized.ends_with(".toml"))
    {
        return Some(ProgrammableCapabilityKind::WasmPlugin);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileChange;
    use std::path::PathBuf;

    #[test]
    fn watcher_pickup_detects_declarative_and_wasm_capability_files() {
        let pickups = detect_programmable_capability_pickups(&ChangeSet {
            changes: vec![
                FileChange {
                    path: PathBuf::from(".theorem/capabilities/declarative/two-step.json"),
                    kind: ChangeKind::Created,
                },
                FileChange {
                    path: PathBuf::from(".theorem/capabilities/wasm/fact-plugin.wasm"),
                    kind: ChangeKind::Modified,
                },
                FileChange {
                    path: PathBuf::from("src/lib.rs"),
                    kind: ChangeKind::Modified,
                },
            ],
        });
        assert_eq!(pickups.len(), 2);
        assert_eq!(
            pickups[0].kind,
            ProgrammableCapabilityKind::DeclarativeSkill
        );
        assert_eq!(pickups[1].kind, ProgrammableCapabilityKind::WasmPlugin);
    }
}
