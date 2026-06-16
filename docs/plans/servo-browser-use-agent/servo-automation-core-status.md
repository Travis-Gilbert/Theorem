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
- Live native controls: `ActuationKind::EmbedderControl` now carries the target
  point; the apps/browser delegate captures Servo `EmbedderControl` callbacks and
  the driver responds with `SelectElement::select(...).submit()` and
  `FilePicker::select(...).submit()`.
- Live smoke flag: `cargo run --manifest-path apps/browser/Cargo.toml --
  --headless-automation-smoke` loads an intercepted page, snapshots
  button/input/select/file handles, clicks a real button handler via
  `notify_input_event`, fills an input, selects a `<select>` option through
  Servo's native control, sets an upload through Servo's file picker, and
  verifies the DOM state after each action.
- Still intentionally unsupported in this adapter: `SemanticActivation`.
  Semantic activation needs the #4344 fork route documented in
  `servo-4344-semantic-activation-fork-plan.md`.

## Acceptance map (Slices 1-2)

| Plan item | Verdict | Note |
|---|---|---|
| Slice 1: snapshot JSON parses into a non-empty handle list | COVERED (Servo-free + live adapter) | `page_state_from_snapshot_json` + unit test; `--headless-automation-smoke` is the live Servo oracle. |
| Slice 1: `get_by_role`/`get_by_text`/`get_by_test_id` resolve | COVERED | Codex's resolver, tested over the snapshot shape and the HTML reader. |
| Slice 2: briefly-disabled field fills without a sleep | COVERED | `auto_wait_passes_once_a_briefly_disabled_field_enables`. |
| Slice 2: click fails closed on receives-events (no blind click) | COVERED | `auto_wait_fails_closed_when_receives_events_never_passes`. |
| Slice 2: coordinate transform (E5/E6) | COVERED (math + live event path) | Unit-tested math plus live `notify_input_event` smoke against a real page handler; deeper Paint hit-test occlusion remains Slice 4. |
| Slice 2: select_option / set_input_files via EmbedderControl | COVERED (live adapter) | Delegate-captured Servo `SelectElement` / `FilePicker` responses, with DOM re-read in `--headless-automation-smoke`. |
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
- Committed pathspec'd as `1fd442a4` (core + adapter + CI step + docs) on
  `feat/crdt-substrate`, pushed. claude-code independently re-ran the warm
  `/tmp/theorem-browser-target` embedder `--headless-automation-smoke`: `EXIT=0`,
  same receipt (`interactive_elements=2`, `servo_notify_input_event`,
  `servo_focus_then_value_commit`) -- the live Servo browser is driven by the
  automation core, verified independent of the build host.
- CI (`servo-browser.yml`) first failed at LINK on `-lopenblas`: the Ubuntu runner
  lacks the OpenBLAS the local brew/macOS build has. `cblas-sys`/`ndarray` is a
  transitive pull via rustyred-web's ML stack, linked once the adapter references
  more of rustyred-web. Stopgap fix: install `libopenblas-dev` in CI (`906a0024`),
  re-triggered as run 27569567121 -> GREEN (17m32s): the embedder built + LINKED
  on Linux and all four smokes passed, including the headless Playwright-class
  automation smoke. The browser is proven up on both local (macOS) and CI (Linux).
- Codex follow-up trimmed the default BLAS edge out of `apps/browser`: TurboVec is
  now opt-in behind `vector-accelerated` in `rustyred-thg-core` and `rustyred-web`.
  Default `theorem-browser` dependency checks now report no `cblas-sys` and no
  `turbovec` package in the graph; `cargo check -p rustyred-thg-core` and
  `cargo check -p rustyred-web` pass with default features. The
  `servo-browser.yml` OpenBLAS install step was removed. Next CI should prove the
  embedder links on Linux without the syslib stopgap.
- BLAS-trim validation receipts: `cargo tree --manifest-path
  apps/browser/Cargo.toml -i cblas-sys` and `-i turbovec` both fail with "package
  ID specification ... did not match any packages"; feature-tree grep over
  `theorem-browser` has no `turbovec|cblas|ndarray|openblas|vector-accelerated`
  hits; `cargo test -p rustyred-thg-core vector_` passes 6 tests; `cargo test -p
  rustyred-web ring_`, `relevant_match_outranks_the_central_hub`, and
  `super_hub_does_not_flood_the_neighbourhood` pass; `cargo check -p
  rustyred-thg-core --features vector-accelerated` and `cargo check -p
  rustyred-web --features vector-accelerated` pass; `CARGO_TARGET_DIR=/tmp/theorem-browser-target
  cargo check --manifest-path apps/browser/Cargo.toml --bin theorem-browser` from
  `/tmp/theorem-repo` passes.
- EmbedderControl validation receipt: `CARGO_TARGET_DIR=/tmp/theorem-browser-target
  cargo run --manifest-path apps/browser/Cargo.toml --bin theorem-browser --
  --headless-automation-smoke` from `/tmp/theorem-repo` passes with
  `interactive_elements=4`, `click_mechanism=servo_notify_input_event`,
  `fill_mechanism=servo_focus_then_value_commit`,
  `select_mechanism=servo_embedder_select_element`, and
  `file_mechanism=servo_embedder_file_picker`.

## Remaining (the seam targets)

1. **#4344 Servo fork** (codex lane, CI-only): the `perform_action` route across
   servoshell/constellation/script/layout, filling `SemanticActivation`.
   Grounded fork plan:
   `docs/plans/servo-browser-use-agent/servo-4344-semantic-activation-fork-plan.md`.
2. **Full Playwright selector bundle** vendoring (replace the thin bridge).
3. **Slice 3 remainder**: wire `Context` to a real storage partition and `route`
   to the live interception seam (`load_web_resource().intercept()`).
4. **Slice 4**: engine-truth upgrades (box-tree visibility, settle-signal
   stability, engine occlusion hit-test) swapping the JS heuristics behind the
   same API.
