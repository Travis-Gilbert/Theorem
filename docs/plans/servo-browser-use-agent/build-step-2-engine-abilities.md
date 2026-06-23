# Servo Browser-Use, build step two: abilities only the engine can do (job-008)

**Repo:** Travis-Gilbert/theorem
**Audience:** Claude Code + Codex, building as one agent
**Plan home:** docs/plans/servo-browser-use-agent/
**Builds on:** build-step-1-parity.md (job-007). Parity and the engine-native reader/executor exist.
**Engine assumption:** the same fork-plus-harness-note posture as step one. New engine surface area here is contributed to the fork; the generic parts are upstreamable later, the agent-specific parts stay in the fork.
**Job linkage:** job-008, kind Feature, priority P1, target_head Either.

## North star

Browser Use drives Chromium from outside through CDP. It cannot see what the engine knows internally, only what CDP exposes. Owning an embeddable engine means the agent reads and acts on internal engine state that no CDP-based agent can reach. These are the abilities that justify the Servo bet. Each is a capability Browser Use structurally cannot match, not a faster version of something it has.

## Deliverables

### D1: engine-truth interactability (beyond the DOM guess)
CDP agents infer clickability from DOM heuristics and screenshots. The engine knows the truth from layout and hit-testing.
- A real occlusion/hit-test query: for an element_id, ask the engine whether a point in its bounds actually hits that node or something painted over it. This removes the single largest class of Browser Use failure (clicking an element covered by an overlay, cookie banner, or modal).
- Effective visibility from computed style and layout, not a screenshot diff: display, visibility, opacity, viewport clipping, zero-size, all read from the box tree.
- Output: PageState interactive_elements gain occluded:bool and truly_clickable:bool sourced from the engine, not inferred.

### D2: the layout-sourced change signal (deterministic, not polled)
Browser Use re-reads state after actions and polls for stability. The engine already computes exactly what changed during layout invalidation.
- Surface a settle signal from the engine: layout/paint quiescence after an action, so WaitFor resolves on real engine quiescence instead of a sleep or a DOM-mutation guess.
- A precise post-action diff: which nodes changed, appeared, or were removed as a direct consequence of the last action, sourced from the TreeUpdate plus layout damage. The driving model gets "this is what your click did," not "here is the whole page again."
- This is both a correctness win (no flaky waits) and a token win (send the diff, not the page).

### D3: render-tier control (the engine is yours to tune)
- Resource policy per session: block images, fonts, media, or third-party subresources at the net layer for fast text-first ingestion runs; full render only when vision or layout-truth is needed. CDP can do some of this awkwardly; in an owned engine it is a first-class session knob.
- A headless render tier and a headed co-browse tier from the same engine, switched per session, sharing the reader and executor.
- WebGL/WebGPU availability noted: Servo supports both, so canvas/3D pages render where many headless stacks fail.

### D4: deep extraction from the engine, not the serialized DOM
- Pull computed text with layout-aware reading order (the box tree gives true visual order, which beats DOM-order extraction on multi-column and reordered layouts).
- Shadow DOM and cross-document (the accessibility tree already grafts child-document trees into the parent per the engine's grafting work), so iframes and web components are one coherent tree to the agent, where CDP needs per-frame juggling.
- Stable cross-navigation node identity where the engine can carry it, so a tracked element survives a soft navigation.

### D5: engine-native record and replay
- Record the TreeUpdate sequence, the issued ActionRequests, and the settle signals as a content-addressed BrowsingRun coupled to the harness replay. Because the engine is deterministic where we drive it, replay is faithful at the accessibility layer, not a screenshot reel.
- This is the verification artifact Browser Use cannot produce: a replayable, inspectable web run, not a video.

## Acceptance criteria

1. An element under a cookie-banner overlay reports occluded:true and is not clicked blind; the agent dismisses the overlay first.
2. WaitFor resolves on an engine settle signal; a known-flaky timing case that needs sleeps in a CDP agent passes here without sleeps.
3. A post-action PageState carries a diff of exactly what the last action changed, and the full page is not re-sent.
4. A session with images/media blocked ingests a heavy page materially faster than the same page fully rendered, measured and recorded.
5. Reading order from a multi-column page matches visual order, not DOM order.
6. An iframe's interactive elements appear in one PageState tree with the parent, addressable by stable id.
7. A BrowsingRun replays from the recorded TreeUpdate/Action/settle sequence and reproduces the run at the accessibility layer.

## Fences

- Generic engine improvements (interactability truth, settle signal, reading order) are candidates to upstream later; the agent-coupling (BrowsingRun shape, session resource policy tied to the harness) stays in the fork.
- No hard fork of Servo: surgical patches plus the embedding API, tracked as a thin patch set, exactly the parent plan's posture.
- This is engine-derived ability. Substrate-derived ability (graph, epistemics, background compute) is step three (job-009).
- Standing no-graph-view fence holds.

## Where it rides

rustyred-web for the engine surface and the reader/executor extensions; harness runtime for the BrowsingRun replay coupling. Engine-side additions are fork commits with harness notes per the working posture.
