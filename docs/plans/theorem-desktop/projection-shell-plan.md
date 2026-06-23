# Implementation Plan: Projection Shell (Omnibox-Centered Canvas)

**Branch**: `011-projection-shell` | **Date**: 2026-06-09 | **Spec**: `docs/plans/theorem-desktop/design-v2-projection-os.md`

**Input**: Feature specification from `docs/plans/theorem-desktop/design-v2-projection-os.md` (the v2 north-star), section 7 resolution: the omnibox is the center of the canvas; the canvas and the SERP are one surface; the application has a browser, it is not a browser.

## Summary

Build the desktop's home surface: an infinite canvas (JSON Canvas 1.0) with the omnibox docked at its center. A query does not navigate to a results page; it materializes results as cards around the omnibox, in two phases (cards arrive plural and in motion from the live event stream; a synthesis card resolves singular and at rest with a collapsed provenance line). Every Theorem capability is a card-producing verb on the same box: search produces result cards, a URL produces a live-page handle card plus an ingestion card, an agent task produces a run card, a repo produces a code-map card, a scene request compiles through scene-os-core's existing selection pipeline (`classify_goal`, `detect_shape`, `select_projection`, `select_chrome`, `compile_scene_package`) into a scene card. The model participates only through bounded vocabularies: canvas-JSON edits for auto-organization, package selection from trusted catalogs, actions defaulting `proposal_only`. The browser is one surface reachable from cards, not the shell.

## Technical Context

**Language/Version**: Rust (workspace toolchain, Tauri v2 shell + rustyredcore_THG crates); TypeScript/React in `apps/desktop` (wry host)

**Primary Dependencies**: scene-os-core (catalogs, compile, select; `ScenePackageV2`), JSON Canvas 1.0 (format, adopted verbatim), tauri-specta 2.0.0-rc.25 with exact `=` pins (typed commands + events; events carry card-materialization streams), cosmos.gl (scene-card graph renderer), local harness node crates (RustyRed/THG embedded, localhost MCP), reconstruction-engine (generative projection class; standalone workspace until its strip session, consumed only behind the scene seam)

**Storage**: the local node (RustyRed/THG). Canvas documents are graph documents of kind canvas; positions live in the canvas doc, identity in the graph; one entity may appear on many canvases. Prolly sync carries canvases across machines; `.canvas` JSON interops with Obsidian through the existing sync seam

**Testing**: cargo test for `projection-directive` and scene-os-core integration (directive validation, preset round-trip, canvas-to-graph write mapping); Vitest for canvas components (card focus order, nudge, preset-to-token resolution); Playwright gates for the omnibox-to-cards flow per the established UI gates doc; css_static + token_lint for the new token aliases

**Target Platform**: macOS desktop first (M1 baseline), Tauri v2 + wry; Servo surfaces arrive via phase 5 and are out of scope here except as card-promotion targets

**Project Type**: desktop-app (monorepo lane `apps/desktop` + `rustyredcore_THG/crates`)

**Performance Goals**: 60fps canvas pan/zoom at 200 cards on the M1 baseline; first result card materializes before the full result set (streamed, not batched); canvas docs up to ~500 cards open without virtualization complaints (virtualize past that)

**Constraints**: offline-capable against the local node (queries over admitted graph state work with no network); JSON Canvas 1.0 conformance with no field extensions; presets-only color writing; reduced-motion zeroing on all pan/zoom/materialization animation; no raw hex outside token files

**Scale/Scope**: one shell surface, four card families (result, run, scene, page-handle), the omnibox verb set (search, URL, /agent, /crawl, /scene, /skill, /organize), canvas persistence + shelf (history as named canvases, one per query session)

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

The de facto constitution for this lane, from the v2 north-star and standing corrections:

- Bounded-vocabulary invariant (models choose within enumerated vocabularies; trusted code interprets): PASS. The model's only write surfaces here are canvas-JSON edits, catalog selections, and `proposal_only` actions.
- Two surfaces never confused (live page holds behavior and actuation; projections are synthesized and inert): PASS. The shell renders projections; a live page opens as the browser surface, with the card as its handle.
- One engine seam, one Servo pin: PASS by exclusion. This feature is wry-hosted; it consumes no Servo API and introduces no second engine path.
- Tokens before pixels, contrast computed not eyed: PASS. New tokens are aliases (`--edge-advisory`, `--chip-advisory`); presets map to existing tokens; reserved presets render neutral rather than inventing hex.
- Named choices are requirements: PASS. JSON Canvas 1.0 verbatim, ScenePackageV2 as the scene payload, presets-only writing, omnibox-at-center are all named in the spec.
- Actions are proposals by default: PASS. `ActionDescriptor.proposal_only` defaults true in scene-os-core; canvas edits by the model land as advisory-styled until accepted.

No violations to justify; Complexity Tracking is empty.

## Project Structure

### Documentation (this feature)

```text
docs/plans/theorem-desktop/
├── design-v2-projection-os.md        # The feature spec (north-star, section 7 resolved)
├── projection-shell-plan.md          # This file
└── projection-shell-checklist.md     # The acceptance checklist
```

### Source Code (repository root)

```text
rustyredcore_THG/crates/
├── projection-directive/             # NEW small crate: ProjectionDirective envelope,
│   └── src/
│       ├── lib.rs                    #   kind discriminator (style|scene|canvas|splat)
│       ├── canvas.rs                 #   JSON Canvas 1.0 types + conformance validation
│       └── validate.rs               #   schema validation; scene kind delegates to
│                                     #   scene-os-core's ScenePackageV2 (no duplication)
├── scene-os-core/                    # EXISTING: catalogs, compile, select (unchanged;
│                                     #   the omnibox calls classify_goal/detect_shape/
│                                     #   select_*/compile_scene_package as-is)
└── reconstruction-engine/            # EXISTING standalone workspace; no coupling here;
                                      #   its strip session is a separate handoff

apps/desktop/
├── src/
│   ├── components/
│   │   ├── canvas/                   # NEW: CanvasSurface, Card (four families),
│   │   │   ├── CanvasSurface.tsx     #   GroupFrame, EdgeLayer, CardFocusRing
│   │   │   ├── Card.tsx
│   │   │   ├── GroupFrame.tsx
│   │   │   └── EdgeLayer.tsx
│   │   └── omnibox/                  # EXISTING combobox contract (job-010);
│   │                                 #   gains the center dock + verb routing
│   ├── lib/
│   │   └── bindings.ts               # tauri-specta generated (replaces hand-rolled
│   │                                 #   commands.ts entries for this feature)
│   └── state/
│       └── canvas.ts                 # canvas doc store + shelf (history of canvases)
└── src-tauri/src/
    └── projection/                   # directive validation entry, canvas doc CRUD
                                      #   against the local node, event emission for
                                      #   card materialization streams
```

**Structure Decision**: one new small crate (`projection-directive`) carrying the envelope and the JSON Canvas types so both the desktop and the harness validate identically; scene-os-core is consumed, not modified; the desktop frontend gains a `components/canvas` family and a `projection` command module behind the tauri-specta seam. The reconstruction engine stays a standalone workspace per its README until its strip session, and nothing in this feature path-deps it.

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

None.
