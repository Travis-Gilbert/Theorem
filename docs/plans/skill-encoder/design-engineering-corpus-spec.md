# Design-Engineering Skill: Corpus and Build Spec

The design analog of rust-engineering, built by the same encoder (`theseus.skill_encoder.v1`, `apps/notebook/encode/`) and served through the same marketplace layout. Locked decision: the gate scores computable correctness (accessibility, token and scale adherence, structural soundness), not taste. Aesthetic judgment stays with the human and the lead model; the pack's measurable contract is correctness.

## Pack identity

- Pack id `skill-pack:design-engineering-general-v0.1`, kind `skill_pack`.
- Source ref `source:design-engineering-external-corpus-v0.1`, source class `code_corpus_v1`.
- Marketplace path `theorems-harness/skills/design-engineering/` (SKILL.md, provenance.json, references/, scripts/, same layout as rust-engineering).
- Provenance carries the same promotion block (state, canonical_ready, benchmark_treatment_beats_baseline, regression_signals, task_count) and ships `scanned` until the gate clears.

## Corpus packets

Three external layers plus first-party, one packet per layer so passes can run separately.

Layer 1, standards as authored rules, not ingested documents. WCAG 2.2 success criteria and ARIA Authoring Practices Guide keyboard contracts enter as authored rules in `_DESIGN_RULES`, citing SC numbers and pattern names. Do not ingest W3C prose into the corpus; the thresholds and interaction contracts are facts, the prose is not needed.

Layer 2, design system repos. Lower tokens, component sources, and documented rules; exclude brand imagery and marketing pages. Record the license in each packet entry during assembly.

- radix-ui/primitives (accessibility-first behavioral primitives)
- uswds/uswds (US government, public domain)
- alphagov/govuk-frontend (research-backed, documented rationale)
- carbon-design-system/carbon
- material-components/material-web
- primer/react
- shadcn-ui/ui (the working first-party stack's upstream)

Layer 3, motion and viz sub-corpus, narrower and advisory-leaning:

- observablehq/plot
- juliangarnier/anime

First-party packet `local://design-engineering-first-party-v0.1`: the token definitions and built components in `Travis-Gilbert/travisgilbert.me` and `rustyredcore_THG/crates/scene-os-web`. Patent/Braun palette (field #F6F5F2, chrome #EAE8E2, blueprint-ink #1F4063, oxblood #7B2E26, ink #2A2823), Atlas amber palette (ground #FBF3D7, panel #F6ECC6, selection #A8301E, focus #E6A23C), IBM Plex Sans Condensed, Vollkorn, IBM Plex Mono and Courier Prime. This layer is what makes the pack first-party rather than a generic linter.

## New lowering: CSS and tokens

`tree_sitter_parser` covers py/rust/ts/js/go/java today; design sources add CSS and token JSON.

- A CSS lowering path (tree-sitter-css grammar, or tinycss2 if staying pure Python) producing atoms for custom-property definitions, declarations, selectors, and media queries. Dialect `css_declaration_view`.
- A token lowering path for JSON token files (the Design Tokens Community Group format and plain JSON) producing `design_token` atoms (name, value, type, aliases). Dialect `design_token_view`.
- Component fixture lift: design-system component examples and stories become `design_component_fixture` atoms (name, source, fixture_hash) with `checker_engine=axe_render`, mirroring `rust_test_body_view` exactly: the real fixture content ships in the artifact, the content hash chains in parent_hashes, execution is delegated to the declared checker.

## Authored rule packs (`_DESIGN_RULES`)

Same shape as `_RUST_RULES` in `apps/notebook/encode/code_corpus.py`: ruleset, rule id, severity, checker, `source_fact_model=design_as_facts`, `validator_strategy=rule_as_checker_query`.

accessibility (promotable):
- contrast_minimum_met: 4.5:1 body text, 3:1 large text and UI components (WCAG 1.4.3, 1.4.11). Checker css_static on resolvable token pairs, axe_render for arbitrary components.
- target_size_minimum: 24px floor (WCAG 2.2, 2.5.8), 44px preferred for touch. css_static where dimensions are declared, axe_render otherwise.
- focus_visible_not_removed: no `outline: none` without a replacement focus style. css_static.
- reduced_motion_respected: a `prefers-reduced-motion` query exists wherever animation or transition is declared. css_static.
- form_controls_labeled: axe_render.
- heading_hierarchy_no_skips: axe_render.
- keyboard_contract_matches_apg: per pattern (dialog focus trap and Escape, combobox, tabs arrow navigation, menu roving tabindex). apg_behavioral, pending until the render substrate.

tokens_and_scale (promotable):
- spacing_on_grid: every spacing value a multiple of the declared base (4 or 8). css_static.
- colors_from_token_palette: no raw color literals outside token definition files. token_lint.
- type_scale_conformance: font sizes drawn from the declared modular scale ratio. css_static.
- radii_and_borders_tokenized: token_lint.

typography (promotable):
- measure_in_range: body text line length within 45-75 characters. css_static.
- line_height_floor: body line-height at least 1.4. css_static.
- minimum_body_size: 16px body default. css_static.

layout_grid (advisory):
- gutters_consistent: gutter values from the spacing scale. css_static.
- breakpoints_tokenized: media query widths from breakpoint tokens. css_static.

motion (advisory):
- duration_within_bounds: UI transition durations 100-500ms unless explicitly exempted. css_static.
- no_unpausable_infinite_animation: css_static.

data_viz (advisory):
- categorical_palette_colorblind_distinguishable: pairwise distance under deuteranopia and protanopia simulation above threshold, computed on declared palettes. css_static.
- axes_and_series_labeled: axe_render or fixture inspection.

## Checker engine runners

Extend `apps/notebook/encode/checker_engine.py` with the same CheckerRunResult contract (passed/failed/pending/unsupported, content-addressed run_ids):

- css_static: parse declarations via the CSS lowering, resolve token references, compute contrast ratios with the WCAG relative-luminance math, spacing multiples, scale membership, measure, line-height, durations, reduced-motion presence. Fully runnable now; the analog of the graph_query runner.
- token_lint: load the system token set, walk component sources for raw values not traceable to a token. Fully runnable now.
- axe_render: render the component fixture headless and run axe-core, mapping violations to rule ids. Declared now, wired when the render substrate lands; this is the same render seam the browser engine provides and the same posture cargo_test/proptest had in the rust pack (declared checkers, execution pending). Interim option if wanted earlier: axe on jsdom covers the non-layout rule subset; record which subset ran.
- apg_behavioral: drive the rendered fixture's keyboard contract through the engine's act/observe surface (Type, Tab, Escape, focus assertions). Pending on the same substrate; reuses the drivable BrowserEngine from docs/plans/servo-browser-use-agent/HANDOFF.md.

## Held-out gate

20 tasks in `apps/notebook/encode/validation_tasks/`, mirroring the rust task count. Task shape: build or repair a component against a stated system (tokens, scale, APG pattern), baseline without the pack vs treatment with it, scored on axe violation count, token adherence rate, contrast pass rate, target-size pass rate, and scale conformance. Same floors, treatment-beats-baseline requirement, and validator-pass-rate policy through `benchmarks.py` and `promotion_router.py`; the provenance promotion block records the result honestly. Dependency to state in provenance: tasks scored on axe violations need axe_render wired; until then the gate runs on the css_static and token_lint axes and says so.

## SKILL.md (authored, same register as rust-engineering)

Domain Map:

| Domain | Look for | Good default |
|---|---|---|
| Tokens and scale | token files, custom properties, raw literals | Every value traceable to a token; spacing on the 4/8 grid. |
| Typography | font-size sets, line-height, measure | Sizes from the modular scale; body 16px+, line-height 1.4+, measure 45-75ch. |
| Color and contrast | palettes, text/background pairs | 4.5:1 body, 3:1 large and UI; check the math, not the eye. |
| Layout and grid | columns, gutters, breakpoints | Gutters and breakpoints from tokens; one grid per surface. |
| Components | dialogs, comboboxes, tabs, menus | Match the APG keyboard contract; the contract is part of the component. |
| Accessibility | focus styles, labels, headings, ARIA | axe-clean; never remove focus without replacing it. |
| Motion | transitions, keyframes | 100-500ms; always pair with prefers-reduced-motion. |
| Data viz | palettes, axes, legends | Colorblind-distinguishable palettes; label directly when series are few. |

Core Posture: tokens before pixels; the system over invention; check the math instead of claiming by eye; reduced-motion is not optional; a component's keyboard contract is part of the component; repair to the system rather than restyling around it.

Anti-Patterns: raw hex outside token files; off-grid spacing; div-as-button; removing focus outlines; animation without a reduced-motion path; contrast judged visually; restyling a primitive instead of using its accessible variant.

Validation Defaults: the css_static and token_lint runner invocations, axe_render once wired. Output Shape mirrors rust: which domain pattern applied, which checkers ran and passed, what remains unvalidated or deferred.

## Acceptance

- The corpus packets lower: design-system sources, the motion/viz sub-corpus, and the first-party tokens produce atoms under `css_declaration_view` and `design_token_view`, with lowered_view_count recorded in provenance.
- `_DESIGN_RULES` ships with the rules above; each appears in the pack as a rule artifact with its checker declared.
- css_static and token_lint run for real against a fixture set and emit CheckerFindings with pass/fail; axe_render and apg_behavioral are declared with pending status until the render substrate lands.
- A component fixture lift exists: at minimum the APG-pattern fixtures ship as `design_component_fixture` artifacts with content hashes chained.
- The pack compiles to `skill-pack:design-engineering-general-v0.1` with honest provenance (scanned, promotion block populated) and exports to the marketplace layout.
- The 20-task held-out set exists; the gate runs on the static axes and records which axes were scored.

## Implementation Notes

- Considered: adding a standalone Python helper package vs a Theorem-native crate.
- Chose: `rustyredcore_THG/crates/design-check`, mirroring `prose-check`, because this checkout already carries the Rust `SkillPack` publishing/apply path while `apps/notebook/encode/` is not present here.
- The first landed slice implements real `css_static` and `token_lint` fixture checks, CSS and token lowering, WCAG contrast math, APG fixture artifacts, corpus packet metadata, the 20-task held-out manifest, honest pending `axe_render`/`apg_behavioral` declarations, and a pack payload for `skill-pack:design-engineering-general-v0.1`.
