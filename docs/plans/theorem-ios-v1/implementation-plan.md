# Theorem iOS v1 — build plan (lane split + seam)

**Date: 2026-05-30**
**Spec: `~/Downloads/SPEC-THEOREM-IOS-V1.md` (the floor, not the ceiling).**
**Co-build: Codex (Rust lane) + Claude Code (Swift lane), coordinating over the
substrate + commit messages (harness coordinate endpoint is down).**

This is the *how*. The spec is the *what*. Every item below backreferences a spec
section. No item is deferred without the user's explicit per-item consent. The
four-projection set ships whole (spec "Spec discipline notes": shipping only
`force_graph` guts the differentiator).

## The seam (where the two lanes meet)

Two seams, both already typed in the repo, plus one new FFI surface:

1. **Network JSON** (already exists, do not invent fields):
   - `SubstrateSearch` ← `rustyred-web/src/search.rs` (snake_case JSON)
   - `ScenePackageV2` ← `scene-os-core/src/{package,atoms}.rs` (camelCase JSON,
     kebab-case `lifecycle`/`space` enums)
2. **UniFFI surface** (new, Rust lane builds, Swift lane consumes):
   ```
   namespace theorem {
     ReprojectResult reproject(scene_package_json: string, projection_id: string);
     sequence<ProjectionAvailability> available_projections(scene_package_json: string);
     string center_node_id(scene_package_json: string, mode: CentralityMode);
   }
   ```
   Spec "The Rust ↔ Swift bridge (UniFFI)". The Swift lane codes against a
   `ReprojectionEngine` protocol; the UniFFI-generated client is one impl, a
   pure-Swift stub is the dev impl until the `.xcframework` lands. This is what
   lets both lanes build in parallel.
3. **Projection catalog** (`scene-os-core/catalogs.rs` + `select.rs`): the Rust
   lane adds 4 `ProjectionCapability` entries + 4 `detect_shape` arms; the Swift
   renderers read `interactions` off the catalog entry to wire taps.

## Lane A — Rust (Codex)

Backref: spec build-sequence steps 1 (Rust half) + 2; "The algorithms to ship in v1".

- [ ] **A1** UniFFI surface (`reproject`, `available_projections`,
  `center_node_id`) over scene-os-core. New crate `apps/ios/theorem-ffi` (or a
  `uniffi` feature on scene-os-core) with `crate-type = ["staticlib", "cdylib"]`.
  `reproject` is layout-only: runs a projection's placement over the CURRENT
  scene's atoms, never fabricates shape. Backref: "UniFFI surface for v1".
- [ ] **A2** Four projection triples, Rust half = `detect_shape` arm
  (`select.rs`) + `ProjectionCapability` (`catalogs.rs`), honest-shape rule:
  - `force_graph` — accepts ≥2 atoms ∧ ≥1 relation. Backref: algo 1.
  - `radial_rings` — accepts atoms carrying `ring`. Backref: algo 2.
  - `tree_layout` — accepts ONLY if BFS from PPR-center node yields a valid
    tree/forest (reject cycles/multi-parent). Backref: algo 3.
  - `fractal_expansion` — accepts relations + ≥1 seed; server streams the
    `push_ppr` push-trace. Backref: algo 4.
  Unit-test detect_shape honesty (tree rejects cycles). Servo-free, fast.
- [ ] **A3** `.xcframework` build: `aarch64-apple-ios` + `aarch64-apple-ios-sim`,
  `xcodebuild -create-xcframework`. Backref: "Crate work".

## Lane B — Swift (Claude Code)

Backref: spec build-sequence steps 1 (Swift half), 3-7; the IA, Island, reader,
theming sections.

### B0 — TheoremKit foundation (SwiftPM package, headlessly testable)
Backref: "data contracts", "projection framework", "Theming".
- [ ] **B0.1** Wire models: `SubstrateSearch`/`SearchHit`/`SearchLink` (snake),
  `ScenePackageV2`/`SceneAtom`/`SceneRelation`/`SourceRef`/`AtomPosition`
  (camel), enums (kebab). Explicit `CodingKeys` (mixed casing forbids one global
  strategy). Round-trip tests against the Rust serde fixtures.
- [ ] **B0.2** `ReprojectionEngine` protocol (the FFI seam) + a pure-Swift
  `StubReprojectionEngine` (radial + tree-BFS + force-prep layouts) so renderers
  build before the xcframework. Layout-only; honest-shape (tree-BFS rejects
  cycles → projection unavailable). Backref: "on-device reprojection sliver".
- [ ] **B0.3** Role-based `Theme` (`nodeCore…textSecondary` roles) + default
  palette (teal/terracotta/amber/purple/gray) + two `Font` tokens
  (`displayFont` Berthold license-gated → falls back to `bodyFont`; `bodyFont`
  IBM Plex Sans SemiCondensed, bundled). Backref: "Theming".

### B1 — Streaming search → scene
Backref: spec step 3; "streaming thin client".
- [ ] **B1.1** Hosted API client: search (`SubstrateSearch`) + compile
  (`ScenePackageV2`) with progressive/streamed atom assembly (streaming, not
  spinner). Heavy retrieval stays server-side.
- [ ] **B1.2** Scene store: atoms assemble as they arrive; recent scenes cache
  locally for instant cold-start.

### B2 — Projection switcher + 4 renderers
Backref: spec step 4; "The algorithms to ship in v1"; "Projection picker UX".
- [ ] **B2.1** `force_graph` renderer — Grape `ForceDirectedGraph`, radius from
  `match_score`, drag/pan/zoom, `.sensoryFeedback(.selection)`. Grape
  `from: 1.1.0`.
- [ ] **B2.2** `radial_rings` renderer — `Canvas`, orbits by `ring`, angle by
  `match_score` desc.
- [ ] **B2.3** `tree_layout` renderer — `Canvas` tidy-tree (Reingold-Tilford),
  root = PPR-center.
- [ ] **B2.4** `fractal_expansion` renderer — `Canvas` over base layout,
  animates the streamed push-trace (node mass + edge activation in push order),
  `alpha=0.15`/`epsilon=1e-4` shown read-only.
- [ ] **B2.5** Switcher: lights exactly `available_projections(scene)`, greys the
  rest with a long-press reason ("links form a cycle — no tree"). Default
  `force_graph`. Instant swap via the sliver.

### B3 — Dynamic Island (the single control surface)
Backref: spec "The Dynamic Island". Scan `Theseus/Design Components/` for the
`dynamic-island-toc` reference before building.
- [ ] **B3.1** Three states: idle pill (dual-zone search/ask), active scene
  (live highest-`match_score` title via `center_node_id`), expanded
  (input field / node-detail list). `matchedGeometryEffect` expansion,
  `.transition(.push)` label swap, `.sensoryFeedback(.selection)` on center
  change. Floats above the TabView, does not page.

### B4 — IA shell
Backref: spec "The IA shell".
- [ ] **B4.1** Paged `TabView` (page style), 5 peer surfaces: Home (graph hero +
  island + search/ask), Projects (file-glyph sidebar as content), Models
  (SELECTION only — hosted GL-fusion vs API key; NOT agent coordination),
  Build (scaffold), Artifacts (saved scenes/captures/dossiers).

### B5 — Reader + theming editor + Models selection
Backref: spec "The reader (host_handoff)", "Theming", IA "Models".
- [ ] **B5.1** Reader: `SFSafariViewController` (WebKit, App-Store compliant; no
  Servo). Capture-on-open is explicit (user taps save) → hosted ingest.
- [ ] **B5.2** Palette editor in Settings (role→color); meaning survives any
  palette via the role indirection.

### B6 — App target + ship
Backref: spec steps 1 (app shell) + 8.
- [ ] **B6.1** Xcode app target hosting TheoremKit + the `.xcframework`
  (integrates A3). `@main App`, Info.plist, asset catalog, ActivityKit
  entitlement for the Island.
- [ ] **B6.2** Archive → TestFlight → review (Apple Developer account). Backref:
  step 8.

## Build order (critical path)

1. **B0** (TheoremKit) + **A1/A2** (Rust FFI + algos) in parallel — neither
   blocks the other; B0 codes against the protocol, A1 against the same JSON.
2. **B2** renderers (against `StubReprojectionEngine`) ∥ **A3** xcframework.
3. **B6.1** swaps the stub for the real UniFFI client once A3 lands.
4. **B1**, **B3**, **B4**, **B5** layer on.

## Verification

- `swift build` + `swift test` on TheoremKit (models round-trip vs Rust serde
  fixtures; layout honesty: tree-BFS rejects cycles; radial places by ring).
- `cargo test` on the Rust lane (detect_shape honesty).
- Xcode/simulator build of the app target once B6.1 lands (XcodeBuildMCP).
- The honest-shape feature is itself a test: a cyclic scene must grey out
  `tree_layout` with a reason, never fabricate a hierarchy.

## Spec-discipline guards (binding)

- All 4 projections ship; no subset.
- Sliver is layout-only; `reproject` never fabricates shape.
- Models = selection for v1; the bring-your-own-agent endpoint stays out (it
  depends on a room layer that does not exist).
- Heavy retrieval stays server-side.
- Berthold is license-gated; Plex is the bundled default.
- No fake UI: every switcher entry, island readout, and node action is wired to
  real scene data; unavailable projections grey honestly.
