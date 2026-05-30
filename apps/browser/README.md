# Theorem Browser (Servo-embedded substrate browser)

**Status:** External embedder build is green in GitHub Actions at `713eded` (run `26669852900`, 2026-05-30). Grounded against the pinned Servo embedding API (`servo::WebViewBuilder`, `WebViewDelegate`, `Servo::spin_event_loop`, `SoftwareRenderingContext`). This crate is the start of the substrate-native browser: the surface where Servo renders both the open web and SceneOS scenes, in-process with the RustyRed substrate.

**Honest state:** the external `cargo build` path has compiled successfully in CI. The checked smoke increment is `cargo run -- --headless-smoke`: create a real WebView with a software rendering context, intercept a known URL through `WebViewDelegate::load_web_resource`, and write that supplied page into `theorem-browser-substrate`. The next scene is `cargo run -- --windowed [url]`, a minimal desktop winit shell that opens a real Servo WebView. The browser now also serves `http://theorem.local/search?q=...` as a graph-native RustyWeb SERP from its browser session substrate. By default that session is memory-backed; pass `--store-dir <path>` or set `THEOREM_BROWSER_STORE_DIR=<path>` to make it a durable RedCore store. This proves the visible browser shell and local search surface without claiming full arbitrary-page response capture yet.

---

## Why Servo, why now

Servo IS the browser: the UI renders in it; there is no front-end without a rendering engine. It is also the longest-lead-time dependency in the browser arc, so starting it early de-risks the long pole (the reason to begin here rather than wait for RustyWeb/SceneOS). The kernel + tools are already resident in this repo (`rustyredcore_THG/` + `apps/orchestrate`), so the browser is the next real surface.

## The current Servo embedding API (grounded)

- **`Servo`** is the engine instance. The embedder constructs it with a rendering context, an `EventLoopWaker`, `Opts`, and `Preferences` (servoshell `App::new` is the reference embedder).
- **`WebViewBuilder::new(&servo).delegate(Rc<dyn WebViewDelegate>).url(Url).size(PhysicalSize).hidpi_scale_factor(Scale).build() -> WebView`** creates a webview.
- **`ServoDelegate`** is the engine-level embedder callback trait: `notify_error`, `notify_animating_changed`, `load_web_resource`, `show_notification`, DevTools hooks.
- **`Servo::spin_event_loop`** drives a frame. The contract: when `notify_animating_changed(true)` fires, the embedder must call `spin_event_loop` at regular intervals to keep painting. `RefreshDriver` is the trait for vsync-driven embedders.
- The embedder owns the winit window + event loop and forwards events to Servo (the servoshell `handle_events_with_winit` pattern).

## The two substrate seams (what makes this Theorem's browser, not just an embedder)

1. **DOM-as-substrate-write: `WebViewDelegate::load_web_resource(&self, webview, WebResourceLoad)`.** Servo calls this when a `WebView` is about to load an HTTP/HTTPS resource; the embedder may `WebResourceLoad::intercept(...)` to supply alternate contents, or let it continue. (Verified against doc.servo.org 2026-05-29: `ServoDelegate::load_web_resource` fires only for loads NOT associated with a `WebView`; page navigations are WebView-associated, so the page seam lives on `WebViewDelegate`, and `WebViewDelegate::notify_load_status_changed(LoadStatus::Complete)` is the "page finished" signal that triggers the substrate write.) This is the hook where a loaded page becomes graph state in the RustyRed substrate, and where substrate-resident content (a generated SceneOS scene) can be served in place of a network load. In-process with RustyRed, no API boundary. The page->graph logic itself is the `theorem-browser-substrate` crate (`ingest_loaded_pages`), already built and unit-tested without Servo.
2. **Scene compositing.** SceneOS scenes (generated atoms placed by D3-backed projections) compose into the same Servo surface as web pages. A generated scene is served as a substrate-resident resource through the same `load_web_resource` interception, or painted as an overlay webview.

## Fork vs embed (recommendation)

The vault framing was "fork Servo." The grounded recommendation:

- **Embed the `servo` crate now** (this crate depends on Servo as a git dependency, pinned) to get a rendering shell with zero engine modification. This is the fastest path to a browser that renders the open web + hosts scenes via `load_web_resource` interception. The two substrate seams above are achievable through the public embedder API (delegates + interception) without touching engine internals.
- **Fork the engine later, only when** the substrate integration needs modifications the embedder API cannot express (e.g. writing the live DOM tree into the substrate at parse time rather than at resource-load time, or compositor-level scene fusion). At that point, fork `servo/servo`, pin this crate to the fork, and carry the engine patches as a maintained delta against upstream.

Forking a 500k-LOC engine before the embedder API proves insufficient buys maintenance cost without capability. Start embedded; fork on evidence (same discipline as the rest of this architecture).

## Build prerequisites (the heavy part)

Servo is not a plain crate dependency. Building it requires the Servo build toolchain and system libraries. Before this crate compiles:
- Install the Servo build deps for the platform (the `./mach bootstrap` set: a C/C++ toolchain, Python, and the platform GL/font/media libraries). On macOS this is the Homebrew set Servo's book lists.
- Pin the `servo` git dependency to a known-good revision (Servo's API moves; pin and bump deliberately).
- Expect a long first build (Servo + its dependency tree).

This is why the build env is its own setup step.

## Build host: GitHub Actions CI (resolved 2026-05-29)

The Servo build runs OFF the developer machine, in CI, reproducibly. It does not have to be local: Servo must be built from source (no prebuilt embeddable crate), but where it builds is our choice, and early verification runs headless (offscreen, no window). Only the eventual interactive windowed desktop browser must run on a macOS desktop, which is a later milestone.

- `.github/workflows/servo-browser.yml` is the build pipeline. v1 validated that Servo builds in CI via its own `./mach bootstrap` + `./mach build` (manual trigger; a full build is heavy). v2 builds this embedder crate as an external consumer of the `servo` crate, then runs the headless WebView substrate smoke.
- External-embed reference: `paulrouget/servo-embedding-example` and Verso (the `servo` crate as a git dependency, not a fork).
- Resource caveat: ubuntu-latest may be undersized for a full Servo build; the first run reveals whether it needs a larger/self-hosted runner or the remote-box fallback.

## Substrate-seam reuse (less new code than it looks)

The browser's `load_web_resource` hook writes a loaded page into the substrate as graph state. That is the SAME operation `rustyred-web` already performs. Concrete seam (grounded against `rustyredcore_THG/crates/rustyred-web/src/lib.rs`, verified 2026-05-29):

1. The hook builds a `FetchedPage` (url + body + status + content_type) from the Servo `WebResourceLoad`.
2. It calls `build_v2_fixture_crawl(CrawlRequest, &[FetchedPage]) -> CrawlRunOutput` (V2: budget + URL guard + scope + `CrawlReceipt`), or `build_fixture_crawl_graph(CrawlConfig, &[FixturePage])` for the no-budget path.
3. It writes the result with `CrawlGraph::apply_to_store(&mut impl GraphStore)`.

That emits `Page`/`Domain`/`ContentSnapshot`/`FetchAttempt` nodes + `LINKS_TO`/`HAS_SNAPSHOT`/`ON_DOMAIN` edges via `extract_links` + `canonicalize_url` + `blake3_hash`. No new page-to-graph code: the only glue the embedder adds is `WebResourceLoad` -> `FetchedPage`. The substrate side is built and buildable today without Servo; the Servo build is needed only for the rendering + the hook itself.

## Session store

The browser owns a `BrowserSessionStore<RedCoreGraphStore>`:

- default: ephemeral RedCore memory mode for constructor/smoke runs
- durable: `cargo run -- --store-dir /tmp/theorem-browser-store --windowed http://theorem.local/smoke`
- env-configured durable: `THEOREM_BROWSER_STORE_DIR=/tmp/theorem-browser-store cargo run -- --headless-smoke`
- explicit throwaway run: add `--memory-store` to ignore the env var

The important point is that `/smoke` writes and `/search?q=...` reads go through the same session object. Switching memory to disk changes only the backing RedCore store, not the browser delegate wiring.

## Next increments

1. Keep the external Servo embedder build green in CI (done for constructor wiring; now includes the headless WebView smoke).
2. Get a minimal WebView rendering a single URL in a winit window (the "it renders the open web" milestone). The `--windowed [url]` entrypoint is now the compile-validated shell for this.
3. Extend the current intercepted smoke seam into true loaded-page capture. Important API note: `load_web_resource` sees requests before load, not response bodies after download, so arbitrary open-web capture will need either interception/fetch ownership or a separate completed-document extraction path.
4. Compose a SceneOS scene into the surface (seam 2), then move into the cost-graded dossier/search chrome.

## Files

- `Cargo.toml`: the embedder crate (depends on the `servo` git crate + winit + url).
- `src/main.rs`: the embedder entrypoint. Default mode constructs the Servo engine, `--headless-smoke` validates WebView + substrate ingest headlessly, and `--windowed [url]` opens the minimal desktop WebView shell.
