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

## iOS app consumption (xcframework + SwiftPM)

`./build-xcframework.sh` builds the Rust core for iOS device + simulator and
produces `TheoremHarnessFFI.xcframework` plus the generated Swift API under
`Sources/TheoremHarness/`. The package (`Package.swift`) vends a `TheoremHarness`
library: the iOS app (`apps/theorem-ios`) depends on this directory as a local
SwiftPM package and uses it:

```swift
import TheoremHarness

let harness = try Harness(dataDir: appSupportDir)
let runId = try harness.startRun(task: "...", actor: "ios", idempotencyKey: UUID().uuidString)
let events = try harness.eventsJson(runId: runId)
```

Verified: `xcodebuild -scheme TheoremHarness -destination 'generic/platform=iOS Simulator' build`
=> **BUILD SUCCEEDED** (the generated Swift links against the xcframework's iOS
slice for arm64 device + arm64/x86_64 simulator). The xcframework (80M) and the
generated `Sources/TheoremHarness/` are build artifacts (gitignored); regenerate
with `./build-xcframework.sh`.

## Python binding (same crate, third language)

UniFFI generates from one annotated facade, so this **same crate** also produces a
Python binding (snake_case, idiomatic) with no extra Rust. Despite the `-swift`
crate name (its primary packaging target is iOS), it is the UniFFI binding crate.

```bash
cargo build --release --lib
cargo run --release --bin uniffi-bindgen -- generate \
  --library target/release/libtheorem_harness_swift.dylib --language python \
  --out-dir generated-python
cp target/release/libtheorem_harness_swift.dylib generated-python/   # next to the .py
python3 smoke.py   # prints SMOKE PASS
```

The Python module loads the dylib via `ctypes` (no extra Python deps). Verified:
`python3 smoke.py` => **SMOKE PASS** (start, events, cancel, status=cancelled,
remember+recall). So one Rust core now drives **Node (NAPI) + Swift + Python (both
UniFFI)**, all passing the identical lifecycle, none re-implementing harness logic.

## Deferred (named, not stubbed)

- The async streaming surface (a Swift `AsyncSequence` over the run event cursor),
  mirroring the Node `streamRun`.
- Hosting the xcframework as a remote binary target (url + checksum) instead of a
  local build, for distribution beyond this repo.

## Cross-agent note

The iOS app (`apps/theorem-ios`, on TestFlight) is the consumer. Wiring this
binding into that app's SwiftPM targets is the iOS lane.
