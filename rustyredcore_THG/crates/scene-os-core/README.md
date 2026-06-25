# scene-os-core

The Rust-native SceneOS director: serde-only scene contracts, trusted projection and chrome catalogs, goal/shape classifiers, and `compile_scene_package` that turns a scene into a `ScenePackageV2` without crossing a Python API boundary. Atoms and relations use the same JSON contract as Index-API/Theseus-UI. serde-only leaf (no substrate dep).

## Key API

- Atoms (`atoms.rs`): `SceneAtom`, `SceneRelation`, `SceneScene`, `AtomPosition`, `CoordinateSpace` (Graph/Geo/Timeline/Rank/Matrix/Diagram/Frame/Gallery/Freeform), `AtomLifecycle`.
- Package (`package.rs`): `ScenePackageV2`, `ProjectionBinding`, `ChromeBinding`, `ActionDescriptor`, `TransitionDescriptor`, `TerminalStateArtifact`, `SCENE_PACKAGE_V2_VERSION`.
- Capabilities (`capabilities.rs`): `ProjectionCapability`, `ChromeCapability`, `ProjectionRequirements`, `ProjectionBudgets`.
- Catalogs (`catalogs.rs`): `production_projection_catalog()`, `mobile_projection_catalog()`, `production_chrome_catalog()`.
- Compile (`compile.rs`): `compile_scene_package(SceneCompileInput) -> Result<ScenePackageV2, SceneCompileError>`. `SceneCompileInput` carries the query and an optional `answer_type` hint that can force a projection.
- Select (`select.rs`): `classify_goal`, `detect_shape`, `select_projection`, `select_chrome`. `Goal` (Compare/Locate/ExplainProcess/InspectEvidence/Rank/FindPattern/TellStory/Summarize/Navigate/Simulate), `DataShape`.
- Mobile (`mobile.rs`): `available_projections`, `center_node_id`, `reproject`, `CentralityMode` (PprMass/Degree).
- Patent (`patent.rs`): `lift_patent_scene_payload`.

Path dep: none beyond serde. Consumed in-process by `apps/theorem-gateway` and `apps/theorem-ios`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p scene-os-core
```

Tests are inline. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
