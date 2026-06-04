# Theorem Harness SDK v2 Architecture Overlay

Date: 2026-06-02
Status: architecture overlay for the Theorem RustyRed switchover plan
Source: `/Users/travisgilbert/Downloads/theorem-harness-sdk-v2-spec.md`
Coordination room: `theorem-rustyred-plugin-switchover`

## Decision

The Theorem RustyRed switchover is the first runtime-substrate slice of
`theorem-harness` SDK v2. It is not an endpoint swap.

SDK v2 is one intentional SDK family named `theorem-harness`: a Rust core as
the source of truth, generated bindings for Python, TypeScript/Node, browser
WASM, and Swift, plus thin idiomatic shells per language. The current THPS
implementation plan remains useful, but it is now the migration-mechanics lane
under this SDK v2 architecture.

## Layer Model

| Layer | Role | Current implication |
|---|---|---|
| `theorem-harness-core` | Pure Rust kernel: state machines, guards, state hashes, contracts | Stabilize the SDK v2 API here before generating bindings |
| `theorem-harness-runtime` | GraphStore-backed persistence, working-memory scopes, receipt writing | Theorem RustyRed becomes the durable runtime substrate |
| `theorem-harness` Rust SDK | Idiomatic Rust surface over core plus runtime | This is the first real SDK surface, not just an internal crate |
| Generated Python and Swift | UniFFI bindings plus thin shells | Keep the FFI boundary slim and runtime-neutral |
| Generated TypeScript/Node | NAPI-RS v2 bindings plus `.d.ts` types | Do not hand-maintain a divergent JS SDK contract |
| Browser SDK | wasm-bindgen over the same core logic | Browser clients share the same harness semantics |
| Plugin MCP/router | Host compatibility layer and route-policy adapter | Useful dogfood path, but not the SDK contract |
| Native MCP/HTTP transports | Operational transports over the runtime | Adapters for hosts and products, not the source of truth |

## SDK Primitives

SDK v2 should expose a small modern agent-SDK surface:

- Streamed runs: start a run and consume typed events. Provide both the full
  typed-event stream and a text-only convenience stream.
- Sessions: continuity handles over AgentBinding working-memory scopes. Runs in
  a session share in-flight working memory; committed graph state crosses
  sessions.
- Affordances, skills, and runs: the three first-class objects exposed by the
  substrate. Affordances are learned tool opportunities, skills are encoded
  abilities, and runs exercise both under the charter.
- Skill operations: submit corpora for encoding, list available skills, invoke
  skills, and feed run outcomes back into the compounding loop.
- Idempotency: every state-changing call takes a client-provided idempotency
  token and resolves retries to the existing result.
- Resumable streaming: every event carries a monotonic sequence number so
  clients can reconnect from the last observed event.
- Cancellation: cancellation is a core operation with a cancel handle and a
  clean transition such as `RUN.CANCELLED`.
- Runtime-neutral async: core awaits only neutral primitives. Network and model
  I/O live behind traits supplied by runtime crates or bindings.
- Receipts and provenance: every event carries graph version, selected
  affordances or skills, evidence, and state hash context.
- Trace export: runs and sessions can export event traces as training data for
  the encoding and Pairformer loops.

## Ordering Rule

Stabilize the Rust core API before generating bindings.

The multi-language bindings are the last step of the first SDK v2 tranche, not
the first. Generating bindings against a moving core would create churn and
false parity. The correct order is:

1. Define the stable Rust core API for streamed runs, sessions, affordances,
   skills, idempotency, cancellation, resumability, receipts, and trace export.
2. Extend runtime persistence over `GraphStore` so the core API has durable
   event, session, idempotency, and trace stores.
3. Route plugin and host compatibility layers through the native runtime while
   preserving user-facing verbs.
4. Validate the Rust API and runtime with parity, replay, and dogfood tests.
5. Generate bindings once the core surface is stable.
6. Add thin idiomatic shells for Python, TypeScript/Node, browser WASM, and
   Swift.

## Impact On The THPS Plan

- THPS should no longer be framed as "Theseus RustyRed to Theorem RustyRed" by
  itself. It is the substrate migration lane of SDK v2.
- The native `rustyred-thg-mcp` and `apps/theorem-harness-server` surfaces are
  transports and product adapters. They should reflect the SDK contract, not
  define it accidentally.
- The Claude plugin proxy is a temporary host-compatibility layer. It is useful
  dogfood and should remain until the deployed native MCP schema is
  Claude-compatible, but it is not part of the SDK's permanent contract.
- The first real implementation target is a stable Rust SDK contract over
  `theorem-harness-core` and `theorem-harness-runtime`, not a broad rewrite of
  every plugin verb.
- Existing plugin verbs remain valuable as compatibility affordances. Route
  them through a `HarnessRoutePolicy` that reports whether the call executed
  through native runtime, product HTTP, or explicit Theseus fallback.

## Product Gates

The SDK v2 switchover is ready for binding generation only when these pass:

- `cargo test -p theorem-harness-core`
- `cargo test -p theorem-harness-runtime`
- Native MCP `tools/list` has no top-level `anyOf`, `oneOf`, or `allOf`
- A native run can stream ordered typed events with sequence numbers and
  receipts
- A run can be cancelled and resumed from a recorded sequence boundary
- A state-changing call with the same idempotency token returns the same
  result without duplicating work
- Plugin route-policy tests prove base harness verbs use Theorem RustyRed by
  default and Theseus only by explicit fallback
- Claude and Codex can coordinate in the native room using the same Theorem
  RustyRed substrate
- Trace export produces a training-ready artifact from a run or session

## Resolved Grounding Decisions

Claude grounded three of the original open questions against the live code and
Travis's SDK v2 decisions. The detailed evidence lives in
`sdk-v2-architecture.md`.

- Event sequence source: use the explicit per-run event sequence counter already
  in `theorem-harness-runtime/src/event_log.rs`, not raw GraphStore append order
  or graph version. Resume-from-seq is a bounded replay over that per-run event
  chain.
- Cancellation: `RUN.CANCELLED` already exists as a first-class transition. The
  remaining work is a parity fixture plus a live cancel handle and polled cancel
  channel.
- Binding generation order: Node/NAPI-RS lands first to retire the transitional
  plugin JavaScript shim, then Swift, Python, and browser WASM.

## Open Design Questions

- Which exact Rust types become the SDK v2 public contract, and which remain
  runtime internals? This is the THPS-011 deliverable.
- How should browser WASM surface resumable streams when the backing runtime is
  remote rather than in-browser?
