# Servo Fetch Decision

**Phase:** 5, Servo agent surfaces
**Status:** fallback seam implemented, Servo direct engine deferred behind the agent-tab interface

## Recommendation

Use the agent-tab interface now, with the current wry-plus-extraction fallback feeding the same ingestion receipt path. Keep direct Servo behind a feature flag until the workspace Servo lane proves a pinned crate version and delegate coverage for final DOM text plus resource provenance.

## Evidence

- General tabs remain wry/system webview per phase one.
- The desktop now has a distinct `agent` tab kind and an ingestion receipt path that records URL, title, capture time, target store, and `open_web_unverified` trust tier.
- The existing `theorem-browser-agent` and MCP `web_consume` surfaces are present in the workspace, but the direct Servo engine pin is not yet established in `apps/desktop`.

## Gaps

- Direct Servo rendering is not linked into the desktop binary yet.
- Delegate interception for resource lists and final rendered DOM still needs the Servo lane's crate pin and validation.
- Headless `konippi/servo-fetch` is not accepted yet because no local proof in this workspace demonstrates stable build and capture coverage.

## Contract

The interface is the contract: agent tabs produce ingestion receipts into the selected harness target and never alter general tab behavior. When Servo is ready, it should replace the capture engine under that same interface, not add a second product surface.
