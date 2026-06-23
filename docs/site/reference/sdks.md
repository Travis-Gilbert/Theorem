# SDKs: Rust, Node, Swift

For embedding the Harness directly in an application — rather than talking to it over HTTP or MCP — Theorem ships native SDKs. They share one Rust core, so no language binding can drift from the others.

## One core, several bindings

| Package | Language | Binding tech | Location |
|---|---|---|---|
| `theorem-harness` | Rust | — (the source of truth) | `rustyredcore_THG/crates/theorem-harness` |
| `theorem-harness-node` | Node.js / TypeScript | NAPI-RS | `apps/theorem-harness-node` |
| `theorem-harness-swift` | Swift (iOS/macOS) | UniFFI | `apps/theorem-harness-swift` |

The Rust crate `theorem-harness` is the **SDK v2 surface** over the harness kernel (`theorem-harness-core`) and runtime (`theorem-harness-runtime`). It adds streamed runs, sessions-as-scopes, idempotency, cancellation, resumable events, and trace export. The Node and Swift packages are thin marshalling layers generated from it; a browser/wasm binding follows the same pattern. Because the bindings are generated from one core, the public surface is the same across languages.

## The shared surface

Every binding exposes the same operations. Names are idiomatic per language (`snake_case` in Rust/Node, `camelCase` in Swift).

| Operation | Purpose |
|---|---|
| `Harness(data_dir)` | Open a durable, file-backed harness store at a directory; recover state on open. |
| `start_run(task, actor, idempotency_key)` | Start a run; returns a `run_id`. |
| `run_status(run_id)` | Current status (created, cancelled, closed, ...). |
| `events_json(run_id)` | The run's event log as JSON. |
| `poll_text(run_id, after_seq)` | A text view of the run from a sequence cursor (for streaming). |
| `cancel(run_id, reason, idempotency_key)` | Cancel a run. |
| `remember(agent_id, kind, title, content)` | Write a memory; returns a receipt. |
| `recall(agent_id, query, limit)` | Retrieve memory by query. |

## Node

```ts
import { Harness } from "theorem-harness-node";

const harness = new Harness("./harness-data");
const runId = harness.start_run("ingest the docs site", "claude-code", "idem-1");

harness.remember("claude-code", "decision", "Docs IA",
  "Adopted Diátaxis split for the GitBook.");

const hits = harness.recall("claude-code", "docs structure", 5);
console.log(harness.run_status(runId), hits);
```

## Swift

```swift
import TheoremHarness

let harness = try Harness(dataDir: "./harness-data")
let runId = try harness.startRun(task: "index the repo",
                                 actor: "ios-client",
                                 idempotencyKey: "idem-1")

try harness.remember(agentId: "ios-client", kind: "note",
                     title: "Offline", content: "Store opened from app sandbox.")

let hits = try harness.recall(agentId: "ios-client", query: "offline", limit: 5)
```

## Rust

```rust
use theorem_harness::Harness;

let harness = Harness::open("./harness-data")?;
let run_id = harness.start_run("compile the graph", "codex", Some("idem-1"))?;
harness.remember("codex", "solution", "Merge", "Three-way merge resolved by confidence.")?;
let hits = harness.recall("codex", "merge", 5)?;
```

## Which surface should I use?

- **MCP** — you have an MCP-capable agent client and want tools wired in. Start with the [MCP tool catalog](mcp-tools.md).
- **HTTP** — you want a language-agnostic network API for runs, rooms, and jobs. See the [HTTP API](api-http.md).
- **SDK** — you are embedding the harness *in-process* in a Rust, Node, or Swift application and want a durable local store with no network hop. This page.
