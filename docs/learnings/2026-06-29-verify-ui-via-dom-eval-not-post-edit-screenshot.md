# Verify UI state with a DOM eval, not a screenshot taken right after an edit

**Kind:** gotcha
**Captured:** 2026-06-29
**Session signature:** `claude:1travisgilbert@worktree:naughty-clarke-780eb7`
**Domain tags:** preview-tooling, nextjs, fast-refresh, verification

## Trigger

I set the FileTree's default `useState` to `{ memory: true, "agent-memory": true }`
(folders open). The preview screenshot kept showing them COLLAPSED, which looked
like a real bug. It wasn't. Two compounding dev-loop traps:

1. **React Fast Refresh preserves component state across edits.** A `useState`
   initializer only runs on first mount; when I changed the default and saved, the
   already-mounted FileTree kept its previous state. So the rendered tree did not
   reflect the new initial state at all.
2. **The `preview_screenshot` tool reloads the configured base route** (`/sandbox`
   here) before capturing, and that reload hit the same Fast-Refresh-preserved
   stale state — and separately, when I had navigated via `eval` to a non-base path
   (`/welcome`), the screenshot snapped back to base so it could not hold the page
   I wanted. Interactive states (an open ⌘K dialog) are similarly fragile.

A `document.querySelectorAll('[role="treeitem"]')` eval on a hard-reloaded
(`?fresh=<ts>`) page proved the truth: 13 items, both folders `aria-expanded=true`.
The code was correct the whole time.

## Rule

After editing a component's initial state or structure in this preview harness,
verify with a `preview_eval` DOM query (it reads the live DOM), or a hard reload
with a cache-busting query (`/path?fresh=<Date.now()>`) — NOT a screenshot taken
right after the edit. Fast Refresh will lie via preserved `useState`; the
screenshot tool resets to the base route and cannot reliably hold an
eval-navigated path or an open-dialog state. Use screenshots for final visual
confirmation on a known-fresh load; use eval for structural/state truth. In
production there is no Fast Refresh, so the stale view is a dev-only artifact —
don't "fix" code that eval shows is already correct.

## Evidence

- Screenshot: folders collapsed. `eval` on `/sandbox?fresh=<ts>`:
  `{ count: 13, Memory aria-expanded:"true", "Agent memory" aria-expanded:"true" }`.
- ⌘K search verified via eval (`[cmdk-input]` present, 5 groups, 13 items, typing
  "post" filtered to just Postmortems) because a screenshot would reload and close
  the dialog.
- Earlier, the only reliable way to keep the screenshot on the landing was to
  promote it to `/` (remove the `app/page.tsx` `redirect("/canvas")`).

## Encoded in

- `docs/learnings/2026-06-29-verify-ui-via-dom-eval-not-post-edit-screenshot.md` (this file)
