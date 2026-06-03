# theorem-harness-swift

The Swift (UniFFI) binding over the `theorem-harness` Rust SDK. The second
language proof of the SDK v2 plan: the Node binding (`apps/theorem-harness-node`,
NAPI-RS) and this Swift binding (UniFFI) wrap the **same** Rust core with
**different** generators, so the two cannot drift.

This crate holds no harness logic, only a thin facade. Every method delegates to
`theorem-harness` (`RunHandle`, `Session`, `IdempotencyToken`), and the Swift API
is *generated* from that facade by `uniffi-bindgen`, not hand-written. It is the
native harness surface for the Theorem iOS app.

## Why standalone

Standalone single-crate workspace (own `[workspace]` table), kept OUT of the
`rustyredcore_THG` workspace members, like `apps/browser` and the Node binding,
so the UniFFI dependency tree stays off the main build/CI. Path-deps into
`rustyredcore_THG/crates/theorem-harness`.

## Surface

A `Harness` UniFFI object over a durable RedCore store (UniFFI cannot export
generics, so the facade picks the concrete store; the SDK is store-agnostic):

| Swift | Delegates to |
|---|---|
| `Harness(dataDir:) throws` | `RedCoreGraphStore::open` |
| `startRun(task:actor:idempotencyKey:) throws -> String` | `RunHandle::start` |
| `cancel(runId:reason:idempotencyKey:) throws` | `RunHandle::cancel` |
| `eventsJson(runId:) throws -> String` | `RunHandle::events` + `export_run_trace` |
| `runStatus(runId:) throws -> String` | `RunHandle::state().status` |
| `remember(agentId:kind:title:content:) throws -> String` | `Session::remember` |
| `recall(agentId:query:limit:) throws -> String` | `Session::recall` |

Errors surface as `enum HarnessError: Error`.

## Build, generate, smoke

```bash
# from this directory
cargo build
cargo run --bin uniffi-bindgen -- generate \
  --library target/debug/libtheorem_harness_swift.dylib \
  --language swift --out-dir generated
swiftc -parse-as-library -emit-executable -o smoke_swift \
  -L target/debug -ltheorem_harness_swift \
  -Xcc -fmodule-map-file=generated/theorem_harness_swiftFFI.modulemap -I generated \
  generated/theorem_harness_swift.swift smoke.swift
DYLD_LIBRARY_PATH=target/debug ./smoke_swift   # prints SMOKE PASS
```

The smoke proves the round-trip Swift -> generated bindings -> Rust SDK ->
RedCore -> Swift: start a run, read its events, cancel it, read the status, then
`remember` and `recall` a memory. The generated `generated/` dir and the compiled
`smoke_swift` are build artifacts (gitignored); `src/lib.rs` is the source of truth.

## Deferred (named, not stubbed)

- An `.xcframework` + Swift Package wrapper for direct Xcode/SwiftPM consumption
  (this slice generates the bindings and proves they run; packaging for the iOS
  target is the next step). It needs a multi-arch build (device + simulator) and
  `xcodebuild -create-xcframework`.
- The async streaming surface (the Swift `AsyncSequence` over the run event
  cursor), mirroring the Node `streamRun`.

## Cross-agent note

The iOS app (`apps/theorem-ios`, on TestFlight) is the consumer. Wiring this
binding into that app's SwiftPM targets is the iOS lane.
