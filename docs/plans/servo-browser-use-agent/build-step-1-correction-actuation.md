# Build step one correction: actuation by coordinate synthesis, not AccessKit actions

Author: claude (claude.ai). Correction layered on `build-step-1-parity.md` (spec)
and `build-step-1-status.md` (what is built). Scope: the open executor half
(codex's `browser_engine.rs::act()` and the `BrowserAction` variants), plus the
session-start accessibility lifecycle and an explicit fork decision on Servo
issue #4344.

This does not touch the built reader/governance half
(`rustyred-web/src/browser_perception.rs`, the `degraded` field, the masking /
domain / upload-allowlist / schema / tabs primitives). It corrects the actuation
model the executor half was about to build against.

## Why this correction exists

`build-step-1-status.md` leaves the executor open with this shape: "`act()`
driven by AccessKit `ActionRequest` (not synthetic events)" and "the Servo-fork
`perform_action` handler the grounding flagged." Source examination of
`servo/servo` shows that path is not a missing handler, it is an unimplemented
end-to-end route, and the accessibility tree at the pinned rev cannot drive
actuation regardless:

- The AccessKit action route is an explicit upstream TODO. In
  `ports/servoshell/desktop/headed_window.rs`, the embedder receives the OS
  AccessKit action and does nothing but leave `// TODO(#4344): Forward action to
  Servo`. There is no `perform_action` anywhere that turns an `ActionRequest`
  into DOM activation.
- The layout tree carries no geometry. In
  `components/layout/accessibility_tree.rs` each node carries only role, label,
  value, html_tag, children. Bounds are never set, so a click target has no
  coordinates from the tree.
- The role table is minimal. The HTML to role mapping covers
  `article, aside, body, footer, h1-h6, header, hr, main, nav, p`. `a`, `button`,
  `input`, `select`, `textarea`, `img`, `ul`, `li`, `table` fall through to
  `GenericContainer`. This is the status doc's "thin interactive roles" follow-up,
  confirmed at the source.
- The tree is grafted and multi-tree. `components/servo/webview.rs`
  (`notify_document_accessibility_tree_id`) emits a WebView root (`NodeId(0)`,
  `ScrollView`) with a graft child (`NodeId(1)`) carrying the document subtree's
  own id space. This is the status doc's "multi-tree grafting" follow-up.

Net: the a11y tree gives a structural and text skeleton, not actionable elements,
and there is no semantic action path. Actuation has to be coordinate synthesis,
with element geometry sourced from injected JavaScript.

Evidence files (current `servo/servo` main): `components/servo/webview.rs`
(SHA 6eff96f), `components/servo/webview_delegate.rs` (SHA 82eaa43),
`components/layout/accessibility_tree.rs` (SHA 3c6a6f2),
`ports/servoshell/desktop/headed_window.rs` (SHA 08367dc). Reconcile signatures
against the fork's pinned rev `b891f04d0819272b27e80ac975e2e57d3cb9e66b` before
implementing (the kind of finding is stable across the two; exact signatures
verify at the pin).

## Servo APIs the corrected executor binds to (all verified at source)

- Actuation: `WebView::notify_input_event(InputEvent) -> InputEventId`. Point
  events (`MouseMove`, `MouseButton`, `Wheel`, `Touch`) are hit-tested through
  Paint, so a synthetic event at the right coordinate routes to the right
  element. The receipt is `WebViewDelegate::notify_input_event_handled(WebView,
  InputEventId, InputEventResult)`, where `InputEventResult` carries `Consumed`
  and `DefaultPrevented` (use `.intersects`).
- Input variants: `InputEvent::MouseMove(MouseMoveEvent::new(point))`,
  `InputEvent::MouseButton(MouseButtonEvent::new(MouseButtonAction::Down|Up,
  MouseButton::Left, point))`, `InputEvent::Keyboard(KeyboardEvent)`,
  `InputEvent::Ime(ImeEvent::Composition(CompositionEvent { state:
  CompositionState::Start|Update|End, data }))`,
  `InputEvent::EditingAction(EditingActionEvent::{Cut,Copy,Paste})`,
  `InputEvent::Wheel(WheelEvent::new(delta, point))`.
- Geometry and structured reads: `WebView::evaluate_javascript(script,
  FnOnce(Result<JSValue, JavaScriptEvaluationError>))`. This returns a value
  (not fire-and-forget), so a snapshot script can return element rectangles and
  attributes.
- Coordinate space: `WebView::device_pixels_per_css_pixel() -> Scale<f32,
  CSSPixel, DevicePixel>` (accounts for page zoom, pinch zoom, HiDPI). Needed
  because `getBoundingClientRect` is CSS pixels viewport-relative and
  `notify_input_event` points are device pixels in the rendering context.
- Scroll: `WebView::notify_scroll_event(Scroll, WebViewPoint)`, or JS
  `scrollIntoView` via `evaluate_javascript`.
- Native control surfaces: `WebViewDelegate::show_embedder_control(WebView,
  EmbedderControl)` delivers `SelectElement`, `FilePicker`, `SimpleDialog`,
  `ColorPicker`, `InputMethod`, `ContextMenu`, each with a `DeviceIntRect`
  position and a structured response handle (for example
  `SelectElement::{options(), select(Vec<usize>), submit()}`,
  `FilePicker::{filter_patterns(), select(&[PathBuf]), submit(), dismiss()}`).
- Navigation: `WebView::load(Url)`; `WebView::url()`, `WebView::page_title()`.
- Accessibility lifecycle: `WebView::set_accessibility_active(bool) ->
  Option<TreeId>`, gated on `pref!(accessibility_enabled)` (returns `None` if the
  pref is off). The returned `TreeId` must be grafted into a host AccessKit tree
  before forwarding updates, or AccessKit panics.

## Corrected executor deliverables

**E1. Two actuation mechanisms, not one.** Replace the single
`ActionRequest`-driven `act()` with:

- Mechanism A, coordinate synthesis via `notify_input_event`, for general DOM
  content: click, type, scroll, hover.
- Mechanism B, EmbedderControl responses, for native UI that Servo renders
  itself: `<select>` dropdowns, file inputs, dialogs, color inputs, IME,
  context menus.

Most `BrowserAction` variants resolve to A; `SelectOption` and `upload_file`
resolve to B (below).

**E2. Click and type (mechanism A).**

- `Click(element)`: resolve the element handle to its rectangle (E5), convert to
  a device point at the rect center (E6), then
  `notify_input_event(MouseMove(center))` followed by
  `notify_input_event(MouseButton(Down, Left, center))` and
  `notify_input_event(MouseButton(Up, Left, center))`. Record the returned
  `InputEventId`s and resolve the act against
  `notify_input_event_handled` (`Consumed` or `DefaultPrevented`) plus a
  post-action page-state diff.
- `SendKeys(element, text)`: `Click` the element to focus it, then either a
  sequence of `Keyboard(KeyboardEvent)` events for keystroke fidelity, or a
  single `Ime(Composition{ Start, then Update with data, then End })` to commit
  bulk text. Name the default in the implementation; verify `KeyboardEvent`
  construction against the fork's `keyboard_types` pin.
- Hover and `ScrollToElement`: `MouseMove(center)` for hover; for scroll, prefer
  `evaluate_javascript("...scrollIntoView...")` then re-read the rectangle, with
  `notify_scroll_event` / `Wheel` as the fallback when JS scroll is insufficient.

**E3. SelectOption and upload_file (mechanism B).**

- `SelectOption(element, option)`: a `Click` on a `<select>` makes Servo fire
  `show_embedder_control(SelectElement)`. The executor responds with
  `SelectElement::select(indices)` then `submit()`, where `indices` are resolved
  against `SelectElement::options()`. Do not coordinate-click native dropdown
  items; they may be OS-drawn outside the page surface.
- `upload_file`: a `Click` on a file input makes Servo fire
  `show_embedder_control(FilePicker)`. The executor responds with
  `FilePicker::select(paths)` then `submit()`, where `paths` come from the
  existing `resolve_upload_path` allowlist gate and `PermissionPolicy.allow_write`,
  and `dismiss()` on refusal. This is how acceptance criterion 5 closes: the gate
  sits on the FilePicker response, not on "setting the file input."
- Script dialogs (`alert`, `confirm`, `prompt`) arrive as
  `EmbedderControl::SimpleDialog` and are answered through their confirm/dismiss
  handles; route them rather than letting them block.

**E4. Geometry and actionable PageState via evaluate_javascript.** Inject a
snapshot script that returns, per actionable element, a structured record:
`{ handle, tag, role, accessible_name, value, visible, rect: {x, y, w, h} }`,
where `role` comes from the element's computed ARIA role (or tag fallback),
`visible` from viewport-and-occlusion checks, and `rect` from
`getBoundingClientRect`. Parse the returned `JSValue` JSON into the executor's
actionable element list. This list, not the a11y tree, is the actuation source.
The built `AccessibilityReader` stays as the structural and text overlay plus a
cheap change signal. This also sidesteps the two flagged a11y follow-ups
(thin interactive roles, multi-tree grafting), which do not affect a per-document
JS snapshot.

**E5/E6. Element resolution and the coordinate transform (named requirement).**
The executor resolves an action's target element to its rectangle from the E4
snapshot, then converts: `device_point = rect_point * device_pixels_per_css_pixel
+ webview_origin_in_rendering_context`. Getting this wrong lands clicks on the
wrong element silently, so it is a first-class acceptance item.

**E7. navigate() and tabs (carry the built primitives across the seam).**

- `navigate(url)`: enforce the existing `DomainPolicy` classifier before
  `WebView::load(url)`; refuse off-set navigations (criterion 8).
- Tabs: bind `TabSet` open/switch/close to `WebView` lifecycle and stamp an
  additive active-tab field into `PageState` / `page_state_payload`
  (criterion 6).

**E8. Accessibility lifecycle at session start.** Enable accessibility once per
session rather than lazily on an OS request: set
`Preferences::accessibility_enabled = true` on the `ServoBuilder`, then call
`WebView::set_accessibility_active(true)` at session start. Honor the graft
contract: graft the returned `TreeId` into a host AccessKit node before
forwarding any `TreeUpdate`, or queue updates until the graft exists, to avoid
the documented AccessKit panic. The resulting tree feeds the structural overlay
(E4), not actuation. This is the read-side enablement; it is independent of the
actuation change.

## The id-space decision (requires Travis's sign-off, not flipped here)

`build-step-1-status.md` records the fence "No second element-id space; NodeId is
the id," with `element_id = NodeId.to_string()`. That fence presumed a11y-sourced
elements actuated by NodeId through `ActionRequest`. Under coordinate synthesis
with JS geometry, the actuation path cannot obtain a NodeId (Servo exposes no DOM
node to NodeId map; `id_for_opaque` is layout-internal), and the a11y tree
supplies neither bounds nor interactive roles at the pin. So the fence and the
corrected actuation model cannot both hold as written. Options:

- Option A (recommended): a layered id model. The actionable element id is the
  E4 snapshot handle (a deterministic stamp, for example a `data-` attribute the
  snapshot script sets, or a stable document-order path). The a11y `NodeId` is
  retained only inside the structural overlay. This is an explicit relaxation of
  the fence to "the actionable id is the JS handle; NodeId is the a11y-overlay
  id."
- Option B: keep the fence unchanged and accept that live actuation stays blocked
  until the fork implements #4344 and rolls interactive roles and populates
  bounds and assembles the graft.
- Option C: fuzzy-join JS elements to a11y NodeIds by role, name, and document
  order. Fragile across dynamic pages; not recommended.

Recommendation is Option A. This is a named fence change, so nothing in the
executor lands against it without sign-off.

## #4344 as an explicit fork decision

Semantic actuation by node id (activate element N rather than synthesize a click
at its coordinates) requires the fork to implement Servo issue #4344: forward
AccessKit `ActionRequest`s from the embedder through the constellation into
script and DOM activation, and add the `perform_action` route the layout tree
references but does not implement. This touches the servoshell/embedder action
handler, the constellation, script, and layout. It is upstream-shaped
engineering, not a configuration flag.

Decision: defer #4344 for V1. Coordinate synthesis (E1, E2) plus EmbedderControl
responses (E3) cover click, type, select, upload, scroll, and dialogs without it.
Revisit #4344 only if AT-grade semantic actuation or accessibility-action parity
becomes a goal. If pursued later, it also closes the status doc's "click by
NodeId" reconciliation item and lets the a11y reader become the actionable source
(still pending roles, bounds, and graft assembly).

## Acceptance criteria (corrected)

- Criterion 3 (clicks and types by id): the executor clicks a reader-listed
  element by its handle and the element's handler fires, observed via
  `notify_input_event_handled` (`Consumed` or `DefaultPrevented`) or a
  post-action page-state diff.
- Criterion 5 (upload): clicking a file input fires `FilePicker`; the executor
  responds with an allowlisted path through `resolve_upload_path` plus
  `allow_write`, dismisses on a disallowed path, and the input reflects the
  accepted file.
- Criterion 6 (tabs): open, switch, and close bound to `WebView` lifecycle, with
  the active-tab field present in `PageState`.
- Criterion 8 (navigation): `DomainPolicy` enforced in `navigate()` before
  `load`; an off-set URL is refused.
- Criterion 9 (browse_for_me end to end): a task completes against the live
  `apps/browser` embedder using coordinate synthesis plus the E4 reader, and
  emits a `BrowsingRun`.
- Criterion 10 (degraded keyboard-operable): a degraded element is operated via
  `Keyboard` events (Tab, Enter, arrows) through `notify_input_event`, per the
  built `keyboard_fallback_for` plan.
- Coordinate transform (E5/E6): a synthetic click at a known element's rect
  center resolves, via Paint hit-testing, to that element.
- Geometry snapshot (E4): `evaluate_javascript` returns valid JSON that parses
  into the actionable element list with non-empty rectangles on a live page.
- Records: the #4344 decision and the id-space decision are recorded in this
  plan directory with Travis's sign-off before the executor lands.

## Reconciliation with the status doc's open items

- Link value semantics: under coordinate synthesis, a link is actuated by a click
  at its rectangle, so the navigation happens because the click activates the
  link, not because the executor calls `navigate(value)`. Drop the assumption
  that `InteractiveElement.value` is a URL for the click path.
- extract(): route `browser_engine.rs::extract`'s stored schema through
  `extract_structured` / `validate_against_schema` so both extract surfaces share
  one contract (unchanged from the status doc; still owed).
- Multi-tree grafting and thin roles: moot for the actionable path under E4
  (JS snapshot is per-document). They remain relevant only if a future #4344
  decision makes the a11y tree the actionable source.
