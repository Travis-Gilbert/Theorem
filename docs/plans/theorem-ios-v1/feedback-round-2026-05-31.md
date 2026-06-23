# Theorem iOS — feedback round (2026-05-31)

Travis reviewed the live app (search wired, 4 projections, 5 surfaces). This is
the durable git copy of the coordination (also in the harness: decision
`dec_672f22e3d5a44df7af82`, intents for room `repo:theorem:branch:main`).

## 1. Search is slow + returns edge-less web hits — wrong endpoint (Codex / backend)

**Observed on-device:** query "knowledge graph reasoning" via
`POST /api/v2/theseus/native-search/` (Index-API Django, Railway) took ~tens of
seconds and returned ~12 lexical-ranked web hits (all Wikipedia) with **zero
`graph_edges`**. Response badges: `search_kernel_ranked`, reason "Ranked by
lexical and source-quality signals".

**Root cause:** that endpoint does a **live web crawl + lexical rank**, so it is
(a) slow (network fetch per query) and (b) edge-less (fresh disconnected web
hits, not a substrate neighborhood).

**The fast, connected path already exists in Theorem:**
`rustyred-web/src/search.rs::search_substrate` reads the `GraphStore`
(`query_nodes` + `neighbors`) and returns `SubstrateSearch { hits, links }` where
`links` ARE the `LINKS_TO` edges among the hits — a graph read (no live fetch),
fast, and connected. `fetch_cascade.rs` is the slow live-fetch (crawl/write) side.

**Fix direction (faster crawl WITHOUT sacrificing depth = streaming-vs-spinner):**
two-tier search.
- **Tier 1 (instant):** RustyWeb `search_substrate` — paints a connected
  neighborhood from the graph immediately.
- **Tier 2 (background):** the Django live-crawl fills in fresh pages behind it,
  patching the scene as results arrive.

**Codex ask:** expose a fast RustyWeb `search_substrate` endpoint for the phone
(the `serp_server` example is close). Give me the path + request/response shape
and I swap `TheoremSearchClient` to hit tier-1 first.

## 2. Dynamic Island redesign — bottom + sole nav (Claude / Swift-UI)

Current: island is top; the projection switcher and the 5-surface tab bar are two
separate bars at the bottom. Travis's intent:

- Island moves to the **bottom**.
- It is the **SOLE navigation surface**: the 5-surface tab bar AND the projection
  switcher **collapse into the island** (the area currently showing the center-node
  pill, e.g. "Commonsense reasoning").
- It **morphs per screen** (different state/content per surface).
- The **algorithm/projection picker is exposed only on island-tap** (tap the
  center / search column to reveal Force/Rings/Tree/Fractal), not as an
  always-visible bar. Possibly the picker lives in **Settings** instead.

Reference: the `dynamic-island-toc` pattern in `Theseus/Design Components/`
(islandTransition easing, expanded/collapsed crossfade, popLayout label swap) →
`matchedGeometryEffect` + `.transition(.push)` in SwiftUI. Design-think before
building (it's a nav rebuild).

## Lane split

- **Codex:** the fast RustyWeb search endpoint + the two-tier search. Pure
  backend/Rust; no `apps/theorem-ios` edits.
- **Claude:** the island/nav redesign (`apps/theorem-ios/Sources/TheoremIOSCore/
  Views/*`) + swapping `TheoremSearchClient` to the fast endpoint once it lands.

Still open (separate): the local↔origin git divergence + Codex's uncommitted
Rust WIP need reconciling before the latest iOS commits (incl. the live-search
wiring `942a7e7`) can push.
