# theorem-browser-sidecar

A minimal Playwright sidecar that satisfies **both** Theorem browser surfaces:

| Surface | Route | Harness env var |
|---|---|---|
| Rendered fetch | `POST /render` | `THEOREM_SERVO_RENDER_ENDPOINT=http://127.0.0.1:9223/render` |
| Live action loop | `POST /sessions/checkout\|snapshot\|actuate` | `THEOREM_LIVE_BROWSER_ENDPOINT=http://127.0.0.1:9223` |

It runs pilot-core's exact `GEOMETRY_SNAPSHOT_SCRIPT` in the page, so the
`PageState` it returns matches what the live Servo driver would emit; the Rust
locator/actionability layer resolves a locator to `{target_handle, kind}` and
this sidecar acts on `[data-theorem-id="<handle>"]`.

When the Rust server sends `include_screenshot: true`, checkout/snapshot/actuate
responses also include `screenshot_base64` + `screenshot_media_type`. That feeds
`THEOREM_VISUAL_PERCEIVER_URL` (`perception_visual` `POST /parse`) so no-DOM
surfaces can still produce visual `PageState.interactive_elements`.

## Run

Via docker compose (recommended):

```bash
docker compose -f scripts/dev/docker-compose.yml up -d browser-sidecar
```

Or locally with Node 18+:

```bash
cd scripts/dev/browser-sidecar
npm install            # postinstall fetches the Chromium build
PORT=9223 npm start
```

## Actuation mapping

`plan.kind.mechanism` -> Playwright:

- `coordinate_synthesis` -> click/hover/double-click the plan point; falls back to `locator.click()`
- `keyboard` -> `locator.fill(kind.text)`
- `scroll` -> `locator.scrollIntoViewIfNeeded()`
- `embedder_control` / `semantic_activation` -> reported as
  `sidecar.unsupported` (V1; returns 200 + current page so the run proceeds)

## Scope / honesty

This is a dev convenience, not the production driver. It does not enforce the
`policy` block (the harness gates state-changing actions before it calls), does
not implement native `<select>`/file-picker control surfaces, and keeps one
Chromium context per session in memory until process exit.
