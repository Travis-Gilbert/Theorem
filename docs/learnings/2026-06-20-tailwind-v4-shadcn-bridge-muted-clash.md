# To make third-party shadcn/21st.dev components inherit a custom palette in Tailwind v4, bridge with `@theme inline` (not `@theme`), resolve the `muted` text-vs-surface name clash, and never verify a utility via a runtime-injected class

**Kind:** gotcha
**Captured:** 2026-06-20
**Session signature:** `claude-code:travisgilbert (harness-console: Tailwind v4 upgrade + shadcn token bridge)`
**Domain tags:** tailwind-v4, shadcn, 21st.dev, design-tokens, harness-console, css-jit

## Trigger

`apps/harness-console` uses a custom palette (`--bg`, `--ink`, `--ox` oxblood, `--surface`, `--muted` = secondary TEXT grey). To let `npx shadcn add <21st-url>` components render oxblood/grey with no per-component restyling, I added a token bridge mapping shadcn's standard semantic names onto the palette. Two real scars:

1. **The `muted` collision.** My first instinct was `--color-muted: var(--muted)` because our `--muted` is the grey text. But shadcn's `muted` is a *surface* (`bg-muted` = light grey background) and the text is `muted-foreground`. Had I shipped that, every imported component's `bg-muted` would have painted the mid-grey TEXT color as a background (too dark), while `text-muted-foreground` would have been undefined.
2. **The verification trap.** I verified the bridge by injecting `document.createElement('div'); el.className='bg-primary'` at runtime and reading `getComputedStyle(el).backgroundColor` → got `rgba(0,0,0,0)` (transparent) and nearly concluded the bridge was broken. Root cause: Tailwind v4 JIT only generates a utility if the class appears in **scanned source**; a class injected by JS at runtime was never scanned, so `.bg-primary` did not exist. The actual proof the bridge works is `text-muted-foreground` (used 137x in real source) resolving to `rgb(110,110,116)` = our `--muted`.

## Rule

- Define raw palette values in `:root` / `[data-theme="dark"]` as plain custom properties; bridge shadcn names in `@theme inline { --color-primary: var(--ox); --color-border: var(--line); ... }`. The `inline` keyword is mandatory: it makes the generated utility reference `var(--ox)` at use-site, so a `[data-theme=dark]` override of `--ox` flows through. Plain `@theme` snapshots the value at build time and dark mode breaks.
- Resolve the `muted` clash by adopting shadcn's split: `--color-muted: var(--surface-2)` (surface), `--color-muted-foreground: var(--your-text-grey)` (text), and rename every one of your own `text-muted` usages to `text-muted-foreground`. shadcn's own rules want `text-muted-foreground` anyway.
- To verify a v4 utility renders, probe a class that ALREADY appears in source (or add a real usage first). A runtime-injected class proves nothing — it is tree-shaken out of the generated CSS.
- The bridge is a *utility-class* bridge (it does not emit raw `--primary`/`--background` CSS vars). That covers shadcn/21st components, which use utility classes. If a component references a raw `var(--primary)`, emit the raw vars too — but then the `muted` clash returns at the raw-var level, so don't reuse your `--muted` name for it.

## Evidence

- `bg-primary` runtime probe → `rgba(0,0,0,0)` (not generated); `text-muted-foreground` probe → `rgb(110,110,116)` (= `--muted`, bridge resolves); `border-border` → `rgba(17,17,19,.16)` (= `--line`).
- `grep -Po 'text-muted(?!-foreground)' src` → 0 after the rename; `text-muted-foreground` → 137.
- v3 → v4 migration (`@import "tailwindcss"` + `@theme inline`, deleted `tailwind.config.ts`, `@tailwindcss/postcss`): `next build` green (17 routes), and the Agent surface screenshot was pixel-identical to v3 (no regression).
