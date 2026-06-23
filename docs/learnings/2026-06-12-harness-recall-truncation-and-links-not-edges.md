# Harness recall saturates + truncates recursively; memory links are not graph edges

**Kind:** gotcha
**Captured:** 2026-06-12
**Session signature:** `claude:travisgilbert@Traviss-Laptop:b944c683`
**Domain tags:** thg, harness, memory

## Trigger

Verifying a linked "compounding-loop" memory (`doc_b2b9f5dec1060f66`) referenced
in another memory's `links` array proved impossible through the read-only MCP
slice. Three surprises stacked:

1. Semantic `recall` for compound-engineering terms kept returning the two dense
   encoder-research docs (which literally quote `compound_engineering.rs`) and
   nothing else — they saturated the top-k for every related query, so the linked
   target never surfaced.
2. The one query that hit the target tenant returned 174 KB and was replaced by a
   `fetch_handle` envelope. Calling `tool_result_fetch` on it re-wrapped the slice
   in ANOTHER 16 KB envelope — the boundary budget applies recursively, so paging
   a large recall result burns tokens without ever surfacing the bytes.
3. `rustyred_thg_graph_query` `neighbors` from the referencing doc returned `[]`:
   memory `links` are stored as a property array, NOT materialized as
   `MEMORY_RELATES` edges, and the read-only slice exposes no get-by-doc-id. So a
   doc named in a `links` array is not retrievable by following the link.

## Rule

To fetch a specific memory by id, do not rely on `recall` (semantic, saturable by
denser neighbors), `tool_result_fetch` (recursively budgeted at 16 KB), or graph
traversal of `links` (property, not edge). Either narrow the recall query to the
target's OWN vocabulary so it ranks #1 and fits under the 16 KB budget, or accept
that the read-only slice cannot do a get-by-id and report that instead of paging a
large result to exhaustion.

## Evidence

- doc_61d164cedb3c9590 / doc_9efd3c4b91050761 both `links` -> doc_b2b9f5dec1060f66.
- `tool_result_fetch` on a 174043-byte RECALL handle returned a fresh truncated
  envelope (`returned_bytes: 16034`, `next_cursor.offset: 16034`).
- `rustyred_thg_graph_query {operation: neighbors, node_id: doc_61...}` -> `{"neighbors": []}`.
- Extends the existing `harness-run-mcp-truncation-budget` memory (harness_run 16 KB).

## Encoded in

- `docs/learnings/2026-06-12-harness-recall-truncation-and-links-not-edges.md` (this file)
