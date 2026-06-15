//! Theorem Browser embedder (Servo) — build-validation and headless smoke entry.
//!
//! This is v2b step 1 of the substrate-native browser. Its only job is to prove
//! that `libservo` builds and links as a Cargo git dependency at the pinned rev
//! (see Cargo.toml) and that our engine-construction wiring compiles against the
//! real Servo API. The optional `--headless-smoke` mode advances v2b step 2:
//! create a real WebView with a software rendering context, intercept a known URL,
//! and write the supplied page into the RustyRed substrate seam.
//!
//! Why so small: the embedder crate compiles AFTER libservo (~30 min from cold),
//! so any error here costs a full libservo rebuild in CI. Step 1 keeps the API
//! surface minimal (confirmed against servo/components/servo/examples/winit_minimal.rs
//! and ports/servoshell/desktop/app.rs at the pinned rev) to maximize one-shot
//! success and warm the build cache. Step 2 is intentionally an intercepted smoke
//! page: Servo's public hook exposes the request before load, not the downloaded
//! response body, so a known intercepted body is the first auditable seam.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::error::Error;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use dpi::PhysicalSize;
use embedder_traits::WebResourceResponse;
use euclid::Scale;
use http::header::{HeaderValue, CONTENT_TYPE};
use http::StatusCode;
use rustyred_web::{
    build_actuation_plan, page_state_from_snapshot_json, A11yTreeUpdate, AccessibilityReader,
    ActuationKind, ActuationPlan, ActuationReceipt, BrowserActionPolicy, BrowserDriver,
    BrowserEngineError, BrowserEngineResult, DevicePoint as AutomationDevicePoint, Locator,
    LocatorAction, PointerKind, RoleOptions, GEOMETRY_SNAPSHOT_SCRIPT,
};
use scene_os_core::{
    compile_scene_package, AtomLifecycle, SceneAtom, SceneCompileInput, SceneRelation, SceneScene,
    SourceRef,
};
use scene_os_web::render_scene;
use serde_json::json;
use serde_json::Value;
use servo::{
    EventLoopWaker, InputEvent, InputEventId, InputEventResult, JSValue, LoadStatus, MouseButton,
    MouseButtonAction, MouseButtonEvent, MouseMoveEvent, Preferences, RenderingContext,
    ServoBuilder, SoftwareRenderingContext, TouchEvent, TouchEventType, TouchId, WebResourceLoad,
    WebView, WebViewBuilder, WebViewDelegate, WindowRenderingContext,
};
use theorem_browser_substrate::{
    browser_affordances, durable_browser_session, memory_browser_session,
    render_substrate_search_result_page, LiveFetchOptions, LoadedPage, RedCoreBrowserSessionStore,
    SubstrateSearch, TriggerGateConfig, UrlGuardPolicy,
};
use url::Url;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;

const SMOKE_URL: &str = "http://theorem.local/smoke";
const AUTOMATION_SMOKE_URL: &str = "http://theorem.local/automation-smoke";
const SEARCH_URL_PREFIX: &str = "http://theorem.local/search";
const SCENE_URL_PREFIX: &str = "http://theorem.local/scene";
const SCENE_SMOKE_URL: &str = "http://theorem.local/scene?q=substrate";
const DEFAULT_SCENE_QUERY: &str = "substrate";
const STORE_DIR_ENV: &str = "THEOREM_BROWSER_STORE_DIR";
const AUTOMATION_SPIN_TIMEOUT_MS: u64 = 5_000;
const SMOKE_HTML: &str = r#"<!doctype html>
<html>
  <head><title>Theorem browser smoke</title></head>
  <body>
    <main>
      <h1>Theorem browser smoke</h1>
      <a href="/substrate">Substrate seam</a>
      <a href="/search?q=substrate">Search the substrate</a>
      <a href="/scene?q=substrate">SceneOS view</a>
    </main>
  </body>
</html>"#;
const AUTOMATION_SMOKE_HTML: &str = r#"<!doctype html>
<html>
  <head><title>Theorem automation smoke</title></head>
  <body>
    <main>
      <h1>Theorem automation smoke</h1>
      <button data-testid="automation-click" onclick="this.dataset.clicked='yes'">Run action</button>
      <label>Search term <input data-testid="automation-input" aria-label="Search term" type="text"></label>
    </main>
  </body>
</html>"#;

#[derive(Clone, Debug, Default)]
struct BrowserStoreOptions {
    data_dir: Option<PathBuf>,
}

impl BrowserStoreOptions {
    fn from_parts(cli_data_dir: Option<PathBuf>, force_memory: bool) -> Self {
        let env_data_dir = std::env::var_os(STORE_DIR_ENV).map(PathBuf::from);
        Self {
            data_dir: if force_memory {
                None
            } else {
                cli_data_dir.or(env_data_dir)
            },
        }
    }

    fn open_session(&self, session_id: &str) -> Result<RedCoreBrowserSessionStore, Box<dyn Error>> {
        match &self.data_dir {
            Some(data_dir) => {
                eprintln!(
                    "theorem-browser: using durable RedCore substrate store at {}",
                    data_dir.display()
                );
                Ok(durable_browser_session(data_dir, session_id)?)
            }
            None => {
                eprintln!("theorem-browser: using ephemeral in-memory substrate store");
                Ok(memory_browser_session(session_id))
            }
        }
    }
}

#[derive(Clone, Debug)]
enum BrowserMode {
    EngineConstructor,
    HeadlessSmoke,
    HeadlessSceneSmoke,
    HeadlessA11ySmoke,
    HeadlessAutomationSmoke,
    Windowed(Url),
}

#[derive(Clone, Debug)]
struct BrowserConfig {
    mode: BrowserMode,
    store: BrowserStoreOptions,
}

/// Minimal event-loop waker.
///
/// `Servo::spin_event_loop` is driven by the embedder; when Servo needs the
/// embedder to pump the loop it calls `wake()`. A headless build-validation run
/// does not pump a real loop, so this is a no-op. Step 2's windowed/headless
/// runtime will wake an actual loop (winit proxy or a condvar).
#[derive(Clone)]
struct HeadlessWaker;

impl EventLoopWaker for HeadlessWaker {
    fn wake(&self) {}

    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }
}

#[derive(Clone)]
struct WindowWaker(EventLoopProxy<WindowWakeEvent>);

#[derive(Debug)]
struct WindowWakeEvent;

impl WindowWaker {
    fn new(event_loop: &EventLoop<WindowWakeEvent>) -> Self {
        Self(event_loop.create_proxy())
    }
}

impl EventLoopWaker for WindowWaker {
    fn wake(&self) {
        let _ = self.0.send_event(WindowWakeEvent);
    }

    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }
}

struct SubstrateSmokeDelegate {
    complete: Cell<bool>,
    ingested: Cell<bool>,
    ingest_on_complete: bool,
    write_count: Cell<usize>,
    graph_delta_hash: RefCell<Option<String>>,
    error: RefCell<Option<String>>,
    session: RefCell<RedCoreBrowserSessionStore>,
}

impl SubstrateSmokeDelegate {
    fn new(store_options: &BrowserStoreOptions) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            complete: Cell::new(false),
            ingested: Cell::new(false),
            ingest_on_complete: true,
            write_count: Cell::new(0),
            graph_delta_hash: RefCell::new(None),
            error: RefCell::new(None),
            session: RefCell::new(store_options.open_session("browser-headless-smoke")?),
        })
    }

    fn new_scene(store_options: &BrowserStoreOptions) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            complete: Cell::new(false),
            ingested: Cell::new(false),
            ingest_on_complete: false,
            write_count: Cell::new(0),
            graph_delta_hash: RefCell::new(None),
            error: RefCell::new(None),
            session: RefCell::new(seed_browser_session(store_options)?),
        })
    }

    fn ingest_completed_page(&self, webview: WebView) {
        if self.ingested.replace(true) {
            return;
        }

        let url = webview
            .url()
            .map(|url| url.to_string())
            .unwrap_or_else(|| SMOKE_URL.to_string());
        let page = LoadedPage::html(url, SMOKE_HTML);
        let mut session = self.session.borrow_mut();
        match session.ingest_loaded_page(page) {
            Ok(receipt) => {
                self.write_count.set(receipt.write_count);
                self.graph_delta_hash
                    .borrow_mut()
                    .replace(receipt.graph_delta_hash);
            }
            Err(error) => {
                self.error.borrow_mut().replace(error.to_string());
            }
        }
    }
}

impl WebViewDelegate for SubstrateSmokeDelegate {
    fn load_web_resource(&self, _webview: WebView, load: WebResourceLoad) {
        intercept_local_theorem_page(load, &self.session);
    }

    fn notify_load_status_changed(&self, webview: WebView, status: LoadStatus) {
        if status == LoadStatus::Complete {
            if self.ingest_on_complete {
                self.ingest_completed_page(webview);
            }
            self.complete.set(true);
        }
    }

    fn notify_new_frame_ready(&self, webview: WebView) {
        webview.paint();
    }
}

/// job-007 D1 live sourcing: a headless delegate that feeds the live Servo
/// accessibility tree into the rustyred-web `AccessibilityReader`. Servo delivers
/// the page's accesskit `TreeUpdate` via `notify_accessibility_tree_update` once
/// the tree is enabled (`WebView::set_accessibility_active(true)` plus
/// `Preferences::accessibility_enabled`). Each update is converted with
/// `A11yTreeUpdate::from_accesskit` and applied to the reader, which reprojects
/// the `PageState` contract from the real engine tree, not intercepted HTML and
/// not CDP. This is the embedder counterpart of the reader and closes the
/// "sourced from the Servo accessibility tree" half of acceptance criterion 1.
struct A11ySmokeDelegate {
    complete: Cell<bool>,
    a11y_update_count: Cell<usize>,
    /// Whether any raw accesskit update carried the page's own content (a node
    /// whose value/label contains the expected heading text). This is the robust
    /// live-sourcing signal: it reads what Servo actually delivered, independent
    /// of how the flat reader assembles it.
    saw_page_content: Cell<bool>,
    reader: RefCell<AccessibilityReader>,
    session: RefCell<RedCoreBrowserSessionStore>,
}

impl A11ySmokeDelegate {
    fn new(store_options: &BrowserStoreOptions) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            complete: Cell::new(false),
            a11y_update_count: Cell::new(0),
            saw_page_content: Cell::new(false),
            reader: RefCell::new(AccessibilityReader::new()),
            session: RefCell::new(store_options.open_session("browser-a11y-smoke")?),
        })
    }
}

impl WebViewDelegate for A11ySmokeDelegate {
    fn load_web_resource(&self, _webview: WebView, load: WebResourceLoad) {
        intercept_local_theorem_page(load, &self.session);
    }

    fn notify_load_status_changed(&self, _webview: WebView, status: LoadStatus) {
        if status == LoadStatus::Complete {
            self.complete.set(true);
        }
    }

    fn notify_new_frame_ready(&self, webview: WebView) {
        webview.paint();
    }

    fn notify_accessibility_tree_update(
        &self,
        webview: WebView,
        tree_update: accesskit::TreeUpdate,
    ) {
        // Robust live-sourcing signal: read the raw accesskit nodes Servo
        // delivered for evidence the page's own content arrived (a text-bearing
        // node carrying the expected heading text). This reads what Servo
        // actually sent, independent of how the flat reader assembles the
        // (multi-tree, grafted) update.
        for (_node_id, node) in &tree_update.nodes {
            let carries_page_text = node
                .value()
                .map(|value| value.contains("Theorem"))
                .unwrap_or(false)
                || node
                    .label()
                    .map(|label| label.contains("Theorem"))
                    .unwrap_or(false);
            if carries_page_text {
                self.saw_page_content.set(true);
            }
        }

        let url = webview.url().map(|url| url.to_string());
        let title = webview.page_title();
        let update = A11yTreeUpdate::from_accesskit(&tree_update, url, title);
        self.reader.borrow_mut().apply_update(update);
        self.a11y_update_count.set(self.a11y_update_count.get() + 1);
    }
}

#[derive(Default)]
struct ServoAutomationEvents {
    input_results: RefCell<Vec<(InputEventId, InputEventResult)>>,
}

impl ServoAutomationEvents {
    fn record_input_result(&self, event_id: InputEventId, result: InputEventResult) {
        self.input_results.borrow_mut().push((event_id, result));
    }

    fn take_input_result(&self, event_id: InputEventId) -> Option<InputEventResult> {
        let mut results = self.input_results.borrow_mut();
        let position = results.iter().position(|(id, _)| *id == event_id)?;
        Some(results.remove(position).1)
    }
}

struct AutomationSmokeDelegate {
    complete: Cell<bool>,
    events: Rc<ServoAutomationEvents>,
}

impl AutomationSmokeDelegate {
    fn new(events: Rc<ServoAutomationEvents>) -> Self {
        Self {
            complete: Cell::new(false),
            events,
        }
    }
}

impl WebViewDelegate for AutomationSmokeDelegate {
    fn load_web_resource(&self, _webview: WebView, load: WebResourceLoad) {
        intercept_local_automation_page(load);
    }

    fn notify_load_status_changed(&self, _webview: WebView, status: LoadStatus) {
        if status == LoadStatus::Complete {
            self.complete.set(true);
        }
    }

    fn notify_new_frame_ready(&self, webview: WebView) {
        webview.paint();
    }

    fn notify_input_event_handled(
        &self,
        _webview: WebView,
        event_id: InputEventId,
        result: InputEventResult,
    ) {
        self.events.record_input_result(event_id, result);
    }
}

struct ServoWebViewAutomationDriver {
    servo: servo::Servo,
    webview: WebView,
    events: Rc<ServoAutomationEvents>,
    origin: AutomationDevicePoint,
}

impl ServoWebViewAutomationDriver {
    fn new(servo: servo::Servo, webview: WebView, events: Rc<ServoAutomationEvents>) -> Self {
        Self {
            servo,
            webview,
            events,
            origin: AutomationDevicePoint::ZERO,
        }
    }

    fn spin_until(
        &self,
        mut done: impl FnMut() -> bool,
        timeout_reason: &str,
    ) -> BrowserEngineResult<()> {
        let deadline = Instant::now() + Duration::from_millis(AUTOMATION_SPIN_TIMEOUT_MS);
        while !done() {
            self.servo.spin_event_loop();
            std::thread::sleep(Duration::from_millis(1));
            if Instant::now() > deadline {
                return Err(BrowserEngineError::UnsupportedAction {
                    reason: timeout_reason.to_string(),
                });
            }
        }
        Ok(())
    }

    fn wait_for_load_complete(&self) -> BrowserEngineResult<()> {
        let webview = self.webview.clone();
        self.spin_until(
            move || webview.load_status() == LoadStatus::Complete,
            "timed out waiting for Servo WebView load before automation",
        )
    }

    fn evaluate_javascript_string(&self, script: impl ToString) -> BrowserEngineResult<String> {
        self.wait_for_load_complete()?;
        let saved_result = Rc::new(RefCell::new(None));
        let callback_result = saved_result.clone();
        self.webview.evaluate_javascript(script, move |result| {
            *callback_result.borrow_mut() = Some(result)
        });

        let spin_result = saved_result.clone();
        self.spin_until(
            move || spin_result.borrow().is_some(),
            "timed out waiting for Servo JavaScript evaluation",
        )?;

        let evaluation = saved_result
            .borrow_mut()
            .take()
            .expect("spin_until waited for a result");
        match evaluation {
            Ok(JSValue::String(value)) => Ok(value),
            Ok(other) => Err(BrowserEngineError::UnsupportedAction {
                reason: format!("geometry script returned non-string JS value: {other:?}"),
            }),
            Err(error) => Err(BrowserEngineError::UnsupportedAction {
                reason: format!("Servo JavaScript evaluation failed: {error:?}"),
            }),
        }
    }

    fn actuate_sync(
        &mut self,
        plan: ActuationPlan,
        _policy: &BrowserActionPolicy,
    ) -> BrowserEngineResult<ActuationReceipt> {
        match &plan.kind {
            ActuationKind::CoordinateSynthesis { point, pointer } => {
                let detail = self.actuate_pointer(*point, *pointer)?;
                Ok(ActuationReceipt {
                    mechanism: "servo_notify_input_event".to_string(),
                    detail,
                })
            }
            ActuationKind::Keyboard { point, text } => {
                let pointer_detail = self.actuate_pointer(*point, PointerKind::Click)?;
                let value_detail = self.set_value_for_handle(&plan.target_handle, text)?;
                Ok(ActuationReceipt {
                    mechanism: "servo_focus_then_value_commit".to_string(),
                    detail: json!({
                        "pointer": pointer_detail,
                        "valueCommit": value_detail,
                    }),
                })
            }
            ActuationKind::Scroll { .. } => {
                let detail = self.scroll_handle_into_view(&plan.target_handle)?;
                Ok(ActuationReceipt {
                    mechanism: "servo_scroll_into_view".to_string(),
                    detail,
                })
            }
            ActuationKind::EmbedderControl { control } => {
                Err(BrowserEngineError::UnsupportedAction {
                    reason: format!("Servo EmbedderControl response is not wired in apps/browser yet: {control:?}"),
                })
            }
            ActuationKind::SemanticActivation { .. } => {
                Err(BrowserEngineError::UnsupportedAction {
                    reason: "semantic activation requires the #4344 Servo fork route".to_string(),
                })
            }
        }
    }

    fn actuate_pointer(
        &self,
        point: AutomationDevicePoint,
        pointer: PointerKind,
    ) -> BrowserEngineResult<Value> {
        let mut events = Vec::new();
        let servo_point = servo_device_point(point);
        match pointer {
            PointerKind::Hover => {
                events.push(
                    self.send_input_event(InputEvent::MouseMove(MouseMoveEvent::new(
                        servo_point.into(),
                    )))?,
                );
            }
            PointerKind::Click => {
                self.push_mouse_click(servo_point, &mut events)?;
            }
            PointerKind::DoubleClick => {
                self.push_mouse_click(servo_point, &mut events)?;
                self.push_mouse_click(servo_point, &mut events)?;
            }
            PointerKind::Tap => {
                events.push(self.send_input_event(InputEvent::Touch(TouchEvent::new(
                    TouchEventType::Down,
                    TouchId(1),
                    servo_point.into(),
                )))?);
                events.push(self.send_input_event(InputEvent::Touch(TouchEvent::new(
                    TouchEventType::Up,
                    TouchId(1),
                    servo_point.into(),
                )))?);
            }
        }
        Ok(json!({
            "pointer": pointer,
            "point": point,
            "events": events,
        }))
    }

    fn push_mouse_click(
        &self,
        point: servo::DevicePoint,
        events: &mut Vec<Value>,
    ) -> BrowserEngineResult<()> {
        events
            .push(self.send_input_event(InputEvent::MouseMove(MouseMoveEvent::new(point.into())))?);
        events.push(
            self.send_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                MouseButtonAction::Down,
                MouseButton::Left,
                point.into(),
            )))?,
        );
        events.push(
            self.send_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                MouseButtonAction::Up,
                MouseButton::Left,
                point.into(),
            )))?,
        );
        Ok(())
    }

    fn send_input_event(&self, event: InputEvent) -> BrowserEngineResult<Value> {
        let event_id = self.webview.notify_input_event(event);
        let result = self.wait_for_input_result(event_id)?;
        Ok(input_event_receipt(event_id, result))
    }

    fn wait_for_input_result(
        &self,
        event_id: InputEventId,
    ) -> BrowserEngineResult<InputEventResult> {
        let saved_result = Rc::new(RefCell::new(None));
        let spin_result = saved_result.clone();
        let events = self.events.clone();
        self.spin_until(
            move || {
                if let Some(result) = events.take_input_result(event_id) {
                    *spin_result.borrow_mut() = Some(result);
                    true
                } else {
                    false
                }
            },
            "timed out waiting for Servo input event receipt",
        )?;
        let result = saved_result
            .borrow_mut()
            .take()
            .expect("spin_until waited for an input result");
        Ok(result)
    }

    fn set_value_for_handle(&self, handle: &str, value: &str) -> BrowserEngineResult<Value> {
        let handle_json = json_string(handle)?;
        let value_json = json_string(value)?;
        let script = format!(
            r#"(function () {{
  var handle = {handle_json};
  var value = {value_json};
  var nodes = document.querySelectorAll("[data-theorem-id]");
  var el = null;
  for (var i = 0; i < nodes.length; i++) {{
    if (nodes[i].getAttribute("data-theorem-id") === handle) {{
      el = nodes[i];
      break;
    }}
  }}
  if (!el) return JSON.stringify({{ ok: false, error: "not_found", handle: handle }});
  el.focus();
  el.value = value;
  el.dispatchEvent(new Event("input", {{ bubbles: true }}));
  el.dispatchEvent(new Event("change", {{ bubbles: true }}));
  return JSON.stringify({{ ok: true, handle: handle, value: el.value }});
}})()"#
        );
        let response = self.evaluate_javascript_string(script)?;
        serde_json::from_str(&response).map_err(|error| BrowserEngineError::UnsupportedAction {
            reason: format!("value commit receipt did not parse: {error}"),
        })
    }

    fn scroll_handle_into_view(&self, handle: &str) -> BrowserEngineResult<Value> {
        let handle_json = json_string(handle)?;
        let script = format!(
            r#"(function () {{
  var handle = {handle_json};
  var nodes = document.querySelectorAll("[data-theorem-id]");
  var el = null;
  for (var i = 0; i < nodes.length; i++) {{
    if (nodes[i].getAttribute("data-theorem-id") === handle) {{
      el = nodes[i];
      break;
    }}
  }}
  if (!el) return JSON.stringify({{ ok: false, error: "not_found", handle: handle }});
  el.scrollIntoView({{ block: "center", inline: "center" }});
  return JSON.stringify({{ ok: true, handle: handle }});
}})()"#
        );
        let response = self.evaluate_javascript_string(script)?;
        serde_json::from_str(&response).map_err(|error| BrowserEngineError::UnsupportedAction {
            reason: format!("scroll receipt did not parse: {error}"),
        })
    }
}

impl BrowserDriver for ServoWebViewAutomationDriver {
    fn snapshot(&self) -> BrowserEngineResult<rustyred_web::PageState> {
        let json = self.evaluate_javascript_string(GEOMETRY_SNAPSHOT_SCRIPT)?;
        let url = self
            .webview
            .url()
            .map(|url| url.to_string())
            .unwrap_or_else(|| "about:blank".to_string());
        page_state_from_snapshot_json(&url, &json)
    }

    fn device_pixels_per_css_pixel(&self) -> f32 {
        self.webview.device_pixels_per_css_pixel().0
    }

    fn webview_origin(&self) -> AutomationDevicePoint {
        self.origin
    }

    async fn actuate(
        &mut self,
        plan: ActuationPlan,
        policy: &BrowserActionPolicy,
    ) -> BrowserEngineResult<ActuationReceipt> {
        self.actuate_sync(plan, policy)
    }
}

fn servo_device_point(point: AutomationDevicePoint) -> servo::DevicePoint {
    servo::DevicePoint::new(point.x, point.y)
}

fn input_event_receipt(event_id: InputEventId, result: InputEventResult) -> Value {
    json!({
        "eventId": serde_json::to_value(event_id).unwrap_or(Value::Null),
        "defaultPrevented": result.contains(InputEventResult::DefaultPrevented),
        "consumed": result.contains(InputEventResult::Consumed),
        "dispatchFailed": result.contains(InputEventResult::DispatchFailed),
    })
}

fn json_string(value: &str) -> BrowserEngineResult<String> {
    serde_json::to_string(value).map_err(|error| BrowserEngineError::UnsupportedAction {
        reason: format!("could not encode JavaScript string literal: {error}"),
    })
}

fn browser_error_box(error: BrowserEngineError) -> Box<dyn Error> {
    format!("{error:?}").into()
}

struct WindowedState {
    window: Window,
    servo: servo::Servo,
    rendering_context: Rc<WindowRenderingContext>,
    webviews: RefCell<Vec<WebView>>,
    session: RefCell<RedCoreBrowserSessionStore>,
}

impl WebViewDelegate for WindowedState {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        self.window.request_redraw();
    }

    fn load_web_resource(&self, _webview: WebView, load: WebResourceLoad) {
        intercept_local_theorem_page(load, &self.session);
    }

    fn notify_load_status_changed(&self, webview: WebView, status: LoadStatus) {
        if status == LoadStatus::Complete {
            if let Some(url) = webview.url() {
                println!("theorem-browser: loaded {url}");
            }
        }
    }
}

enum WindowedApp {
    Initial {
        waker: WindowWaker,
        initial_url: Url,
        store_options: BrowserStoreOptions,
    },
    Running(Rc<WindowedState>),
}

impl WindowedApp {
    fn new(
        event_loop: &EventLoop<WindowWakeEvent>,
        initial_url: Url,
        store_options: BrowserStoreOptions,
    ) -> Self {
        Self::Initial {
            waker: WindowWaker::new(event_loop),
            initial_url,
            store_options,
        }
    }
}

impl ApplicationHandler<WindowWakeEvent> for WindowedApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Self::Initial {
            waker,
            initial_url,
            store_options,
        } = self
        {
            let display_handle = event_loop
                .display_handle()
                .expect("failed to get display handle");
            let window = event_loop
                .create_window(Window::default_attributes().with_title("Theorem Browser"))
                .expect("failed to create theorem browser window");
            let window_handle = window.window_handle().expect("failed to get window handle");

            let rendering_context = Rc::new(
                WindowRenderingContext::new(display_handle, window_handle, window.inner_size())
                    .expect("could not create rendering context for window"),
            );
            let _ = rendering_context.make_current();

            let servo = ServoBuilder::default()
                .event_loop_waker(Box::new(waker.clone()))
                .build();
            servo.setup_logging();

            let state = Rc::new(WindowedState {
                window,
                servo,
                rendering_context,
                webviews: RefCell::new(Vec::new()),
                session: RefCell::new(
                    seed_browser_session(store_options)
                        .expect("failed to open browser substrate session"),
                ),
            });

            let webview = WebViewBuilder::new(&state.servo, state.rendering_context.clone())
                .url(initial_url.clone())
                .hidpi_scale_factor(Scale::new(state.window.scale_factor() as f32))
                .delegate(state.clone())
                .build();

            state.webviews.borrow_mut().push(webview);
            *self = Self::Running(state);
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: WindowWakeEvent) {
        if let Self::Running(state) = self {
            state.servo.spin_event_loop();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if let Self::Running(state) = self {
            state.servo.spin_event_loop();
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                if let Self::Running(state) = self {
                    if let Some(webview) = state.webviews.borrow().last() {
                        webview.paint();
                        state.rendering_context.present();
                    }
                }
            }
            WindowEvent::Resized(new_size) => {
                if let Self::Running(state) = self {
                    if let Some(webview) = state.webviews.borrow().last() {
                        webview.resize(new_size);
                    }
                }
            }
            _ => {}
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    install_rustls_crypto_provider();

    let config = parse_config(std::env::args().skip(1))?;
    match config.mode {
        BrowserMode::HeadlessSmoke => run_headless_smoke(&config.store),
        BrowserMode::HeadlessSceneSmoke => run_headless_scene_smoke(&config.store),
        BrowserMode::HeadlessA11ySmoke => run_headless_a11y_smoke(&config.store),
        BrowserMode::HeadlessAutomationSmoke => run_headless_automation_smoke(),
        BrowserMode::Windowed(url) => run_windowed(url, config.store),
        BrowserMode::EngineConstructor => {
            run_engine_constructor(&config.store);
            Ok(())
        }
    }
}

fn install_rustls_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn parse_config(args: impl IntoIterator<Item = String>) -> Result<BrowserConfig, Box<dyn Error>> {
    let mut cli_data_dir = None;
    let mut force_memory = false;
    let mut mode_args = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--store-dir" => {
                let value = args.next().ok_or("--store-dir requires a path")?;
                cli_data_dir = Some(PathBuf::from(value));
            }
            "--memory-store" => {
                force_memory = true;
            }
            _ => mode_args.push(arg),
        }
    }

    let mode = match mode_args.first().map(String::as_str) {
        Some("--headless-smoke") => BrowserMode::HeadlessSmoke,
        Some("--headless-scene-smoke") => BrowserMode::HeadlessSceneSmoke,
        Some("--headless-a11y-smoke") => BrowserMode::HeadlessA11ySmoke,
        Some("--headless-automation-smoke") => BrowserMode::HeadlessAutomationSmoke,
        Some("--windowed") => {
            let url = mode_args
                .get(1)
                .cloned()
                .unwrap_or_else(|| SMOKE_URL.to_string());
            BrowserMode::Windowed(Url::parse(&url)?)
        }
        Some(other) => {
            return Err(format!("unknown theorem-browser mode or argument: {other}").into());
        }
        None => BrowserMode::EngineConstructor,
    };

    Ok(BrowserConfig {
        mode,
        store: BrowserStoreOptions::from_parts(cli_data_dir, force_memory),
    })
}

fn run_engine_constructor(store_options: &BrowserStoreOptions) {
    // Construct the engine with defaults (Opts/Preferences default; only the
    // waker is required). Proves the git-dep builds and the wiring compiles.
    let _servo = ServoBuilder::default()
        .event_loop_waker(Box::new(HeadlessWaker))
        .build();

    println!("theorem-browser: Servo engine constructed (build validation OK)");
    if let Some(data_dir) = &store_options.data_dir {
        println!(
            "theorem-browser: durable substrate store configured at {}",
            data_dir.display()
        );
    } else {
        println!("theorem-browser: substrate store mode is memory");
    }
    print_browser_affordances();
}

fn software_rendering_context() -> Result<Rc<dyn RenderingContext>, Box<dyn Error>> {
    let rendering_context: Rc<dyn RenderingContext> = Rc::new(
        SoftwareRenderingContext::new(PhysicalSize {
            width: 800,
            height: 600,
        })
        .map_err(|error| format!("could not create software rendering context: {error:?}"))?,
    );
    rendering_context
        .make_current()
        .map_err(|error| format!("could not make rendering context current: {error:?}"))?;
    Ok(rendering_context)
}

fn run_headless_smoke(store_options: &BrowserStoreOptions) -> Result<(), Box<dyn Error>> {
    eprintln!("theorem-browser: starting headless WebView substrate smoke");
    let rendering_context = software_rendering_context()?;

    let servo = ServoBuilder::default()
        .event_loop_waker(Box::new(HeadlessWaker))
        .build();
    servo.setup_logging();

    let delegate = Rc::new(SubstrateSmokeDelegate::new(store_options)?);
    let _webview = WebViewBuilder::new(&servo, rendering_context)
        .url(Url::parse(SMOKE_URL)?)
        .hidpi_scale_factor(Scale::new(1.0))
        .delegate(delegate.clone())
        .build();

    eprintln!("theorem-browser: WebView created; spinning Servo until load complete");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !delegate.complete.get() {
        servo.spin_event_loop();
        std::thread::sleep(Duration::from_millis(5));

        if Instant::now() > deadline {
            return Err("timed out waiting for Servo WebView smoke load".into());
        }
    }

    if let Some(error) = delegate.error.borrow().as_ref() {
        return Err(format!("substrate ingest failed: {error}").into());
    }

    let graph_delta_hash = delegate
        .graph_delta_hash
        .borrow()
        .clone()
        .ok_or("substrate ingest completed without a graph delta hash")?;

    println!(
        "theorem-browser: headless WebView smoke OK; writes={}, graph_delta_hash={}",
        delegate.write_count.get(),
        graph_delta_hash
    );
    Ok(())
}

fn run_headless_scene_smoke(store_options: &BrowserStoreOptions) -> Result<(), Box<dyn Error>> {
    eprintln!("theorem-browser: starting headless SceneOS WebView smoke");
    let rendering_context = software_rendering_context()?;

    let servo = ServoBuilder::default()
        .event_loop_waker(Box::new(HeadlessWaker))
        .build();
    servo.setup_logging();

    let delegate = Rc::new(SubstrateSmokeDelegate::new_scene(store_options)?);
    let _webview = WebViewBuilder::new(&servo, rendering_context)
        .url(Url::parse(SCENE_SMOKE_URL)?)
        .hidpi_scale_factor(Scale::new(1.0))
        .delegate(delegate.clone())
        .build();

    eprintln!("theorem-browser: SceneOS WebView created; spinning Servo until load complete");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !delegate.complete.get() {
        servo.spin_event_loop();
        std::thread::sleep(Duration::from_millis(5));

        if Instant::now() > deadline {
            return Err("timed out waiting for Servo WebView scene smoke load".into());
        }
    }

    if let Some(error) = delegate.error.borrow().as_ref() {
        return Err(format!("SceneOS smoke failed: {error}").into());
    }

    println!("theorem-browser: headless SceneOS WebView smoke OK; url={SCENE_SMOKE_URL}");
    Ok(())
}

fn run_headless_a11y_smoke(store_options: &BrowserStoreOptions) -> Result<(), Box<dyn Error>> {
    eprintln!("theorem-browser: starting headless accessibility-tree substrate smoke");
    let rendering_context = software_rendering_context()?;

    // Accessibility must be enabled at the engine level for Servo to build and
    // emit the a11y tree; the per-WebView toggle below activates delivery.
    let mut preferences = Preferences::default();
    preferences.accessibility_enabled = true;

    let servo = ServoBuilder::default()
        .event_loop_waker(Box::new(HeadlessWaker))
        .preferences(preferences)
        .build();
    servo.setup_logging();

    let delegate = Rc::new(A11ySmokeDelegate::new(store_options)?);
    let webview = WebViewBuilder::new(&servo, rendering_context)
        .url(Url::parse(SMOKE_URL)?)
        .hidpi_scale_factor(Scale::new(1.0))
        .delegate(delegate.clone())
        .build();

    // Turn on the accessibility tree for this WebView; Servo then delivers
    // accesskit TreeUpdates to A11ySmokeDelegate::notify_accessibility_tree_update.
    webview.set_accessibility_active(true);

    eprintln!(
        "theorem-browser: WebView created with accessibility active; spinning until load complete and the a11y tree arrives"
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    while !delegate.complete.get() || !delegate.saw_page_content.get() {
        servo.spin_event_loop();
        std::thread::sleep(Duration::from_millis(5));

        if Instant::now() > deadline {
            return Err(format!(
                "timed out waiting for live a11y page content; complete={}, a11y_updates={}, saw_page_content={}",
                delegate.complete.get(),
                delegate.a11y_update_count.get(),
                delegate.saw_page_content.get()
            )
            .into());
        }
    }

    let page = delegate.reader.borrow().page_state();
    let live_nodes = delegate.reader.borrow().live_node_count();
    let text_preview: String = page.distilled_text.chars().take(200).collect();

    println!(
        "theorem-browser: headless a11y smoke OK; a11y_updates={}, saw_page_content={}, live_nodes={}, interactive_elements={}, title={:?}",
        delegate.a11y_update_count.get(),
        delegate.saw_page_content.get(),
        live_nodes,
        page.interactive_elements.len(),
        page.title
    );
    println!("theorem-browser: a11y-sourced distilled_text[0..200]={text_preview:?}");

    // The robust live-sourcing proof is saw_page_content (guaranteed true by the
    // loop exit): Servo delivered the page's own accessibility content to the
    // embedder, and A11yTreeUpdate::from_accesskit + the reader ran on the real
    // accesskit tree without panicking. The flat reader's distilled_text /
    // interactive_elements are printed for evidence but NOT asserted: Servo sends
    // a multi-tree grafted update (a WebView ScrollView tree plus the grafted
    // document subtree, each with an independent NodeId space), which the flat
    // reader does not yet assemble; tree_id/graft-aware assembly and
    // interactive-role rolling (Link/Button/Input) are the named follow-ups.
    // Sanity check that the reader held live nodes from the real tree:
    if live_nodes == 0 {
        return Err("accessibility reader produced no live nodes from the Servo tree".into());
    }

    Ok(())
}

fn run_headless_automation_smoke() -> Result<(), Box<dyn Error>> {
    eprintln!("theorem-browser: starting headless automation-driver smoke");
    let rendering_context = software_rendering_context()?;

    let servo = ServoBuilder::default()
        .event_loop_waker(Box::new(HeadlessWaker))
        .build();
    servo.setup_logging();

    let events = Rc::new(ServoAutomationEvents::default());
    let delegate = Rc::new(AutomationSmokeDelegate::new(events.clone()));
    let webview = WebViewBuilder::new(&servo, rendering_context)
        .url(Url::parse(AUTOMATION_SMOKE_URL)?)
        .hidpi_scale_factor(Scale::new(1.0))
        .delegate(delegate.clone())
        .build();

    let deadline = Instant::now() + Duration::from_secs(60);
    while !delegate.complete.get() {
        servo.spin_event_loop();
        std::thread::sleep(Duration::from_millis(5));

        if Instant::now() > deadline {
            return Err("timed out waiting for Servo automation smoke load".into());
        }
    }

    let mut driver = ServoWebViewAutomationDriver::new(servo.clone(), webview, events);
    let page = driver.snapshot().map_err(browser_error_box)?;
    if page.interactive_elements.len() < 2 {
        return Err(format!(
            "automation snapshot produced too few interactive elements: {}",
            page.interactive_elements.len()
        )
        .into());
    }

    let button_locator = Locator::get_by_test_id("automation-click");
    let button = button_locator
        .resolve(&page)
        .into_iter()
        .next()
        .ok_or("automation smoke button did not resolve by test id")?;
    let click_plan =
        build_actuation_plan(&driver, &button, &LocatorAction::Click).map_err(browser_error_box)?;
    let click_receipt = driver
        .actuate_sync(click_plan, &BrowserActionPolicy::default())
        .map_err(browser_error_box)?;
    let clicked = driver
        .evaluate_javascript_string(
            "document.querySelector('[data-testid=\"automation-click\"]').dataset.clicked || ''",
        )
        .map_err(browser_error_box)?;
    if clicked != "yes" {
        return Err(
            format!("automation smoke click did not reach page handler: {clicked:?}").into(),
        );
    }

    let input_locator = Locator::get_by_role(
        "text",
        RoleOptions {
            name: Some("Search term".to_string()),
        },
    );
    let input = input_locator
        .resolve(&driver.snapshot().map_err(browser_error_box)?)
        .into_iter()
        .next()
        .ok_or("automation smoke input did not resolve by role")?;
    let fill_plan = build_actuation_plan(
        &driver,
        &input,
        &LocatorAction::Fill {
            value: "servo".to_string(),
        },
    )
    .map_err(browser_error_box)?;
    let fill_receipt = driver
        .actuate_sync(fill_plan, &BrowserActionPolicy::default())
        .map_err(browser_error_box)?;

    let post_fill_page = driver.snapshot().map_err(browser_error_box)?;
    let filled_value = input_locator
        .resolve(&post_fill_page)
        .into_iter()
        .next()
        .and_then(|handle| handle.value);
    if filled_value.as_deref() != Some("servo") {
        return Err(format!(
            "automation smoke fill did not round-trip through snapshot: {filled_value:?}"
        )
        .into());
    }

    println!(
        "theorem-browser: headless automation smoke OK; url={}, interactive_elements={}, click_mechanism={}, fill_mechanism={}",
        page.url,
        page.interactive_elements.len(),
        click_receipt.mechanism,
        fill_receipt.mechanism
    );
    Ok(())
}

fn run_windowed(
    initial_url: Url,
    store_options: BrowserStoreOptions,
) -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::with_user_event().build()?;
    let mut app = WindowedApp::new(&event_loop, initial_url, store_options);
    Ok(event_loop.run_app(&mut app)?)
}

fn print_browser_affordances() {
    for affordance in browser_affordances() {
        println!(
            "theorem-browser affordance: {} [{}] - {}",
            affordance.id, affordance.provider, affordance.label
        );
    }
}

fn intercept_local_theorem_page(
    load: WebResourceLoad,
    session: &RefCell<RedCoreBrowserSessionStore>,
) {
    let url = load.request().url.clone();
    let (body, status, content_type) = if url.as_str() == SMOKE_URL {
        (
            SMOKE_HTML.as_bytes().to_vec(),
            StatusCode::OK,
            "text/html; charset=utf-8".to_string(),
        )
    } else if url.as_str().starts_with(SEARCH_URL_PREFIX) {
        let query = url
            .query_pairs()
            .find(|(key, _)| key == "q")
            .map(|(_, value)| value.to_string())
            .unwrap_or_default();
        let gate_config = browser_search_gate_config_from_url(&url);
        let body = match session.borrow_mut().search_or_crawl_blocking(
            &query,
            &browser_open_web_options(),
            &gate_config,
        ) {
            Ok(result) => render_substrate_search_result_page(&result.final_search),
            Err(error) => {
                eprintln!("theorem-browser: search crawl failed for {url}: {error}");
                open_web_error_page(&url, &error.to_string())
            }
        };
        (
            body.into_bytes(),
            StatusCode::OK,
            "text/html; charset=utf-8".to_string(),
        )
    } else if url.as_str().starts_with(SCENE_URL_PREFIX) {
        let query = scene_query_from_url(&url);
        let body = match render_browser_scene_page(&session.borrow(), &query) {
            Ok(body) => body,
            Err(error) => {
                eprintln!("theorem-browser: SceneOS route failed: {error}");
                scene_os_web::render_scene_html("null")
            }
        };
        (
            body.into_bytes(),
            StatusCode::OK,
            "text/html; charset=utf-8".to_string(),
        )
    } else if is_open_web_url(&url) {
        match session
            .borrow_mut()
            .fetch_and_ingest_open_web_page_blocking(url.as_str(), &browser_open_web_options())
        {
            Ok((page, receipt)) => {
                eprintln!(
                    "theorem-browser: fetched {} into substrate delta {}",
                    page.url, receipt.graph_delta_hash
                );
                (
                    page.body.into_bytes(),
                    status_code_from_u16(page.status),
                    page.content_type,
                )
            }
            Err(error) => {
                eprintln!("theorem-browser: open-web fetch failed for {url}: {error}");
                (
                    open_web_error_page(&url, &error.to_string()).into_bytes(),
                    StatusCode::BAD_GATEWAY,
                    "text/html; charset=utf-8".to_string(),
                )
            }
        }
    } else {
        return;
    };

    let mut headers = http::HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );

    let response = WebResourceResponse::new(load.request().url.clone())
        .headers(headers)
        .status_code(status)
        .status_message(
            status
                .canonical_reason()
                .unwrap_or_default()
                .as_bytes()
                .to_vec(),
        );
    let mut intercepted = load.intercept(response);
    intercepted.send_body_data(body);
    intercepted.finish();
}

fn intercept_local_automation_page(load: WebResourceLoad) {
    let url = load.request().url.clone();
    if url.as_str() != AUTOMATION_SMOKE_URL {
        return;
    }

    let mut headers = http::HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    let response = WebResourceResponse::new(load.request().url.clone())
        .headers(headers)
        .status_code(StatusCode::OK)
        .status_message(b"OK".to_vec());
    let mut intercepted = load.intercept(response);
    intercepted.send_body_data(AUTOMATION_SMOKE_HTML.as_bytes().to_vec());
    intercepted.finish();
}

fn is_open_web_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https") && url.host_str() != Some("theorem.local")
}

fn browser_open_web_options() -> LiveFetchOptions {
    LiveFetchOptions {
        user_agent: "Theorem Browser/RustyWeb".to_string(),
        timeout_seconds: 10,
        guard_policy: UrlGuardPolicy::default(),
        respect_robots: true,
        allow_impersonate: false,
        rendered_endpoint: None,
    }
}

fn browser_search_gate_config_from_url(url: &Url) -> TriggerGateConfig {
    let crawl_mode = url
        .query_pairs()
        .find(|(key, _)| key == "mode" || key == "crawl")
        .map(|(_, value)| value.to_string())
        .unwrap_or_default();
    if crawl_mode.eq_ignore_ascii_case("broad") {
        TriggerGateConfig::broad()
    } else {
        TriggerGateConfig::conservative()
    }
}

fn status_code_from_u16(status: u16) -> StatusCode {
    StatusCode::from_u16(status).unwrap_or(StatusCode::OK)
}

fn open_web_error_page(url: &Url, error: &str) -> String {
    format!(
        "<!doctype html><html><head><title>Theorem fetch failed</title></head><body><main><h1>Theorem fetch failed</h1><p>{}</p><pre>{}</pre></main></body></html>",
        escape_html_text(url.as_str()),
        escape_html_text(error)
    )
}

fn escape_html_text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn scene_query_from_url(url: &Url) -> String {
    url.query_pairs()
        .find(|(key, _)| key == "q" || key == "query")
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_SCENE_QUERY.to_string())
}

fn render_browser_scene_page(
    session: &RedCoreBrowserSessionStore,
    query: &str,
) -> Result<String, String> {
    let search = session.search_substrate(query);
    let scene = scene_from_substrate_search(&search);
    let compile_query = if query.trim().is_empty() {
        DEFAULT_SCENE_QUERY
    } else {
        query
    };
    let package = compile_scene_package(SceneCompileInput {
        query: compile_query.to_string(),
        answer_type: Some("tree_hierarchy".to_string()),
        title: Some(format!("Substrate scene: {compile_query}")),
        scene,
        trace_id: Some(scene_trace_id(compile_query)),
        manifest_ref: Some(format!("browser-scene:{compile_query}")),
        provenance: BTreeMap::from([
            ("route".to_string(), json!("theorem.local/scene")),
            ("query".to_string(), json!(search.query)),
            ("matchedCount".to_string(), json!(search.matched_count)),
            ("keptCount".to_string(), json!(search.kept_count)),
        ]),
    })
    .map_err(|error| error.to_string())?;
    render_scene(&package).map_err(|error| error.to_string())
}

fn scene_from_substrate_search(search: &SubstrateSearch) -> SceneScene {
    let atoms = search
        .hits
        .iter()
        .map(|hit| {
            let label = if hit.title.trim().is_empty() {
                hit.url
                    .trim()
                    .strip_prefix("http://")
                    .or_else(|| hit.url.trim().strip_prefix("https://"))
                    .unwrap_or(&hit.node_id)
                    .to_string()
            } else {
                hit.title.clone()
            };
            SceneAtom {
                id: hit.node_id.clone(),
                kind: if hit.ring == 0 { "claim" } else { "concept" }.to_string(),
                label: Some(label.clone()),
                position: None,
                weight: Some(hit.match_score.max(1.0) + 1.0 / (hit.ring + 1) as f64),
                color: None,
                opacity: None,
                glyph: None,
                scale: None,
                lifecycle: AtomLifecycle::Present,
                metadata: BTreeMap::from([
                    ("url".to_string(), json!(hit.url)),
                    ("snippet".to_string(), json!(hit.snippet)),
                    ("ring".to_string(), json!(hit.ring)),
                    ("ringLabel".to_string(), json!(hit.ring_label)),
                    ("matchScore".to_string(), json!(hit.match_score)),
                ]),
                source_refs: vec![SourceRef {
                    kind: "Page".to_string(),
                    id: hit.node_id.clone(),
                    label: Some(label),
                    metadata: BTreeMap::from([("url".to_string(), json!(hit.url))]),
                }],
            }
        })
        .collect();

    let relations = search
        .links
        .iter()
        .map(|link| SceneRelation {
            id: format!("{}->{}:links_to", link.source, link.target),
            source_id: link.source.clone(),
            target_id: link.target.clone(),
            kind: "links_to".to_string(),
            weight: Some(1.0),
            color: None,
            opacity: None,
            glyph: None,
            lifecycle: AtomLifecycle::Present,
            metadata: BTreeMap::new(),
            source_refs: Vec::new(),
        })
        .collect();

    SceneScene { atoms, relations }
}

fn scene_trace_id(query: &str) -> String {
    let mut out = String::from("browser-scene");
    let mut pending_dash = true;

    for ch in query.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash {
                out.push('-');
                pending_dash = false;
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }

    if out == "browser-scene" {
        out.push_str("-substrate");
    }

    out
}

fn seed_browser_session(
    store_options: &BrowserStoreOptions,
) -> Result<RedCoreBrowserSessionStore, Box<dyn Error>> {
    let mut session = store_options.open_session("browser-seed")?;
    session.ingest_loaded_page(LoadedPage::html(SMOKE_URL, SMOKE_HTML))?;
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded_session() -> RedCoreBrowserSessionStore {
        let mut session = memory_browser_session("scene-test");
        session
            .ingest_loaded_page(LoadedPage::html(SMOKE_URL, SMOKE_HTML))
            .expect("seed should write to substrate");
        session
    }

    #[test]
    fn scene_url_query_defaults_to_substrate() {
        let url = Url::parse("http://theorem.local/scene").unwrap();
        assert_eq!(scene_query_from_url(&url), DEFAULT_SCENE_QUERY);

        let url = Url::parse("http://theorem.local/scene?query=browser").unwrap();
        assert_eq!(scene_query_from_url(&url), "browser");
    }

    #[test]
    fn browser_scene_page_uses_lane_a_package_and_lane_b_renderer() {
        let session = seeded_session();
        let html = render_browser_scene_page(&session, "substrate").expect("render scene");

        assert!(html.contains("window.__SCENE_PACKAGE__ = {"));
        assert!(html.contains("\"projection\":{\"id\":\"tree_hierarchy\""));
        assert!(html.contains("\"chrome\":{\"id\":\"document_rail\""));
        assert!(html.contains("\"route\":\"theorem.local/scene\""));
        assert!(html.contains("SceneOS"));
    }

    #[test]
    fn search_result_maps_to_scene_atoms_and_relations() {
        let session = seeded_session();
        let search = session.search_substrate("substrate");
        let scene = scene_from_substrate_search(&search);

        assert!(!scene.atoms.is_empty());
        assert!(scene.atoms.iter().any(|atom| atom.kind == "claim"));
        assert!(scene
            .atoms
            .iter()
            .all(|atom| atom.lifecycle == AtomLifecycle::Present));
        assert!(scene
            .atoms
            .iter()
            .all(|atom| atom.metadata.contains_key("url")));
    }

    #[test]
    fn scene_trace_id_slug_collapses_separators() {
        assert_eq!(
            scene_trace_id("Substrate browser scene"),
            "browser-scene-substrate-browser-scene"
        );
        assert_eq!(scene_trace_id("???"), "browser-scene-substrate");
    }
}
