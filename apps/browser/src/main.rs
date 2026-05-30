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
use std::error::Error;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use dpi::PhysicalSize;
use embedder_traits::WebResourceResponse;
use euclid::Scale;
use http::header::{HeaderValue, CONTENT_TYPE};
use servo::{
    EventLoopWaker, LoadStatus, RenderingContext, ServoBuilder, SoftwareRenderingContext,
    WebResourceLoad, WebView, WebViewBuilder, WebViewDelegate, WindowRenderingContext,
};
use theorem_browser_substrate::{
    browser_affordances, durable_browser_session, memory_browser_session, LoadedPage,
    RedCoreBrowserSessionStore,
};
use url::Url;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;

const SMOKE_URL: &str = "http://theorem.local/smoke";
const SEARCH_URL_PREFIX: &str = "http://theorem.local/search";
const STORE_DIR_ENV: &str = "THEOREM_BROWSER_STORE_DIR";
const SMOKE_HTML: &str = r#"<!doctype html>
<html>
  <head><title>Theorem browser smoke</title></head>
  <body>
    <main>
      <h1>Theorem browser smoke</h1>
      <a href="/substrate">Substrate seam</a>
      <a href="/search?q=substrate">Search the substrate</a>
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
            write_count: Cell::new(0),
            graph_delta_hash: RefCell::new(None),
            error: RefCell::new(None),
            session: RefCell::new(store_options.open_session("browser-headless-smoke")?),
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
            self.ingest_completed_page(webview);
            self.complete.set(true);
        }
    }

    fn notify_new_frame_ready(&self, webview: WebView) {
        webview.paint();
    }
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

fn run_headless_smoke(store_options: &BrowserStoreOptions) -> Result<(), Box<dyn Error>> {
    eprintln!("theorem-browser: starting headless WebView substrate smoke");
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
    let body = if url.as_str() == SMOKE_URL {
        SMOKE_HTML.to_string()
    } else if url.as_str().starts_with(SEARCH_URL_PREFIX) {
        let query = url
            .query_pairs()
            .find(|(key, _)| key == "q")
            .map(|(_, value)| value.to_string())
            .unwrap_or_default();
        session.borrow().render_search_page(&query)
    } else {
        return;
    };

    let mut headers = http::HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );

    let response = WebResourceResponse::new(load.request().url.clone()).headers(headers);
    let mut intercepted = load.intercept(response);
    intercepted.send_body_data(body.into_bytes());
    intercepted.finish();
}

fn seed_browser_session(
    store_options: &BrowserStoreOptions,
) -> Result<RedCoreBrowserSessionStore, Box<dyn Error>> {
    let mut session = store_options.open_session("browser-seed")?;
    let _ = session.ingest_loaded_page(LoadedPage::html(SMOKE_URL, SMOKE_HTML));
    Ok(session)
}
