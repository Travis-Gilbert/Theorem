# Lane C — Servo SceneOS integration (shipped)

**Date: 2026-05-30**
**Owner: Codex. Status: slice 1 complete. Surface: `apps/browser`.**

Lane C wires the two completed halves into the actual browser:

1. Lane A (`scene-os-core`) compiles a typed `ScenePackageV2`.
2. Lane B (`scene-os-web`) renders that package as a self-contained HTML page.
3. Lane C (`apps/browser`) serves that page through Servo's existing
   `WebViewDelegate::load_web_resource` interception seam.

## What shipped

`apps/browser` now intercepts:

- `http://theorem.local/smoke` — known local page, ingested into the browser
  session substrate on load.
- `http://theorem.local/search?q=...` — graph-native SERP from the same browser
  session substrate.
- `http://theorem.local/scene?q=...` — SceneOS page from the same browser
  session substrate.

The scene route does not call Index-API. It runs in-process:

```
BrowserSessionStore.search_substrate(q)
  -> SearchHit/SearchLink neighbourhood
  -> SceneScene atoms + relations
  -> scene_os_core::compile_scene_package(answer_type="tree_hierarchy")
  -> scene_os_web::render_scene(&package)
  -> WebResourceLoad::intercept(...)
```

The route currently asks Lane A for `tree_hierarchy`, which pairs with the
shipped `document_rail` chrome. `scene-os-core` now supports explicit
production projection hints, so the browser does not depend on catalog order or
accidentally route generic browser scenes through `patent_diagram`.

## Validation boundary

Fast checks cover the Servo-free parts:

- `cargo test --manifest-path apps/browser-substrate/Cargo.toml`
- `cargo test --manifest-path rustyredcore_THG/Cargo.toml -p scene-os-core`
- `cargo test --manifest-path rustyredcore_THG/Cargo.toml -p scene-os-web`

The heavy Servo check is in `.github/workflows/servo-browser.yml`:

- `cargo run -- --headless-smoke`
- `cargo run -- --headless-scene-smoke`

The second command creates a real WebView for
`http://theorem.local/scene?q=substrate`; the local substrate is seeded first,
then the scene route is served through the same WebView interception path.

## Remaining follow-ups

- Use arbitrary completed-page capture once the browser owns fetch/response
  bodies beyond local intercepted pages.
- Replace the current search-neighbourhood-to-atoms mapping with richer graph
  typed nodes when RustyWeb emits claim/evidence semantics directly.
- Add visual screenshot CI for the scene page once the Servo runner has a cheap
  browser-screenshot path; Lane B already has a headless Chromium visual proof
  outside Servo.
