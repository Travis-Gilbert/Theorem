# Theorem iOS — design-system addendum implementation plan

**Mode: plan (Theorem's Harness).** Source of truth:
`~/Downloads/SPEC-THEOREM-IOS-V1-ADDENDUM-DESIGN.md`. This addendum **supersedes**
the dark theme shipped in `f3fe033` and the placeholder visuals. Register: a
single continuous precision instrument — a Braun-era workbench (chrome) with
patent drafting sheets (content).

Lane: Swift/UI (`apps/theorem-ios`, claude-code). Backend dependencies
(patent-register scene packages from Scene OS) are flagged for Codex.

**Prerequisite check (done):** the OFL font zips exist in `~/Downloads/`
(`karrik_fonts-main.zip`, `Terminal-Grotesque-master.zip`, `jgs-main.zip`); IBM
Plex Sans is already bundled. The font phase is unblocked.

**Open decision to resolve before D7:** patent-callout drill-down is (A) a new
full plate that stacks/replaces, or (B) an in-place note expansion. Addendum
recommends (A); (B) is the lighter v1. Needs the user before the patent renderer.

---

## Phase D1 — Instrument palette + font stack + hierarchy flip (foundation)

The biggest immediate visual shift; replaces the dark theme. No backend dependency.

- [x] **D1.1** Replace `TheoremTheme.defaultPalette` with the instrument tokens:
  field `#F6F5F2`, chrome `#EAE8E2` (deepen to `#E2E0DA` if more zone separation
  wanted — do NOT tint blue), pebble `#C8C4BC`, edge `#2A2823`, hairline
  `rgba(42,40,35,0.12)`, rule `…0.42`, rule-strong `…0.72`, blueprint-ink
  `#1F4063`, signal/oxblood `#7B2E26`, text-muted `…0.62`, text-faint `…0.42`.
  Re-map the role names (nodeCore/web/tool/ring* collapse — the graph is now
  monochrome; keep `signal` for selection only). Backref: "Color tokens".
- [x] **D1.2** Font stack (all OFL, extract from the Downloads zips → bundle +
  register): **Karrik** Regular = display + section headers (REPLACES Archivo
  Black; hero 32px one-per-surface, headers 22/17px); IBM Plex Sans = body 14/400
  (already bundled); **JetBrains Mono** = data readouts 13/500 tabular;
  **Terminal Grotesque** = code/flavor labels (NOT data); **jgs9** = patent-frame
  ornament (optional, file as texture). Update `TheoremFonts`: `display` →
  Karrik, add `data` (JetBrains Mono) + `flavor` (Terminal Grotesque) tokens.
  Backref: "Font stack".
- [x] **D1.3** Hierarchy flip: kill the giant `selectedTitle`/"Substrate Scene"
  screen-title. The **query** is the headline (Karrik or Plex SemiCondensed 600,
  ~22px); the scene-type caption ("FORCE · 5 NODES") is a small muted
  instrument-label above it (Plex Sans 600, 11px, uppercase, +0.08em tracking,
  text-muted). Prominent type = query + center-node name, never a generic label.
  Edit `TheoremSceneView.sceneHeader`. Backref: "Hierarchy flip".

## Phase D2 — Monochrome ink graph + restored annotations

- [x] **D2.1** Graph goes monochrome (remove the jewel-tone node colors):
  *(done in the D1 pass — TheoremSceneView Canvas + ForceGraphView Grape hero)*
  nodes = `edge` ink outline (1.2px) filled with `field`; edges = `rule` lines;
  selection flips the node to `signal`/oxblood at 2px stroke. Edit
  `TheoremSceneView` `color(for:)` / draw. Backref: "Color tokens" graph para +
  acceptance.
- [x] **D2.2** Annotations: render `relation.kind` as edge labels (JetBrains
  Mono) and `atom.label` as node labels. Above ~12 nodes, edge labels only for
  the selected node's edges; others fade in on zoom. Backref: "Annotations
  restored". (A graph of unlabeled dots is decoration; labeled edges are the
  product.)

## Phase D3 — MT19937 + hex-blueprint substrate watermark

- [x] **D3.1** MT19937 generator, native Swift (~40 lines, canonical 624-word
  state + init/temper constants). Must reproduce the project's generator outputs
  for the same seed (coordinate the canonical constants/seed source if one
  exists). Backref: "MT19937 + hexbin substrate texture".
- [x] **D3.2** Hex-grid geometry (~15 lines: centers offset x by `r*1.5`, y by
  `r*sqrt(3)`, alt rows staggered `r*sqrt(3)/2`, flat-top hex path) + seeded fill
  (walk fixed order; per hex draw next MT value in [0,1); `< frequency` →
  blueprint-ink, else field). Default coverage 8–15% (watermark, not
  checkerboard). Static per scene; seed once, render once; seed changes between
  scenes (per-scene fingerprint). Behind the field, content at full contrast.
  Do NOT use d3-hexbin (binning ≠ tiling). Backref: same.

## Phase D4 — Algorithm switcher revealed on search (not always-on)

- [x] **D4.1** Rework Codex's always-on `controlDeck`: idle home = field + the
  search/ask pill only (no algorithm bar). Tap the SEARCH half → the pill expands
  upward and the algorithm options live INSIDE that expanded search surface
  (same Dynamic Island expand mechanic). After a scene resolves, the active
  algorithm shows as a small caption/dock label; tapping it re-opens the switcher
  to re-project. Surfaces (the 5 IA tabs) and projections are distinct concerns —
  reassess whether surfaces also belong in the island vs a separate affordance.
  Honest-shape rule still gates which projections light. Backref: "Interaction
  change: algorithms revealed on search".

## Phase D5 — Motion discipline + Pow

- [ ] **D5.1** Add `Pow` (`movingparts-io/Pow`) to Package.swift. Chrome motion:
  fast/crisp 150–250ms, `cubic-bezier(0.22,1,0.36,1)`, no spring overshoot.
  Reserve spring for DATA motion only (graph layout convergence, fractal
  wavefront). `prefers-reduced-motion` → ~0ms. Backref: "Motion" + "Libraries".

## Phase D6 — Scramble-text reveal (bigger)

- [ ] **D6.1** Native scramble-text (~40 lines): `TimelineView(.animation)` tick;
  per-char show random scramble-set `["░","▒","▓","█"]`+alnum until reveal time,
  then lock real char; reveal front advances L→R; render MONOSPACE (Terminal
  Grotesque/JetBrains Mono) so width doesn't reflow; `.sensoryFeedback(.selection)`
  on settle; reduced-motion → final text immediately. Use for page results,
  snippets, AI summaries — NOT chrome labels / numeric readouts. (Requires a
  text-result surface to exist first; depends on the answer/summary UI.) Backref:
  "Interaction: scramble-text reveal".

## Phase D7 — Patent-callout plates (biggest; backend-coupled)

- [ ] **D7.0** RESOLVE the open A/B drill-down decision with the user.
- [ ] **D7.1** Native patent plate renderer (`Canvas`): white sheet (`field`),
  black ink (`edge`) linework, numbered callouts with thin lead lines, serif
  title block, figure views, sheet footer. Reimplement the d3-annotation grammar
  natively (label/callout/badge + connector lead-lines + tap-to-reveal-note);
  d3-annotation is the REFERENCE, not a dependency. Backref: "Interaction:
  patent-callout click-through".
- [ ] **D7.2** Node tap / "how does X work" answer → lay a patent plate over the
  field; callouts drill into deeper plates per the resolved A/B decision. Backref:
  same + acceptance.
- [ ] **D7.3 [Codex / backend dependency]** Scene OS must emit patent-register
  scene packages (`ScenePackageV2` with a patent projection + callout/figure/
  legend structure) for the iOS renderer to draw. Coordinate: this is the content
  side of the patent plate; the native renderer (D7.1) draws what Scene OS
  produces. Backref: "Platform split".

---

## Lane split

- **claude-code (Swift/UI):** D1–D6 + D7.1/D7.2 — all in `apps/theorem-ios`.
- **Codex (backend/Rust):** D7.3 (patent scene-package emission from Scene OS);
  optionally the canonical MT19937 seed/constants source for D3.1; the fast
  RustyRed search endpoint (already in flight from the prior round).

## Acceptance (from the addendum "What good looks like")

1. Screen reads as zones divided by visible ink rules, not a flat plane.
2. Only color = oxblood on selection/active (2–3%) + blueprint-blue in the hex
   watermark.
3. Hex texture reads as a faint seeded substrate, not a checkerboard.
4. Query + center-node name are the prominent type; no giant meaningless title.
5. Graph edges labeled; monochrome with oxblood selection.
6. Algorithm switcher hidden until search is tapped.
7. Results/snippets/summaries decode via scramble-text in monospace.
8. Node tap / "how X works" lays a patent plate (white sheet, black ink, numbered
   callouts) over the field; callouts drill into deeper plates.
9. Karrik carries display + headers without feeling heavy; data is JetBrains
   Mono; code/flavor is Terminal Grotesque.

## Verification

- Per phase: `xcodebuild` build clean + a simulator screenshot (the visual gate;
  the addendum is a visual spec, so each phase is judged on a screenshot against
  the acceptance list).
- D3 determinism: same seed → byte-identical hex pattern (a unit check on the
  MT19937 + fill walk).
- D2.2 / D4.1 / hierarchy: judged on the screenshot (labels present, switcher
  hidden at idle, query is the headline).

## Build order

D1 (palette + fonts + hierarchy) → D2 (mono graph + labels) → D3 (hex watermark)
→ D4 (on-search reveal) → D5 (Pow + motion) → then the bigger D6 (scramble-text)
and D7 (patent plates, gated on the A/B decision + Codex's scene data). D1 is the
single biggest visual correction and lands first.
