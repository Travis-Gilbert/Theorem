# Acceptance Checklist: Projection Shell (Omnibox-Centered Canvas)

**Purpose**: Observable acceptance criteria for the projection shell: the omnibox-centered canvas where the SERP and the workbench are one surface
**Created**: 2026-06-09
**Feature**: `docs/plans/theorem-desktop/projection-shell-plan.md` (plan) and `docs/plans/theorem-desktop/design-v2-projection-os.md` (spec, section 7 resolution)

## Envelope and Canvas Format

- [ ] CHK001 `projection-directive` crate validates all four kinds; an unknown kind and an unknown field each fail validation with a typed error, proven by cargo tests
- [ ] CHK002 The `scene` kind round-trips a `ScenePackageV2` produced by `compile_scene_package` byte-faithfully (no field loss, no re-modeling), proven by an integration test against scene-os-core
- [ ] CHK003 A canvas payload conforms to JSON Canvas 1.0: the four node types, sides, ends, and z-order-by-array-position parse and re-serialize without extension fields
- [ ] CHK004 A `.canvas` file exported from Obsidian imports, renders, and re-exports with hex colors preserved round-trip while the app itself emits presets only

## Omnibox to Materialization

- [ ] CHK005 A query typed in the centered omnibox produces result cards on the surrounding canvas without any navigation event (no route change, no page load)
- [ ] CHK006 Cards materialize from the streamed event sequence: the first card renders before the result set completes, observed in the Playwright gate
- [ ] CHK007 A synthesis card resolves after generation with the collapsed provenance line (participants, tensions, sources, cost) expanding on activation
- [ ] CHK008 A URL entered in the omnibox produces a page-handle card; opening it promotes to the browser surface and the card remains as the handle
- [ ] CHK009 An `/agent` task produces a run card whose expanded state shows the evidence, cost, and outcome rails read from the local node's run events
- [ ] CHK010 A `/scene` request routes through `classify_goal` and `detect_shape`; a selection refusal (`ProjectionSelectionRefusal` or `ChromeSelectionRefusal`) renders as an honest refusal card, not a fallback render
- [ ] CHK011 A `/crawl` on an ingested repo produces a code-map card (orientation projection), and the same repo's everything-projection opens in a cosmos.gl scene card

## Bidirectionality and Epistemic Status

- [ ] CHK012 Dragging a card writes the position into the canvas doc only; the referenced graph entity is unchanged, proven by a before/after graph read
- [ ] CHK013 A user-drawn edge between two cards creates a graph edge marked user-asserted; a model-proposed edge renders with `--edge-advisory` styling (reduced alpha plus dashed stroke) until accepted
- [ ] CHK014 An `/organize` pass lands only as canvas-JSON edits (positions, groups, preset colors); the diff contains no node deletions and no non-canvas writes, and the proposal is reversible in one action
- [ ] CHK015 Dragging cards into a group scopes them to a room context the model can read, observed by the next query's context including the grouped entities
- [ ] CHK016 An admitted page's card visibly flips from link to file (the ingestion badge generalized), driven by the ingestion event, not a poll

## Tokens, Keyboard, Motion

- [ ] CHK017 Preset colors resolve to tokens (1 to error, 3 to `--accent-agent`, 4 to `--accent-memory`; 2, 5, 6 neutral) with zero raw hex outside token files, proven by token_lint
- [ ] CHK018 Preset colors appear only as fills, borders, and chips; card body text uses ink tokens, with contrast computed by css_static (4.5:1 body, 3:1 non-text)
- [ ] CHK019 Cards take roving tabindex in z-order array order; arrow keys nudge by the spacing grid step; Enter opens; Escape returns focus to the canvas
- [ ] CHK020 All pan, zoom, and materialization animation runs through the motion tokens and zeroes under prefers-reduced-motion
- [ ] CHK021 The focus ring token renders on a focused card identically to controls

## Persistence and Shelf

- [ ] CHK022 A query session persists as a named canvas doc in the local node; the shelf lists past canvases and reopens one with positions intact
- [ ] CHK023 A canvas doc syncs across machines via Prolly sync and renders identically on the second machine
- [ ] CHK024 With networking disabled, a query over admitted graph state still materializes cards (offline floor)

## Out-of-Scope Guards

- [ ] CHK025 No Servo API is consumed by this feature; the browser surface promotion is an existing-tab handoff, verified by dependency inspection
- [ ] CHK026 The `splat` kind validates as reserved (accepted by the envelope, refused by the renderer with the spike-gates message) until the brush spike clears
- [ ] CHK027 reconstruction-engine remains outside workspace members and is not path-depped by `projection-directive` or the desktop, verified in Cargo metadata

## Notes

- Check items off as completed: `[x]`
- CHK010 and CHK014 are the invariant made testable: refusals render as refusals, and model writes are bounded, advisory, and reversible
- Link Playwright runs and cargo test output inline as items close
