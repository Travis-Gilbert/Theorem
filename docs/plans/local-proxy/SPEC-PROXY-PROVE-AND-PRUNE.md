# Execution Spec: Prove and Prune (proxy-resident memory ranking, tool pruning, proactive coordination, staleness, and self-measurement)

Date: 2026-06-28. Register: execution. Read `CONVENTIONS.md` first; its rules apply. Builds on `SPEC-LOCAL-PROXY-MVP.md` (deliverable 1 shipped: `apps/theorem-proxy`, faithful Anthropic Messages passthrough) and `SPEC-PROXY-RESIDENT-CAPABILITIES.md`.

## Purpose

The proxy makes the harness ambient. This spec is the five changes that make it ambient *well*: inject the right memory not all of it, advertise only the tools that are genuine actions, push coordination instead of polling it, keep memory honest as code moves, and measure the whole thing so the benefit is provable instead of believed. The first four make the harness cheaper and more correct; the fifth makes it auditable. Together they answer the open product question -- does the harness help, and at what cost -- with numbers from the proxy rather than vibes.

These deliverables came out of a working session (2026-06-28) where the harness's value and its costs were both visible in one run: ambient memory genuinely shaped good first moves, while a large unused MCP tool surface taxed every turn, the coordination remote was degraded for half the session, and a shared file went red under one head mid-edit with no proactive signal. Each deliverable below targets one of those observed facts.

## Governing principle

Everything ambient runs at the cache-stable suffix the launch proxy already respects: never mutate the system prefix or the tools array, fail open. Measurement is non-interfering: the A/B arm and the counters observe, they never change a turn's returned result. Pruning removes advertisement, not capability: a removed tool still exists and is still reachable through the affordance router; it is simply not in the model's context unless an action genuinely needs it.

## What exists (verified; do not rebuild)

- The launch proxy: `apps/theorem-proxy`, faithful Messages passthrough at the cache-stable position, streaming and non-streaming, headers / body / SSE preserved. Deliverable 1 of `SPEC-LOCAL-PROXY-MVP` shipped and tested (passthrough + SSE order).
- Graph-native memory with bitemporal validity: `rustyred-thg-memory` carries `valid_at_ms` / `invalid_at_ms` validity edges and `include_expired` filtering. The validity machinery exists; the gap is that it is not wired to the ambient memory the agent actually receives (today a flat `MEMORY.md` is injected wholesale and never passes through validity).
- The affordance router (`rustyred-thg-affordances`: `tool_search` / `invoke`) and the harness MCP tool manifest to be pruned.
- The coordination substrate (per-room streams, intents) and the multi-head traffic the proxy already sits on.
- A retrieval path for relevance ranking: `hippo_retrieve` / the index-context path.

## Deliverables

### 1. Relevance-ranked ambient memory injection (retire wholesale `MEMORY.md`; remove the `recall` tool)
Build: on the incoming turn the proxy retrieves over the tenant's memory, ranks against the current prompt (`hippo_retrieve` / index-context), and injects only the top-k the turn justifies at the cache-stable suffix. The flat `MEMORY.md` dump is retired in favor of per-turn relevance. The `recall` MCP tool is removed: memory is ambient, never elected.
Acceptance: a turn receives only task-relevant memories (a memory irrelevant to the turn is not injected), the cached prefix is unchanged so the provider prompt cache still hits, and no `recall` tool appears in the advertised tools. Verify by comparing the injected memory set for two different prompts against the same store, and confirming the cache-hit usage block across two identical requests.

### 2. Tool-surface pruning to the action set
Build: from a usage audit across a batch of real sessions (which tools actually fire), reduce the advertised harness tool set to the operations that are genuine actions -- write, compute, `invoke`. Everything that is really "inject context" moves to proxy injection (deliverable 1 and the resident-capabilities affordance path). Removed tools remain implemented and reachable through the router; they are just not advertised every turn.
Acceptance: the advertised tool list is the small action set; a never-called tool identified by the audit is absent from the model's context; and the audit (counts per tool across N sessions) is the recorded basis for each cut. Verify against the audit output and the advertised tools list.

### 3. Proxy-mediated proactive coordination (push, not poll)
Build: the proxy sits on every head's traffic. When a turn's imminent action targets a file or symbol another head wrote within a recency window, the proxy injects a heads-up at the suffix ("file X edited by head B 90s ago") before the model acts. This replaces "remember to call `coordination_intent`" and survives the degraded coordination remote, because the signal rides the proxy and the local graph (kept fresh by the notify watcher), not an MCP round-trip.
Acceptance: head B edits file X; within the window, head A's next turn whose action targets X receives the recency injection naming B and the timestamp, with no MCP call. Verify with two proxied sessions sharing one file. (Direct fix for the `standing_seed.rs` red-then-green race of 2026-06-28.)

### 4. Staleness-aware memory and memory CI
Build: each memory records the files / symbols / flags it references. On injection the proxy down-weights or flags any memory whose referenced code changed since the memory was written -- wiring the existing `rustyred-thg-memory` validity layer to the ambient path. A periodic memory-CI re-checks that each memory's named file / symbol / flag still exists and marks the dead ones invalid.
Acceptance: a memory citing a since-changed file is flagged when injected; memory-CI marks invalid a memory whose named symbol no longer exists in the tree; a fresh memory is untouched. Verify by mutating a referenced file and re-injecting, and by running memory-CI over a known-stale memory. This converts silent misdirection (a confidently-recalled but outdated fact) into a visible warning.

### 5. Built-in measurement and a value readout in `doctor`
Build: the proxy measures the harness honestly because it sees every prompt, response, and token count. It A/Bs itself -- a random arm served with injection off -- and tracks: rediscovery rate (a prompt asks what an existing-but-not-injected memory answers, a miss that should have hit), collisions prevented (deliverable 3 fired before a clobber), and tokens spent vs the no-inject arm. `theorem doctor` reports the value, not only connectivity: "this session: injected 3 relevant memories, flagged 1 stale, prevented 1 collision, ~Xk tokens saved vs the no-inject arm."
Acceptance: the proxy serves a random fraction with injection off and records the paired comparison; `doctor` prints the per-session value readout; rediscovery and collisions-prevented are counted over a session; and the A/B never alters a turn's returned result. Verify the off-arm assignment, the counters, and the doctor output.

## Build Table

| # | Current state | Feature | Location | Action | Desired outcome | Test |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | `MEMORY.md` dumped wholesale; `recall` is an MCP tool | Relevance-ranked top-k injection at the suffix; `recall` removed | `apps/theorem-proxy` + retrieval path | Build | Only task-relevant memory injected, cache still hits, no `recall` tool | [~] local proxy/MCP tests green; provider cache smoke script added; live run needs `ANTHROPIC_API_KEY` |
| 2 | Dozens of tools advertised, ~4 used per session | Advertise only the action set; context tools move to injection | harness MCP manifest + affordance router | Build | Never-called tools absent from context; cuts backed by the audit | [-] |
| 3 | Coordination is poll-and-remember; remote degrades | Proxy injects a recency heads-up before an action targets a contended file | `apps/theorem-proxy` + coordination graph | Build | Head A warned of head B's recent edit pre-action, no MCP call | [-] |
| 4 | Ambient memory never passes through validity; can misdirect | Reference-tracked staleness flagging + periodic memory-CI | `apps/theorem-proxy` + `rustyred-thg-memory` | Build | Stale memory flagged on injection; dead memory marked invalid | [-] |
| 5 | No way to measure if the harness helps | Proxy A/B + rediscovery / collisions / tokens counters + `doctor` readout | `apps/theorem-proxy` + `theorem doctor` | Build | The benefit is a number per session, not a vibe | [-] |

Test legend: `[-]` open, `[x]` verified against the acceptance criterion, `[~]` deferred with a reason that names a real external blocker.

## Dependencies and gates

- D1 needs the launch proxy (shipped) plus a retrieval path (`hippo_retrieve` / index-context, exists).
- D2 needs the usage audit (in progress on a separate head's branch); the cut is evidence-gated, not by gut.
- D3 needs the proxy seeing multi-head traffic plus a recency index of edits (the notify watcher keeps the graph fresh).
- D4 wires the existing bitemporal validity layer to the ambient path; the memory-CI is new but cheap.
- D5 builds on D1 and D3; the `doctor` readout extends the onboarding `theorem doctor` command (`SPEC-ONECLICK-ONBOARDING` deliverable 5).

## Verify first

Confirm before building: the `hippo_retrieve` / index-context retrieval signature and the relevance score it returns; the `rustyred-thg-memory` validity-edge API (`valid_at_ms` / `invalid_at_ms`) and how `include_expired` filters on the read path; the harness MCP manifest source and which tools the affordance router can resolve without advertisement; the coordination stream / recency surface the proxy reads for D3; and the `theorem doctor` command surface the readout extends. Build against the real surfaces.

## Where it lands

- Ranking, injection, A/B, counters, proactive pushes, staleness flagging: `apps/theorem-proxy`.
- Retrieval and bitemporal validity: `rustyred-thg-memory` and the index-context path.
- Tool pruning: the harness MCP manifest and the affordance router.
- Doctor readout: the `theorem doctor` command (`SPEC-ONECLICK-ONBOARDING`).
