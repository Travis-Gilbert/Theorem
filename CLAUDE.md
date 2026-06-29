# Theorem

Theorem is the **Rust-native substrate spine and harness**: the RustyRed/THG graph engine, the local-proxy + ambient memory/coordination harness, the substrate-native browser, the RustyWeb crawler/search, CommonPlace, and the binary-reconstruction / symbolic-engine lines. **This repo is canonical for the Rust substrate.**

This file is the navigation map. Read it first. Order of truth: this map, then the code, then plan docs (plans lag code). If a session adds, renames, or removes a crate, app, or entrypoint, update this file before ending.

**Theseus relationship (heritage, not canon):** the Python layer here (`apps/notebook/`, `apps/orchestrate/`) mirrors Theseus's inference layer (separate repo `github.com/Travis-Gilbert/Theseus` / local `Index-API`), and the PyO3 bridge exports to Python as `theseus_native`. Keep *those* parity contracts reconciled with Theseus; everything else in this repo is canonical here. Cross-repo sync history lives in `README.md`'s "Last sync" line, not here. Background framing: `Theseus/Theorem.md`.

## Repository Layout (`apps/` and top-level)

| Path | What it is |
|------|------------|
| `rustyredcore_THG/` | The RustyRed/THG Cargo workspace **and** the PyO3 bridge. Root `src/` is the `#[pymodule]` exported to Python as `theseus_native` (maturin). Own `Dockerfile` + `railway.toml`. Crates table below. |
| `apps/browser/` | `theorem-browser`: Servo-embedded substrate-native browser (Servo as a pinned git dep, NOT a fork). Standalone crate; CI-only build. |
| `apps/browser-substrate/` | `theorem-browser-substrate`: Servo-free page->substrate seam (`LoadedPage` -> rustyred-web -> `GraphStore`). Builds in seconds. |
| `apps/rustyred-embedded/` | `rustyred-embedded`: in-process embedded engine over a local dir, no server (`Engine::open`). Typed GraphQL + MCP-over-stdio, DocTree filesystem (`fs_*`), restart rehydration, file embeddings. Area E complete. Spec: `docs/plans/rustyred-multimodel/`. |
| `apps/rustyred-fuse/` | W6 filesystem proof (DocTree<->inode); pure-Rust by default, optional `mount` feature for macFUSE. |
| `apps/rustyred-git/` | `rustyred-git`: git-as-truth (W2). `WorkspaceRepo` over the git CLI + `GixWorkspaceRepo` pure-Rust `gix` backend (commit/branch/merge/diff, smart-HTTP push, GitHub PR REST seam). |
| `apps/rustyred-workspace/` | W0 import + W3 execution bridge: walk a checkout, write source into rustyred-embedded, materialize+run a toolchain in a sandbox, sync changed sources back; W6 `DocTreeMount`. |
| `apps/theorem-ios/` | SwiftPM scaffold for the native iOS client (SwiftUI shell, ScenePackageV2, Dynamic Island). Full simulator/archive needs Xcode. |
| `apps/theorem-harness-server/` | Axum JSON/HTTP transport over `theorem-harness-runtime` + RedCore (runs/rooms/presence/intents/records for iOS/web). Standalone Cargo root. |
| `apps/theorem-proxy/` | `theorem-proxy`: local Anthropic Messages passthrough proxy on `ANTHROPIC_BASE_URL` that makes harness memory ambient (no MCP round-trip). Standalone (Axum/reqwest/ureq). `proxy` forwards `POST /v1/messages` byte-for-byte (streaming/SSE) and injects relevance-ranked memory at the cache-stable suffix (last user message only). Memory is a `MemorySource` trait: `DirectoryMemorySource` (fast default), `HttpMemorySource` (node `hippo_retrieve`, fail-open). `wrap -- <cmd>` = one-command connect; `doctor` = stack check; `--membrane-threshold` samples oversized `tool_result`. Scripts: `start-proxied-session.sh`, `node-local.sh` (embedded RedCore node on SSD; auto-detects a local OpenAI-compat embedding endpoint, else hash), `seed-node.py`. Embedder is endpoint-agnostic (`QWEN3_EMBEDDING_4B_URL`: Ollama/mistral.rs/any). Plans: `docs/plans/local-proxy/`. |
| `apps/theorem-substrate-sync/` | `theorem-substrate-sync`: standalone local-to-hosted substrate sync daemon. Drives Prolly version-pack rounds over `rustyred_thg_graph_version_*`, uses Valkey-backed outbox/cursors plus harness stream verbs for freshness, exposes `/status` and `/trigger`, and is launched default-off via `THEOREM_SYNC_ENABLED=1` from the proxied-session script. Plan: `docs/plans/substrate-sync/`. |
| `apps/theorem-grpc/` | `theorem-grpc`: pure-Rust gRPC `SearchService` over the substrate. Standalone; own `Dockerfile`/`railway.toml`. Binds `[::]:$PORT` (50071) + `/health`/`/ready`; exposes remote-doctor probes. |
| `apps/theorem-gateway/` | Browser-facing GraphQL gateway (Axum + async-graphql + tonic) calling `theorem-grpc` + GL-Fusion. SceneOS add-on: `sceneForInput` -> `ScenePackageV2`, `GET /scene/{id}`. Standalone; own `Dockerfile`/`railway.toml`. |
| `apps/ios/TheoremKit/` | Swift Package shared kit layer (distinct from `apps/theorem-ios`). |
| `apps/commonplace-api/` | `commonplace-api`: typed consumer GraphQL over the `commonplace` model, per-instance API-key gated. In-memory or durable RedCore backing. `ask`/`briefing`/`discover`/`organize`/`export` + a `commonplace-mcp` stdio bin. Standalone; powers `travisgilbert.me` Auto Organize. |
| `apps/commonplace-desktop-runtime/` | Standalone local-instance runtime for CommonPlace: notify ambient watcher (read-only tree, sidecar-only writes), durable sidecar store, loopback endpoint, device pairing, relay tunnel, process executor. |
| `apps/commonplace-clipper/` | obsidian-clipper fork adding a CommonPlace save target (posts `CapturedObject` to the capture endpoint). |
| `apps/desktop/` | `desktop`: Tauri shell. Rust backend owns the local harness node (`127.0.0.1:17888`), CommonPlace API loopback (`17890`), the in-process `theorem-proxy` sidecar (`17891`) + Connect-Claude-Code control, keychain, receiver, browser tabs, coordination/job/model commands. |
| `apps/copresence-editor/` | Vite+React browser adapter for `theorem-copresence` (Velt/Yjs + Tiptap, OpenAI-compatible Gemma co-writer). |
| `apps/harness-console/` | Next.js 16 / React 19 control surface for Theorems Harness (`harness.theoremsweb.com`): Agent/Memory/Skills/Rooms/Runs/Keys/Providers/Usage/MCP-Hub, cosmos.gl graph, Dynamic Island omnibar, CommonPlace code-workspace shell. |
| `apps/theorem-harness-node/` | Node (NAPI-RS) binding over the `theorem-harness` Rust SDK. |
| `apps/theorem-harness-swift/` | Swift (UniFFI) binding over the `theorem-harness` Rust SDK. |
| `apps/web/` | Empty staging dir (not a product surface yet; keep web work in `harness-console`/`theorem-gateway`). |
| `apps/obsidian-sync/` | `theorem-harness-sync`: Obsidian plugin mirroring memory docs into a vault and writing notes/`[[wikilinks]]` back via the harness (`upsert_note` MCP). npm package. |
| `apps/jobintel/` | `jobintel`: standalone job-intel + outreach CLI; pure HTTP consumer of a running RustyRed (HN/ATS ingest, rank by vector+PPR, Gmail draft outreach). `Embedder` = hash/http/bge. |
| `apps/notebook/` | **Python** mirror of Theseus's inference layer: reference engines, native-vs-Python routing kernel, byte-parity + cost gates, evolution callers. |
| `apps/orchestrate/runtime/` | **Python** `map_elites_tick.py` (native MAP-Elites archive tick). |
| `docs/plans/` | The plan tree. |
| `docs/reference/` | Source architecture context, incl. `status-current-direction.md` (per-strand status). |
| `.github/workflows/servo-browser.yml` | CI build for the Servo embedder (heavy; manual trigger). |

### RustyRed/THG crates (`rustyredcore_THG/crates/`)

| Crate | Purpose |
|-------|---------|
| `rustyred-thg-core` | The graph engine core: `GraphStore` trait + impls, mutations/transactions, indexes, TTL, AOF durability, symbolic engines, post-commit hooks (`hooks.rs`), the native relational core (`access_method`/`relational`/`planner`), and the cold storage spine (`object_store`/`doc_tree`/`cold_index`/eviction). The heart of the substrate. |
| `rustyred-thg-fuse` | Read-only graph-archive filesystem over `CompiledGraphPack` (rkyv zero-copy); optional `mount` feature. |
| `rustyred-web` | RustyWeb crawler/search: URL canon, page->graph emission, browser-use web primitives; trust-ahead-of-relevance ranking. |
| `rustyred-hipporag` | HippoRAG 2 candidate generation (`Page`/`Phrase`/`Hub`, query-specific PPR, RAPTOR summary hubs). |
| `rustyred-membrane` | Context admission/eviction membrane (admits candidates before they enter graph-backed context). |
| `rustyred-code-embedding` | Shared code-embedding seam: `hash` default, feature-gated `http` + `local` (Candle BGE `bge-small-en-v1.5`, D=384). |
| `rustyred-rerank` | Reranker scorers for membrane admission (lexical cross-encoder seam). |
| `rustyred-thg-adapters` | Adapters into the core. |
| `rustyred-thg-affordances` | Connector-as-substrate learning registry: MCP tools become `Affordance` nodes; PPR-over-outcomes selection scoped per agent (`CapabilityScope`). |
| `rustyred-thg-binformat` | Binary-reconstruction loader (artifact/section/symbol/string/reloc/entrypoint facts -> GraphStore). |
| `rustyred-thg-behavior-ir` | Language-neutral feature-port contracts: source slice, behavior IR, target plan, patch set, and validation receipt shapes for reverse-engineer emit. |
| `rustyred-thg-code` | Code graph and compiler runtime: source -> code KG, code spec/features/obligations/drift, reverse-engineer compose, Datawave projection, and behavior-IR feature-port scaffolds. |
| `rustyred-thg-connectors` | Live outbound MCP transport: connect over stdio, walk `tools/list`, register each as a learnable `Affordance`; invoke bridge gated by `InvokePolicy`. |
| `rustyred-thg-disasm` | Binary-reconstruction decoder (`iced-x86` -> `InstructionFact` nodes). |
| `rustyred-thg-lift` | Binary-reconstruction THIR lifter (instructions -> SSA-like `ThirFunction`). |
| `rustyred-thg-reconstruct` | Binary-reconstruction compiler (semantic-role/component hypotheses + validation receipts). |
| `rustyred-thg-reconstruct-harness` | Capability pack `theorem.reconstruct.binary` (load/analyze/lift/.../receipt; no raw disasm surfaced to agents). |
| `rustyred-thg-datawave` | DATAWAVE-style intake: any record -> normalized field-facts + entity-edges + self-describing dictionary + masking + content/fuzzy hashing, over `GraphStore`. The general ingest front door binary-reconstruction facts compose with. Plan: `docs/plans/datawave-ingest-edge/`. |
| `rustyred-thg-datawave-harness` | Capability pack `theorem.ingest.datawave` (`ingest.describe/record/batch/lookup/intersect`; data-driven `HelperSpec`). |
| `rustyred-thg-compat-server` | Legacy HTTP control server (command exec/run/state-hash/health over memory or Redis). |
| `rustyred-thg-geotemporal` | Geo + temporal indexing (`time_series` access method for `TimeRange`). |
| `rustyred-thg-fractal` | Native fractal expansion over RustyRed+RustyWeb (crawls/admits web state into a quarantined lower-trust tier). |
| `rustyred-thg-mcp` | Native Rust MCP server (graph reads/algorithms; GraphQL surface, adaptive-index inspection, `compute_code`/`code_ingest`, compiler tools, `reconstruct_binary`, `datawave_ingest`, and `reverse_engineer_*`). The in-process MCP seam. |
| `rustyred-thg-ml` | Graph tensor + message-passing primitives (`GraphTensorBatch`, scatter; feature-gated Burn). |
| `rustyred-thg-offload` | Compute-offload planner (operation algebra, reuse cache, pushdown/fusion, cascade routing); one routed affordance `compute_offload.route_operation`. |
| `rustyred-thg-graphblas` | GraphBLAS sparse-matrix compute core (SuiteSparse v9.4.5 + LAGraph v1.2.1 FFI/safe layer, masked `mxv`/`mxm`, the 7 LAGraph algorithms). NOT a workspace member; pulled via core's optional `graphblas` feature. Build needs `RUSTYRED_GRAPHBLAS_PREFIX` + `LIBCLANG_PATH`; downstream binaries need `DYLD_FALLBACK_LIBRARY_PATH`. |
| `rustyred-thg-memory` | Graph-native memory: PPR-seeded `recall`, `consolidate`, `decay`, validity/contradiction, project-scope bias; cold-tier eviction/rehydrate (`ColdTier`, `evict_decayed`, `recall_with_cold_tier`, park/unpark scope). |
| `rustyred-thg-pg-server` | Postgres wire-protocol server over native views (simple + extended query, SQL->`QueryIr`). |
| `rustyred-thg-resp-server` | RESP (Redis-shaped) command loop over `OrderedIndexRegistry`. |
| `rustyred-thg-server` | Product HTTP/gRPC/MCP surface: tenant graph routes, query/Cypher, harness coordination, browser actions, fractal expansion, TTL, adaptive-index routes. The runnable node (embedded RedCore on a data dir). |
| `rustyred-proxy` | First-class local Anthropic Messages proxy binary; reuses `theorem-agentd`'s proxy module (`/v1/messages`, tool-result sampling, ambient injection, `/v1/presence*`) without the local model loop. |
| `rustyred-thg-catalog` | Compatibility sqlx/Postgres catalog (tenants/projects/billing/auth + legacy cold rows); the hot path now lives in core. Builds with no live DB. |
| `commonplace` | CommonPlace consumer data layer: `Item`/`Collection`/`Tag` graph-native over `GraphStore` + `BlobStore`; `IngestPipeline` (embed/classify/auto-collections/`SIMILAR_TO`); `ItemKind::Task` + routing/organize. |
| `rustyred-thg-intake` | CommonPlace source intake: `SourceSpoke`/`sync_source`/`MappedSpoke` + 5 curated spokes (Gmail/GSuite/Outlook/Notion/Linear, live REST over an injected HTTP seam) + the C2 `ActSeam`. |
| `design-check` | Static design-engineering checker + skill-pack (CSS/token/typography/motion/a11y); owns the Design Scout callable validator. |
| `ensemble` | Capability-pack registry, budgeted selector, trust ladder; emits replayable `EnsembleDecision` records. |
| `pilot-core` | Servo-free browser-automation core (locators, actionability/auto-wait, geometry, web-first assertions behind `BrowserDriver`). |
| `prose-check` | Deterministic writing-engineering style checker + skill-pack. |
| `theorem-copresence` | Headless co-presence peer + surface-adapter seam: structure on a graph CRDT, free text in `yrs` `TextRegion`s, awareness on the `working_log`; note + code adapters (code merges through W2 git, not Yrs). |
| `theorem-harness-core` | Rust-native harness kernel: pure run state, transition executor, guards, state hashing, replay/fork receipts. Parity-tested vs Python corpora. |
| `theorem-harness-runtime` | GraphStore-backed harness runtime: persists transition receipts as `HarnessRun`/`HarnessEvent`; holds `job_queue` (Dispatch v2 board). |
| `theorem-harness` | SDK v2 surface (run handles, sessions-as-scopes, idempotency, cancellation, replay, trace export). Source for the Node/Swift/WASM bindings. |
| `theorem-acp` | CommonPlace ACP host for external coding agents (spawns ACP subprocesses over stdio, injects the Harness MCP, stages writes/PTY as approval cards). |
| `theorem-agentd` | Local assistant daemon: OpenAI-compatible model loop, schema-guarded MCP tool host, receiver sidecar, capture/relay, compute-offload ledger. Descriptive bin alias `theorem-localmodel`. |
| `theorem-browser-agent` | Rust-native browser-use perceive/govern/afford kernel (Servo-free; web I/O via `rustyred-web`). |
| `theorem-dispatch` | Postgres hot execution queue for Dispatch v2 (`dispatch_jobs`, `FOR UPDATE SKIP LOCKED` claims, leases, retries). The THG board stays canonical for coordination. |
| `theorem-receiver` | Dispatch v2 receiver: outbound-only launcher that reads pending jobs and spawns the local `claude`/`codex` CLI in a worktree; `HeadAdapter` trait; `SandboxRuntime` (OpenSandbox + `LocalProcessSandbox`). |
| `reconstruction-engine` | Generative reconstruction engine (in `crates/` but NOT a workspace member; built separately). |
| `scene-os-core` | Rust SceneOS director: `compile_scene_package -> ScenePackageV2` (goal/shape classify -> projection select). serde-only leaf. |
| `scene-os-web` | SceneOS renderer bundle: `render_scene -> String` (self-contained canvas+d3, spawn->settle->annotate). Served by the gateway's `GET /scene/{id}`. |

## Languages + the PyO3 bridge

- **Rust** (`rustyredcore_THG/` + `apps/*`): the substrate engine, harness, browser, RustyWeb, native symbolic engines.
- **Python** (`apps/notebook/`, `apps/orchestrate/`): the Theseus-inference mirror (reference engines, routing kernel, parity + cost gates).
- **Swift** (`apps/theorem-ios/`): native phone client over ScenePackageV2 (Swift Package today; Xcode for sim/archive).
- **PyO3 bridge**: `rustyredcore_THG/src/lib.rs` is a `#[pymodule]` exported as **`theseus_native`** (maturin). The fn is named `rustyredcore_THG` but `#[pyo3(name = "theseus_native")]` overrides the Python-visible name. **Do NOT remove that override** -- without it the symbol is `PyInit_rustyredcore_THG` and `import theseus_native` fails silently into the slow Python fallback.

## GraphStore: three impls, one trait

`rustyred-thg-core` defines `GraphStore`. Three impls:

- **`InMemoryGraphStore`** -- ephemeral, in-process (tests/scratch).
- **`RedCoreGraphStore`** -- durable, file-backed, in-process (AOF + snapshots). `open(data_dir, RedCoreOptions)`; default `AofEverysec`, use `AofAlways` for fsync-per-commit determinism. The "in-process substrate with no API boundary." Reads serve from an in-memory mirror that `recover()` rebuilds from the AOF.
- **`RedisGraphStore`** -- connects to a Redis/RustyRed (Valkey) server (out-of-process API boundary).

`RedCoreGraphStore`/`RedisGraphStore` expose inherent `get_node`/`upsert_node` that **shadow** the trait methods -- call the trait method via UFCS: `GraphStore::get_node(&store, id)`.

## The substrate-native browser

- `apps/browser/` embeds Servo as a pinned git dep (not a fork), built via `cargo` after `./mach bootstrap`. DOM->graph seam: `WebViewDelegate::load_web_resource` -> on `LoadStatus::Complete` build a `LoadedPage` and call `theorem_browser_substrate::ingest_loaded_pages` (writes into a `GraphStore`).
- `apps/browser-substrate/` is the Servo-free seam (`LoadedPage` -> `build_v2_fixture_crawl` -> `apply_to_store`). Keep it Servo-free so it stays cheap.

## Build & Dev Commands

**No root Cargo workspace** -- pick the workspace/crate. `-p <crate>` works inside `rustyredcore_THG`; `apps/*` are standalone crates reached via `--manifest-path` or `cd`.

```bash
cd rustyredcore_THG && cargo check --workspace          # type-check the engine workspace
cd rustyredcore_THG && cargo test -p <crate>            # e.g. rustyred-thg-core, theorem-harness-core
cd rustyredcore_THG && maturin develop                  # build+install the theseus_native wheel
cargo test --manifest-path apps/browser-substrate/Cargo.toml   # Servo-free seam (fast)
cd apps/theorem-grpc && cargo build -p theorem-grpc     # gRPC search server (50071)
cd apps/theorem-harness-server && cargo test
swift build --package-path apps/theorem-ios             # iOS scaffold (CLT-friendly)
gh workflow run servo-browser.yml --ref main            # Servo embedder (CI-only, heavy)
```

Python parity / cost gates live under `apps/notebook/benchmarks/` and the inference-engine tests.

## Conventions & Gotchas

- **No root workspace**: `-p <crate>` only works inside the relevant workspace; `apps/*` are standalone crates via `--manifest-path`. Path-deps cross from `apps/` into `rustyredcore_THG/crates/`.
- **Multi-agent repo (Codex)**: Codex is frequently active here. Commit only with an explicit pathspec (`git commit -- <paths>`), never a bare `git commit` (the shared index can carry another agent's staged files). Before editing a file another head may be on, check git for live work; reconcile, never overwrite. The harness `coordinate` endpoint is often down -- git history + commit messages are the fallback coordination channel.
- **Theseus parity**: keep the Python inference mirror (`apps/notebook/`) and the PyO3 surface reconciled with Theseus; native Rust symbolic engines must byte-match the Python reference receipts (gates in `apps/notebook/benchmarks/`). Surface drift, don't bury it.
- **Servo build is the long pole**: CI-only and heavy. Pin the `servo` rev deliberately; the embedder's `rust-toolchain.toml` must match Servo's.
- **No emojis. No em/en dashes** (use colons, parens, semicolons).
- **No time/effort estimates** in plans or reports.
- **Deps named in a spec are information, not gates** -- check the tree, decide, note the reasoning.

## Status / Current Direction

Per-strand status lives in [docs/reference/status-current-direction.md](docs/reference/status-current-direction.md) to keep this file a lean map. Update *that* file when a strand changes; keep only the repo map + conventions here.
