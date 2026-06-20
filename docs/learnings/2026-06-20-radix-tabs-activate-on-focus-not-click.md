# Radix Tabs activate on FOCUS (activationMode="automatic"), so a programmatic `.click()` will not switch tabs in browser verification — dispatch focus()+pointer events, or you will misdiagnose the tab's content (e.g. a cosmos.gl/WebGL canvas) as broken

**Kind:** gotcha
**Captured:** 2026-06-20
**Session signature:** `claude-code:travisgilbert (harness-console: preview verification of Memory cosmos.gl graph)`
**Domain tags:** radix-ui, tabs, preview-verification, cosmos.gl, webgl, harness-console

## Trigger

Verifying the Memory surface in the preview browser, I switched its view mode from List to Graph by finding the Radix `TabsTrigger` labeled "Graph" and calling `el.click()` in `preview_eval`. The eval reported the button was found and clicked (`clickedGraph: true`), but the tab did NOT switch: the cosmos.gl graph never mounted, `canvasCount` stayed at 1 (only the DotGrid ambient canvas), and the page screenshot still showed List mode. I started down a false trail — checking whether WebGL was unavailable in the headless context and whether the graph container had zero height. Both were fine. The real cause: Radix Tabs default `activationMode="automatic"` activates a tab on FOCUS, and a bare programmatic `.click()` does not focus the element first. Dispatching `el.focus()` then `pointerdown`/`mousedown`/`pointerup`/`click` switched the tab, and a second canvas (2220x1076) appeared — cosmos.gl rendering correctly all along.

## Rule

- When driving a Radix Tabs (or any focus-activated control) in preview/headless verification, do not rely on `.click()`. Use the harness's real click tool (which focuses), or in `eval` do `el.focus()` then dispatch the full pointer sequence (`pointerdown`, `mousedown`, `pointerup`, `click`).
- Before concluding that a tab's *content* (a WebGL canvas, a lazily-mounted component) is broken, first confirm the tab actually activated: check `[role="tab"][data-state="active"]` and that the matching `[role="tabpanel"]` mounted. A "clicked but nothing rendered" symptom is the activation, not the renderer.
- cosmos.gl (`@cosmos.gl/graph` v3) assigns its `<canvas>` asynchronously during device init; count canvases (`document.querySelectorAll('canvas')`) after a short wait rather than expecting it synchronously — and remember a full-viewport DotGrid canvas is already present, so "2 canvases" is the success signal on the Memory graph.

## Evidence

- `el.click()` only → graph tab `data-state` stayed inactive, `canvasCount=1`, screenshot still List.
- `el.focus()` + pointerdown/mousedown/pointerup/click → graph tab `data-state="active"`, `canvasCount=2` (DotGrid 2880x1800 + cosmos 2220x1076).
- WebGL was available the whole time: `document.createElement('canvas').getContext('webgl2')` was truthy.
