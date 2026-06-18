# Memory docs in the THG substrate are edgeless AND unembedded out of the box: any "cluster / color / galaxy / native-graph over memory" task must first BUILD an embed->kNN->MEMORY_SIMILAR edge builder, because nothing computes doc-to-doc semantic edges

**Kind:** gotcha
**Captured:** 2026-06-18
**Session signature:** `claude-code:travisgilbert (obsidian navigable vault + edges-over-memory)`
**Domain tags:** memory, embeddings, graph-edges, MEMORY_SIMILAR, knn, obsidian-graph, galaxy, persist_memory_document

## Trigger

Asked to "make the theorem graph compute edges over memory" so the Obsidian native graph
(and the Theseus galaxy) could cluster. I assumed some edge/embedding layer already existed.
It does not:
- `theorem-harness-runtime::memory::persist_memory_document` writes ZERO embeddings. No
  `semantic_vec`, no vector designation, on the memory write path.
- The ONLY doc-to-doc edges are authored `MEMORY_RELATES` wikilinks (the live tenant had 4
  of 236 docs linked, 5 links total) plus lifecycle edges (`MEMORY_SUPERSEDES`,
  `DERIVED_FROM`, `MEMORY_IN_PROJECT`). No `MEMORY_SIMILAR`, no kNN, no cosine anywhere over
  memory docs.
- Only `CodeSymbol` nodes get embedded (via `rustyred-thg-code/code_embed_hook.rs`, gated on
  the `CodeSymbol` label) — never memory docs.
- The read endpoint `GET /v1/tenants/:tenant/memory/docs` serializes only the doc's `links`
  array, not arbitrary graph edges, so even an edge you write won't reach the plugin without
  an endpoint change.

So the requested "viridis cluster look" was impossible from a theme, from wikilinks, or from
any existing substrate call — the graph was genuinely empty of computable structure.

## Rule

Before any task that wants memory docs to cluster, color by community, feed a galaxy, or
render a useful native graph: there is no semantic edge or embedding layer to lean on. You
must compute it. Use `rustyred_thg_memory::similarity::compute_memory_similarity_edges(store,
tenant, embedder, opts)` (embed -> O(n^2) cosine kNN -> write `MEMORY_SIMILAR` edges with a
deterministic `memory_edge_id` so re-runs are idempotent). Reuse the crate's private
`memory_nodes` (enumerates `MemoryDocument`-labeled nodes by tenant), `prop_str`, and
`memory_edge_id` from a child module — do not re-walk the store. The default `HashEmbedder`
is a deterministic offline token-hash bag-of-words: it proves the mechanism and is testable,
but it carries NO learned semantics, so cluster QUALITY needs a real SBERT/GL-Fusion encoder
behind the `MemoryEmbedder` trait. And surfacing the edges to a reader (the Obsidian plugin's
`similar` field, the galaxy) is a SEPARATE step: the docs endpoint must be extended to query
`MEMORY_SIMILAR` neighbors per doc.

## Evidence

- `compute_memory_similarity_edges` shipped in `rustyred-thg-memory/src/similarity.rs`
  (commit `bd6f3cab`); 4 acceptance tests in `tests/similarity_acceptance.rs` prove
  intra-cluster linking, no cross-cluster edges, tenant scoping, idempotent re-run, threshold
  pruning.
- `persist_memory_document` (theorem-harness-runtime/src/memory.rs ~1610) creates only
  `MEMORY_RELATES` edges from the `links` array; no embed call.
- `memory_docs_list` (rustyred-thg-server/src/router.rs ~5424) returns `links` only; a new
  `similar` field is the named follow-up to surface the edges.
- Related but distinct from the dual-recall gotcha (2026-06-17): there are two memory
  subsystems (`rustyred-thg-memory` crate vs `theorem-harness-runtime::memory`); the docs the
  plugin syncs are the harness-runtime ones, but both share the `MemoryDocument` label, so the
  similarity builder in the memory crate sees the same nodes.
