# System overview

Theorem has three language sides over one substrate, and a set of transports and clients on top.

## The three sides and the bridge

Rust is the substrate: the graph engine (`rustyredcore_THG/` and its crates), the browser (`apps/browser*`), RustyWeb, and the native symbolic and harness engines.

Python is the mirror of Theseus's inference layer (`apps/notebook/`, `apps/orchestrate/`): reference engines, the routing kernel, and the parity and cost gates that keep the Rust ports honest.

Swift is the native phone client surface (`apps/theorem-ios/`).

A PyO3 bridge ties Rust to Python. `rustyredcore_THG/src/lib.rs` is a `#[pymodule]` exported to Python as `theseus_native`. The crate function is named `rustyredcore_THG` but a `#[pyo3(name = "theseus_native")]` override sets the Python-visible name. Removing that override breaks `import theseus_native` silently and drops Python into a slow fallback path, so it stays.

## The layers, bottom to top

At the bottom is the GraphStore: the `GraphStore` trait and its three implementations. Everything persists here.

Above the store sit the engine crates that own graph shape for a domain without coupling to any transport: `rustyred-thg-code` for code search, `rustyred-web` for crawl and search, `rustyred-thg-memory` for recall and decay, `rustyred-thg-geotemporal` for spatial and temporal indexing.

Above those sit the learning and selection crates: `rustyred-thg-affordances` learns which MCP tool to reach for, `ensemble` selects capability packs under a budget, `rustyred-membrane` and `rustyred-rerank` decide what enters the context window, and `rustyred-thg-adapters` catalogs LoRA adapters. The shared invariant across these is that learned code ranks and steers within bounded enumerated spaces; it never authors graph mutations.

The harness crates (`theorem-harness-core`, `theorem-harness-runtime`, `theorem-harness`) sit beside the store as the coordination and memory layer.

## Transports and clients

The substrate reaches the outside through several transports: a native Rust MCP server (`rustyred-thg-mcp`), RESP-protocol servers (`rustyred-thg-server`, `-resp-server`, `-compat-server`), a gRPC search server (`apps/theorem-grpc`), and the Axum harness HTTP server (`apps/theorem-harness-server`).

Clients build on those: the desktop app (Tauri), the iOS scaffold and its Swift kit, the Obsidian sync plugin, and the SDK bindings (`apps/theorem-harness-node`, `apps/theorem-harness-swift`) generated from the one Rust SDK so they cannot drift.

For the full enumerated surface, see the [crate reference](../reference/crates.md) and [app reference](../reference/apps.md).
