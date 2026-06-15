# Servo automation core (Playwright-class) status: Slices 1-2 + live Servo adapter

Authors: claude-code (the driver seam, `browser_driver.rs`) + codex (the contract,
`browser_automation.rs`, the `browser_engine.rs` enrichment, and the
`apps/browser` live adapter). Sibling to
`servo-automation-core-playwright-class.md` (the plan). Records what the two-head
build landed for Slices 1-2 and the first live Servo bridge, honestly mapped to
the plan's acceptance items.

## Grounding finding (the plan's premise was stale)

The plan opens "Builds on: the actuation correction (`9ee18e7e`), which already
settled coordinate synthesis, the JS geometry snapshot, EmbedderControl responses."
That is not true in this repo: `9ee18e7e` does not exist, and the only actuation
commit (`2bb78561`) is **docs-only** (it added
`build-step-1-correction-actuation.md`). No actuation primitive
(`notify_input_event`, `evaluate_javascript`, `device_pixels_per_css_pixel`,
`FilePicker`, `getBoundingClientRect`, snapshot) existed in code; `vendor/` held
only `d3.min.js`. So Slices 1-2 were a **from-scratch actuation build**, not a
layer on existing work. The reader/governance half
(`browser_perception.rs`) and the `BrowserAction` vocabulary did exist.

## Gating decisions (Travis sign-off, this session)

- **id-space: Option A (approved).** The actionable element id is the JS
  geometry-snapshot `data-theorem-id` stamp; the AccessKit `NodeId` stays in the
  structural overlay only. Realized by `GEOMETRY_SNAPSHOT_SCRIPT` +
  `page_state_from_snapshot_json` (the snapshot stamps `t{i}` in document order;
  that stamp becomes `InteractiveElement.element_id`).
- **#4344: PURSUE (approved), landed in the engine phase.** Not deferred. Made
  structural now as `ActuationKind::SemanticActivation { node_id, action }`; V1
  drivers reject it, the forked Servo driver will execute it. The fork itself
  (servoshell action handler + constellation + script + layout `perform_action`)
  is an open apps/browser/Servo lane.
- **Selector engine: borrow the parser, own the actionability (approved).** Only
  a thin selector bridge is vendored (`vendor/playwright_selector_bridge.js`,
  Microsoft Playwright v1.61.0, **Apache-2.0** not MIT as the plan said). The 6
  actionability checks are ours (`browser_automation.rs` +
  `browser_driver.rs`), so the differentiated substrate is Servo-native from line
  one. Full generated injected-bundle vendoring is a named follow-up.

## Lane split

- **claude-code** owns the Servo-free seam: `rustyred-web/src/browser_driver.rs`.
- **codex** owns the Playwright-shaped contract (`browser_automation.rs`), the
  `browser_engine.rs` enrichment, and the two open Servo lanes below.
- Architecture recorded durably in the coordination room (decision
  `record_defe5b2d16bfd235`).

## What landed (Slices 1-2, Servo-free)

codex (`browser_automation.rs`, `browser_engine.rs`, `browser_run.rs`):
- `Locator` (css/role/text/label/test_id/filter/nth) resolving over
  `PageState.interactive_elements`; `ActionabilityRequirement::for_action`
  matching Playwright's per-action check matrix; `force` drops receives-events.
- `Context` (storage partition + permissions + `route`), `UrlPattern`,
  `RouteAction` (Continue/Abort/Fulfill/ContinueWith); web-first `expect()`
  (`to_be_visible`/`to_be_enabled`/`to_have_text`/`to_have_count`);
  `AutomationActionReceipt` carrying the actionability verdict.
- `InteractiveElement` enriched with `test_id`/`enabled`/`editable` (additive,
  serde-default), `BrowserAction::Hover`, `seed_page_state`, richer attribute
  extraction (disabled/readonly/aria-/data-testid/inline display:none).
- `BrowsingRunRecorder::record_actionability` so a trace shows which check gated.

claude-code (`browser_driver.rs`):
- `BrowserDriver` trait: the minimal Servo I/O surface (`snapshot`,
  `device_pixels_per_css_pixel`, `webview_origin`, async `actuate`). The seam the
  apps/browser embedder implements; the contract above it stays Servo-free.
- `GEOMETRY_SNAPSHOT_SCRIPT` (E4) + `page_state_from_snapshot_json` (Servo-free
  parser): the live driver runs the script through `evaluate_javascript` and
  parses the result; Codex's `Locator::resolve` then filters the handles
  unchanged. Realizes Option A.
- E5/E6 coordinate transform (`device_point_at_rect_center`, `css_to_device_point`).
- `run_action`: the auto-wait loop the one-shot `perform_locator_action` lacked.
  Re-snapshots on a ~16ms cadence until the gate passes or the deadline elapses;
  a briefly-disabled field succeeds without a sleep, a never-receiving-events
  click fails closed at the deadline, a never-attached locator times out as
  `ElementNotFound`.
- `ActuationKind` (CoordinateSynthesis / Keyboard / EmbedderControl / Scroll /
  SemanticActivation); `impl BrowserDriver for FetchCascadeBrowserEngine` (the
  fast test path, maps plans back to `BrowserAction` by handle).

## What landed (apps/browser live Servo adapter)

codex (`apps/browser/src/main.rs`):
- `ServoWebViewAutomationDriver`: a local wrapper implementing
  `BrowserDriver` for a live Servo `WebView` plus `Servo` event-loop handle.
  Rust coherence prevents implementing the external `rustyred-web` trait
  directly for Servo's external `WebView` type, so the wrapper is the concrete
  apps/browser adapter.
- Live E4 snapshot: waits for `LoadStatus::Complete`, runs
  `GEOMETRY_SNAPSHOT_SCRIPT` through `WebView::evaluate_javascript`, requires a
  `JSValue::String`, and feeds it to `page_state_from_snapshot_json`.
- Live E6 coordinate scale: returns `WebView::device_pixels_per_css_pixel()` and
  uses the wrapper origin (zero for the current rendering context) for
  `device_point_at_rect_center`.
- Live coordinate synthesis: sends `MouseMove`, `MouseButton(Down/Up)`, double
  click, hover, and tap through `WebView::notify_input_event`; the delegate
  captures `notify_input_event_handled` into structured receipts.
- Live smoke flag: `cargo run --manifest-path apps/browser/Cargo.toml --
  --headless-automation-smoke` loads an intercepted page, snapshots button/input
  handles, clicks a real button handler via `notify_input_event`, fills an input,
  and re-snapshots to verify the value round-tripped.
- Still intentionally unsupported in this adapter: Servo EmbedderControl
  responses and `SemanticActivation`. EmbedderControl needs the native-control
  response path; semantic activation needs the #4344 fork route documented in
  `servo-4344-semantic-activation-fork-plan.md`.

## Acceptance map (Slices 1-2)

| Plan item | Verdict | Note |
|---|---|---|
| Slice 1: snapshot JSON parses into a non-empty handle list | COVERED (Servo-free + live adapter) | `page_state_from_snapshot_json` + unit test; `--headless-automation-smoke` is the live Servo oracle. |
| Slice 1: `get_by_role`/`get_by_text`/`get_by_test_id` resolve | COVERED | Codex's resolver, tested over the snapshot shape and the HTML reader. |
| Slice 2: briefly-disabled field fills without a sleep | COVERED | `auto_wait_passes_once_a_briefly_disabled_field_enables`. |
| Slice 2: click fails closed on receives-events (no blind click) | COVERED | `auto_wait_fails_closed_when_receives_events_never_passes`. |
| Slice 2: coordinate transform (E5/E6) | COVERED (math + live event path) | Unit-tested math plus live `notify_input_event` smoke against a real page handler; deeper Paint hit-test occlusion remains Slice 4. |
| Slice 2: select_option / set_input_files via EmbedderControl | PARTIAL | Plan + mechanism present; the live EmbedderControl response is the apps/browser GAP. |
| Fence: rustyred-web stays Servo-free | COVERED | No libservo; the Servo surface is the `BrowserDriver` trait. |

## Validation receipts

- `cargo test -p rustyred-web --lib`: 125 passed, 0 failed, **0 warnings**
  (non-test and test builds both clean). The 8 new `browser_driver` tests plus
  Codex's `browser_automation` tests are in that count.
- `cargo test -p rustyred-web`: 125 lib tests + integration tests green
  (`epistemic_parity`, `fixture_crawl`, `frontier_acceptance`) after the live
  adapter landed.
- `cargo test -p theorem-browser-agent`: 5 passed.
- `CARGO_TARGET_DIR=/tmp/theorem-browser-target cargo check --manifest-path
  apps/browser/Cargo.toml --bin theorem-browser` from `/tmp/theorem-repo`: green
  against pinned Servo `b891f04d`.
- `CARGO_TARGET_DIR=/tmp/theorem-browser-target cargo run --manifest-path
  apps/browser/Cargo.toml --bin theorem-browser -- --headless-automation-smoke`
  from `/tmp/theorem-repo`: green. Receipt:
  `interactive_elements=2`, `click_mechanism=servo_notify_input_event`,
  `fill_mechanism=servo_focus_then_value_commit`.
- Direct `cargo check --manifest-path apps/browser/Cargo.toml --bin
  theorem-browser` from the real checkout fails before app code because
  `tikv-jemalloc-sys` rejects a configure prefix containing spaces
  (`Tech Dev Local`). The no-space `/tmp/theorem-repo` symlink plus
  `/tmp/theorem-browser-target` is the local workaround; CI paths are already
  space-free.
- Earlier Claude handoff also reported downstream server/MCP `--no-run` receipts;
  this Codex pass reran `rustyred-web`, `theorem-browser-agent`, the live
  `theorem-browser` check/smoke, and `git diff --check`.
- #4344 source scan completed against Servo
  `b891f04d0819272b27e80ac975e2e57d3cb9e66b`; the fork route is captured in
  `servo-4344-semantic-activation-fork-plan.md`.
- Uncommitted: both heads' work sits in the working tree on `feat/crdt-substrate`.
  Commit with an explicit pathspec (shared index); do not bare-commit.

## Remaining (the seam targets)

1. **apps/browser EmbedderControl responses** (codex lane, CI-only): wire
   `select_option` / `set_input_files` through Servo's native-control response
   path and return receipts alongside the coordinate-synthesis receipts.
2. **#4344 Servo fork** (codex lane, CI-only): the `perform_action` route across
   servoshell/constellation/script/layout, filling `SemanticActivation`.
   Grounded fork plan:
   `docs/plans/servo-browser-use-agent/servo-4344-semantic-activation-fork-plan.md`.
3. **Full Playwright selector bundle** vendoring (replace the thin bridge).
4. **Slice 3 remainder**: wire `Context` to a real storage partition and `route`
   to the live interception seam (`load_web_resource().intercept()`).
5. **Slice 4**: engine-truth upgrades (box-tree visibility, settle-signal
   stability, engine occlusion hit-test) swapping the JS heuristics behind the
   same API.
