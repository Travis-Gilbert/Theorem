# Servo automation core: a Playwright-class driver on embedded Servo

Plan home: `docs/plans/servo-browser-use-agent/` (sibling to `build-step-1-parity.md`, `build-step-1-correction-actuation.md`, `build-step-2-engine-abilities.md`).
Audience: Claude Code + Codex, building as one agent.
Builds on: the actuation correction (`9ee18e7e`), which already settled coordinate synthesis, the JS geometry snapshot, EmbedderControl responses, and the `#4344` defer. Grounded against `apps/browser` (embedded, non-forked `servo` crate, green in CI) and the existing `rustyred-web` engine (`browser_engine.rs`, `browser_perception.rs`, `browser_run.rs`).

## North star

Browser Use is an agent. Playwright is the substrate an agent like that should sit on. The existing parity slice (job-007) builds a flat, agent-facing `interactive_elements` list. This plan builds the layer underneath it: a Playwright-class automation library for Servo, with locators, actionability, contexts, request routing, web-first assertions, and tracing. The perceive/govern/afford loop and `browse_with_me` / `browse_for_me` become clients of this engine rather than the engine itself. One durable primitive, many drivers.

The wedge, in one line: a Playwright-shaped API whose actionability is computed from Servo engine truth (box tree, frame-accurate stability, Paint hit-testing) instead of injected DOM heuristics. Playwright's reliability is its auto-wait. Servo can do auto-wait better than CDP because it owns the layout and the frame loop. That is the reason to build this, not a faster clone.

## The de-risking spine

Every Playwright actionability check has two implementations: Playwright's own injected-JS method, and a Servo engine-truth method.

- V1 ships parity entirely through injected JS (the geometry snapshot, the selector engine, the six checks as page-side JS) over the public embedding API, with zero engine forking. This matches the `apps/browser` posture (embed now, fork on evidence) exactly, and it resolves the tension where `build-step-1-parity.md` assumed AccessKit roles, bounds, and an action path that source examination disproved.
- The Servo "exceed CDP" abilities (engine hit-test for true occlusion, frame-accurate settle, box-tree visibility, layout-order extraction) are upgrades that swap a JS heuristic for engine truth. They land as fork patches in job-008 and raise fidelity without changing the API.

So the Playwright-class core is buildable on embedded Servo today. The fork buys reliability, not capability to exist.

## Protocol decision (the fork the existing docs skip)

Three ways to expose Servo automation. The existing plan goes straight to the in-process embedding API without weighing the alternatives. Weighed:

- Classic W3C WebDriver via Servo's own `components/webdriver_server` (verified present: `server.rs`, `session.rs`, `actions.rs`, `capabilities.rs`, `user_prompt.rs`). Rejected as the substrate. It is HTTP request/response, synchronous, WPT-shaped. It has no event stream, no auto-wait, no request interception, and it re-introduces the process boundary the in-process thesis exists to remove (browsing as immediate graph ingestion). It is a conformance surface, not an automation engine.
- WebDriver BiDi server. Servo has no BiDi (no `bidi` module under `webdriver_server`; the standard is still living, last spec touch April 2026). BiDi is the converging target and Playwright itself is migrating onto it. This is the multi-language scripting story: a BiDi front end would let real Playwright, Puppeteer, or WebdriverIO clients drive Servo.
- In-process Rust API. Fast, matches the in-process thesis, and is exactly what the agent loop needs.

Named choice: build the in-process Rust API for V1. Shape its command and event vocabulary after the BiDi module taxonomy (`session`, `browsingContext`, `input`, `network`, `script`, `log`) so a BiDi WebSocket server is a thin adapter over the same core later, not a rewrite. Do not build the BiDi server in V1. This keeps the in-process speed now and leaves the door open for "real Playwright drives Servo" without a second engine.

## Gating decisions (sign-offs that unblock the build)

These three are the prerequisites. The first two are already flagged as pending Travis sign-off in the correction doc; this plan adds the third.

1. id-space: Option A. The actionable element id is the JS snapshot handle (a deterministic `data-theorem-id` stamp the snapshot script sets, stable per document). The AccessKit `NodeId` is retained only inside the structural overlay. Coordinate synthesis cannot obtain a `NodeId` (Servo exposes no DOM-node to NodeId resolver; `id_for_opaque` is layout-internal), so the "NodeId is the id" fence and the corrected actuation model cannot both hold. Option A is the relaxation. This is the single change that unblocks the actuation path.
2. `#4344`: defer for V1. Coordinate synthesis plus EmbedderControl cover click, type, select, upload, scroll, and dialogs without semantic activation by node id. Revisit only if AT-grade semantic actuation becomes a goal.
3. Selector engine: reuse Playwright's injected selector engine (MIT). It is page-side JavaScript (CSS, text, role, label, placeholder, alt, title, test-id, and layout selectors like `:near` and `:right-of`), license-compatible, and it runs through `evaluate_javascript` unchanged. Writing a selector engine from scratch is months of surface area for no differentiation. Reuse it, vendor it under `rustyred-web/src/vendor/`, and wrap it in the Rust locator API. The differentiation is the actionability substrate, not the selector parser.

## Object model (Playwright -> Servo -> existing code)

| Playwright | Servo automation core | Backed by |
| --- | --- | --- |
| `Playwright` (entry) | `BrowserEngine` handle | the `Servo` instance + `BrowserPool` (already specced) |
| `Browser` | the `Servo` engine instance | `servo::Servo`, `ServoBuilder` (`apps/browser`) |
| `BrowserContext` (isolated cookies/storage/permissions) | `Context` (storage partition + permission set + a `WebView` group) | `Preferences`/`Opts` + `request_permission` / `request_authentication` callbacks |
| `Page` | `Page` | one `servo::WebView` |
| `Frame` | `Frame` | child document, addressed via per-frame `evaluate_javascript` context |
| `Locator` (lazy selector, chainable, `getByRole`/`getByText`/`getByTestId`) | `Locator` | injected selector engine, resolved at action time |
| `ElementHandle` | `ElementHandle` | JS snapshot handle + rect + computed state |
| Selectors engine | injected JS (reused) | `evaluate_javascript` |
| auto-wait / actionability | the actionability gate | JS snapshot now, engine truth later |
| `page.route` (network) | `route` / `fulfill` / `abort` / `continue` | `WebViewDelegate::load_web_resource().intercept()` |
| Tracing / trace viewer | `BrowsingRun` + harness replay | `browser_run.rs` + harness run ledger |
| `expect(locator)` (web-first) | web-first assertions | the same actionability/snapshot machinery |
| dialogs / downloads / file chooser | EmbedderControl + download detect | `SimpleDialog`, `FilePicker`, observe-delta |

## The actionability core (the spine)

A `Locator` resolves to one or more handles at action time. Before any action the gate runs the required checks, retrying on the snapshot cadence until they pass or the deadline elapses, then actuates by coordinate synthesis. The six Playwright checks, each mapped to a V1 (JS) method and a job-008 (engine-truth) upgrade:

- attached: connected to a Document or ShadowRoot. V1: the snapshot includes the node, or `isConnected`. Engine upgrade: TreeUpdate presence.
- visible: non-empty bounding box and no `visibility:hidden`; zero-size and `display:none` are not visible. V1: `getBoundingClientRect` plus computed style in the snapshot. Engine upgrade: box-tree visibility (display, visibility, opacity, viewport clip, zero-size) read directly, the job-008 D1 deliverable.
- stable: same bounding box for two consecutive animation frames. V1: injected `requestAnimationFrame` measuring the rect across two frames, exactly Playwright's method. Engine upgrade: compare the rect across two `spin_event_loop` frames, frame-accurate because Servo owns the loop, plus the layout-quiescence settle signal (job-008 D2).
- enabled: not `[disabled]`, not inside a disabled `<fieldset>`, not under `aria-disabled=true`. V1: computed in the snapshot. Engine: same, no upgrade needed.
- editable: enabled and not readonly (`[readonly]` or `aria-readonly=true` on a supporting role). V1: snapshot. Engine: same.
- receives events: the element is the hit target at the action point. V1: injected `document.elementFromPoint(x, y)` at the resolved rect center, exactly Playwright's method. Engine upgrade: a real engine occlusion query (job-008 D1), which sees occlusion `elementFromPoint` cannot (cross-origin overlays, pointer-events quirks, paint-order truth).

Per-action requirements follow Playwright's matrix: click, check, dblclick, setChecked, tap require visible + stable + receives-events + enabled; fill requires visible + enabled + editable; hover requires visible + stable + receives-events. A `force` flag drops the non-essential checks (receives-events), parity with Playwright.

Actuation after the gate passes, per the correction doc: resolve the handle to its rect from the snapshot, convert `device_point = rect_center * device_pixels_per_css_pixel + webview_origin_in_rendering_context`, then `notify_input_event(MouseMove)`, `notify_input_event(MouseButton(Down))`, `notify_input_event(MouseButton(Up))`. The receipt is `notify_input_event_handled` (`Consumed` or `DefaultPrevented`) plus a post-action snapshot diff.

Sketch:

```rust
pub struct Locator { page: PageId, selector: String, chain: Vec<Step> }

impl Locator {
    pub fn get_by_role(&self, role: AriaRole, opts: RoleOptions) -> Locator;
    pub fn get_by_text(&self, text: &str, opts: TextOptions) -> Locator;
    pub fn get_by_label(&self, text: &str) -> Locator;
    pub fn get_by_test_id(&self, id: &str) -> Locator;
    pub fn filter(&self, has: Option<Locator>, has_text: Option<&str>) -> Locator;
    pub fn nth(&self, i: usize) -> Locator;

    pub async fn click(&self, opts: ClickOptions) -> Result<ActReceipt>;
    pub async fn fill(&self, value: &str) -> Result<ActReceipt>;
    pub async fn select_option(&self, opt: OptionRef) -> Result<ActReceipt>; // EmbedderControl
    pub async fn set_input_files(&self, paths: &[PathBuf]) -> Result<ActReceipt>; // EmbedderControl
    pub async fn wait_for(&self, state: ElementState, deadline: Duration) -> Result<()>;
}

struct Actionability { visible: bool, stable: bool, enabled: bool, editable: bool, hit_target: bool, attached: bool }

// resolve(locator) -> Vec<ElementHandle> via injected selector engine
// gate(handle, required_checks, deadline) -> retries snapshot until pass or timeout
// actuate(handle, action) -> notify_input_event sequence + receipt
```

## Request routing (near-free, the seam exists)

Wrap the existing interception seam in a Playwright-style API. `WebViewDelegate::load_web_resource(...).intercept()` already returns an `InterceptedWebResourceLoad` with `send_body_data` / `finish` / `cancel` (verified in the embedding examination). That is `fulfill` / `continue` / `abort`.

```rust
impl Context {
    pub fn route(&self, pattern: UrlPattern, handler: impl Fn(Route) -> RouteAction);
}
enum RouteAction { Continue, Abort, Fulfill { status: u16, headers: Headers, body: Vec<u8> }, ContinueWith { headers: Headers } }
```

This gives request mocking, blocking, and modification, plus the job-008 D3 resource policy (block images, fonts, media for fast text-first runs) as a built-in route rule. It also feeds the substrate: a fulfilled or observed response is the same `FetchedPage` to `FetchedPage` to graph write seam `apps/browser` already runs.

## Web-first assertions

A thin retrying layer over the snapshot and actionability machinery. `expect(locator).to_be_visible()`, `.to_have_text(...)`, `.to_be_enabled()`, `.to_have_count(n)`, each polling the snapshot until the predicate holds or the deadline elapses. No new engine surface. This is what turns the core from an automation toy into something a test suite or an agent can assert against.

## Tracing (already richer than Playwright)

The Playwright trace viewer is a screenshot reel plus action and network logs. The `BrowsingRun` plus harness replay (`browser_run.rs`, job-008 D5) is engine-native: the TreeUpdate sequence, the issued input events, the settle signals, the actionability verdicts, the route decisions, all content-addressed and replayable at the accessibility and action layer, forkable like an EnsembleDecision. Record the actionability verdict per action (which check gated, for how long) so a flaky resolution is inspectable, which Playwright's trace cannot show. This is the verification artifact and the billing telemetry in one.

## Frames

Servo grafts child-document a11y trees into the parent (job-008 D4), so structure is one coherent tree. The actionable path is the JS snapshot, which is per document, so a `Locator` carries a frame target and runs its snapshot and selector resolution in the target frame's `evaluate_javascript` context. Confirm at the pin whether `evaluate_javascript` accepts a browsing-context or frame target; if it does not yet, that is a small fork patch (route the script to the target pipeline) and a job-008 item. Cross-frame coordinate transforms compose the frame origin into the device-point conversion.

## Build sequence

Slices that slot into the existing job structure. Each is observable end to end. No engine fork is required through slice 3; slice 4 begins the exceed-CDP upgrades that land as fork patches in job-008.

Slice 1, the snapshot and selector substrate. Vendor the Playwright injected selector engine under `rustyred-web/src/vendor/`. Build the geometry snapshot script (per actionable element: `{ handle, tag, role, accessible_name, value, visible, rect }`, with the `data-theorem-id` stamp). Parse the returned `JSValue` JSON into the handle list. Acceptance: `evaluate_javascript` returns valid JSON that parses into a non-empty handle list on a live page, and `get_by_role` / `get_by_text` / `get_by_test_id` resolve to the right handles.

Slice 2, the actionability gate and the action set. Implement the six checks as injected JS (the V1 column above), the per-action requirement matrix, the retry loop on a deadline, and the coordinate-synthesis actuation with the E5/E6 transform. Route click, fill, hover, scroll through the gate; route select_option and set_input_files through EmbedderControl. Acceptance: a click on a button under a cookie banner waits and fails closed on receives-events (not a blind click); a fill into a field that is briefly disabled then enabled succeeds without a sleep; the coordinate transform lands a synthetic click on the intended element via Paint hit-testing.

Slice 3, contexts, routing, assertions, tracing. The `Context` storage and permission partition over a `WebView` group; `route` over the interception seam with the resource-policy rules; web-first `expect`; the `BrowsingRun` recording the actionability verdicts. Acceptance: two contexts do not share cookies; a route blocks images and a heavy page ingests materially faster; `expect(locator).to_be_visible()` retries and passes; a run replays with per-action actionability verdicts.

Slice 4, the engine-truth upgrades (begins job-008). Swap the JS checks for engine truth where it wins: box-tree visibility, frame-accurate stability via the settle signal, the engine occlusion hit-test for receives-events, layout-order extraction. Same API, higher fidelity. Acceptance: an element occluded in a way `elementFromPoint` misses is correctly reported occluded by the engine query; `wait_for` resolves on the layout settle signal with no sleep on a known-flaky timing case.

Slice 5 (optional, gated on a multi-language scripting goal), the BiDi adapter. A WebSocket front end mapping `browsingContext` / `input` / `network` / `script` commands and events onto the in-process core, so real Playwright, Puppeteer, or WebdriverIO clients drive Servo. Only if external scripting becomes a goal.

## Risks and kill criteria

- Coordinate-transform drift. Getting `device_pixels_per_css_pixel` or the webview origin wrong lands clicks on the wrong element silently. Mitigation: a calibration fixture that clicks a known rect center and asserts the hit target via Paint; it is a first-class acceptance item, not a smoke test.
- `evaluate_javascript` per-frame targeting. If Servo cannot target a child frame's context at the pin, cross-frame locators need a small fork patch sooner than slice 4. Confirm at build start; it changes when the fork begins, not whether the core ships.
- Selector-engine coupling. Vendoring Playwright's injected engine means tracking its updates. Mitigation: pin it, treat it as a vendored dependency with a recorded upstream rev, bump deliberately. It is page-side JS, so a bump cannot break the Rust build, only selector behavior, which the fixtures catch.
- Snapshot cost on large pages. A full-page snapshot per actionability poll is the obvious hot path. Mitigation: scope the snapshot to the candidate subtree the selector resolved, and consume the TreeUpdate diff (job-008 D2) as the change signal so the poll is incremental, not a full re-walk. If snapshot latency dominates action latency on a real page after that, the engine-truth path (slice 4) moves up.
- Kill criterion for the protocol bet: if a real flaky case still needs a sleep after the engine settle signal lands in slice 4, the engine-truth claim is not paying off and the project is a Playwright clone, not an exceed. That is the test of whether the Servo bet was worth it.

## Where it rides

`rustyred-web` owns the engine, the selector vendor, the snapshot, the actionability gate, the executor, the router, the assertions, and the run recorder (all outbound web I/O, per the parent fence). The vendored selector engine lives under `rustyred-web/src/vendor/`. The perceive/govern/afford stack in core calls the `Locator` and action API rather than a flat element list. The MCP surface (`browse_with_me`, `browse_for_me`, `web_consume`) registered in `rustyred-thg-mcp` becomes a thin client of this core. `apps/browser` stays the live embedder the end-to-end acceptance runs against.

## Open

- Confirm `evaluate_javascript` per-frame context targeting at the fork pin (`b891f04d`).
- Confirm the `Context` storage-partition surface in the embedding API (cookie jar and storage isolation per WebView group), or whether it needs a fork patch.
- Decide whether the agent's flat `interactive_elements` list is derived from the `Locator` core (recommended, one source of truth) or kept parallel for the existing parity slice during transition.
