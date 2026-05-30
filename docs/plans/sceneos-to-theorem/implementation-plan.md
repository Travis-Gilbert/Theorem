# SceneOS → Theorem: port plan

**Date: 2026-05-30**
**Status: Grounding plan for a multi-agent port. Goal: the substrate-native browser owns the full SceneOS scene pipeline in-repo — scene direction AND rendering — so Servo renders generated/placed scenes in-process with RustyRed, no API boundary to Index-API.**

---

## Why

Today SceneOS lives in Index-API: the Python backend director
(`apps/notebook/scene_os/`: atoms, compile, LIDA `projection_select` /
`chrome_select`, catalogs) and the TypeScript renderer
(`Theseus-UI/src/scene-os/`: substrate, projections, SceneHost). The browser
(Theorem) has zero SceneOS integration. For the browser to display SceneOS
scenes natively — the GUI-for-AI surface where search, the reconstruction
engine, and SceneOS converge — SceneOS must be IN Theorem, in-process with the
Rust browser + RustyRed.

The reconstruction engine is already ported (`reconstruction-engine` crate) and
already emits SceneOS atoms from inside Theorem (`scene_atoms.rs`). That is the
generative half. This plan brings the rest: the director that turns a response
into a scene package, and the renderer that draws it.

## The seam

The **atoms JSON contract** (`SceneScene { atoms, relations }`, the v2 wire
format: camelCase `sourceId`/`targetId`, `lifecycle`, optional `position`)
splits the work cleanly:

- **Above the seam (director, lane A):** decides which projection + chrome, and
  produces the atoms + relations. Rust, in-process.
- **Below the seam (renderer, lane B):** takes a scene package (atoms +
  projection + chrome) and draws it. Web bundle Servo serves.

The contract already exists on both sides (Rust `scene_atoms.rs`; Python
`atoms.py` to_dict/from_dict). It is the stable interface the two lanes build to
independently.

## Lanes

### Lane A — Scene director in Rust (Codex)

Reimplement the SceneOS backend in Rust, in `Theorem/rustyredcore_THG/crates/`
(or `apps/browser-substrate`-adjacent), in-process with RustyRed:

1. **atoms** — Rust `Atom`/`Relation`/`AtomPosition`/`SceneScene` (lifecycle,
   optional visual attrs). Started: `reconstruction-engine/src/scene_atoms.rs`
   has `SceneAtom`/`SceneRelation`. Promote to a shared `scene-os-core` crate.
2. **catalogs** — `ProjectionCapability` / `ChromeCapability` registry (the 6
   production projections + 2 chromes; data, no Python).
3. **compile** — turn a response (TheseusResponse / a graph slice) into a scene
   package: select atoms, build relations, attach provenance.
4. **LIDA select** — `projection_select` + `chrome_select`: classify goal + data
   shape, pick the projection + chrome from the catalog.

Output: a scene package (atoms + relations + chosen projection id + chrome id +
camera hint) as the JSON the renderer consumes.

Decision for Codex: Rust reimplementation (in-process, the ideal, larger) vs a
thin Python port to `Theorem/apps/notebook/scene_os` with a process boundary
(faster, but not in-process). Travis's steer: Rust, in-process.

### Lane B — Renderer as a Theorem web bundle (Claude Code)

Extract the SceneOS TS renderer into a self-contained web bundle the Servo
browser serves locally (the SERP pattern: no CDN, no Theseus-UI deployment
dependency):

1. The projection adapters (`projections/*.ts`: tree, sankey, numeric-series,
   categorical-set, flow-layered, patent-diagram + the d3 layouts) — pure
   placement functions, already self-contained.
2. The substrate + renderer (`substrate/`, `AtomSubstrate.tsx`, glyphs) — the
   canvas/SVG that draws atoms by projection-computed positions.
3. `SceneHost` shell + chrome.
4. Bundle entry: takes a scene-package JSON (lane A's output) → renders.
   esbuild IIFE, inlined deps (d3, etc.), like the SERP, so Servo serves one
   self-contained asset.

The renderer is engine-agnostic: it draws whatever atoms + projection it's
handed, so it works against lane A's Rust director or (transitionally) the
Python one.

### Lane C — Servo integration (Codex, step 7)

`apps/browser` intercepts a scene URL → lane A produces the scene package → serve
lane B's bundle with the package injected (the SERP injection pattern) → Servo
renders. In-process: the director and the graph are in the same process as the
browser.

Status: first slice shipped in `apps/browser`. The browser now intercepts
`http://theorem.local/scene?q=...`, turns browser-session substrate search into
a `SceneScene`, compiles it with `scene-os-core`, and serves the `scene-os-web`
HTML through Servo's `load_web_resource` hook. See
`lane-c-servo-integration.md`.

## Sequencing

1. Lane A atoms + catalogs (data layer; small, builds on `scene_atoms.rs`).
2. Lane B bundle extraction (parallel; renderer is independent of the director).
3. Lane A compile + LIDA-select (the director logic).
4. Lane C wiring (needs A's package output + B's bundle).

Lanes A and B are parallel across the atoms-JSON seam. C is last.

## Acceptance

A scene URL in the Servo browser → the Rust director produces a scene package →
the web bundle renders it in Servo, in-process, with no call to Index-API. The
reconstruction engine's generated atoms (already in Theorem) flow through the
same director + renderer.

## What stays in Index-API

Index-API's Python SceneOS + Theseus-UI keep serving the existing web app
(`/theseus`, the console). The Theorem copy diverges for the browser — same
"they do different things" discipline as the reconstruction engine + civic. The
atoms-JSON contract keeps both renderers interoperable.
