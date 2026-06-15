# Theorem Desktop: design-language reference

**Plan home:** docs/plans/theorem-desktop/
**Companion to:** design-and-agent-surface-synthesis.md (job-010), the projection-shell plan, and apps/desktop/src/styles/tokens.css
**Purpose:** The chosen direction (below) plus the three reference extractions it draws from, read against the projection shell's own system. The standing fences (no-graph-view, no raw hex outside tokens.css, calm chrome) hold over everything here.

## Direction (chosen 2026-06-10): patent paper, oxblood pencil

This supersedes the old token system. The prior chrome (PCB green `#2a8b6c` on a cool grey-neutral ground, dark mode a bland cool grey) is retired. The new ground is patent paper with an oxblood pencil accent, hairline rules, a faint engineering grid, and warm depth. The source exemplar is the Theseus Atlas iPhone build, which already encodes the language (paper `#f5ecd6`, pencil `#a8301e`, rule `#bcae90`, Vollkorn serif display, a 26px grid). It is not a pixel target; it is the proof the direction holds.

What landed in code this turn, all contrast-checked (math below):

- **Field:** warm paper `--bg #f1e7cf`, cards lift lighter (`--surface #f7efda`). Ink-on-paper is 15.5:1, the readability lift.
- **Accent (the pencil):** oxblood/terracotta `--accent #a8301e`. One accent carries primary action, focus, and the grounding/memory mark. 5.5:1 on paper, legible even as body text (the old green failed body text).
- **Agent/ingestion:** ochre `--accent-agent #b09468`. Fills, borders, chips, and large text only (2.3:1), the same role brass held.
- **Danger:** crimson `--danger #9e2b3f`, a separate hue from the oxblood so destructive never reads as the brand mark.
- **Hairlines:** warm tan `--border #cabb98`, never grey. `--hairline` token added.
- **Texture:** a faint engineering grid (two 1px `--grid-rule` gradients at `--grid-size 24px`, adjusted from the exemplar's 26 to sit on the 4/8 grid), applied to the field in global.css.
- **Depth:** warm ink shadows with a paper-edge inset highlight (`--shadow-pop`), the lifted-card look from the exemplar.
- **Motion:** the exemplar's deliberate ease-out `--ease cubic-bezier(.22,.9,.27,1)` with a `--stagger` for list enters (the anime.js motion read, see below), chrome-only, zeroed under reduced motion.
- **Type:** body moves to full-width IBM Plex Sans (the Condensed face is now opt-in `--font-ui-condensed`); display is a Vollkorn serif. Both fall back to system/Georgia until bundled.
- **Dark mode:** kept, but warm toasted-paper, not cool grey. Accent brightens to `#d2553f` for the dark ground.

Migration: tokens.css rewritten; 26 `--pcb-green` and 7 `--brass` references in global.css and app.css swapped to `--accent`/`--accent-agent`; focus tokenized; grid texture wired. Zero `pcb-green`/`brass` references remain and no raw hex sits outside tokens.css.

Corrected reading of the references (the earlier draft over-weighted their color):

- **Gemini is a structure reference, not a color one.** Take the omnibox as the primary command surface, the side-rail composition, and the way it builds soft tonal containers. Drop its blue entirely. The Material rule that an accent is a fill carrying a deep label, never thin colored text, is the one color lesson, and it matches the oxblood usage rules.
- **anime.js is a motion reference, not a color one.** Take its animation breakdown: deliberate eased entrances, staggered list reveals, motion spent on moments and still at rest. Drop its dark palette and red/green/orange. The `--ease` and `--stagger` tokens are this read made concrete.
- **JSON Canvas stays the restraint baseline** and the domain neighbor (the infinite-canvas node/edge format).

Open decisions for Travis:

- **Oxblood vs danger proximity.** Both are reds, separated by hue (oxblood orange-brick `#a8301e`, danger crimson `#9e2b3f`) and reserved-by-context. If you would rather danger not be red at all now that red is the brand, say so and I will move it.
- **Fonts not yet bundled.** There are no `@fontsource` deps; Vollkorn and IBM Plex Sans currently fall back to Georgia and the system sans. Add `@fontsource/vollkorn` + `@fontsource/ibm-plex-sans` to make them real.
- **Base font size stays 14px** for chrome density; the readability gain came from contrast and dropping the Condensed face. Bump to 16px if you want it larger, at some layout cost.
- **Two-accent semantics preserved** (memory = pencil, agent = ochre). The reserve categorical hues (sage, indigo, teal, mauve, lilac, ochre) are defined but idle, for future tags and node categories.

## Provenance and honesty

These tokens were extracted on 2026-06-09 with Firecrawl's `branding` format (hosted JS render, computed-style sampling), not the `designlang` CLI. The CLI path needs a headless Chromium that would not install in this sandbox (the Playwright browser download fails on system-deps), so the deliverable is the design.md you asked for, sourced from a renderer that handles SPAs. Each section carries the extractor's confidence and any value I could not trust.

URL note on the third site: the link you gave was `github.com/obsidianmd/jsoncanvas.git`. Pointing an extractor at that renders GitHub's own chrome, not JSON Canvas. The format's real site is **jsoncanvas.org**, so that is what I extracted. It is also the most domain-relevant of the three, since JSON Canvas is the open file format for infinite-canvas node/edge data, which is the projection shell's own subject.

To regenerate later (once a browser is available):

```bash
npx designlang https://animejs.com/ --dark --screenshots
npx designlang https://jsoncanvas.org/ --screenshots
npx designlang https://gemini.google.com/ --screenshots
# or, renderer-as-a-service, no local browser:
# firecrawl_scrape <url> formats=["branding"]
```

---

## 1. anime.js (animejs.com)

Dark, high-energy, opinionated. A custom system with a display face and a bold accent trio on near-black. Extractor confidence 0.9; raw CSS confirms the dark ground (`--background-color: #1f1f1f`, body text `#ebebeb`).

| Axis | Value |
|------|-------|
| Scheme | dark |
| Background | `#252423` (brand sample) / `#1f1f1f` (raw widget chrome); both near-black |
| Accent (primary) | `#FF4B4B` red |
| Accent (secondary) | `#00FFAA` spring green |
| Accent (link) | `#FFA828` amber |
| Neutral fill | `#D5D3D1` light stone (primary button), `#353433` input ground |
| Body text | light: `#F6F4F2` / `#ebebeb` (see artifact note) |
| Heading face | DIN (geometric, condensed display) |
| Body face | Helvetica Neue, stack `DIN, Helvetica Neue, Helvetica, Arial, sans-serif` |
| Type scale | hero h2 `64px`, body `20px`; the reported h1 `16px` reads as an eyebrow/label, not a hero |
| Base unit | 4px |
| Radius | small (1 to 4px) |
| Personality | modern, medium energy, developer audience |

Component signature worth noting: the subscribe field pairs an **input with only its left corners rounded** (`4px` top-left/bottom-left, `0` right) against a **primary button with only its right corners rounded** (`0` left, `4px` top-right/bottom-right), so the two fuse into one pill. Secondary button is transparent with a `#625D5B` 1px border and `#93908E` text at full `4px` radius.

Artifact flagged: the extractor reported `textPrimary #252423`, identical to the background. That is wrong (it would be invisible body text); the real body text is the light value confirmed in CSS. Do not copy `textPrimary` from the raw output.

**Read against the shell:** anime.js is the closest reference to the projection shell's dark surface and its "novelty budget spent only on the epistemic moments" discipline. It is an animation library that keeps its chrome still and spends energy deliberately, which is the same posture as D4's epistemic moments. The split-corner pill is a concrete motif for the Omnibox/composer if you ever want the input and its commit affordance to read as one unit. The three-accent system is hotter than the shell's two-story rule (green for memory, brass for agent); treat it as a reference for accent confidence, not a palette to import.

---

## 2. JSON Canvas (jsoncanvas.org)

Minimal, near-chromeless, system-font. A single plum accent ramp on off-white. The most restrained of the three, and the most domain-adjacent. Extractor color confidence 0.9; overall 0.45 only because the page has no buttons to classify.

| Axis | Value |
|------|-------|
| Scheme | light |
| Background | `#FAFAFA` off-white |
| Accent (deep) | `#3F062D` deep plum (text, links, headings) |
| Accent (mid) | `#68154C` magenta-plum |
| Accent (soft) | `#A28397` muted mauve |
| Body/heading text | `#3F062D` |
| Type | system stack: `-apple-system, BlinkMacSystemFont, Segoe UI, Helvetica, Arial, sans-serif` (extractor named Inter/IBM Plex Sans as nearest matches; the real stack leads with system fonts) |
| Type scale | h1/h2 `32px`, body `16px` |
| Base unit | 4px |
| Radius | 4px |
| Personality | modern, medium energy, developer audience |

Lineage note: the site is built on the JSON Feed template (the extracted `logoAlt` still reads "JSON Feed"). That is why it is so close to a plain documentation page: one accent, system type, no component layer.

**Read against the shell:** this is the calm-chrome baseline made literal. A spec/docs surface with one accent, system fonts, and a 4px grid is exactly the "calm by default, legible on demand" target. Two direct uses: (a) it is a model for how little chrome the projection shell needs around text-only surfaces like the known-context strip; (b) JSON Canvas itself (nodes + edges, the infinite-canvas file format) is the conceptual neighbor of the projection shell's node surfaces, so its visual restraint is a useful anchor for what a node/edge surface looks like before the no-graph-view fence decides how much of it ever renders. The plum mono-accent is the inverse of the shell's two-accent semantics; it shows how far a single-hue ramp can carry a whole site.

---

## 3. Google Gemini (gemini.google.com)

Material 3. Calm, soft-tonal, pill-shaped. Captured at the signed-out entry surface (the extractor was redirected to `/app` and saw the Sign in CTA), so this is Gemini's Material marketing/entry face, not the in-app chat UI, which is auth-walled. Extractor confidence 0.925.

| Axis | Value |
|------|-------|
| Scheme | light (Material 3) |
| Background | `#FDFCFC` near-white |
| Accent (tonal fill) | `#C2E7FF` light blue |
| Accent (tonal mid) | `#9DD2FF` |
| Accent (deep) | `#004A77` deep blue (text-on-fill, links) |
| Text | `#000000` |
| Type | Google Sans Flex (primary), Google Sans (display), Google Sans Text, Roboto; usable stack `Google Sans Flex, Google Sans, Helvetica Neue, sans-serif` |
| Type scale | h1/h2 `32px`, body `17px` |
| Base unit | 4px |
| Radius | `10px` containers; buttons fully rounded (`9999px` pill) |
| Personality | modern, medium energy, tech-savvy audience |

Component signature: the primary button is a Material 3 tonal pill, light-blue fill (`#C2E7FF`) with deep-blue label (`#004A77`), full `9999px` radius, no shadow.

Artifact flagged: the extractor's heading/body `fontStacks` came back as `["Times New Roman"]`, which is a fallback-resolution bug, not Gemini's real type. The Google Sans family in the `paragraph` stack and the `fonts` list is the real story; ignore the Times New Roman entry.

**Read against the shell:** Gemini is the reference for the shell's own usage rule that accents are fills and borders, never small body text. Material's tonal button does exactly that: a soft accent fill carries a deep accent label, and the accent never appears as thin text on white. That is the same constraint the synthesis computed from contrast (pcb-green and brass as fills/borders/large-text only). The 10px container radius and the full pill are softer than anime.js or JSON Canvas; if the shell wants a single radius story, these three sites bracket it (1 to 4px hard, 4px neutral, 10px-to-pill soft). Gemini's restraint with one cool accent on near-white is the closest mainstream analog to the calm-chrome target, minus the shell's epistemic-moment spend.

---

## Synthesis: what the three say to the projection shell

Note: this section was written against the prior palette (green/brass). The accent hues are now oxblood/ochre per the Direction section above; the structural points (4/8 grid, accents as fills not text, calm-by-default, motion on chrome only) still hold and are what carried into the new system.

The shell's system already exists in tokens.css and the job-010 synthesis: neutral grounds, a 4/8 spacing grid, two accent stories (green = memory/grounding, brass = agent/ingestion), motion on chrome only, accents as fills and borders rather than small text. The three references line up cleanly against it.

Shared grid. All three use a **4px base unit**. Nothing here challenges the shell's 4/8 grid.

Radius is the one open axis. The references bracket it: JSON Canvas and anime.js sit at hard 1-to-4px, Gemini runs soft 10px containers and full pills. The shell has not committed a radius story in the tokens shown; these three mark the range to choose within, and the choice is a personality decision (hard reads as engineered/spec, soft reads as consumer/assistant).

Accent discipline converges from two directions. Gemini (tonal fill + deep label) and the shell (fills/borders/large-text only) enforce the same rule that an accent is never thin body text. JSON Canvas proves a single-hue ramp can carry an entire surface, which is reassurance for the shell's two-accent restraint. anime.js is the outlier: a three-accent system at higher energy, useful as evidence of how confident an accent can be, not as a palette to adopt.

Calm-by-default is the through-line. JSON Canvas is the chromeless baseline, Gemini is calm with one cool accent, anime.js is the proof that an energetic brand still keeps its chrome still and spends novelty deliberately. That is precisely D4's "calm by default, legible on demand," seen at three points on the spectrum.

Type. The shell can read these as three legible defaults: system stack (JSON Canvas, zero-cost, spec-like), a managed sans family (Gemini's Google Sans, consumer-warm), or a display face over a system body (anime.js's DIN over Helvetica Neue, for a single expressive moment). For a tool surface under the calm-chrome fence, the system or single-managed-family route fits; a display face would be a novelty-budget spend, reserved for an epistemic moment if at all.

Domain note. JSON Canvas is not just a style reference; it is the file format for infinite-canvas node/edge data. Its existence and its restraint are an input to whatever the no-graph-view fence eventually allows a node surface to look like.
