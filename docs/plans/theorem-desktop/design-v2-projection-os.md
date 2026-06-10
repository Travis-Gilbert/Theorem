# Theorem Desktop v2: the projection OS

Author: claude (claude.ai). Successor north-star to
`design-and-agent-surface-synthesis.md` (job-010). Synthesizes four inputs into
one design: the UI-as-database-projection principle (the malleable-UI
examination, grounded in Servo source), the harness UI spec (the app as the
harness, 2026-05-31, previously unbuilt and now live-relevant because the
desktop embeds a local harness node), the reflexive RustyRed plan (three
learned organs, 2026-06-09), and two external formats examined at source:
JSON Canvas 1.0 (obsidianmd/jsoncanvas) and brush (ArthurBrussee/brush,
Gaussian splatting on Burn + wgpu).

Register: north-star with concrete contracts. Named choices are requirements.
The job-010 deliverables (tokens, APG contracts, epistemic moments, fence
correction) stand unchanged underneath this; nothing here edits them.

---

## 1. The governing invariant: one rule on both sides of the seam

The reflexive RustyRed plan closes on one rule: a learned model ranks or
steers within a bounded, enumerated space; it never authors free-form output.
Densification gates and quarantines inferred edges. The optimizer steers among
the native planner's candidates. The browser-use agent picks from a fixed
action catalog. The context scorer ranks atoms under a budget.

The projection principle is the same rule wearing a UI costume: the model
orchestrates a constrained vocabulary (utility classes, semantic edges,
scene directives, canvas documents), each interpreted by a trusted renderer.
It never authors raw CSS or raw JS.

Stated once, for both sides of the database/UI seam:

> Models choose within enumerated vocabularies; trusted code interprets the
> choice. The vocabulary gives a safety floor, small data needs, and a blast
> radius you can reason about. This holds for what the database writes to
> itself and for what the model paints on screen.

Everything below is this invariant applied to surfaces.

---

## 2. The projection architecture

The desktop holds the local node (RustyRed/THG embedded as a crate, localhost
MCP, Prolly sync). Once a page, a run, a repo, or a document is admitted to
the local graph, presentation is downstream and ours. The UI is a set of
projections over that graph.

### Two kinds of surface, never confused

- The live page: a real site in a Servo webview. It holds behavior (handlers,
  auth, sessions) and is the actuation target for the browser-use lane
  (coordinate synthesis per `build-step-1-correction-actuation.md`). It is
  never mutated to restyle it.
- The projection: a synthesized view backed by the same graph. It is a faithful
  rendering of admitted state and structure, inert by default; operating on
  the remote system routes intent back through the live page via the executor.
  Projection captures state and structure, not behavior. The two halves
  (project out, actuate back) are one product.

### Three render hosts, each doing what it is mature at

- wry (system webview): app-chrome projections. The canvas workbench, run
  views, graph views (cosmos.gl), dossiers, settings. Full web-platform
  maturity, the existing apps/desktop frontend, the tauri-specta seam.
- Servo: live-web and agent surfaces (phase 5), plus web-document projections,
  which are synthesized HTML served in-process via `WebResourceLoad::intercept`
  (or data URLs) with the design system injected through
  `UserContentManager::add_stylesheet` (both verified in servo source:
  `components/servo/user_content_manager.rs`,
  `components/servo/webview_delegate.rs`). Updates apply on reload, which fits
  the projection lifecycle (a projection re-renders, it does not live-patch).
- brush (native wgpu): the geometric scene surface, exploratory, section 5.
  Pure Rust beside the local node, no web engine involved.

### The directive envelope (the one new contract)

One envelope, one validation layer, one renderer per kind:

```
ProjectionDirective {
  kind: "style" | "scene" | "canvas" | "splat",
  version: u32,
  source: { graph_query | doc_id | run_id },
  payload: <kind-specific, schema-validated>
}
```

- `style`: utility-class assignments and token-backed semantic classes on a
  synthesized document. The vocabulary is the injected design-system sheet
  (a curated utility subset compiled from tokens.css, served once through
  `UserContentManager`); the model assigns classes, the sheet defines them.
  Dynamic state arrives as semantic edges in the graph (for example a
  flagged-error edge) that the projector maps to token-backed classes
  (`.is-error`, `.is-flagged`), not inline styles. Raw inline style is the
  rare escape hatch, never the mechanism.
- `scene`: the existing cosmos.gl SceneDirective, unchanged, now formally one
  kind in the envelope.
- `canvas`: a JSON Canvas 1.0 document, section 4.
- `splat`: reserved for the brush surface, section 5.

The schema validator already hardened in build-step-1
(`validate_against_schema`) is the natural validation layer for payloads:
unknown kind is an error, unknown fields are errors, the directive either
validates or does not render.

### The reflexive consequence: epistemic status is visible by construction

The reflexive plan quarantines what models invent: inferred edges carry
`admission_tier` / `confidence_ceiling` and are advisory until corroborated.
Projections render that status, always:

- Advisory (inferred, uncorroborated) edges and nodes render visually distinct
  from corroborated ones in every projection kind. Token aliases, not new hex:
  `--edge-corroborated` (default ink), `--edge-advisory` (reduced-alpha ink +
  dashed stroke), `--chip-advisory` (neutral chip + brass border, consistent
  with `--accent-agent` meaning "a machine did this").
- The dashed-stroke channel is deliberate: advisory status must survive
  grayscale and colorblind rendering, so the encoding is stroke style plus
  color, never color alone (non-text contrast duty, 3:1, computed not eyed).
- This generalizes job-010's epistemic moments: the known-context strip and
  ingestion badge were point features; advisory styling is the same honesty
  applied to every painted edge.

The steered planner (Bao-style) gets the same treatment in the run surface:
the candidate set, the pick, and the value model's abstention (cold start,
native plan ran) are recorded run events, rendered in the cost rail. Why this
plan, one tap away.

---

## 3. The app as the harness, landed on the desktop

The harness UI spec was written for Commonplace V3 and never built. Its
premise (the UI renders the RUN, ambient but legible) is now directly
buildable here because the desktop embeds the local harness node: the run
events, contributions, tensions, costs, and traces are rows in the local
graph, which makes every surface in that spec a projection in this
architecture. The spec's surfaces map as follows, unchanged in intent,
re-grounded in mechanism:

- The room renders the run. Generation phase: the event stream rendered live
  (plural, in motion), honest because it is the actual event stream from the
  local node, not loader theater. Resolution phase: one synthesized answer in
  one voice, with the provenance line collapsed beneath it ("synthesized from
  N participants, K tensions, grounded in M sources, $cost"), expanding into
  the detail. The provenance layer is a projection over the run subgraph.
- The run detail rails are three named projections: evidence (the context
  artifact's atoms, maps loaded, sources), cost (token ledger, cascade
  decisions, and now the steered-planner candidates), outcome (files changed,
  validators run, writes back, learning proposed). Replay and fork are
  buttons on a past run because the kernel supports them.
- Maps are orientation, not file trees: a curated projection ("the terrain
  that matters"), explicitly distinct from the exhaustive graph view. The
  map is the orientation projection; cosmos.gl is the everything projection;
  the canvas (section 4) is the workbench projection. Three projections, three
  jobs.
- Presence is honest status, no avatars or idle theater, per the spec.

Ambient but legible, restated for this architecture: ambient because the user
works in surfaces (canvas, browser, room) and never operates the harness;
legible because every surface is a projection of graph state, so what did it
read, what did it cost, what did it do is always one projection away, and
never a reconstruction.

---

## 4. JSON Canvas as the workbench and the interaction model

Adopt JSON Canvas 1.0 verbatim (obsidianmd/jsoncanvas, spec/1.0.md, read at
source) as the `canvas` projection format. Named choice, therefore a
requirement: the canvas payload is a conforming JSON Canvas document, no
extensions to the core fields in v1.

Why it fits, precisely:

- It is a bounded vocabulary by design: four node types (text, file, link,
  group), four sides, two edge ends, six preset colors whose values the spec
  intentionally leaves undefined "so that applications can tailor the presets
  to their brand colors." It was built to be token-mapped. A model editing a
  canvas chooses among enumerations; it cannot author style. This is the
  governing invariant shipped as a file format.
- It is a file format, so the workbench is durable, diffable, Prolly-syncable,
  and interoperable with Obsidian through the existing Obsidian sync seam
  (canvas docs ride the same pipe as notes; `.canvas` files are JSON).
- It is tiny (the whole spec is under 4KB), so the renderer and the writer are
  small, and conformance is checkable.

### The preset-to-token mapping (defined now, token-disciplined)

- `"1"` red: the error/signal token (maps to the existing error semantic).
- `"3"` yellow: `--accent-agent` (brass). Agent and ingestion artifacts.
- `"4"` green: `--accent-memory` (pcb-green). Memory and corroborated
  knowledge.
- `"2"` orange, `"5"` cyan, `"6"` purple: reserved. They render as neutral
  chip styling until a semantic need names them; no hex is invented to fill
  a slot (tokens before pixels).
- Contrast duties carry over from the job-010 findings: preset colors are
  fills, borders, and chips on cards, never body text on light surfaces
  (brass fails at ~2.4:1; pcb-green fails body at ~4.2:1). Card text is ink
  tokens; presets color the card, not the words.
- Writing policy: the app and the model write presets only. Hex values in
  `canvasColor` are accepted on import for interop and preserved round-trip,
  never emitted.

### The interaction model: cards on an infinite canvas, both directions

The canvas is the workbench where heterogeneous things compose spatially:

- A run is a card (text node referencing `run_id`, rendering the resolution
  surface compact, expanding to the rails).
- A dossier, a note, a memory doc: file nodes referencing graph documents.
- A live page or its projection: link nodes (the ingestion seam can admit the
  URL; an admitted page's card flips from link to file, visibly, which is the
  ingestion badge generalized).
- A group is a room: spatial containment maps to room scoping, so dragging
  cards into a group is an act of context curation the model can read.

Bidirectional, by construction:

- Graph to canvas: the model (or a rule) emits or edits a canvas document as a
  `canvas` directive. Layout proposals, clustering, tidying: all canvas-JSON
  edits, validated, rendered by trusted code.
- Canvas to graph: user manipulation (drag, connect, group, label) writes back.
  An edge drawn between two cards is a real graph edge (advisory styling
  applies if a model drew it; user-drawn edges are user-asserted). Positions
  and sizes are canvas-local layout, stored in the canvas document itself, not
  on the graph entities; the canvas doc is a graph document of kind canvas
  whose nodes reference entities by id. One entity can sit on many canvases at
  many positions, which is correct: layout is per-projection, identity is
  per-graph.

Keyboard contract (design-engineering, named now since there is no canonical
APG pattern for infinite canvas): cards take roving tabindex in document
order (z-order array order, which JSON Canvas already defines); arrow keys
nudge the focused card by the spacing grid step; Enter opens; Escape returns
focus to the canvas; zoom on standard keys; all pan/zoom animation behind the
motion tokens with the reduced-motion zeroing block. Focus ring token applies
to cards exactly as to controls.

---

## 5. Brush, honestly placed: the geometric surface

What brush is (verified at README, ArthurBrussee/brush): a Gaussian-splatting
reconstruction engine and viewer built on Burn and WebGPU-compatible tech,
shipping dependency-free binaries on macOS, Windows, Linux, Android, and the
browser, with training and rendering faster than gsplat.

Why it is stack-relevant rather than a shiny object: the reflexive plan's
learned organs are hand-rolled Burn layers (the SAGA message passing, the
Pairformer blocks), and the representation sidecar holds Burn tensors keyed by
node and edge id. Brush renders Gaussians whose parameters are Burn tensors.
So there is one tensor substrate from learned representation to rendered
geometry: sidecar embeddings project to Gaussian parameters (position, scale,
rotation, color, opacity) without leaving Rust or Burn, and the whole path is
differentiable.

The differentiable path is the genuinely novel capability, named so it is not
lost: learned layout. Optimize node positions by gradient descent through the
rasterizer against a rendered loss (edge length, occlusion, cluster
compactness). Graph drawing via gradient descent exists as research; doing it
through a production splat renderer over the database's own representation
sidecar is the new composition. Second capability, cheaper: embedding-space
flythrough, rendering the sidecar's actual geometry as a navigable field, the
graph seen in the model's space instead of a force layout's.

Scope, stated as a requirement: cosmos.gl remains the standard graph surface
(the everything projection, web-tech, proven at 50K+ points, already the
named renderer decision). Brush is the candidate for the native geometric
scene surface only, behind a flag, and `splat` stays a reserved directive
kind until the spike clears its gates.

Spike gates (observable, before any adoption decision):

- Compositing: a brush wgpu surface hosted in the Tauri v2 shell on macOS
  (separate window acceptable for the spike; composited layer is the target),
  rendering beside the wry frontend without fighting the event loop.
- Edges: Gaussians do not draw lines. The spike demonstrates one of:
  anisotropic splats elongated along edge direction, or a separate line pass
  over the same camera. Pick by visual quality at 1K edges.
- Budget: splat count and frame time at 10K nodes on the M1 baseline,
  reported, not estimated.
- Source: parameters read from a real sidecar export (even a synthetic one in
  the sidecar's schema), so the spike proves the substrate path, not a demo
  path.

Placement note: the native wgpu path deliberately bypasses both web engines.
Servo's WebGPU is young, and the system webview's WebGPU availability varies;
the geometric surface needs neither, which is the architectural argument for
brush over a web-based splat viewer here.

---

## 6. What lands where (dependency order, prose)

First, the directive envelope plus the canvas renderer: smallest surface,
highest leverage, the JSON Canvas spec is tiny, the preset-to-token table is
defined above, and it immediately gives the workbench and the bidirectional
interaction model. The schema-validation layer comes with it.

Second, the run-as-projection room view: the harness UI spec's part 1 reading
the local node's event stream, with the resolution surface and the collapsed
provenance line. Depends on the local node emitting persisted run events,
which is the same dependency the original spec named.

Third, advisory styling: the token aliases and dashed-stroke channel for
inferred edges, applied across canvas, scene, and run surfaces. Tiny, and it
is the reflexive plan's quarantine made visible.

Fourth, web-document projections: the injected design-system sheet through
`UserContentManager::add_stylesheet` and intercept-served synthesized
documents. Depends on the Servo embed being live in the desktop (phase 5
work), so it trails the wry-hosted projections deliberately.

Fifth, the brush spike against its gates, in parallel with any of the above
since it touches nothing shared.

---

## 7. Decisions requiring sign-off, and what is settled

Settled by this doc (named choices): JSON Canvas 1.0 verbatim as the canvas
format; presets-only writing policy; the preset-to-token table; cosmos.gl
retained as the standard graph surface; brush scoped to the geometric surface
behind gates; advisory styling as stroke-plus-color, never color alone; the
ProjectionDirective envelope with schema validation; projections hosted in
wry, live-web and agent surfaces in Servo, geometry native.

Open for Travis: whether the canvas workbench is the desktop's home surface
(the thing that opens on launch, with the omnibox over it) or a peer tab to
the browser surface. The harness UI spec's "the browser is the ambient
environment" suggests browser-as-home; the workbench model suggests
canvas-as-home with the browser one card-tap away. Both are coherent; the
choice shapes the launch frame and belongs to you.
