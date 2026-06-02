# THPS-011: theorem-harness Rust SDK surface (freeze)

Date: 2026-06-02
Author: Claude Code
Status: slice 1 shipped and validated; surface frozen for binding generation
Crate: `rustyredcore_THG/crates/theorem-harness`
Parent: `sdk-v2-architecture-overlay.md` (ordering rule: stabilize the Rust core
SDK surface before generating bindings)

## What this is

The `theorem-harness` Rust crate is the SDK v2 source of truth: the idiomatic
Rust surface over `theorem-harness-core` (pure run state machine) and
`theorem-harness-runtime` (GraphStore-backed persistence). The Python (UniFFI),
Node (NAPI-RS), Swift (UniFFI), and browser (wasm-bindgen) bindings are GENERATED
from this surface in THPS-012, so they cannot drift. This is the crate the
binding generation waits on, which is why its surface is frozen first.

## Frozen surface

| Type | Role | Backed by |
|---|---|---|
| `RunHandle` | A run as a sequence of typed events: `start`, `append`, `events`, `events_since` (resume), `state`, `replay`, `cancel`, `attach`, `cancel_token` | `theorem-harness-runtime` event log (`append_transition_from_store`, `load_events`, `load_run`, `replay_persisted_run`) |
| `Session` | Continuity handle over an AgentBinding scope; within-session scratchpad vs across-session published scope | `theorem-harness-core` `BindingMemoryScope` / `PublishedScope` |
| `Event` / `RunEventKind` | Typed view over the canonical transition log, carrying seq and post-transition state hash | `theorem-harness-core` `EventState` |
| `IdempotencyToken` | Client-provided token threaded onto every state-changing call | `theorem-harness-core` `TransitionInput.idempotency_key` |
| `CancelToken` | Runtime-free polled flag checked before each append; drives a clean `RUN.CANCELLED` | std `Arc<AtomicBool>` |
| `export_run_trace` / `TraceRow` | A run's events exported as training rows | the typed event log |

## Real now vs deferred (no stubs presented as working)

Real and tested this slice:
- The full run lifecycle over a real `GraphStore`: start -> read events ->
  resume from a sequence boundary -> read state -> deterministic replay ->
  cancel -> refused-append-after-cancel. (`run::tests::start_read_resume_replay_then_cancel`)
- Cancellation flag semantics, idempotency token wire shape, typed event mapping,
  sessions over binding scopes, and trace export to JSON rows.

Deferred and named (not stubbed, with their owning step):
- Live async push-stream (subscribe-and-await new events): the binding layer's
  job (NAPI tokio, UniFFI RustFuture polling). The core resumable primitive
  `RunHandle::events_since` is what each binding wraps. Keeps the core
  runtime-free per the SDK v2 async discipline.
- Idempotency short-circuit ENFORCEMENT (return the prior result for a seen
  token): a runtime delta, THPS-003. Requires the event log to persist the token
  on the event so a repeat is detectable. The token wire shape is frozen now so
  enforcement lands behind a stable surface.
- Richer SFT / preference-pair export shaping: THPS-013, on top of the lossless
  `TraceRow`.

## Why the surface maps cleanly (grounding)

The SDK v2 substrate was already ~70% built (see `sdk-v2-architecture.md`). This
crate is a thin idiomatic surface, not new substrate: `events_since` filters the
existing per-run `seq`; `cancel` drives the existing `RUN.CANCELLED` transition;
`Session` wraps the existing binding scopes; `replay` calls the existing
`replay_persisted_run`. That is why slice 1 is real rather than scaffolding.

## Validation receipt

- `cargo build -p theorem-harness`: clean.
- `cargo test -p theorem-harness`: 13 passed, 0 failed.
- `cargo clippy -p theorem-harness --all-targets --no-deps -- -D warnings`: clean.
- `cargo fmt -p theorem-harness -- --check`: clean.
- `git diff --check`: clean.

## Next

- THPS-012: generate the Node binding (NAPI-RS) from this surface; it retires the
  plugin's hand-written JS shim. Then Swift / Python (UniFFI), then wasm.
- THPS-005 runtime side: persist the idempotency token on events to enable
  enforcement; add the live push-stream in the binding layer.
- The remaining overlay open question ("which exact Rust types are the public
  contract vs runtime internals") is answered concretely by this crate's public
  exports.
