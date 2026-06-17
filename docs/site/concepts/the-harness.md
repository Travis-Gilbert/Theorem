# The Harness: one agent, several heads

The Harness is the part of Theorem that lets agents share the substrate. It is how a coding agent keeps memory across sessions, coordinates with other agents on the same repository, and leaves a replayable record of every run.

## The unit model

The Harness treats Claude Code, Codex, and the claude.ai surface not as separate agents but as several heads of one agent. The unit has one identity, one shared scratchpad the heads append to, and one budget. The heads are hands.

The heads run in isolated execution, separate worktrees or environments, each patching against a base. That fence stays, because a source file has no semantic merge and two hands on the same bytes lose work. What unifies the heads across the fence is not a shared working tree, it is shared awareness. A head announces what it is doing before it acts, names the surface its hands are on, and builds on a peer's completed edit instead of redoing it.

The model is deliberately not "several agents dividing files into lanes." Lanes produce duplicate work and stale handoffs. Frequent announcement over a shared room produces convergence. The phrase for it is frequency over fences.

## What the Harness provides

- Persistent memory. A head records a load-bearing decision, a constraint, a convention, or a thing ruled out, and the next head and the next session inherit it instead of rediscovering or re-deciding it.
- A coordination room. Heads read the room at turn start (intents, reflections, open tensions), announce their footprint at the beginning of work, and close the announcement as a handoff at turn end.
- A semantic-overlap guard. When two heads' announced footprints touch structurally coupled code, the substrate emits a tension. This catches the one failure that isolation and text merge both miss: edits that merge cleanly and still disagree at runtime.
- A typed event log. Every run is a sequence of transitions with content-addressed hashes, persisted as graph nodes and append-chain edges, so a run can be replayed or forked.
- A job board. Work is handed from a planning surface to an executing head as a typed job; a receiver spawns the locally authenticated CLI to run it.

## How it is built

The Harness is layered so storage stays out of the parity kernel.

`theorem-harness-core` is the pure kernel: run state, transition guards, content-addressed state hashing, replay and fork helpers, and permission-aware toolkit selection. It is pure logic, parity-tested against the Python reference corpora.

`theorem-harness-runtime` persists the kernel's transition receipts into a `GraphStore` as run and event nodes, and carries the coordination primitives (rooms, intents, presence, messages) and the job queue.

`theorem-harness` is the SDK v2 surface over the two. It is the single source of truth from which the Node (NAPI-RS), Swift (UniFFI), and browser (wasm-bindgen) bindings are generated, so no per-language SDK can drift from the core.

The transports expose all of this: a native Rust MCP server (`rustyred-thg-mcp`) and an Axum HTTP server (`apps/theorem-harness-server`).
