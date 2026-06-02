# theorem-harness-node

The Node.js (NAPI-RS) binding over the `theorem-harness` Rust SDK. THPS-012
slice 1 of the SDK v2 plan (`docs/plans/theorems-harness-plugin-switchover/`).

This crate is a thin idiomatic shell: it holds no harness logic, only the FFI
marshalling. Every method delegates straight to `theorem-harness`
(`RunHandle`, `RunStream`, `IdempotencyToken`), so the Node surface cannot drift
from the Rust core. This is the binding that retires the plugin's hand-written JS
clients: the plugin calls these native methods instead of re-implementing run
logic in JavaScript.

## Why standalone

This is a standalone single-crate workspace (its own `[workspace]` table), kept
OUT of the `rustyredcore_THG` workspace members, the same isolation convention as
`apps/browser` and `apps/theorem-grpc`. The napi dependency tree therefore does
not couple the main workspace build or CI. The crate path-deps into
`rustyredcore_THG/crates/theorem-harness`.

## Surface (slice 1)

A `Harness` class over an in-process graph store:

| JS method | Delegates to | Returns |
|---|---|---|
| `new Harness(dataDir)` | `RedCoreGraphStore::open(dataDir)` (durable, AOF) | the harness |
| `startRun(task, actor, idempotencyKey)` | `RunHandle::start` | run id (string) |
| `cancel(runId, reason, idempotencyKey)` | `RunHandle::cancel` | void |
| `eventsJson(runId)` | `RunHandle::events` + `export_run_trace` | JSON array string |
| `pollText(runId, afterSeq)` | `RunStream::resume_from(..).poll_text` | new text (string) |

The harness is durable: state persists to a `RedCoreGraphStore` opened from
`dataDir` (AOF-backed, recovered on open). A run written in one process is visible
to the next, as the two-process durability test below proves. The store type
appears only in this binding; the SDK surface is store-agnostic.

## Build and smoke

No napi CLI required: `build.rs` calls `napi_build::setup()`, which emits the
macOS `dynamic_lookup` link args, so plain `cargo build` produces a loadable
`.node`.

```bash
# from repo root
cargo build --manifest-path apps/theorem-harness-node/Cargo.toml
cp apps/theorem-harness-node/target/debug/libtheorem_harness_node.dylib \
   apps/theorem-harness-node/theorem_harness_node.node
node apps/theorem-harness-node/smoke.mjs   # prints SMOKE PASS

# or, from this directory
npm run build:debug && npm run smoke
```

The smoke test proves the round-trip JS -> Rust SDK -> GraphStore -> JS: start a
run, read its events, cancel it, and read the text projection back (`SMOKE PASS`).

Durability is proven across two processes (process 1 writes and exits, process 2
recovers from the AOF):

```bash
DIR=$(mktemp -d)
RUNID=$(node smoke.mjs "$DIR" | grep '^RUNID=' | cut -d= -f2)
node recover.mjs "$DIR" "$RUNID"   # fresh process: RECOVER PASS
```

## Deferred (named, not stubbed)

- `.d.ts` generation and the npm package shape via the `@napi-rs/cli` (slice 1
  ships the working addon + a hand-written method table; the CLI adds typed
  declarations and prebuilt-binary distribution).
- The live async stream: `pollText` / `eventsJson` are the synchronous cursor;
  wrapping them in a Node async iterator (a tokio-backed push stream) is the next
  binding slice.
- The remaining SDK surface (`Session`, full `Event` objects rather than JSON).

## Cross-agent note

The plugin swap (point `theorems-harness/mcp/server.mjs` at this binding instead
of its hand-written JS clients) is Codex's lane. This crate is the native surface
that swap targets.
