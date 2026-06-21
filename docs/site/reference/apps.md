# App reference

Standalone crates and clients under `apps/`. Each depends on the `rustyredcore_THG` workspace but builds on its own.

| App | What it is |
|-----|------------|
| `browser` | `theorem-browser`: the Servo-embedded substrate-native browser. Standalone crate, not in the workspace. CI-only build. |
| `browser-substrate` | `theorem-browser-substrate`: the Servo-free page-to-substrate seam. Ingests a `LoadedPage` into a `GraphStore`. Builds in seconds. |
| `commonplace-api` | CommonPlace interoperability API seam. Serves the typed consumer GraphQL profile and MCP stdio tools over the `commonplace` object model, with optional durable RedCore plus disk backing; also exposes an embeddable loopback server for the desktop shell. |
| `copresence-editor` | Browser adapter for `theorem-copresence`: Velt/Yjs plus Tiptap collaboration with an OpenAI-compatible Gemma co-writer seam. |
| `desktop` | Tauri desktop shell. Owns the local harness node, the CommonPlace API loopback server, keychain, receiver, browser tabs, and native command bridge; SPEC-9 repoints the main window to the CommonPlace Next.js frontend export. |
| `harness-console` | Standalone Next.js 16 / React 19 control surface for Theorems Harness at `harness.theoremsweb.com`. Greenfield app with Agent, Memory, Skills, Rooms, Runs, API Keys, Providers, Usage, Connections/MCP Hub, Settings, collaborative CodeMirror/Yjs editor, cosmos.gl memory graph, Dynamic Island omnibar, and tokenized 4px design-math lint. |
| `ios` | `TheoremKit`: a Swift Package shared kit layer, distinct from `theorem-ios`. |
| `jobintel` | Standalone job-intelligence CLI. A light HTTP consumer of a running RustyRed; no path-deps into the substrate. |
| `notebook` | Python mirror of Theseus's inference layer: reference engines, the native-vs-Python routing kernel, byte-parity and cost gates. |
| `obsidian-sync` | `theorem-harness-sync`: device-side Obsidian community plugin (TypeScript). Mirrors memory docs into a vault and writes note edits back into the graph. |
| `orchestrate` | Python MAP-Elites orchestration tick (`map_elites_tick.py`). |
| `theorem-agentd` | Local assistant daemon runtime: an OpenAI-compatible model loop, local GGUF models, MCP tool host config, and the receiver sidecar. |
| `theorem-grpc` | Theorem's first gRPC server. Serves `theseus_search.v1.SearchService` in pure Rust over the substrate. Own Railway service. |
| `theorem-harness-node` | Node.js (NAPI-RS) binding over the `theorem-harness` Rust SDK. A thin shell so the Node surface cannot drift from the core. |
| `theorem-harness-server` | Standalone Axum JSON/HTTP transport over `theorem-harness-runtime`. Serves run list and detail plus coordination reads for the iOS and web surfaces. |
| `theorem-harness-swift` | Swift (UniFFI) binding over the `theorem-harness` Rust SDK. Same surface as the Node binding, generated from the same core. Serves the iOS app. |
| `theorem-ios` | SwiftPM native iOS client scaffold: SwiftUI shell, Dynamic Island control surface, projection picker, smoke executable. |

This table is maintained by hand because the apps draw descriptions from mixed manifests (Cargo, package.json, Package.swift). When you add or rename an app, update this row and the app table in `CLAUDE.md`, then run `scripts/check-doc-drift.sh --refresh`.
