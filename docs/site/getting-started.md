# Getting started

Theorem is a polyglot repository with no single root build. You pick the workspace or crate for the task. This page covers the most common entry points.

## Fast path: install the dev front door

For a fresh developer machine, install the `theorem` wrapper and start
`theorem-localmodel` with no-secret defaults:

```bash
curl -fsSL https://raw.githubusercontent.com/Travis-Gilbert/Theorem/main/scripts/install.sh | bash
theorem init
theorem once "hello from Theorem"
```

The generated `theorem-localmodel.toml` uses the deterministic rule provider, the
local MCP route (`http://127.0.0.1:8380/mcp`), tenant `Travis-Gilbert`, and no
bearer/model env vars. Hosted sync and OpenAI-compatible local models are opt-in
edits once those surfaces are available.

For agent self-onboarding, point Codex, Claude Code, or another coding head at
[`llms-full.txt`](llms-full.txt).

## Layout in one minute

- `rustyredcore_THG/` is the main Cargo workspace: the graph engine, the harness kernel and runtime, and the PyO3 bridge. Its crates live in `rustyredcore_THG/crates/`.
- `apps/` holds standalone crates and clients that depend on the workspace but build on their own: the browser, the gRPC search server, the harness HTTP server, the SDK bindings, the iOS and desktop clients, and the Python mirror.
- `docs/plans/` holds the plan tree. `docs/learnings/` holds dated, single-lesson notes from prior sessions.

There is no root Cargo workspace. The `-p <crate>` flag only works inside the relevant workspace. The `apps/browser*` crates are reached with `--manifest-path`.

## Build the engine

```bash
cd rustyredcore_THG && cargo check --workspace
cd rustyredcore_THG && cargo test -p rustyred-thg-core
```

Use `maturin develop` for the root PyO3 module. A plain workspace build reaches
the Python extension link step and is not the right compile oracle for member
crates.

## Test the harness kernel and runtime

```bash
cd rustyredcore_THG && cargo test -p theorem-harness-core
cd rustyredcore_THG && cargo test -p theorem-harness-runtime
```

## Run the harness HTTP transport

```bash
cd apps/theorem-harness-server
PORT=50080 THEOREM_HARNESS_DATA_DIR=harness-data cargo run
```

To share one populated store between the MCP server and this HTTP server, point both at the same RedCore tenant directory.

## Run the gRPC search server

```bash
cd apps/theorem-grpc && cargo build -p theorem-grpc
```

It binds `0.0.0.0:$PORT` (default `50071` local) and serves `theseus_search.v1.SearchService` in pure Rust over the substrate.

## Build the Python wheel (PyO3)

```bash
cd rustyredcore_THG && maturin develop
```

This builds and installs the `theseus_native` module into the active virtual environment. The Python-visible module name is `theseus_native` even though the Rust crate is `rustyredcore_THG`; the override is deliberate and must stay.

## The Servo browser

The Servo embedder in `apps/browser` builds in CI only because libservo is heavy from cold. The Servo-free seam in `apps/browser-substrate` builds and tests in seconds and is the right place to iterate on page-to-graph ingestion.
