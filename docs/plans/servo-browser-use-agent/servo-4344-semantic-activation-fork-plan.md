# Servo #4344 semantic activation fork plan

Sibling to `servo-automation-core-playwright-class.md` and
`servo-automation-core-status.md`. This is the source-grounded fork plan for
filling `ActuationKind::SemanticActivation` after the live apps/browser adapter.

## Goal

Route an AccessKit action request from the servoshell embedder into the active
Servo document tree so Theorem can activate an element by semantic identity
instead of synthesized coordinates. The first target is AccessKit
`Action::Default` for button/link/input style activation, backed by Servo's
existing DOM activation path.

## Grounded source facts

Pinned Servo source inspected at `b891f04d0819272b27e80ac975e2e57d3cb9e66b`.

- `ports/servoshell/desktop/headed_window.rs`: `handle_winit_app_event` receives
  `egui_winit::accesskit_winit::WindowEvent::ActionRequested(req)`. If
  `req.target_tree != accesskit::TreeId::ROOT`, the current code hits
  `TODO(#4344): Forward action to Servo`.
- `ports/servoshell/desktop/gui.rs`: root-tree action requests are already
  routed to egui with `self.context.egui_winit.on_accesskit_action_request(...)`.
  Non-root document-tree actions are the missing Servo route.
- `components/servo/webview.rs`: `WebView::accesskit_tree_id()`,
  `set_accessibility_active(...)`, `notify_document_accessibility_tree_id(...)`,
  and `process_accessibility_tree_update(...)` already bridge WebView-level and
  document-level accessibility trees. The WebView root tree graft node points at
  the document tree id.
- `accesskit::ActionRequest` has the fields `action`, `target_tree`,
  `target_node`, and `data`. The request carries the document tree id and node id
  needed to cross the #4344 gap.
- `components/layout/accessibility_tree.rs`: `AccessibilityTree` stores
  `nodes: FxHashMap<NodeId, AccessibilityNode>` and
  `opaque_node_to_id: FxHashMap<OpaqueNode, NodeId>`. The reverse lookup needed
  for activation is not currently public.
- `components/script/dom/html/interactive_element_command.rs`:
  `InteractiveElementCommand::perform_action(...)` already fires the DOM action
  for anchors, buttons, inputs, options, and generic HTML elements.
- `components/script/dom/document/document_event_handler.rs`: the access-key
  path already resolves an `InteractiveElementCommand`, checks disabled/hidden
  and connected state, focuses/scrolls the element, then calls
  `perform_action(...)`.
- `components/constellation/constellation.rs` and
  `components/script/script_thread.rs`: accessibility activation state already
  flows from embedder to constellation to script via
  `SetAccessibilityActive`. The new action route should follow this shape.

## Patch route

1. Add a public Servo WebView API, for example
   `WebView::notify_accessibility_action(request: accesskit::ActionRequest)`.
   Root-tree requests can remain embedder-owned; non-root document-tree requests
   should be forwarded to constellation with the `WebViewId`.
2. Add an embedder-to-constellation message such as
   `ForwardAccessibilityAction(WebViewId, AccessibilityActionRequest)`. Use the
   native `ActionRequest` only if it crosses Servo's IPC/channel boundaries
   cleanly; otherwise define a compact internal struct carrying `target_tree`,
   `target_node`, `action`, and optional data.
3. In constellation, map `WebViewId` to the active top-level pipeline and forward
   the action to script. Preserve enough tree or epoch information to ignore
   stale requests safely.
4. In layout or the accessibility-tree owner, add the reverse resolution needed
   for `NodeId -> OpaqueNode` for the current document accessibility tree.
   The forward map already exists as `opaque_node_to_id`; this route needs the
   inverse for action dispatch.
5. In script/document, add a semantic activation helper that mirrors the
   access-key path:
   resolve `OpaqueNode` to the DOM node, build an
   `InteractiveElementCommand`, reject disabled/hidden/disconnected targets,
   focus and scroll, then call `perform_action(...)`.
6. Initially map AccessKit `Action::Default` to DOM activation and
   `Action::Focus` to focus-only behavior if the source route exposes it cleanly.
   Return an explicit unsupported receipt for other actions until each has a
   source-backed mapping.
7. In `apps/browser`, update
   `ServoWebViewAutomationDriver::actuate_sync` so
   `ActuationKind::SemanticActivation { node_id, action }` constructs and sends
   the WebView accessibility action once the forked API exists.

## Acceptance tests

- Servo fork test: enable accessibility, load a page with a button handler, find
  the document tree id and node id, issue an AccessKit `Action::Default`, and
  assert the handler mutates page state without mouse coordinates.
- Negative Servo test: stale or wrong tree id is ignored and does not activate
  the element.
- Negative Servo test: disabled or hidden interactive elements do not perform
  activation.
- Theorem smoke extension: add
  `--headless-automation-smoke --semantic-activation` to exercise
  `ActuationKind::SemanticActivation` against the forked Servo API.

## Fork mechanics

- Create or update a `Travis-Gilbert/servo` branch from Servo
  `b891f04d0819272b27e80ac975e2e57d3cb9e66b`.
- Keep the fork patch narrow: servoshell action handoff, WebView API,
  constellation message, script/layout action dispatch, and tests.
- After the fork is green, update `apps/browser/Cargo.toml` Servo dependencies
  (`servo`, `embedder_traits`, and any sibling git pins used by the app) to the
  fork revision.
- Keep `apps/browser/rust-toolchain.toml` aligned with the pinned Servo checkout.

## Open decisions

- Whether `accesskit::ActionRequest` can cross the relevant Servo channels
  directly, or whether Servo should define a compact serializable request type.
- Whether the activation receipt should be best-effort only, or whether Servo
  should emit an explicit action-completed/action-rejected callback to the
  embedder.
- Whether the first implementation should handle only the active top-level
  document tree or also route iframe document trees in the same patch.
