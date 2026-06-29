# Execution Spec: Proxy-Resident Capabilities (transparent affordances, cascade, verification)

Date: 2026-06-27. Register: execution. Read `CONVENTIONS.md` first; its rules apply. Builds on `SPEC-LOCAL-PROXY-MVP.md`. These are the capabilities that run once the proxy exists; the launch proxy ships without them.

## Purpose

Once the proxy sits on every turn, three capabilities become reachable that the MCP boundary cannot reach. The proxy can run the harness affordances itself so the user gets them without installing the MCP. The proxy can route a turn to the local model and escalate only when needed. The proxy can check the model's output against the substrate and tell the model what the substrate found. Each runs inside the proxy, transparent to the client, at the cache-stable position the launch proxy already respects.

## Governing principle

The proxy resolves and checks; it does not block by default. An affordance the proxy runs returns its result into context the same as any tool result, and a consequential or irreversible action routes to the phone authorization surface rather than executing silently. A verification finding is advisory context the model can act on, not a gate that fails the turn, until the corpus shows blocking is safe. Routing to the local model is transparent to the client, and the credential and topology rules of the launch proxy hold unchanged.

## What exists (do not rebuild)

- The launch proxy (`theorem-proxy`): the Messages surface, the native-tool membrane, ambient injection, the cache-stable suffix, the two-token separation.
- The affordance router: `rustyred-thg-affordances` with `tool_search` and `invoke`, the same surface the federated MCP exposes.
- The local model host (`theorem-localmodel`): the local Gemma inference loop, always running with the local node.
- The matrix and symbolic core: `rustyred-thg-graphblas`, where graph consistency and Datalog-style reachability run as matrix operations.
- The phone authorization surface and the binding action tiers in `theorem-harness-core/src/agent_binding.rs`.
- The behavior corpus (`SPEC-BEHAVIOR-CORPUS.md`), which produces the scored difficulty and outcome labels the cascade calibrates on.

## Deliverables

### 1. Transparent affordance execution
Build: the proxy injects a selected set of harness affordances from `rustyred-thg-affordances` into the outgoing request's tools array. When the model emits a `tool_use` for one of those, the proxy intercepts it, does not forward it to the client, resolves it against the affordance router itself, and returns the `tool_result` into context, looping until the model proceeds. The client never installs the MCP and never sees these as external tools. Injection of the tool definitions lands at the cache-stable position and preserves prefix caching. A consequential or irreversible affordance (the binding's tier two and tier three) routes to the phone authorization surface and waits for approval before the proxy executes it.
Acceptance: a Claude Code session with no MCP installed calls a harness affordance, the proxy resolves it and returns the result, the session continues, and a tier-three affordance waits for phone approval before executing. Verify both a reversible affordance resolving inline and a tier-three affordance holding.

### 2. The cascade (gated on corpus calibration)
Build: the proxy routes a turn to the local model when the turn is within the local model's competence and escalates to the upstream Anthropic model otherwise, transparent to the client. The routing decision uses an isotonic-calibrated confidence over the turn, and the calibrator is fit on the behavior corpus's scored difficulty and outcome labels. This deliverable depends on the corpus producing that calibration data; it lands after `SPEC-BEHAVIOR-CORPUS.md` deliverables 3 and 5. The escalation is seamless to the client, which sees one response.
Acceptance: a turn the calibrator scores as within local competence is served by the local model and a turn above the threshold is escalated, the client sees a single coherent response in both cases, and the calibrator was fit on corpus labels rather than a hand-set constant. Verify a routed pair and confirm the calibration source.

### 3. Verification offload (advisory-first)
Build: the proxy runs the model's output through substrate checks before or alongside returning it: graph consistency (does the asserted claim match the graph), Datalog-style reachability over the symbolic layer in `rustyred-thg-graphblas`, and constraint checks where a checker applies. Findings inject as advisory context (the substrate flags this claim, with the basis) at the cache-stable position, so the model can revise, rather than failing the turn. Advisory-first is the named posture; blocking is not in this deliverable and waits on corpus evidence that a check is reliable enough to gate on.
Acceptance: a model output containing a claim the graph contradicts produces an advisory finding injected into context with the contradicting basis, the turn is not failed, and the model can act on the finding. Verify with a constructed contradiction, and confirm no turn is blocked.

## Build Table

| # | Current state | Feature | Location | Action | Desired outcome | Test |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Affordances reachable only via installed MCP | Proxy injects and resolves affordances itself, tier-2/3 to phone | `theorem-proxy` + `rustyred-thg-affordances` + `agent_binding.rs` | Build | No-MCP session uses harness affordances; tier-3 holds for phone | [-] |
| 2 | All turns hit upstream | Cascade to local model with corpus-calibrated escalation | `theorem-proxy` + local model host + `rustyred-thg-ml` | Build | Easy turn served locally, hard turn escalated, one coherent response | [-] |
| 3 | No output check | Advisory verification against graph and symbolic layer | `theorem-proxy` + `rustyred-thg-graphblas` | Build | Contradicted claim gets an advisory finding, turn not blocked | [-] |

Test legend: `[-]` open, `[x]` verified against the acceptance criterion, `[~]` deferred with a reason that names a real external blocker.

## Dependencies and gates

Deliverable 1 is reachable as soon as the launch proxy exists. Deliverable 2 is gated on the behavior corpus producing calibration labels (`SPEC-BEHAVIOR-CORPUS.md` deliverables 3 and 5); the gate is real, not stylistic, because routing on an uncalibrated threshold degrades quality. Deliverable 3 is reachable on the launch proxy plus the symbolic layer, and stays advisory until corpus evidence supports gating.

## Verify first

Confirm the `rustyred-thg-affordances` `invoke` contract the proxy will call, the tier mapping in `theorem-harness-core/src/agent_binding.rs` and how the phone authorization hold is signaled, the local model host's invocation surface for routing, and the Datalog and constraint-check surface in or beside `rustyred-thg-graphblas`. Build against the real surfaces.

## Where it lands

- Affordance injection and resolution, cascade routing, verification orchestration: `theorem-proxy`.
- Affordance execution: `rustyred-thg-affordances`.
- Local model routing target: the renamed local model host.
- Calibration: `rustyred-thg-ml`, fit on the behavior corpus.
- Graph and symbolic checks: `rustyred-thg-graphblas`.
- Authorization: `theorem-harness-core/src/agent_binding.rs` and the phone surface.
