# Theorem Desktop, phase five: Servo agent surfaces (job-005)

**Repo:** Travis-Gilbert/theorem
**Plan home:** docs/plans/theorem-desktop/
**Requires:** phase one complete; phase two for local-node ingestion targets.
**Job linkage:** job-005, kind Feature, priority P2, target_head Either.

## Decision basis

General tabs stay on wry (phase one decision, unchanged). Agent surfaces, tabs whose purpose is browsing-as-ingestion rather than user browsing, run on the direct servo crate, where the delegate hooks give interception the system webview cannot: every loaded resource and the rendered DOM are observable on the way through. This is the engine-seam decision; the Servo browser-use-agent work elsewhere in this repo is the sibling lane, and shared code belongs in a common crate, not copied.

## Deliverables

### D1: agent tab type
A feature-flagged second tab type, agent tab, rendering through the servo crate (pin the version the workspace already resolves; verify against the Servo browser-use-agent lane's pin before adding a second). Visually identical chrome to a normal tab plus a small ingestion indicator. Users open one deliberately; nothing auto-converts.

### D2: interception to ingestion
Delegate hooks capture the final DOM text, the resource list, and navigation provenance for each load in an agent tab, and feed them to the existing web_consume ingestion path with URL provenance, into whichever store the phase-two switch targets. Quarantine posture per the standing open_web_unverified tier: agent-tab content enters the lower-trust tier and is promoted only through the existing filter, never directly.

### D3: headless fetch evaluation
Evaluate konippi/servo-fetch as the engine for headless web_consume (no window). Deliverable is a short decision record in this plan directory: works, partially works with named gaps, or rejected with the reason, plus the recommendation. Code only if the evaluation passes; the record is the required artifact either way.

### D4: fallback seam
If the servo crate blocks on a hard gap (build, stability, or missing delegate coverage), the fallback is wry plus injected extraction behind the same agent-tab interface, with the gap documented in the decision record and the servo path left feature-flagged, not deleted. The interface is the contract; the engine is swappable. A fallback is a documented decision, never a silent substitution.

## Acceptance criteria

1. An agent tab loads a page and produces an ingestion receipt in the target store carrying URL, title, and capture time.
2. Ingested content lands in the unverified tier and does not appear in grounded recall until promoted by the existing filter.
3. General tabs are untouched: same engine, same behavior, same performance as phase one.
4. The decision record for servo-fetch exists in this directory with a clear recommendation.
5. With the feature flag off, no servo code is reachable and the binary builds without it.

## Fences

- Actuation routes to the native Servo browser-use lane (docs/plans/servo-browser-use-agent/, jobs 007 through 009), which shares the engine and the theorem-browser-agent crate with this phase. This phase ships ingestion tabs; the co-browse and autopilot surfaces ride the same agent-tab interface when that lane lands. No external Browser Use dependency and no Chromium anywhere.
- No crawling UI; single-tab ingestion only in this phase.
- The standing no-graph-view fence holds.
