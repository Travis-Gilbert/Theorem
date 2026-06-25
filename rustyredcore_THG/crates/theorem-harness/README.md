# theorem-harness

The SDK v2 idiomatic Rust surface over `theorem-harness-core` (the pure run state machine, guards, hashes, contracts) and `theorem-harness-runtime` (GraphStore-backed persistence). Synchronous and GraphStore-backed, with no async runtime.

This crate is the SDK v2 source of truth: the bindings for Python and Swift (UniFFI), Node (NAPI-RS), and the browser (wasm-bindgen) are generated from this surface, so there is no hand-maintained per-language contract that can drift.

## Surface

- `RunHandle` (`run.rs`): a run as a sequence of typed events. `start`, `attach`, `append`, `events`, `events_since(store, after_seq)` (the resumable primitive), `state`, `replay`, `cancel`, `cancel_token()`.
- `RunStream` (`stream.rs`): a resumable, poll-based cursor over a run's events (`poll`, `poll_text`); the synchronous core each binding wraps into a language-native async stream.
- `Session` (`session.rs`): continuity over an AgentBinding scope (`open`, `with_tenant`, `remember`, `recall`).
- `Event` / `RunEventKind` (`event.rs`): the typed view of the transition log.
- `IdempotencyToken` (`idempotency.rs`), `CancelToken` (`cancel.rs`, runtime-free polled flag).
- Trace export (`export.rs`): `export_run_trace`, `export_run_sft`, `export_preference_pair`.

Re-exports `MemoryRecallItem` and `RememberMemoryReceipt` from runtime so SDK consumers need not depend on runtime directly. Path deps: `rustyred-thg-core`, `theorem-harness-core`, `theorem-harness-runtime`.

The live async push-stream is intentionally not in this crate: it belongs to the binding layer (NAPI's tokio, UniFFI's RustFuture), so the core surface never drags an async runtime across the FFI boundary. The resumable primitive that is here is `RunHandle::events_since`, a synchronous bounded replay every binding wraps into its own stream.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p theorem-harness
```

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
