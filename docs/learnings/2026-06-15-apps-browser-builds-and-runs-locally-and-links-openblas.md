# apps/browser (the Servo embedder) DOES build + run locally despite the "CI-only" lore: no-space path symlink + OpenBLAS, and CI needs libopenblas-dev too

**Kind:** gotcha
**Captured:** 2026-06-15
**Session signature:** `claude:travisgilbert (servo-automation-core / playwright-class)`
**Domain tags:** apps/browser, servo, libservo, jemalloc, openblas, ci, local-build

## Trigger

Two surprises bringing the Servo embedder up this session. (1) CLAUDE.md and the
plan docs say `apps/browser` "cannot be compiled locally (libservo ~30 min)", but
Codex built it on this macOS machine -- the only blocker was `tikv-jemalloc-sys`
rejecting a configure prefix containing a space (`/Users/.../Tech Dev Local/...`).
The workaround is a no-space symlink `/tmp/theorem-repo -> <repo>` plus
`CARGO_TARGET_DIR=/tmp/theorem-browser-target`. With a warm target I ran the 388 MB
binary directly: `/tmp/theorem-browser-target/debug/theorem-browser --headless-automation-smoke`
-> `EXIT=0`, a real Servo `WebView` driven by the automation core
(`interactive_elements=2`, `click_mechanism=servo_notify_input_event`,
`fill_mechanism=servo_focus_then_value_commit`) -- a few seconds, no rebuild, fully
independent of CI. (2) The CI run failed at LINK on `rust-lld: error: unable to find
library -lopenblas` (exit 101) while the local build was green: the macOS build
links brew OpenBLAS (`/opt/homebrew/opt/openblas`), the Ubuntu runner had none.
`cblas-sys`/`ndarray` is a transitive pull via rustyred-web's ML/embedding stack,
linked once the `BrowserDriver` adapter references more of rustyred-web. Fixed with
`sudo apt-get install -y libopenblas-dev` in `servo-browser.yml` (`906a0024`).

## Rule

`apps/browser` builds AND the headless smokes RUN locally -- do not treat "CI-only"
as gospel. Build/run via the no-space symlink `/tmp/theorem-repo` (jemalloc's
configure rejects the space in the real checkout path) with
`CARGO_TARGET_DIR=/tmp/theorem-browser-target`; a warm target makes a smoke a
few-second re-run and the gold-standard independent verification. The embedder
links OpenBLAS (brew locally, `libopenblas-dev` on the Ubuntu CI runner) because
`cblas-sys`/`ndarray` ride in transitively via rustyred-web's ML stack once the
embedder references enough of rustyred-web -- so a green local link does NOT
predict a green CI link. Pre-verifying API signatures (the cached-source method)
catches compile errors, not system-library link errors; check both classes. Clean
follow-up: feature-gate the BLAS/ML path out of the embedder's reachable graph so
the browser binary links no BLAS at all.

## Evidence

- `/tmp/theorem-repo` is a symlink to the real checkout; `/tmp/theorem-browser-target/debug/theorem-browser` is a 388 MB warm binary (built 12:02).
- Local smoke: `EXIT=0`, `headless automation smoke OK; url=http://theorem.local/automation-smoke, interactive_elements=2, click_mechanism=servo_notify_input_event, fill_mechanism=servo_focus_then_value_commit`.
- CI run 27568408123: `X Build theorem-browser embedder ... error: linking with cc failed ... rust-lld: error: unable to find library -lopenblas ... exit code 101`. Fix: `906a0024`.
- `/opt/homebrew/opt/openblas` exists locally (why the macOS link succeeded).

## Encoded in

- `docs/learnings/2026-06-15-apps-browser-builds-and-runs-locally-and-links-openblas.md` (this file)
