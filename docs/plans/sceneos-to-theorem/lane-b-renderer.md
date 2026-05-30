# Lane B — SceneOS renderer bundle (shipped)

**Date: 2026-05-30**
**Owner: Claude Code. Status: slice 1 complete + verified. Crate: `rustyredcore_THG/crates/scene-os-web`.**

Lane B of the [SceneOS → Theorem port](./implementation-plan.md): the renderer
half. Lane A (`scene-os-core`, Codex) is the director that produces a
`ScenePackageV2`; Lane B takes that package and serves the page that DRAWS it,
as one self-contained asset Servo serves — the SERP pattern, not the React SPA.

## What shipped

A new crate `scene-os-web` (sibling of `scene-os-core`), structured like
`rustyred-web`'s SERP:

```
crates/scene-os-web/
├── Cargo.toml                      # deps: serde_json, scene-os-core (typed convenience)
├── src/lib.rs                      # render_scene_html(json) + render_scene(&ScenePackageV2)
├── examples/render_sample.rs       # emits a sample page (Lane C reference + visual check)
└── web/                            # the renderer bundle (TypeScript)
    ├── package.json                # d3-hierarchy / d3-scale / d3-sankey + esbuild
    ├── build.mjs                   # esbuild IIFE → dist/scene-os.bundle.js (d3 inlined)
    ├── scene-host.html             # host page (studio-journal palette) + 2 inject markers
    ├── dist/scene-os.bundle.js     # built bundle, COMMITTED (embedded via include_str!)
    ├── src/
    │   ├── atoms/types.ts           # copied verbatim from Theseus-UI (the wire contract)
    │   ├── v2-package.ts            # copied: ScenePackageV2 + validateScenePackageV2
    │   ├── capabilities.ts          # copied: ProjectionCapability type
    │   ├── substrate/projection.ts  # copied: ProjectionAdapter contract + FREEFORM + genericTerminalState
    │   ├── projections/
    │   │   ├── shared.ts             # copied: defaultGlyphForKind etc.
    │   │   ├── {PatentDiagram,TreeHierarchy,NumericSeries,
    │   │   │    CategoricalSet,FlowLayered,SankeyFlow}Projection.ts  # the 6 Lane A emits
    │   │   └── productionRegistry.ts # NEW: id → adapter, freeform fallback (never throws)
    │   ├── renderer/
    │   │   ├── palette.ts            # NEW: kind→color, glyph→shape (substrate + SERP vocab)
    │   │   ├── sceneGeometry.ts      # NEW: pure layoutScene + fitTransform + grid fallback
    │   │   └── SceneRenderer.ts      # NEW: vanilla 2D-canvas renderer (DPR, hover, labels)
    │   └── entry.ts                 # NEW: IIFE entry, reads injected package, mounts, empty states
    └── test/
        ├── smoke.ts                 # headless geometry checks (40)
        └── render-harness.mjs       # headless draw-path checks (13, recording canvas)
```

### Scope of slice 1 (faithful to the plan)

- The **six production projections** Lane A's `production_projection_catalog`
  emits: `patent_diagram`, `tree_hierarchy`, `numeric_series`,
  `categorical_set`, `flow_layered`, `sankey_flow`. Ported verbatim (pure
  `Atoms → positions` functions); only dead type-imports trimmed.
- A **vanilla 2D-canvas renderer** (NOT the React `AtomSubstrate`/cosmos.gl —
  that is the later enrichment): runs the chosen projection, fits the scene to
  the viewport, draws relations as lines + atoms as kind-glyphs with labels,
  hover hit-tests real positions. Visual vocabulary is continuous with the
  substrate baseline (`genericTerminalState`: circles + lines) and the browser
  chrome (`serp.html`: paper/ink/terracotta, JetBrains Mono).
- **esbuild IIFE bundle** (51.5 KB, d3 inlined) embedded in the Rust crate and
  injected into `scene-host.html` at serve time, exactly like `rustyred-web`
  inlines vendored d3 into `serp.html`.
- **Rust serve helper** `render_scene_html(package_json)` +
  `render_scene(&ScenePackageV2)` — the SERP injection pattern, with the same
  `<`/`>`/`&` → `\uXXXX` escaping so an untrusted atom label cannot break out
  of the `<script>` payload block.

### Honesty guarantees (no fake UI)

- Unknown projection id → freeform fallback, **named in the header note**
  ("Projection X is not available in this build — rendered in freeform space"),
  never a blank canvas pretending to be the requested view.
- No atoms / invalid package / null package → explicit empty state, never
  fabricated content.
- Hover/select are wired to real atom data (source-ref counts), no theater.
- The `?mock` shadow-product path does not exist here; the renderer draws only
  the package it is handed.

## The seam (unchanged)

Lane B consumes the **scene-package-v2 wire JSON** Lane A produces. The Rust
`ScenePackageV2` (`#[serde(rename_all = "camelCase")]`) serializes
`manifestRef`, `sourceId`/`targetId`, kebab `lifecycle`/`space` — byte-identical
to the TS `v2-package.ts` + `atoms/types.ts` this bundle validates against.
The renderer is engine-agnostic: it draws whatever atoms + projection it is
handed, so it works against Lane A's Rust director OR (transitionally) the
Python one.

## Verification

| Check | Result |
|-------|--------|
| `cargo test -p scene-os-web` (5) | pass — injection, marker consumption, JSON re-parse after escaping, script-breakout neutralized, null empty page, typed/string parity |
| `tsc --noEmit` (strict) | clean |
| `node build.mjs` | dist/scene-os.bundle.js, 51.5 KB, d3 inlined, self-contained IIFE |
| `test/smoke.ts` (40) | all 6 projections place every atom at finite positions, non-degenerate bounds; unknown-id → freeform; positionless → grid |
| `test/render-harness.mjs` (13) | paint path runs without throwing; setTransform/arc/rect/stroke/fill/moveTo/lineTo/fillText all fire; header + empty state correct |
| `cargo run --example render_sample` | wrote a 57 KB self-contained page (the exact bytes Servo would serve) |
| browser screenshot (headless chromium) | scene renders correctly + immediately (≤150 ms): tidy-tree layout, kind-colored glyphs (teal sources, green person, terracotta claim/evidence), arrowed relations, labels with paper halos, no console errors |

Visual verification surfaced + fixed a real bug: the original enter-fade was
rAF-driven, and `requestAnimationFrame` is throttled in headless / background
documents, so the canvas stayed blank until the page was foregrounded. The
recording-context harness could not catch it (it stubs rAF to fire
synchronously). Fixed by painting synchronously at full opacity — lifecycle
per-atom opacity (entering/leaving) stays because it is data-driven, not
time-driven. Reproduce the screenshot via `test/screenshot.mjs` (optional
playwright, documented in the file).

## Handoff to Lane C (Codex, plan step 7)

`apps/browser` intercepts a scene URL → Lane A produces the `ScenePackageV2` →
call **`scene_os_web::render_scene(&package)`** (or `render_scene_html(&json)`
for the string form / Python-produced package) → serve the returned HTML. This
is the exact analog of how `apps/browser` already wires
`rustyred_web::render_serp_html`. `examples/render_sample.rs` is a runnable
reference of the call.

## Later enrichments (not slice 1)

- **Done (2026-05-30):** the `graph` coordinate space now has a real layout —
  `graph_force` runs d3-force SYNCHRONOUSLY in the projection (zone-anchored
  charge/collide/link recipe from `graphLayout.ts`) and returns settled
  positions, so the canvas draws a well-spaced constellation with NO cosmos.gl
  and NO React. The Theseus-UI `graph_force` adapter only seeded a ring and
  delegated the solve to cosmos.gl; this one solves itself. A cosmos.gl/WebGL
  force engine is only needed later for very large graphs (live GPU sim); normal
  scenes settle fine in-canvas. See `GraphForceProjection.ts`.
- Graph **reactivity** to match theoremweb.com/coordination-room: drag (pin +
  re-settle), a selection inspector panel + metric cards (the SceneHost chrome),
  edge labels, node glyph initials, and the dashed editorial annotations. The
  layout (well-spaced) is done; this is the interaction + chrome layer.
- Choreographed enter / morph transitions between recompiles (the substrate's
  lifecycle/transition machinery). Slice 1 paints synchronously at full opacity;
  time-based motion was removed after it proved fragile under rAF throttling and
  returns with the real choreographer, which must keep the scene visible from
  frame 0.
- The patent `<PatentDiagram>` Graphviz chrome (slice 1 renders patent atoms in
  the column-placement baseline the adapter provides).
- Additional projections (geo, cinematic, matrix, image, heatmap) when Lane A's
  catalog grows to emit them — kept out now so the bundle renders exactly what
  the director can produce, no speculative dead code.
