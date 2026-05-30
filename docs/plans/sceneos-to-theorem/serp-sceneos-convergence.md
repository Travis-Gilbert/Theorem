# SERP + SceneOS render convergence — plan

**Date: 2026-05-30**
**Harness run: `run:5485143eb6e849a0af4ed1d0c71f0dc0` (actor claude-code, mode plan)**
**Status: plan. Renderer-side checklist owned by claude-code; routing convergence owned by Codex (coordination doc 635, awaiting ack).**

## Why (spec audit outcome)

Travis: *"the SERP and render are intended to be one thing but it would need the
search box regardless."* A review against
[`browser-procedural-sceneos-plan.md`](../../../../Downloads/browser-procedural-sceneos-plan.md)
Part C/D found five deviations in what Lane B shipped:

1. **Subset of d3, not the full library as projection math.** Part C slates the
   full d3 (scale, force, hierarchy, **geo, shape, contour, interpolate**) as the
   math that makes the renderer "capable across every coordinate space." The
   bundle has only hierarchy/scale/sankey/force.
2. **A scene was ported.** `render_coordination.rs` reproduces the
   coordination-room screen — *"what was on the other screen is not relevant to
   the actual browser."* The intent is general math/principles, not a copy.
3. **No search box / no search-as-graph.** Part D's core surface is search →
   nodes+edges. The SERP already has the box + full d3 + a force graph; the
   SceneOS surface (`scene-host.html`) has neither.
4. **No `d3-annotation`** (build-order step 4: the callout/explanation layer).
5. **No Mosaic** (`@uwdata/mosaic` + `duckdb-wasm`, the cross-filter data layer).

## Current state (two renderers, two routes)

`apps/browser` (Codex, Lane C `84bce56`) serves, from one `BrowserSessionStore`:

| Route | Renderer | Search box | d3 | Graph |
|---|---|---|---|---|
| `/search?q=` | `rustyred-web` `serp.html` (`render_serp_html`) | **yes** (`<form action=/search>`) | full vendored `d3.min.js` | bespoke `forceSimulation` (charge -230, link 62, collide, drag, zoom, ring colors) |
| `/scene?q=` | `scene-os-web` bundle (`render_scene`) | no | d3 module subset | SceneOS projection system (7 projections incl. `graph_force`) |

Two force graphs, two renderers, one of them search-capable. That is the split
the convergence closes.

## Target (one search-driven surface)

```
search box (always present)
  -> query
  -> search_substrate / native_search  (atoms + relations)
  -> SceneOS scene package
  -> SceneOS renderer (scene-os-web) with the FULL d3 math
  -> d3-annotation explanation layer
  (+ Mosaic cross-filter over node/edge attributes)
```

The SceneOS renderer becomes the single graph renderer; `serp.html`'s bespoke
force graph is generalized INTO the projection system (the `graph_force`
projection already is the d3-force layout — the SERP's ring/score coloring
becomes projection params, not a separate renderer). The search box from the
SERP lives on the unified surface.

## Checklist — renderer side (claude-code, `scene-os-web`)

- [ ] **R1 [Part C]** Add the full d3 math set as bundle deps: `d3-geo`,
  `d3-shape`, `d3-contour`, `d3-interpolate` (alongside scale/force/hierarchy/
  sankey). The math is present so the renderer can handle arbitrary scenarios,
  per Travis's "add the mathematics" directive — not gated on each projection.
- [ ] **R2 [Part C, build-order 3]** Wrap the new d3 layouts as SceneOS
  projections that prove the math generalizes: a `geo` projection (d3-geo
  Mercator/Albers over `geo` coordinate space) and a `heatmap_grid` projection
  (d3-contour). Each is the established adapter pattern; no speculative extras
  beyond proving capability across spaces.
- [x] **R3 [Part D]** Search input box on the unified surface
  (`scene-host.html`), `<form action="/search" method="get">` matching the
  existing route + studio-journal palette. (Done this pass — see below.)
- [ ] **R4 [Part D, build-order 5]** Search-as-graph: the SceneOS renderer
  consumes search results (`SearchHit`/`SearchLink` → atoms+relations →
  `graph_force`), generalizing the SERP's graph through the projection system.
  **Seam-gated**: depends on the unified data shape the route injects (Codex C1).
- [ ] **R5 [Part C, build-order 4]** `d3-annotation` explanation layer: callouts
  on nodes, with stated-vs-inferred provenance surfaced inline.
- [ ] **R6 [Mosaic]** `@uwdata/mosaic` + `@duckdb/duckdb-wasm` cross-filter layer
  over node/edge attributes (the data-driven scenarios path).
- [x] **R7 [audit #2]** Delete `render_coordination.rs` (rejected scene port).
  (Done this pass.)

## Checklist — routing side (Codex, `apps/browser`) — awaiting ack (doc 635)

- [ ] **C1** Converge `/search` + `/scene` to serve the unified `scene-os-web`
  surface: search box → `search_substrate` → scene package → `render_scene`.
  Define the single data shape the unified surface receives (scene package vs.
  the SERP's `SubstrateSearch`); R4 designs to whatever C1 settles.
- [ ] **C2** Retire (or keep as fallback) `serp.html`'s bespoke force graph once
  the SceneOS renderer is the single graph renderer.

## Lane split + coordination

- claude-code: R1–R7 (the renderer bundle). All renderer-side, no `apps/browser`
  edits.
- Codex: C1–C2 (`apps/browser` routing) + the data-shape seam. Coordinated via
  doc 635 (urgency: ask). No `apps/browser` edits from claude-code without ack.
- The seam is the unified data shape injected into the surface. R4 is gated on it.

## Validation

- `npx tsc --noEmit` + `node build.mjs` (bundle builds with full d3).
- `test/smoke.ts` (every projection places atoms) + `test/render-harness.mjs`
  (draw path) + `cargo test -p scene-os-web` (serve seam).
- Browser screenshot of the unified surface (search box + graph) once R4 lands.

## Done this pass

- R7: `render_coordination.rs` deleted.
- R3: search box added to `scene-host.html`.
- Plan + coordination claim recorded; build (R1, R2, R4–R6) gated on user review
  (message lag) + Codex C1 ack for the data-shape seam.
