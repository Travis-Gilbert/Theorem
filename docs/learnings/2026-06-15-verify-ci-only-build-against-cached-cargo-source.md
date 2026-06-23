# Pre-verify a CI-only (libservo) Rust build against the pinned source already cached in ~/.cargo/git/checkouts before spending a ~30-min CI cycle

**Kind:** method
**Captured:** 2026-06-15
**Session signature:** `claude:travisgilbert (servo-automation-core / playwright-class)`
**Domain tags:** apps/browser, servo, libservo, ci-only, cargo, de-risk

## Trigger

`apps/browser` embeds libservo (CI-only, ~30 min cold build, cannot compile
locally). Codex git-only landed `ServoWebViewAutomationDriver` implementing my
`rustyred-web::BrowserDriver` trait over a live Servo `WebView`. The only place
that code compiles is the `servo-browser.yml` CI run, so verifying it meant
pushing and eating a full ~30-min cycle just to learn whether its Servo API calls
were even spelled right. Instead, the pinned Servo rev was already on disk at
`~/.cargo/git/checkouts/servo-e53a6e7b994a25fe/b891f04` (cargo had fetched it for
a prior CI/embedder build), so I grepped + read the real API: `notify_input_event(InputEvent) -> InputEventId` (webview.rs:531), `evaluate_javascript<T: ToString>(script, FnOnce(Result<JSValue, JavaScriptEvaluationError>))` (webview.rs:662), `device_pixels_per_css_pixel() -> Scale<f32, CSSPixel, DevicePixel>` (the `.0`), the `MouseMoveEvent`/`MouseButtonEvent`/`TouchEvent::new(... point: WebViewPoint)` constructors, and the `InputEventResult` u8 bitflags. The one non-obvious bit was the adapter's `servo::DevicePoint.into()` into a `WebViewPoint`-typed constructor; that compiles only because of `impl From<DevicePoint> for WebViewPoint` at `components/shared/embedder/lib.rs:75`, which I confirmed exists. Every load-bearing call matched, so I pushed with high confidence the adapter compiles, instead of gambling a cycle.

## Rule

For a CI-only / locally-unbuildable Rust dependency (libservo and friends), before
pushing to trigger an expensive CI compile, check whether the pinned rev is in
`~/.cargo/git/checkouts/<dep>-<hash>/<shortrev>/` and read the ACTUAL signatures
for every load-bearing call the new code makes: method signatures, enum variants,
and especially any `.into()`/`From` conversion between near-identical types
(`DevicePoint` vs `WebViewPoint`). A `From` impl one component away is the
difference between "compiles" and a 30-min CI round-trip to discover a type error.
This is the cheapest de-risking available when you cannot run the compiler yourself.

## Evidence

- `ls ~/.cargo/git/checkouts/servo-e53a6e7b994a25fe` -> `b891f04` (the exact pinned rev from `apps/browser/Cargo.toml` + `rust-toolchain.toml`).
- `impl From<DevicePoint> for WebViewPoint` at `components/shared/embedder/lib.rs:75` is what makes the adapter's `servo_point.into()` legal.
- All checked signatures matched the adapter's usage; CI (run 27568408123) was triggered only after this pass.

## Encoded in

- `docs/learnings/2026-06-15-verify-ci-only-build-against-cached-cargo-source.md` (this file)
