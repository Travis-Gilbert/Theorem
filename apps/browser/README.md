# Theorem Browser (Servo-embedded substrate browser)

**Status:** Foundation. Grounded against the current Servo embedding API (`servo::WebViewBuilder`, `ServoDelegate`, `Servo::spin_event_loop`, pulled from doc.servo.org 2026-05-29). This crate is the start of the substrate-native browser: the surface where Servo renders both the open web and SceneOS scenes, in-process with the RustyRed substrate.

**Honest state:** this lays the embedder skeleton + build plan against the real API. It is NOT yet built or rendering. Servo is a heavy build (its own `mach`/cargo toolchain + system deps); getting it to compile + render a page is the next increment and needs a Servo build environment, not a plain `cargo build`. The skeleton marks every seam; the constructor wiring (Opts/Preferences/RenderingContext) is validated when the Servo build env is stood up.

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

1. **DOM-as-substrate-write: `ServoDelegate::load_web_resource(WebResourceLoad)`.** Servo calls this when it is about to load an HTTP/HTTPS resource; the embedder may `WebResourceLoad::intercept(...)` to supply alternate contents, or let it continue. This is the hook where a loaded page becomes graph state in the RustyRed substrate (write the page/DOM as nodes/edges) and where substrate-resident content (a generated SceneOS scene) can be served in place of a network load. In-process with RustyRed, no API boundary.
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

- `.github/workflows/servo-browser.yml` is the build pipeline. v1 validates that Servo builds in CI via its own `./mach bootstrap` + `./mach build` (manual trigger; a full build is heavy). v2 adds this embedder crate as an external consumer of the `servo` crate, builds it headless, and runs the substrate-seam test.
- External-embed reference: `paulrouget/servo-embedding-example` and Verso (the `servo` crate as a git dependency, not a fork).
- Resource caveat: ubuntu-latest may be undersized for a full Servo build; the first run reveals whether it needs a larger/self-hosted runner or the remote-box fallback.

## Substrate-seam reuse (less new code than it looks)

The browser's `load_web_resource` hook writes a loaded page into the substrate as graph state. That is the SAME operation `rustyred-web` already performs (`build_fixture_crawl_graph` turns pages into `Page`/`ContentSnapshot`/`LINKS_TO`/`Domain` nodes). So the seam feeds Servo's loaded pages into rustyred-web's existing page-to-graph logic. The substrate side is largely built and buildable today without Servo; the Servo build is needed only for the rendering + the hook itself.

## Next increments

1. Stand up the Servo build env (mach bootstrap deps; pin the `servo` git rev). Validate the constructor wiring (Opts/Preferences/RenderingContext) against the pinned rev.
2. Get a minimal WebView rendering a single URL in a winit window (the "it renders the open web" milestone).
3. Implement `load_web_resource` interception writing the loaded page into the RustyRed substrate as graph state (seam 1).
4. Compose a SceneOS scene into the surface (seam 2).
5. Then the cost-graded dossier + search-as-graph chrome (these trigger the design-gate).

## Files

- `Cargo.toml`: the embedder crate (depends on the `servo` git crate + winit + url).
- `src/main.rs`: the embedder skeleton against the current API, with the two substrate seams marked. Not yet built; the foundation to validate against a Servo build env.
