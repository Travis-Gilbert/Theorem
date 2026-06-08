# Build step two (job-008) status: engine-only abilities

Author: claude-code. Sibling to `build-step-2-engine-abilities.md` (the spec).
Records progress after Travis removed the strict 007->008->009 sequence (job-008
started in parallel with Codex's job-007 executor).

## The split: reader-side (buildable now) vs engine-gated (CI/fork/Codex)

job-008's abilities divide cleanly:
- **Reader-side, locally verifiable (mine, no live engine):** record/replay (D5),
  layout-aware reading order (D4), the precise post-action diff (D2). Built and
  unit-tested against synthetic accesskit DTOs.
- **Engine-gated or Codex-coupled (CI/fork or joint design):** occlusion/hit-test
  truth (D1), the live settle signal (D2), render-tier control (D3), and the
  graft-aware unified iframe/shadow tree (D4) which changes `element_id` to the
  accesskit `(tree_id, node_id)` pair the executor resolves against.

## Acceptance map

| # | Criterion | Verdict | Note |
|---|-----------|---------|------|
| 1 | Occluded element reports occluded:true, not clicked blind | GAP | Needs a Servo hit-test query + additive `occluded`/`truly_clickable` fields on `InteractiveElement` (Codex's struct). Joint design flagged. |
| 2 | WaitFor resolves on an engine settle signal, no sleeps | PARTIAL | The `Settle` step is modelled in the run ledger (`browser_run`); live resolution needs Servo's layout/paint quiescence (likely via `notify_animating_changed` false), an engine/CI piece. |
| 3 | Post-action PageState carries a diff of exactly what changed | COVERED | `A11yDiff` (job-007) is the precise node delta; `BrowsingRunRecord::post_action_diffs` attributes each action to the diff of the observation that followed it. The live act->observe->diff pairing is the executor's (Codex). Full page is never re-sent (incremental updates). |
| 4 | Resource-blocked session ingests a heavy page faster, measured | GAP | Needs Servo net-layer resource policy (block images/media) via `Preferences`/net config; engine/CI. |
| 5 | Reading order matches visual order, not DOM order | COVERED | `AccessibilityReader` reprojects document (DFS) order into visual column-major order from the box-tree bounds (`visual_order` + column detection); tested on a 2-column interleaved tree and single-column. |
| 6 | An iframe's interactive elements appear in one PageState tree, addressable by stable id | GAP | The graft-aware multi-tree assembly. Servo sends a grafted multi-tree update (WebView ScrollView tree + document subtree, independent NodeId spaces); correct assembly makes `element_id` the `(tree_id, node_id)` pair, which the executor resolves against -> joint design with Codex before the reader refactor. |
| 7 | A BrowsingRun replays from the recorded TreeUpdate/Action/settle sequence | COVERED | `browser_run`: content-addressed `BrowsingRunRecord` (BLAKE3 over the step ledger), `replay()` reproduces the PageState sequence + diffs through a fresh `AccessibilityReader`, `fork()` shares a prefix. The verification artifact a video cannot be. |

## What landed (this session)

New module `rustyred-web/src/browser_run.rs` (D5 + D2 post-action diff):
- `BrowsingRunStep` (Observe/Action/Settle), `BrowsingRunRecorder`,
  content-addressed `BrowsingRunRecord` (`run_id` = BLAKE3 of the ledger),
  `replay() -> BrowsingRunReplay`, `fork()`, `post_action_diffs()`.
- Engine-agnostic: records the `A11yTreeUpdate` DTO and an action `label`, not
  Servo types or the `BrowserAction` enum, so it does not couple to the executor.
- 7 tests: content-address determinism, replay reproduction, replay-vs-live
  fidelity, fork, post-action diff attribution, empty run, serde round-trip.

`rustyred-web/src/browser_perception.rs` (D4 reading order):
- `reading_order` now reprojects document order into visual column-major order
  via `visual_order` + `column_boundaries`/`column_index`/`median`. 2 tests.

## Deferred, with reasons (named, not cut)

- **D4 graft-aware unified tree (c6):** changes the `element_id` contract to
  `(tree_id, node_id)`; the executor resolves elements by `element_id`, so this is
  a joint design with Codex, not a unilateral reader refactor.
- **D1 occlusion (c1):** `occluded`/`truly_clickable` are additive fields on
  Codex's `InteractiveElement`, populated from a Servo hit-test (engine/CI).
- **D2 live settle (c2):** the run ledger models settle; live resolution is a
  Servo quiescence signal (engine/CI).
- **D3 render-tier (c4):** Servo net-layer resource policy (engine/CI).

## Validation receipts

- `cargo test -p rustyred-web`: 99 lib + 2 + 12 green, 0 warnings.
- `cargo test -p rustyred-web --features accesskit`: green, 0 warnings.
- All job-008 work so far is reader-side and unit-tested without libservo; the
  engine-gated criteria (1, 2, 4) and the graft (6) are CI/fork/joint-design.
