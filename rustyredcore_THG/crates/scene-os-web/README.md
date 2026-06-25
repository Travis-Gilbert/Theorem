# scene-os-web

The SceneOS renderer (Lane B): turn a `ScenePackageV2` into one self-contained HTML page that draws it, with script-safe payload escaping. Lane A (`scene-os-core`) produces the package; this crate serves the page. The browser's `load_web_resource` hook (Lane C) intercepts a scene URL, calls Lane A, and serves the HTML this module returns, mirroring how `rustyred-web` serves its SERP graph page.

## Key API

- `render_scene(package: &ScenePackageV2) -> Result<String, serde_json::Error>`: typed entry.
- `render_scene_html(package_json: &str) -> String`: engine-agnostic entry over already-serialized JSON (pass `"null"` for the honest empty state).
- `scene_payload_json(package_json: &str) -> String`: escapes `<`/`>`/`&` to `\uXXXX` so a label containing `</script>` cannot break out of the script block.

The page is `web/scene-host.html` plus the esbuild bundle `web/dist/scene-os.bundle.js` (d3 inlined), both embedded via `include_str!` and committed so the crate is self-contained: no bundler, npm, or CDN at serve time. XSS defense is two-layer: all DOM text is set via `textContent`/`createElement` (never `innerHTML`), plus `scene_payload_json`. TypeScript renderer source lives under `web/src/`. Path dep: `scene-os-core`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p scene-os-web
cargo run -p scene-os-web --example render_sample -- /tmp/scene.html
```

Tests are inline (injection/marker consumption, payload-stays-valid-JSON, script-breakout neutralized, null-package empty page). No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
