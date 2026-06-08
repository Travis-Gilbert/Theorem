# Theorem Desktop: design and agent-surface synthesis (job-010)

**Repo:** Travis-Gilbert/theorem
**Audience:** Claude Code + Codex, building as one agent
**Plan home:** docs/plans/theorem-desktop/
**Builds on:** the desktop baseline (commit 1c65be2, apps/desktop landed end to end as the CC/Codex seam), phase-5-servo-agent-surfaces.md (job-005), and the Servo browser-use lane (docs/plans/servo-browser-use-agent/, jobs 007 through 009).
**Job linkage:** job-010, kind Feature, priority P1, target_head Either.

## Decision basis

Two lanes now build against the same engine and the same agent stack, and this doc is the seam statement that keeps them from forking.

Browser-use is Servo-native. Jobs 007 through 009 build the perceive/govern/afford stack on the embedded Servo engine in rustyred-web, with the theorem-browser-agent crate carrying the contracts. The phase-5 fence that treated web actuation as an external affordance to register (Browser Use, Chromium-backed) predates that decision and is corrected by D1 below. There is no Chromium anywhere in this architecture and no external actuation dependency.

The phase-5 agent tab is the desktop face of that stack, not a parallel implementation. An ingestion tab is web_consume with chrome. The co-browse surface (browse_with_me, the shared live session with pre-action preview and veto) rides the same agent-tab interface when the browser-use lane lands. One engine seam, one theorem-browser-agent crate, one Servo crate pin shared between the desktop lane and the browser-use lane. Phase-5 D1 already requires verifying against the browser-use lane's pin before adding a second; that requirement stands and this doc makes it bidirectional.

The design half completes what the baseline started. tokens.css landed clean (4/8 spacing grid, neutral grounds, accents constant across light and dark) but is missing motion, focus, and semantic tokens, and the interactive primitives have no stated keyboard contracts. The Dia discipline applies: calm chrome, novelty budget spent only on the epistemic moments. D2 through D4 are that spend, specified in tokens and APG contracts rather than taste.

Contrast constraints below are computed, not eyeballed: text-dim on bg is about 5.9:1 (passes body text), text-faint is about 3.3:1 (fails 4.5:1 body, passes 3:1 large text and UI), pcb-green on white is about 4.2:1 (fails body, passes large text and UI), brass on white is about 2.4:1 (fills, borders, and large decorative only).

## Deliverables

### D1: phase-5 fence correction

Replace the stale actuation fence in phase-5-servo-agent-surfaces.md. The line

> No standalone web-automation product surface; this is ingestion, not actuation. Web actuation at scale is an affordance to register (Browser Use), not a thing to rebuild.

becomes

> Actuation routes to the native Servo browser-use lane (docs/plans/servo-browser-use-agent/, jobs 007 through 009), which shares the engine and the theorem-browser-agent crate with this phase. This phase ships ingestion tabs; the co-browse and autopilot surfaces ride the same agent-tab interface when that lane lands. No external Browser Use dependency and no Chromium anywhere.

The rest of phase-5 stands unchanged, including the single-pin requirement in its D1. (This edit is applied alongside this doc; the deliverable here is that the corrected text is what agents read and that no other plan in this directory still references external Browser Use or Chromium for actuation.)

### D2: token completion in apps/desktop/src/styles/tokens.css

Add the following, aliased to existing values; no new raw hex anywhere outside tokens.css.

```css
/* Motion. Chrome transitions only; content is never animated. */
--motion-fast: 120ms;
--motion: 200ms;
--motion-slow: 320ms;
--ease: cubic-bezier(0.2, 0, 0, 1);

/* Focus. Visible on every focusable; never removed without replacement. */
--focus-ring: 2px solid var(--pcb-green);
--focus-ring-offset: 2px;

/* Semantic accents. Memory and grounding are green; agent activity and
   ingestion are brass. These are the only two stories the accents tell. */
--accent-memory: var(--pcb-green);
--accent-memory-soft: var(--pcb-green-soft);
--accent-agent: var(--brass);
--accent-agent-soft: var(--brass-soft);
```

And the reduced-motion pairing:

```css
@media (prefers-reduced-motion: reduce) {
  :root {
    --motion-fast: 0ms;
    --motion: 0ms;
    --motion-slow: 0ms;
  }
}
```

Usage rules, derived from the computed contrast above and enforced in review:
- pcb-green and brass are never body-text colors on light surfaces. Large text (19px+ bold or 24px+) and UI accents only.
- text-faint is never a body-text color. Labels, metadata, and large text only.
- The unverified tier (open_web_unverified content in the known-context strip and ingestion receipts) renders with a dashed 1px var(--border-strong) border, var(--accent-agent-soft) fill, and a text-dim "unverified" label. Grounded recall hits render with a solid 2px var(--accent-memory) left border on var(--surface). The tier difference is structural (border style and color), not opacity, so it survives dark mode and color-vision differences.

### D3: APG keyboard contracts for the four interactive primitives

Each contract is part of the component, not a QA pass. Visible focus via --focus-ring on every focusable element; no outline: none anywhere without an equally visible replacement.

- **Omnibox** (combobox pattern): Cmd+L focuses and selects-all. ArrowDown moves into the suggestion list, ArrowUp from the first suggestion returns to the input. Enter commits the highlighted suggestion or the raw input. Esc closes suggestions first, then returns focus to the page. aria-expanded and aria-activedescendant reflect state.
- **MentionPopover** (listbox pattern): @ in the chat rail opens it anchored to the caret. Arrow keys navigate options, Enter selects, Esc closes and returns focus to the input. aria-activedescendant tracks the highlighted option; the input keeps DOM focus throughout.
- **Sidebar tabs** (roving tabindex): one tab stop for the list, arrows move the active item, Cmd+1 through Cmd+9 jump to position. Drag reorder has a keyboard alternative: Alt+ArrowUp/Alt+ArrowDown moves the focused tab. Enter or Space activates. Delete or Cmd+W closes the focused tab.
- **Settings** (dialog pattern): focus moves to the first field on open, Tab is trapped within, Esc closes, and focus returns to the invoking control on close. aria-modal and a labelled heading.

### D4: the epistemic moments (the novelty budget spend)

Four components, each calm by default and legible on demand. All transitions use the motion tokens, which means they vanish under reduced motion.

1. **Known-context strip.** Text-only recall hits above the chat rail input. Grounded hits per the D2 tier treatment (memory-green left border); unverified hits per the unverified treatment. Dismissible per turn, max-height capped with internal scroll. This is the dossier pattern from the wider plan applied to chrome: the cheap summary is the default, expensive actions are opt-in per item.
2. **Agent tab ingestion indicator.** A small brass badge on the tab item carrying the receipt count for the session. New receipts fade the count in over var(--motion); no pulse, no spinner, nothing animates at rest. The badge is the only place an agent tab differs visually from a general tab, per phase-5 D1.
3. **Pre-action preview.** The browse_with_me veto affordance, built now as a component behind the agent feature flag so the surface exists when job-007 lands the live session. Renders from an ActionCandidate fixture: action label, target element name, and a risk chip mapped from ActionRisk (read_only on surface-2 neutral, external_web on accent-agent-soft, hot_graph_write and canonical_write and remember with an accent-memory border, state_changing on danger). Two buttons: Approve and Take the wheel. Enter approves, Esc vetoes, focus lands on Approve when the preview appears. The chip colors follow the D2 usage rules (fills and borders, never small text in accent colors).
4. **Mention chips.** @-mentioned tabs and memories render as chips in the composer with a 4px stack offset when more than three accumulate, collapsing to "+N" past five. Chip removal is keyboard reachable (focus chip, Delete).

### D5: wiring evidence for the job-001 verifier

Not new scope; job-001 owns phase-one completion and its nine acceptance criteria remain the gate. This deliverable is the observed gap list so the verifier checks the real seams rather than rediscovering them:

- model_chat returns a dev placeholder outside Tauri and names the Rust model-client as the live seam; provider calls and the DeepSeek keyless default are not yet proven live.
- harness_remember, harness_recall, harness_settings, and harness_bearer_set exist on both sides of the seam; live calls against the hosted MCP with bearer plus tenant are not yet proven.
- Direct Servo is not linked into the desktop binary (per servo-fetch-decision.md); the agent tab runs on the wry fallback behind the same interface, which is the documented decision, not a gap.
- The receiver defaults off; receiver_settings and receiver_status are seam-complete.

## Acceptance criteria

1. phase-5-servo-agent-surfaces.md no longer contains the "affordance to register (Browser Use)" fence; the corrected fence names the native lane (jobs 007 through 009), the shared theorem-browser-agent crate, and the single Servo pin.
2. tokens.css contains the motion, ease, focus, and semantic accent tokens and the prefers-reduced-motion block; a token lint pass finds no raw hex outside tokens.css.
3. Each of the four primitives passes its APG keyboard walkthrough as specified in D3, verified by hand against the running app.
4. Every focusable element shows the focus ring; a grep across apps/desktop/src finds no outline: none (or outline: 0) without an adjacent visible replacement.
5. No body text renders in pcb-green, brass, or text-faint on light surfaces; accents appear only as fills, borders, chips, and large text.
6. The pre-action preview renders from an ActionCandidate fixture behind the agent feature flag, with the risk chip mapping and Enter/Esc handling working.
7. The known-context strip visually distinguishes unverified-tier hits from grounded hits per the D2 treatment, in both light and dark.
8. With the OS reduced-motion setting on, chrome transitions resolve to 0ms and the ingestion badge appears without a fade.

## Fences

- The standing no-graph-view fence holds.
- No new raw hex outside tokens.css.
- This is chrome work; nothing here touches the engine, the reader, or the executor (those are jobs 007 through 009).
- One Servo pin across the desktop lane and the browser-use lane; this doc does not add a second and neither does any work it spawns.
- This job does not re-declare job-001 done; phase-one acceptance criteria 1 through 9 remain that job's gate, with D5 as evidence input.
