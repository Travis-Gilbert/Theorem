# `gh run watch --exit-status` can exit non-zero on a transient gh auth/API blip while the run is still building: confirm the conclusion with `gh run view`, never the watch exit code

**Kind:** gotcha
**Captured:** 2026-06-15
**Session signature:** `claude:travisgilbert (servo-automation-core / playwright-class)`
**Domain tags:** gh, github-actions, ci-monitoring, false-failure

## Trigger

To get notified when the ~30-min Servo CI run (27568408123) concluded, I
backgrounded `gh run watch <id> --exit-status > log 2>&1; echo "WATCH-EXIT=$?"`.
It returned `WATCH-EXIT=1` after ~8 minutes -- which, read naively as the run's
conclusion, says "CI failed." It had not: the log ended with
`failed to get annotations: HTTP 401: Bad credentials (.../check-runs/.../annotations)  Try authenticating with: gh auth refresh`,
and the job tree above it showed "Build theorem-browser embedder" and all four
smokes still pending (`*`). The non-zero exit was a transient gh auth error on the
annotations endpoint, not a failing run. An immediate `gh run view <id>` showed the
run still in progress and `gh auth status` showed the token healthy (a momentary
blip). Reporting "CI failed" off WATCH-EXIT=1 would have been a false alarm and
sent us debugging a build that was fine.

## Rule

Do not equate `gh run watch --exit-status`'s exit code with the run's conclusion.
It also exits non-zero on transient gh API/auth errors (e.g. a 401 fetching
annotations) mid-watch. Always confirm the real outcome with `gh run view <id>`
(and check `gh auth status`) before reporting pass/fail. Keep the
`> log 2>&1; echo "WATCH-EXIT=$?"` wrapper -- the log body (not the exit code) is
what reveals "auth blip vs real failure." If the watch dies early while the run is
still in progress, just re-arm it.

## Evidence

- `WATCH-EXIT=1` with log tail `failed to get annotations: HTTP 401: Bad credentials` while the embedder build step was still running.
- `gh run view 27568408123` immediately after: `* servo-build` (in progress); `gh auth status`: logged in, token active.

## Encoded in

- `docs/learnings/2026-06-15-gh-run-watch-exit-not-run-conclusion.md` (this file)
