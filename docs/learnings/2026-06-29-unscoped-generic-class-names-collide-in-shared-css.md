# Generic class names in an unscoped global stylesheet silently bleed across components

**Kind:** gotcha
**Captured:** 2026-06-29
**Session signature:** `claude:1travisgilbert@worktree:naughty-clarke-780eb7`
**Domain tags:** harness-console, css, layout

## Trigger

Building the RustyRed "stacked panels" section I added bare selectors `.field`
and `.panel` to the marketing CSS. But `.field` was already the hero's email-input
wrapper (`.signup .field`) and `.panel` was already the hero's "organized today"
card. Because both definitions are bare (not scoped), the substrate's
`.field{ height:600px; display:flex; perspective:900px; mask-image:... }` cascaded
straight onto the signup input and the CTA, stretching the input to 600px tall.
The bug presented as "the hero signup looks broken" with no obvious cause — the
offending rule was in a section 5 screens away. Same class of bug bit twice:
nested `<button>` inside `<button>` in the sidebar FileTree row auto-closed the
outer button and broke the `node + .children` sibling selector so folder children
never rendered.

## Rule

In a single large shared stylesheet (`globals.css`) plus a sibling surface sheet
(`marketing.css`), never reuse generic component class names (`.field`, `.panel`,
`.card`, `.row`, `.stack`). Namespace every component's classes
(`.substrate-field`, `.sub-panel`) or it will collide with another component's
identical bare selector and bleed across screens. When a layout breaks far from
where you edited, grep the class name across all CSS before debugging the
component — the culprit is usually a second bare definition. And mirror the shadcn
primitive's DOM shape (rows are `<div role="treeitem">`, actions are `<button>`)
to avoid invalid nested-interactive-element auto-close.

## Evidence

- `marketing.css` substrate block defined bare `.field`/`.panel`; the hero
  `.signup .field` (email input) and hero `.panel` (organized-today card) inherited
  the substrate rules. Fix: renamed to `.substrate-field` / `.sub-panel`; signup
  input reverted to normal.
- FileTree v1 had `<button class="node"> ... <button class="add"> </button></button>`;
  browser reparented the inner button out, orphaning the "+" and breaking
  `.node[data-open] + .children{display:flex}`. Fix: row → `<div role="treeitem">`.

## Encoded in

- `docs/learnings/2026-06-29-unscoped-generic-class-names-collide-in-shared-css.md` (this file)
