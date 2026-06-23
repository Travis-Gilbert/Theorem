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

- `coordinate_synthesis` -> `locator.click()`
- `keyboard` -> `locator.fill(kind.text)`
- `scroll` -> `locator.scrollIntoViewIfNeeded()`
- `embedder_control` / `semantic_activation` -> reported as
  `sidecar.unsupported` (V1; returns 200 + current page so the run proceeds)

## Scope / honesty

This is a dev convenience, not the production driver. It does not enforce the
`policy` block (the harness gates state-changing actions before it calls), does
not implement native `<select>`/file-picker control surfaces, and keeps one
Chromium context per session in memory until process exit.
