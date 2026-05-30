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
use std::rc::Rc;
use std::time::{Duration, Instant};

use dpi::PhysicalSize;
use embedder_traits::WebResourceResponse;
use euclid::Scale;
use http::header::{HeaderValue, CONTENT_TYPE};
use rustyred_thg_core::graph_store::InMemoryGraphStore;
use servo::{
    EventLoopWaker, LoadStatus, RenderingContext, ServoBuilder, SoftwareRenderingContext,
    WebResourceLoad, WebView, WebViewBuilder, WebViewDelegate,
};
use theorem_browser_substrate::{ingest_loaded_pages, LoadedPage};
use url::Url;

const SMOKE_URL: &str = "http://theorem.local/smoke";
const SMOKE_HTML: &str = r#"<!doctype html>
<html>
  <head><title>Theorem browser smoke</title></head>
  <body>
    <main>
      <h1>Theorem browser smoke</h1>
      <a href="/substrate">Substrate seam</a>
    </main>
  </body>
</html>"#;

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

struct SubstrateSmokeDelegate {
    complete: Cell<bool>,
    ingested: Cell<bool>,
    write_count: Cell<usize>,
    graph_delta_hash: RefCell<Option<String>>,
    error: RefCell<Option<String>>,
    store: RefCell<InMemoryGraphStore>,
}

impl SubstrateSmokeDelegate {
    fn new() -> Self {
        Self {
            complete: Cell::new(false),
            ingested: Cell::new(false),
            write_count: Cell::new(0),
            graph_delta_hash: RefCell::new(None),
            error: RefCell::new(None),
            store: RefCell::new(InMemoryGraphStore::new()),
        }
    }

    fn ingest_completed_page(&self, webview: WebView) {
        if self.ingested.replace(true) {
            return;
        }

        let url = webview
            .url()
            .map(|url| url.to_string())
            .unwrap_or_else(|| SMOKE_URL.to_string());
        let page = LoadedPage::html(url.clone(), SMOKE_HTML);
        let mut store = self.store.borrow_mut();
        match ingest_loaded_pages(&mut *store, "browser-headless-smoke", vec![url], &[page]) {
            Ok((output, writes)) => {
                self.write_count.set(writes.len());
                self.graph_delta_hash
                    .borrow_mut()
                    .replace(output.receipt.graph_delta_hash);
            }
            Err(error) => {
                self.error.borrow_mut().replace(error.to_string());
            }
        }
    }
}

impl WebViewDelegate for SubstrateSmokeDelegate {
    fn load_web_resource(&self, _webview: WebView, load: WebResourceLoad) {
        if load.request().url.as_str() != SMOKE_URL {
            return;
        }

        let mut headers = http::HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );

        let response = WebResourceResponse::new(load.request().url.clone()).headers(headers);
        let mut intercepted = load.intercept(response);
        intercepted.send_body_data(SMOKE_HTML.as_bytes().to_vec());
        intercepted.finish();
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

fn main() -> Result<(), Box<dyn Error>> {
    if std::env::args().any(|arg| arg == "--headless-smoke") {
        return run_headless_smoke();
    }

    run_engine_constructor();
    Ok(())
}

fn run_engine_constructor() {
    // Construct the engine with defaults (Opts/Preferences default; only the
    // waker is required). Proves the git-dep builds and the wiring compiles.
    let _servo = ServoBuilder::default()
        .event_loop_waker(Box::new(HeadlessWaker))
        .build();

    println!("theorem-browser: Servo engine constructed (build validation OK)");
}

fn run_headless_smoke() -> Result<(), Box<dyn Error>> {
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

    let delegate = Rc::new(SubstrateSmokeDelegate::new());
    let _webview = WebViewBuilder::new(&servo, rendering_context)
        .url(Url::parse(SMOKE_URL)?)
        .hidpi_scale_factor(Scale::new(1.0))
        .delegate(delegate.clone())
        .build();

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
