# scene-os-web

Theorem SceneOS renderer bundle (Lane B): embeds the self-contained canvas renderer and serves a scene-package-v2 as one HTML asset, the SERP injection pattern.

## What it is

SceneOS renderer serving — turn a scene package into the browser's scene PAGE.

This is Lane B of the SceneOS -> Theorem port: the renderer half. Lane A
(`scene-os-core`) is the director that produces a [`ScenePackageV2`]; this
crate takes that package and serves the page that DRAWS it. The browser's
`load_web_resource` hook (Lane C) intercepts a scene URL, calls Lane A to
produce the package, and serves the HTML this module returns — exactly as
`rustyred-web` serves its SERP graph page.

The page is a single self-contained HTML document (`web/scene-host.html`,
embedded via `include_str!`) with the renderer bundle (`web/dist/
scene-os.bundle.js`, an esbuild IIFE with d3 inlined) injected in place of a
placeholder. No bundler at serve time, no npm, no CDN: Servo serves one
asset. The only dynamic part is the scene package, injected in place of a
`null` marker.

Security: the page renders atom labels / kinds that may come from CRAWLED
pages or agent output, which are untrusted. Two defenses, both required and
mirroring `rustyred-web::serp`:
  1. `scene-host.html` + the bundle set every piece of DOM text via
     `textContent` / `createElement`, never `innerHTML`.
  2. [`scene_payload_json`] escapes `<`, `>`, `&` to their `\uXXXX` forms so
     a label containing `</script>` cannot break out of the `<script>` block
     the payload is injected into.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p scene-os-web
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
