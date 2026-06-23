# A function named `*_incremental` that reports `reused`/`changed` counts can still be O(graph): `compile_graph_pack_incremental` cloned every prior object and rebuilt the whole Prolly tree, then set-diffed hashes to LOOK incremental

**Kind:** gotcha
**Captured:** 2026-06-17
**Session signature:** `claude-code:travisgilbert (three-substrate-specs / prolly-incremental-commit)`
**Domain tags:** rust, rustyred-thg-core, versioned_graph, prolly, performance, audit, incremental-commit

## Trigger

Spec 1 (O(changed) incremental commit) said to "verify the current commit cost before changing anything." `versioned_graph.rs` already had `compile_graph_pack_incremental`, and it RETURNED `changed_tree_nodes` / `reused_tree_nodes` -- a receipt that looks like real incremental accounting. But the body:

```rust
let mut objects_by_key = prior_pack.objects.iter().cloned().map(...).collect::<BTreeMap<_,_>>(); // clones EVERY prior object
for mutation in &batch.mutations { ... objects_by_key.insert(...); }
let objects = objects_by_key.into_values().collect::<Vec<_>>();
let tree = build_prolly_tree(&objects);  // re-chunks + re-hashes ALL objects, O(graph)
// ... then set-diff tree.nodes vs prior_pack.tree.nodes to compute reused/changed
```

The `reused`/`changed` numbers were computed by a set-diff AFTER a full rebuild, so the receipt was incremental while the computation was O(graph) in both CPU (re-hash every entry) and memory (clone every NodeRecord with payload). Trusting the name + the receipt would have "verified" an optimization that wasn't there.

## Rule

When auditing an `*_incremental` / `*_delta` / `*_diff` path, do not trust the function name or a `reused`/`changed` receipt -- read the body for the actual work. A genuine O(changed) tree commit must REUSE prior chunk nodes by hash (re-chunk only the changed leaves + the O(log n) spine) and must NOT materialize/clone the whole prior object set. If the body calls the full builder (`build_prolly_tree(all_objects)`) or clones `prior.objects`, the receipt is cosmetic. Prove flatness with a per-commit chunk/byte counter across two graph sizes (e.g. 1k vs 40k nodes), and gate byte-identity against a full rebuild behind a validation flag.

## Evidence

- Old `compile_graph_pack_incremental` (pre-fix): `objects_by_key` cloned from `prior_pack.objects`, then `build_prolly_tree(&objects)` over all objects.
- Fix: `build_prolly_tree_incremental` (windowed re-chunk reusing prior nodes by hash) + `tests/prolly_incremental_commit.rs::acceptance_commit_cost_is_flat_in_graph_size` (chunks_written stays <= ~80 and flat for a 1-node change in 1k vs 40k graphs) + `prolly_validation_enabled()` asserting incremental == full on every commit in debug.
