# Round 2 navigation patch (staged, not applied)

This is the byproduct of the round-1 per-crate README pass. You deferred the navigation-map refresh, so this is staged for you to apply to `CLAUDE.md` and `README.md` rather than applied silently. Every line is grounded in the crate's own `Cargo.toml` description or `//!` header.

Source of truth for the drift numbers: `scripts/check-doc-drift.sh --full`.

## Crate table: 13 rows to add to CLAUDE.md

The `CLAUDE.md` crate table documents 15 of 28 crates. Add these rows:

| Crate | Purpose |
|-------|---------|
| `ensemble` | Capability-pack registry, budgeted selector, and trust ladder over RustyRedCore-THG. The pack-level layer above `rustyred-thg-affordances` (tools) and `theorem-harness-runtime::skill_pack` (one kind). |
| `pilot-core` | Servo-free, Playwright-class browser automation core: locator, actionability and auto-wait, geometry snapshot, web-first assertions behind a `BrowserDriver` trait. In-place precursor to a WebDriver BiDi front end for Servo. |
| `prose-check` | Deterministic writing-engineering style checker for harness receipts. |
| `rustyred-membrane` | Shared context admission and eviction membrane. The context window is treated as a cache over graph-resident nodes; a `Scorer` ranks candidates, a shared gate admits a budgeted subset and converts overflow into recoverable graph handles. |
| `rustyred-rerank` | Reranker `Scorer` implementations for membrane admission. Hot-path default is a SequenceClassification-style single forward pass behind `CrossEncoder`. |
| `rustyred-thg-compat-server` | Standalone HTTP control server for THG-Core (compatibility surface). |
| `rustyred-thg-fractal` | Native Rust fractal expansion over RustyRed and RustyWeb. A run is corpus growth, not graph-only retrieval: it builds a web crawl request and ingests admitted web state as a lower-trust quarantined tier. |
| `rustyred-thg-memory` | Graph-native memory recall, consolidation, decay, and validity plugin. |
| `rustyred-thg-resp-server` | Redis/RESP-protocol server surface over the core. |
| `scene-os-web` | SceneOS renderer bundle (Lane B): embeds the self-contained canvas renderer and serves a `ScenePackageV2` as one HTML asset, the SERP injection pattern. Lane A is `scene-os-core`. |
| `theorem-agentd` | Local assistant daemon plus MCP tool host: a resident local model chooses one schema-guarded tool call at a time; `theorem-receiver` stays the only component that launches Claude or Codex sessions. |
| `theorem-dispatch` | Postgres hot execution queue for Dispatch v2. Owns only hot execution state: claim leases, retries, completion, dead-letter. The canonical coordination thread stays in the THG Dispatch v2 board. |
| `theorem-harness` | The `theorem-harness` SDK v2: the idiomatic Rust surface over `theorem-harness-core` and `theorem-harness-runtime`. The source of truth from which the Python, Node, Swift, and WASM bindings are generated, so they cannot drift. |

Note: `rustyred-thg-server`, `-resp-server`, and `-compat-server` are documented in `CLAUDE.md` as one combined shorthand row. The drift checker matches full backtick tokens, so it reports `-resp-server` and `-compat-server` as missing. Giving each its own row (above) both clears the signal and is more accurate, since `rustyred-thg-server` is the product HTTP server while the other two are distinct surfaces.

Also already on disk and worth a status line, not only a table row: `scene-os-core` (in the iOS status) and `rustyred-thg-code` (in the hook status) appear in `CLAUDE.md` prose but not as table rows. Add table rows for completeness.

## App table: 4 apps to add

`CLAUDE.md` documents the apps in prose path form; these four are absent entirely.

| App | What it is |
|-----|------------|
| `desktop` | Tauri plus React plus TypeScript desktop client (Vite). |
| `theorem-agentd` | Local assistant daemon runtime: OpenAI-compatible model loop config, local GGUF models, MCP tool host, receiver sidecar. The deployment side of the `theorem-agentd` crate. |
| `theorem-harness-node` | Node.js (NAPI-RS) binding over the `theorem-harness` Rust SDK. THPS-012 slice 1. |
| `theorem-harness-swift` | Swift (UniFFI) binding over the `theorem-harness` Rust SDK. Serves the Theorem iOS app. |

## README fixes

1. The README intro is corrupted. The current first line reads "Theorem is the Rust-native Graph based:" and several crate-table cells are garbled ("this database is Theorems Harness", empty bridge names). Replace the intro and the corrupted cells. The clean intro is in `CLAUDE.md` and in `docs/site/README.md`.
2. The README `Last sync` line still reads 2026-05-29. Bump it when you next re-sync with Theseus.

## After applying

Run `scripts/check-doc-drift.sh --refresh` and confirm `scripts/check-doc-drift.sh --full` reports 0 undocumented crates and 0 undocumented apps.
