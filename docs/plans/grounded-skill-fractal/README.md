# Grounded Skill API + Native Rust Fractal Expansion (two-spec co-execution)

Date: 2026-06-02
Owners: Claude Code (Lane A) / Codex (Lane B)
Status: PLAN locked. Co-execution across two agents, two seams.
Source specs: `~/Downloads/grounded-skill-api-spec.md`,
`~/Downloads/rust-fractal-expansion-spec.md` (both 2026-06-02).
Embedder decision (Travis 2026-06-02): the 8B (Qwen3-Embedding-8B +
Qwen3-Reranker-8B).

## Why these are one task

The two specs interlock. The grounded-skill-api query path (spec 1) "runs code
search and fractal expansion over the ingested corpora": that fractal expansion
IS spec 2. And both retrieval paths run on the same vectors. Spec 1's embedder
upgrade populates the HNSW index that spec 2's stage-1 frontier reads, and spec
2's stage-4 cross-encoder is the same Qwen3 family. So the embedder is a shared
dependency and the fractal tool is a shared consumer. Building them together
forces exactly the two seams the synthesis needs, with no grand integration.

## What already exists (do not rebuild)

- Python skill encoder: `Index-API/apps/notebook/encode/` (17 modules):
  `build_capability_pack_spec`, `canonical_content_hash`, `run_encoding_pipeline`,
  `UseReceipt`; `CapabilityPackSpec` at
  `Index-API/apps/orchestrate/registry/capability_packs.py:53`. Rust corpus
  worker (Lane G G0/G1) done. Prior plans:
  `Theorem/docs/plans/skill-encoder-theorem-port/implementation-plan.md` and
  `Index-API/docs/plans/skill-encoder/`.
- Rust substrate primitives (confirmed in tree 2026-06-02, do not invent):
  - PPR: `rustyred-thg-core/src/instant_kg.rs:335 InstantKG::ppr`
  - global PageRank: `rustyred-thg-core/src/graph.rs:377 pagerank`
  - HNSW vector kNN: `rustyred-thg-core/src/graph_store.rs:1287` + `:2336 vector_search`
  - full-text / BM25: `rustyred-thg-core/src/fulltext.rs` + `fulltext_tantivy.rs`
  - web search: `rustyred-web/src/search.rs:461 search_substrate` + `SubstrateSearch`
  - live fetch: `rustyred-web/src/fetch_cascade.rs`; crawl + ingest:
    `rustyred-web/src/lib.rs` `CrawlRequest` / `LiveFetchOptions` /
    `CrawlGraph::apply_to_store`
  - lower-trust provenance + license: `rustyred-web/src/lib.rs` `WebCommons*` +
    `SourceLicense`; source typing: `source_class.rs`; gating precedent:
    `trigger_gate.rs`
  - MCP exposure: `rustyred-thg-mcp/src/lib.rs`
- `reconstruction-engine` does NOT own gap/admit/frontier (grep empty
  2026-06-02), so spec 2 is a NEW crate, not a module there.

## The split

| Lane | Owner | Spec | Surface | Collision |
|---|---|---|---|---|
| A | Claude Code | spec 1 product + shared embedder | Index-API (Python/Modal) + Theorem serving cross-ref | none (no hot Rust) |
| B | Codex | spec 2 Rust fractal crate | Theorem `rustyredcore_THG/crates/rustyred-thg-fractal` (greenfield) + MCP exposure | greenfield is safe; MCP `lib.rs` is hot, claim first |

Rationale: Codex is already deep in `rustyred-thg-mcp` / `rustyred-web` and owns
Rust-in-process work, so the fractal crate is his territory. The embedder and the
open-standard product layer are Python/Modal (encoder-stays-Python), zero
collision with his hot files. Both agents drive in parallel.

## The two seams

Seam 1 (embedder -> vectors). Lane A stands up Qwen3-Embedding-8B and backfills
code+text vectors into RustyRed HNSW via `rustyred_thg_vector_designate`. The
contract Lane A publishes to Lane B: vector property name, dimension(s) (pinned
at build), and which node classes carry which embedder. Lane B's stage-1
`vector_search` reads exactly those. Same atoms-seam pattern this repo already
uses across agents.

Seam 2 (fractal tool -> query path). Lane B exposes the fractal expansion MCP
tool (supersedes the Python `theorem_fractal_expansion`). Lane A's skill-API
query path calls it (or the existing Python tool until B lands). Contract: query
in, grounded source + provenance out.

Shared model: Qwen3-Reranker-8B cross-encoder. Lane A stands it up on Modal;
Lane B's stage-4 calls it. One model family end to end (spec 2 "hybrid retrieval
cascade").

## Build sequence

1. Plans locked (this dir). [done]
2. Lane A: open-standard skill exporter (product output contract, zero infra). [in progress]
3. Lane A: Qwen3-Embedding-8B + Qwen3-Reranker-8B Modal apps + vector backfill;
   publish seam-1 contract.
4. Lane B: rustyred-thg-fractal stages 1-2 (hybrid frontier + rerank), reading
   seam-1 vectors.
5. Lane B: stages 3-5 (web reach + cross-encoder gate + lower-trust ingest) +
   MCP exposure (seam 2).
6. Lane A: upload->tenant path + query path wired to seam 2.
7. Both: integration acceptance (real query -> grounded skill; real fractal run
   -> lower-trust ingest).

Baseline embedder now; substrate fine-tune later is spec 1's own explicit
sequencing, not a scope cut.

## Validation gates

- Lane A unit: exporter emits a spec-valid Agent Skills folder (frontmatter
  name + verb-description, scripts/ executable, provenance block), test-first.
- Lane A infra: backfill writes N vectors; `vector_search` returns them.
- Lane B: `cargo test -p rustyred-thg-fractal`; the pipeline type has no
  graph-only terminal state; stage-5 admits as `open_web_unverified` + quarantine.
- Integration: a real query through the live product yields a grounded skill
  folder whose provenance cites real ingested source. No mock data, no demo route
  (project rules).

## Coordination

The native rustyred-thg MCP coordination server is UP; the plugin Django backend
is returning 500. Channel: `mcp__rustyred-thg__coordinate` plus path-scoped
commit messages (always `git commit -- <pathspec>`, never a bare commit, shared
index). Codex's live workstream is the plugin-switchover (THPS-*), separate from
this; the one open mention to claude-code (THPS-005 event surface) is unrelated
to this task and is not consumed here.
