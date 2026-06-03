# Lane B: Native Rust Fractal Expansion (spec 2) - CODEX

Date: 2026-06-02
Owner: Codex (planned + handed by Claude Code)
Source: `~/Downloads/rust-fractal-expansion-spec.md`

## Goal + the defining invariant

Fractal expansion is a CORPUS-EXPANSION mechanism, not retrieval. It must NEVER
terminate at the graph. Every run exhausts the graph to the gap frontier, then
ALWAYS reaches the web, then ALWAYS ingests survivors. The web step is
unconditional. Make this a property of the TYPE: the pipeline exposes no
"graph only" terminal state. [spec "The defining invariant"]

Contrast: `similar_situation_search` is the gated counterpart (web as last
resort, to solve). Fractal always reaches the web (to grow). Same crawler,
opposite trigger. [spec "The defining invariant"]

## Where it lives (resolved 2026-06-02)

NEW crate `rustyredcore_THG/crates/rustyred-thg-fractal`. `reconstruction-engine`
does NOT own gap detection / admission (grep for gap|admit|ingest|frontier was
empty), so it is a crate, not a module there. [spec "Where it lives", "Sequencing"
step 1]

Composes (real primitives, do not reimplement):
- frontier: `rustyred-thg-core` `InstantKG::ppr` (instant_kg.rs:335) +
  `GraphStore::vector_search` (graph_store.rs:1287) + `fulltext` /
  `fulltext_tantivy` BM25 for seed selection.
- web: `rustyred-web` `search_substrate` (search.rs:461) + `fetch_cascade.rs` +
  `CrawlRequest` / `LiveFetchOptions`.
- ingest: `rustyred-web` `CrawlGraph::apply_to_store` + `WebCommons*` +
  `SourceLicense` (the lower-trust tier rides this existing provenance machinery).
- exposure: `rustyred-thg-mcp` (hot lib.rs, CLAIM the seam before editing).

## The five stages (each backrefs a spec section)

- [ ] B1 Hybrid frontier: `ppr` (structural) UNION `vector_search` HNSW kNN
  (semantic), deduped; optional BM25 seed selection. The gap is where the walk
  thins (sparse neighborhoods, or query-relevant vector regions with few graph
  nodes nearby). Reads Lane A seam-1 vectors. [spec "Stage one"]
- [ ] B2 Frontier rerank (cheap, over the whole frontier): learned-scorer signal
  + the PPR score + the vector cosine, NO cross-encoder. Output = the web-launch
  seeds. [spec "Stage two"]
- [ ] B3 UNCONDITIONAL web reach: `search_substrate` + crawl seeded by the
  reranked frontier (titles, entities, embeddings become the web queries). Runs
  every time. [spec "Stage three"]
- [ ] B4 Cross-encoder rank-before-ingest: Qwen3-Reranker-8B (CodeRankLLM for
  code) on the small fetched set; a relevance gate AND a quality gate. Calls Lane
  A's reranker service. [spec "Stage four"]
- [ ] B5 Lower-trust BATCHED ingest: tie each admitted doc to the closest
  frontier seed (the gap it closed); provenance class `open_web_unverified`; a
  confidence ceiling; a quarantine flag keeping fresh web nodes out of
  authoritative retrieval until corroborated or promoted; one batched commit per
  run. [spec "Stage five", "Write-path discipline"]

### Borrowed elements (non-optional where the spec says so)
- [ ] B6 Security / quarantine (AgentShield): a distinct lower-trust tier; content
  sanitized for prompt-injection BEFORE it touches the graph; stage B4 is the
  enforcement point (quality + sanitization gate). [spec "Security and quarantine"]
- [ ] B7 Extract-before-ingest (Scrapling): extract the relevant content from each
  fetched doc before ranking and admission; the graph stores claims, not
  boilerplate. [spec "Extract before ingest"]
- [ ] B8 Adaptive element tracking (Scrapling) for rustyred-web: store a
  similarity fingerprint of the targeted elements; relocate content by similarity
  when a source site changes structure. [spec "Adaptive element tracking"]

### Exposure + supersession
- [ ] B9 Expose through `rustyred-thg-mcp` as the fractal expansion tool (seam 2;
  Lane A's query path calls it). [spec "Where it lives" exposure line]
- [ ] B10 Supersede the Python `theorem_fractal_expansion` at the MCP layer:
  retire it or rename it to a graph-only explorer so it is not mistaken for
  fractal expansion. `similar_situation_search` stays separate and gated (shares
  the web reach + cross-encoder, differs only in triggering). [spec "Relationship
  to the existing tools"]

## Seam with Lane A

- Reads: Lane A publishes the vector contract (property name, dim(s),
  class->embedder map). B1's `vector_search` must match it.
- Calls: Lane A's Qwen3-Reranker-8B service at B4.
- Exposes: the fractal MCP tool Lane A's query path consumes.

## Sequencing (spec)

Hybrid frontier -> frontier rerank -> unconditional web reach -> cross-encoder
gate -> lower-trust batched ingest with sanitization. The web reach and the
lower-trust ingest are the two pieces that make it fractal expansion rather than
graph search; build them as NON-OPTIONAL stages. [spec "Sequencing"]

## Acceptance (the floor)

- `cargo test -p rustyred-thg-fractal` green.
- The pipeline TYPE has no graph-only terminal state (the invariant is
  structural, not a runtime flag).
- A real run: query -> hybrid frontier -> rerank -> web reach -> cross-encoder
  gate -> batched admit of survivors as `open_web_unverified` + quarantine, each
  tied to its gap seed.
- The MCP tool supersedes the Python one; `similar_situation_search` stays gated
  and separate.

## Codex handoff note

Greenfield crate = safe to build without claiming. The ONLY hot seam is
`rustyred-thg-mcp/src/lib.rs` (B9), claim before editing (the duplicate-module
lesson). The cross-encoder you call at B4 is the same component the
retrieval-cascade spec wants, so B4 also serves `similar_situation_search`.
