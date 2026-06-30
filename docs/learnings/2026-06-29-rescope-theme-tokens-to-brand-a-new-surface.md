# Brand a new surface by rescoping the existing @theme token names, not a parallel system

**Kind:** method
**Captured:** 2026-06-29
**Session signature:** `claude:1travisgilbert@worktree:naughty-clarke-780eb7`
**Domain tags:** harness-console, css, tailwind-v4, shadcn, design

## Trigger

Building the parchment marketing landing (`apps/harness-console/src/app/(marketing)/`)
needed a whole brand palette + depth system that differs from the console's
white/oxblood theme. The instinct is to write new components or a new token set.
Instead, `apps/harness-console/src/app/globals.css` already has the load-bearing
machinery: an `@theme inline` bridge mapping shadcn semantic names onto the house
palette (`--color-primary: var(--ox)`, `--color-background: var(--bg)`, ...), a
`.material` Card that reads `--elev-1/2/3`, and a DotGrid. Rebuilding any of that
would have been a parallel system that drifts.

The win: scope a single `.marketing { }` block that re-defines the *same token
names* (`--bg`, `--surface`, `--ink`, `--ox`, `--ox-hover`, `--elev-1/2/3`,
`--font-title`) to parchment + oxblood + warm depth. The existing
`<Button variant="primary">` (`bg-ox`) and `<Card lift>` (`= .material`) then
inherit the brand with ZERO per-component restyling. Later, "deepen the orange to
oxblood everywhere it appears" was a literal two-line edit (`--ox` + `--cta-*`):
the CTA, the omnibar send button, the nav dot, and the feature-card icon tints all
shifted together because none of them hard-coded a color.

## Rule (method)

When adding a differently-branded surface to an app that already has a shadcn
`@theme` token bridge: override the existing token *names* inside a scoped class
(`.marketing`, `[data-surface=x]`), don't invent parallel tokens or restyle
components. Mirror the existing component DOM shape too (e.g. the FileTree's
`<div role="treeitem">` + `<button>` actions), because its structure encodes
constraints hand-rolled markup trips over. The payoff is that global theme changes
become one-line token edits instead of component hunts. Corollary: a component that
hard-codes hex (the first UploadDock used `#ede3d8`) breaks the instant its surface
flips (it went invisible on parchment) — use the semantic token classes so it
inherits whatever surface it lands on.

## Evidence

- `marketing.css` `.marketing{ --ox:#a8301e; --elev-2: <warm two-layer + top
  highlight>; --font-title: var(--font-amarna); ... }` — shadcn `<Button>`/`<Card>`
  rendered parchment+oxblood untouched; `npm run typecheck` = 0 errors.
- "Deepen orange → oxblood" = edit to `--ox`/`--ox-hover`/`--cta-1`/`--cta-2` only;
  every accent followed.
- Amarna (brand-new on Google Fonts, not reliably in `next/font/google`) was
  self-hosted via `next/font/local` from a downloaded `Amarna.woff2` and exposed as
  `--font-amarna` on the marketing wrapper.

## Encoded in

- `docs/learnings/2026-06-29-rescope-theme-tokens-to-brand-a-new-surface.md` (this file)
