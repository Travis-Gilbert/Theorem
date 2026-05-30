# Theorem

Theorem is the **Rust-native substrate spine**: the programmable Rust projection that mirrors the Theseus / Index-API workspace. It carries the RustyRed/THG substrate engine, the PyO3 bridge and parity surfaces that exercise it, the substrate-native browser, the RustyWeb crawler/search engine, and the symbolic-engine plans.

This file is the navigation truth for the repo. Read it first. Order of truth: this map, then the actual code, then the plan docs (plans lag code). If a session changes architecture (new crate, renamed module, removed subsystem, new entrypoint), update this file before ending the session.

## The Mirror Rule (read this before touching anything)

- **Theseus is canonical.** Theseus = the Python + Memgraph + Postgres application/runtime (separate repo: `github.com/Travis-Gilbert/Theseus`, the local `Index-API` checkout). Theorem **mirrors** the Rust projection and its bridge contracts so Rust-side work can move at its own cadence while staying auditable against the source workspace.
- **Do not let Theorem and Theseus diverge silently.** The Python files here (`apps/notebook/`, `apps/orchestrate/`) are mirrors of their Theseus counterparts. Changes that affect the contract (parity receipts, affordance gates, the PyO3 surface) must stay reconciled with Theseus.
- Last sync recorded in `README.md` (2026-05-29 at time of writing). Bump it when you re-sync.
- Open question still live in `Theseus/Theorem.md`: whether a good tool built on the Theorem (Rust) layer becomes a **promotion candidate** to the canonical Theseus layer. Treat Rust-side tools as candidates, not canon.

## Repository Layout

| Path | What it is |
|------|------------|
| `rustyredcore_THG/` | The RustyRed / THG Rust workspace **and** the PyO3 bridge. A real Cargo workspace (`Cargo.toml [workspace]`). Root lib (`src/`) is the PyO3 module exported to Python as `theseus_native` (maturin). Has its own `Dockerfile` + `railway.toml`. |
| `rustyredcore_THG/crates/` | The graph-engine crates (see table below). |
| `apps/browser/` | `theorem-browser`: the Servo-embedded substrate-native browser. Standalone Cargo crate (NOT in the rustyredcore_THG workspace). |
| `apps/browser-substrate/` | `theorem-browser-substrate`: the Servo-free page->substrate seam. Standalone crate; depends only on `rustyred-web` + `rustyred-thg-core` (no Servo), so it builds + tests in seconds. |
| `apps/notebook/` | **Python** mirror of Theseus's inference layer: `inference_engines/` (symbolic-engine contracts, Python reference engines, native bridge adapters, Gate-0 affordance coverage, tests), `inference_kernel/` (native-vs-Python routing/execution), `benchmarks/` (byte-parity + cost gates), `discovery_runs/` (Rust-theorem callers routing archive/policy-evolution through the native evolution engine). |
| `apps/orchestrate/runtime/` | **Python**: `map_elites_tick.py`, the orchestration tick for native MAP-Elites archive throughput. |
| `Theseus/Theorem.md` | Source framing: Theseus-as-canonical / Theorem-as-Rust-projection, and the promotion-candidate idea. The "why this repo exists" doc. |
| `docs/plans/` | The plan tree (see Status). `rust-theorem-symbolic-engines/`, `rusty-red-web/`, `rustyweb-v1-design/`, `commonplace-substrate-reconciliation/`. |
| `docs/reference/` | Source architecture context, incl. `commonplace-substrate-architecture-part-4-1.md` (symbolic compute offload). |
| `.github/workflows/servo-browser.yml` | CI build for the Servo embedder (heavy; manual trigger). |

### RustyRed/THG crates (`rustyredcore_THG/crates/`)

| Crate | Purpose |
|-------|---------|
| `rustyred-thg-core` | The graph engine core: `GraphStore` trait, the store impls, mutations/transactions, indexes, TTL, AOF durability, symbolic engines (`symbolic.rs`). The heart of the substrate. |
| `rustyred-web` | RustyWeb. V0 fixture crawler kernel + V2 hardening: URL canonicalization, `a[href]` extraction, BLAKE3 content snapshots, page->graph emission (`build_fixture_crawl_graph` / `build_v2_fixture_crawl`), application into a `GraphStore`. |
| `rustyred-thg-server` / `-resp-server` / `-compat-server` | The Redis/RESP-protocol server surfaces over the core. |
| `rustyred-thg-adapters` | Adapters into the core. |
| `rustyred-thg-geotemporal` | Geo + temporal indexing. |
| `rustyred-thg-mcp` | Native Rust MCP server (graph reads/algorithms without a Python process in the loop). |
| `reconstruction-engine` | Generative reconstruction engine (in `crates/` but NOT a workspace member; built separately). |

## The Two Language Sides + the Bridge

- **Rust** (`rustyredcore_THG/` + `apps/browser*/`): the substrate engine, the browser, RustyWeb, native symbolic engines.
- **Python** (`apps/notebook/`, `apps/orchestrate/`): the mirror of Theseus's inference layer (reference engines, routing kernel, parity + cost gates, evolution callers).
- **PyO3 bridge**: `rustyredcore_THG/src/lib.rs` is a `#[pymodule]` exported to Python as **`theseus_native`** (maturin, `pyproject.toml`). The Rust fn is named `rustyredcore_THG` but `#[pyo3(name = "theseus_native")]` overrides the Python-visible module name. Do NOT remove that name override: without it the symbol is `PyInit_rustyredcore_THG` and Python's `import theseus_native` fails silently into the slow Python fallback path.

## GraphStore: three stores, one trait

`rustyred-thg-core` defines the `GraphStore` trait. Three impls, used for different durability needs:

- **`InMemoryGraphStore`** — ephemeral, in-process. Tests + scratch.
- **`RedCoreGraphStore`** — durable, file-backed, in-process (AOF + snapshots). `open(data_dir, RedCoreOptions)`; `RedCoreOptions::default()` is `AofEverysec`, use `AofAlways` when you need fsync-per-commit determinism. **This is the "in-process substrate with no API boundary" the browser persists to.** It implements `GraphStore` (writes delegate to the inherent AOF-backed durable upserts; reads serve from an in-memory mirror that `recover()` rebuilds from the AOF on open). TTL methods keep the trait defaults (durable TTL is a follow-up; delegating TTL writes to the mirror would skip the AOF).
- **`RedisGraphStore`** — connects to a Redis/RustyRed server (an API boundary, out-of-process).

`RedCoreGraphStore` and `RedisGraphStore` also expose inherent `get_node`/`upsert_node` etc. that can **shadow** the trait methods. When you need the trait method on those types, call it via UFCS: `GraphStore::get_node(&store, id)`.

## The substrate-native browser

- **`apps/browser/`** embeds Servo as a Cargo **git dependency** pinned to a known-good rev (NOT a fork). It builds `libservo` via plain `cargo` after `./mach bootstrap` provides system libs (the external-embed path, like Verso). The embedder constructs `ServoBuilder::default().event_loop_waker(...).build()`, a headless `SoftwareRenderingContext`, and a `WebView` via `WebViewBuilder::new(&servo, rendering_context)`.
- **Substrate seam 1 (DOM -> graph state)**: `WebViewDelegate::load_web_resource(&self, webview, WebResourceLoad)` (NOT `ServoDelegate`, which fires only for non-WebView loads). On `notify_load_status_changed(LoadStatus::Complete)`, build a `LoadedPage` and call `theorem_browser_substrate::ingest_loaded_pages` / `durable_browser_session_with_options`, which writes the page into a `GraphStore` (use `RedCoreGraphStore` for persistence).
- **`apps/browser-substrate/`** is the Servo-free seam: `LoadedPage -> rustyred-web (build_v2_fixture_crawl) -> CrawlGraph::apply_to_store`. Keep it Servo-free so it stays cheap to iterate.

## Build & Dev Commands

There is **no root Cargo workspace**. Pick the right workspace/crate:

```bash
# RustyRed/THG core workspace (the engine + bridge)
cd rustyredcore_THG && cargo build            # build the workspace
cd rustyredcore_THG && cargo test -p rustyred-thg-core   # test the core
cd rustyredcore_THG && cargo test --no-run -p rustyred-thg-core  # compile-only coherence check

# The seam crate (Servo-free, fast: no libservo)
cargo test --manifest-path apps/browser-substrate/Cargo.toml

# The Servo embedder builds in CI only (heavy: ~30 min libservo from cold).
# Trigger: gh workflow run servo-browser.yml --ref main
#   - servo pinned in apps/browser/Cargo.toml (rev) + apps/browser/rust-toolchain.toml (1.95.0)
#   - the workflow reclaims ~20-25GB of preinstalled SDKs + sets CARGO_PROFILE_DEV_DEBUG=0

# PyO3 wheel (theseus_native) via maturin, from rustyredcore_THG
cd rustyredcore_THG && maturin develop        # build + install the wheel into the active venv

# Python parity / cost gates (mirror of Theseus inference)
#   live under apps/notebook/benchmarks/ and apps/notebook/inference_engines/.../tests/
```

## Conventions & Gotchas

- **Mirror discipline**: this is the projection, not the canon. Keep parity receipts + the PyO3 surface reconciled with Theseus. Surface drift, don't bury it.
- **No root workspace**: `-p <crate>` only works inside the relevant workspace (e.g. `cd rustyredcore_THG`); `apps/browser*` are standalone crates reached via `--manifest-path`. Path deps cross from `apps/` into `rustyredcore_THG/crates/`.
- **Byte-parity discipline**: native Rust symbolic engines must byte-match the Python reference receipts. The differential gates live in `apps/notebook/benchmarks/`. A native port is not done until its parity gate is green over the reference set.
- **Servo build is the long pole**: it is CI-only and heavy. Pin the `servo` rev deliberately; bump on evidence. The embedder's `rust-toolchain.toml` must match Servo's (the whole graph builds with one toolchain). `mach` needs its Python deps (`toml`) in the invoking interpreter before its venv activates.
- **Editing a file another agent (Codex) is on**: Codex is frequently active in this repo. Commit only with an explicit pathspec (`git commit -- <paths>`), never a bare `git commit` (the shared index can carry another agent's staged files). Claim a seam before overlapping. The harness `coordinate` endpoint has been returning 503 (down); use git history + commit messages as the coordination channel meanwhile.
- **No emojis. No em/en dashes** (match the user's style across repos: use colons, parens, semicolons).
- **No time/effort estimates** in plans or reports.

## Status / Current Direction

- **Rust theorem symbolic engines**: implemented through RT-5.2a (native evolution parity). Remaining RT-5 ports (e-graph generalization, the cold engines) stay demand-driven / profile-gated. Plan: `docs/plans/rust-theorem-symbolic-engines/`.
- **RustyWeb**: active. `rustyred-web` holds the V0 fixture crawler kernel + V2 hardening (budget, URL guard, scope, `CrawlReceipt`). The designed **V1 search layer** (graph-yielding broker, two-fidelity AnswerDraft, license-tiering) is still **unbuilt** (design: `docs/plans/rustyweb-v1-design/`). "V0->V2" refers to the crawler only.
- **Substrate-native browser**: the external Servo embedder builds green in CI (`apps/browser`, libservo as a pinned Cargo git-dep). The page->substrate seam (`apps/browser-substrate`) ingests pages into a `GraphStore`, and `RedCoreGraphStore` now implements `GraphStore`, so browser-ingested pages persist durably to the in-process substrate. Next: wire `apps/browser/main.rs`'s real delegate to a `RedCoreGraphStore` so the live embedder persists; then the cost-graded dossier + search-as-graph chrome.
- **Reconciliation** between the Rust-theorem, RustyWeb, and kernel-object lanes: `docs/plans/commonplace-substrate-reconciliation/`.
