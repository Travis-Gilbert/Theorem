# GraphMutation is upsert-only: retire stale edges by generation filter, not delete

**Kind:** gotcha
**Captured:** 2026-06-10
**Session signature:** `claude-code:travisgilbert@Theorem:73b72efc`
**Domain tags:** rust, thg, graphstore, rustyredcore_thg, code-graph

## Trigger

Making code-graph reindex incremental, I needed to remove a `CALLS_SYMBOL`
edge from a carried (unchanged) symbol to a target that had *moved* inside a
changed file. Symbol ids fold in the line number
(`stable_hash(repo_id, path, kind, name, line)`), so the moved target gets a
new id and the old edge `caller -> old_target_id` dangles. I went looking for
an `EdgeDelete` / `EdgeExpire` mutation to drop it and found:

```rust
// rustyredcore_THG/crates/rustyred-thg-core/src/graph_store.rs:798
pub enum GraphMutation {
    NodeUpsert(NodeRecord),
    EdgeUpsert(EdgeRecord),
}
```

There is no edge-delete in a `commit_batch`. Nodes have TTL (`set_node_ttl` +
`purge_expired_nodes`, which write a `NodeDelete` AOF op internally) but edges
have no deletion path at all. `session_delta.rs` has `removed_edge_ids`, but
those are consumed by the in-memory `HarnessInstantKg` overlay, NOT the durable
store.

## Rule

To retire a durable edge you cannot delete, stamp every edge with a
`generation` and filter at READ time: re-emit all live edges at the new
generation on each write, and make traversal skip neighbors whose node
generation != the repo's latest generation. The latest-generation gate that
already governs symbol search then transitively hides edges into superseded
(moved/old-gen) nodes, with no delete mutation. Never assume a graph edge can
be removed in `rustyred-thg-core` — design retirement as a read-time filter or
a node-TTL expiry of an endpoint.

## Evidence

- `rustyredcore_THG/crates/rustyred-thg-core/src/graph_store.rs:798` — the
  upsert-only `GraphMutation` enum.
- `rustyredcore_THG/crates/rustyred-thg-code/src/lib.rs` — `node_is_current_generation`,
  threaded through `neighbor_symbol_names` / `expand_symbol_edges` /
  `graph_edge_record`; reindex re-emits all edges over `fresh ++ carried`.
- Commit `73b72efc`; test `reindex_edges_match_a_full_ingest_after_a_moved_target`.

## Encoded in

- `docs/learnings/2026-06-10-graphmutation-upsert-only-retire-edges-by-generation.md` (this file)
