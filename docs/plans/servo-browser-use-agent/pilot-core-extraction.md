# Pilot-core: extract the Servo-free automation core in place (toward "WebDriver BiDi for Servo")

Author: claude-code. Greenlit by Travis. Sibling to
`servo-automation-core-playwright-class.md` (the plan) and
`servo-automation-core-status.md` (what shipped). This plan covers turning the
already-built, already-green automation core into a clean, lean, reusable crate
**inside the Theorem repo** (NOT extracted to a standalone repo yet), as the
de-risking step toward an open-source "WebDriver BiDi for Servo".

## Goal + falsifiable success

A new crate `rustyredcore_THG/crates/pilot-core` that holds the entire Servo-free
Playwright-class contract with **lean deps (serde + tokio only) and zero
substrate**. Success is falsifiable: `cargo tree -p pilot-core` shows **no
`rustyred_thg_core`, no `ndarray`, no `cblas-sys`, no `openblas`**. Every existing
rustyred-web + apps/browser test stays green because rustyred-web re-exports the
moved types.

## Why in place first (not straight to a standalone repo)

Extracting straight to a new repo risks discovering hidden coupling only after the
copy. Building pilot-core in place proves the boundary against the real
dependency graph; once `cargo tree` is clean, mirroring to a standalone repo is a
copy, not surgery. Travis: do not lift it out yet.

## The boundary

- **pilot-core (Servo-free, substrate-free)** = `browser_driver.rs` (the
  `BrowserDriver` trait, `ActuationKind`, snapshot script + parser, E5/E6
  transform, `run_action` auto-wait) + `browser_automation.rs` (Locator,
  actionability matrix, Context/routing, web-first `expect`, receipts) + the
  **pure data types** from `browser_engine.rs` (`ElementBox`,
  `InteractiveElement`, `PageState`, `BrowserAction`, `WaitCondition`,
  `BrowserActionPolicy`, `BrowserEngineError`/`Result`).
- **rustyred-web keeps**: `FetchCascadeBrowserEngine` (it `impl`s
  `pilot_core::BrowserDriver`), the crawl/fetch stack, `web_consume_to_graph`,
  the `GraphStore` ingest, `browser_perception`. It **re-exports** pilot-core
  (`pub use pilot_core::...`) so no current consumer changes.
- **apps/browser keeps** `ServoWebViewAutomationDriver`; it retargets
  `pilot_core::BrowserDriver` (a mechanical use-path bump). Codex's lane.

## Coupling audit (the three tendrils slice 1 must sever)

The "pure" data types are not quite pure; each tendril has a chosen cut:

1. **`PageState.fetch: Option<FetchTierResult>`** (rustyred-web fetch stack).
   Cut: drop `fetch` from pilot-core's `PageState`; `FetchCascadeBrowserEngine`
   keeps the `FetchTierResult` in its own bookkeeping (a parallel map keyed by
   history index, or a rustyred-web wrapper). Verify no consumer reads
   `PageState.fetch` outside the fetch engine.
2. **`BrowserActionPolicy.sensitive_data: SensitiveData`** (from
   `browser_perception`). Decide: if `SensitiveData` is pure (masking logic, no
   substrate), move it into pilot-core too; else replace the field with a
   pilot-core-native masking type and let rustyred-web adapt.
3. **`BrowserEngineError::RustyWeb { message }` + `impl From<RustyWebError>`**.
   Cut: pilot-core's error drops the `RustyWeb` variant and the `From`; rustyred-web
   maps its `RustyWebError` into a pilot-core error at its own boundary.

## Slices

1. **Crate + data types.** Create `pilot-core` (added to the workspace), move the
   data types with the three tendrils severed, rustyred-web re-exports. Green:
   `cargo test -p rustyred-web` + `cargo tree -p pilot-core` clean.
2. **Move the logic.** `browser_driver.rs` + `browser_automation.rs` into
   pilot-core; rustyred-web re-exports; `FetchCascadeBrowserEngine`'s
   `impl BrowserDriver` stays in rustyred-web. Green.
3. **Prove + harden.** A CI/test assertion that pilot-core's dependency tree is
   substrate-free; move the dep-light test driver (`FakeDriver`, and optionally a
   `HtmlDriver`) into pilot-core so the core tests without libservo. Ping Codex to
   bump the apps/browser adapter's use paths.
4. **Clean-core features (toward OSS parity):** vendor the full Playwright
   injected selector bundle (replace the thin bridge), EmbedderControl responses,
   frames. (Shared / Codex Servo-side.)
5. **`pilot-bidi` (the headline):** a WebDriver BiDi WebSocket server mapping
   `browsingContext` / `input` / `network` / `script` commands + events onto
   pilot-core, so real Playwright / Puppeteer / WebdriverIO (any language) drive
   Servo. The in-process vocabulary was already shaped after the BiDi taxonomy,
   so this is an adapter, not a second engine.

## Coordination

claude-code drives the extraction (the crate + the moves + re-exports). Codex
holds refactors of `browser_engine.rs` / `browser_automation.rs` during the move
(code is preserved + relocated, not clobbered) and later retargets the
apps/browser adapter's use paths. Decision recorded in the room
(`record_b488e7b03626a243`).

## Naming + license (working, renameable before any OSS lift)

- Crate `pilot-core`; project `servo-pilot` (candidates: servo-pilot, playservo,
  ferroplay, servo-actuate). Travis to confirm before extraction.
- License `Apache-2.0` (OSS-friendly; compatible with consuming Servo's MPL-2.0
  and matches the vendored Playwright selector bridge's Apache-2.0). The rest of
  the workspace is MIT; per-crate license is fine.
