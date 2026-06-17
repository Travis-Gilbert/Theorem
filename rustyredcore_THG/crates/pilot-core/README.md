# pilot-core

Servo-free, Playwright-class browser automation core: locator, actionability + auto-wait, geometry snapshot, coordinate synthesis, and web-first assertions behind a BrowserDriver trait. In-place precursor to an open-source 'WebDriver BiDi for Servo'.

## What it is

`pilot-core`: a Servo-free, Playwright-class browser automation core.

This crate holds the entire automation contract with lean dependencies
(serde + tokio only) and **zero substrate coupling**, so one set of logic can
drive any backend through the [`BrowserDriver`] trait: a live `servo::WebView`
(the `apps/browser` adapter), the fetch-cascade engine (rustyred-web), a fake
driver (tests), and -- later -- a WebDriver BiDi front end that lets real
Playwright / Puppeteer / WebdriverIO clients drive Servo.

The differentiation over CDP-class tooling is that actionability is computed
from engine truth (box tree, Paint hit-testing, frame-accurate settle) rather
than injected DOM heuristics. Playwright's reliability is auto-wait; Servo can
do auto-wait better because it owns the layout and the frame loop.

Status: scaffold. Content is being migrated **in place** from rustyred-web
(`browser_driver.rs`, `browser_automation.rs`, and the data-type half of
`browser_engine.rs`) per
`docs/plans/servo-browser-use-agent/pilot-core-extraction.md`. Until the move
completes, rustyred-web remains the home and this crate is empty by design.

Falsifiable boundary check: `cargo tree -p pilot-core` must show no
`rustyred_thg_core`, no `ndarray`, no `cblas-sys`, no `openblas`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p pilot-core
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
