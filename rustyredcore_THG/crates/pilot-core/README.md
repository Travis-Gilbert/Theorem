# pilot-core

A Servo-free, Playwright-class browser-automation core: locator contracts, actionability and auto-wait, geometry snapshots, coordinate synthesis, web-first assertions, and sensitive-data masking, all behind a `BrowserDriver` trait with zero substrate coupling. One set of logic can drive any backend: a live `servo::WebView` (the `apps/browser` adapter), the fetch-cascade engine (`rustyred-web`), or a fake driver (tests).

The differentiation over CDP-class tooling is that actionability is computed from engine truth (box tree, hit-testing, frame-accurate settle) rather than injected DOM heuristics.

## Key API

- Types (`types.rs`): `BrowserAction`, `WaitCondition`, `ElementBox`, `InteractiveElement`, `PageState`, `BrowserActionPolicy`, `BrowserEngineError`, `BrowserEngineResult`. (These are re-exported by `rustyred-web::browser_engine`.)
- Masking (`masking.rs`): `SensitiveData` (domain-scoped secrets resolved at the engine boundary, masked from logs/trace/receipts), `MaskedText`.
- Automation (`automation.rs`): `Locator` (`css`/`get_by_role`/`get_by_text`/`get_by_label`/`get_by_test_id`/`frame`/`filter`/`nth`/`resolve`), `Actionability`, `LocatorAction`, `Context`/`RouteRule`/`UrlPattern`, `expect(Locator) -> LocatorExpectation` (`to_be_visible`/`to_be_enabled`/`to_have_text`/`to_have_count`), `SELECTOR_BRIDGE_SCRIPT` (vendored Playwright selector bridge, Apache-2.0).
- Driver (`driver.rs`): `BrowserDriver` trait (`snapshot`, async `actuate`), `run_action` (adds re-snapshot auto-wait), `build_actuation_plan`, geometry helpers (`css_to_device_point`, `DevicePoint`), `GEOMETRY_SNAPSHOT_SCRIPT`, `page_state_from_snapshot_json`, `ActuationPlan`/`ActuationReceipt`.

The automation contract (`types`, `masking`, `automation`, `driver`) is populated and wired with a working test suite; this is no longer a scaffold. The only vendored asset is `vendor/playwright_selector_bridge.js`.

Crate facts: version 0.0.0, `publish = false`, license Apache-2.0 (differs from the MIT crates around it). Deps: serde plus tokio (`time`); zero substrate coupling (no `rustyred-thg-core`, no BLAS).

## Build and test

```bash
cd rustyredcore_THG && cargo test -p pilot-core
```

Tests live in `driver.rs` against a `FakeDriver`. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
