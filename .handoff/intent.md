# Handoff: turbovec first slice

Implement the turbovec first slice. The full spec is in the harness memory (recall `doc_36cac7917873a814` / turbovec-integration-spec.md, tenant "default") if the harness MCP is available to you; the scope below is self-contained if it is not.

Scope to the first two sequencing steps ONLY. Leave the rustyred-web/search.rs repoint and the MCP exposure for a follow-up PR.

## Step 1: dependency hygiene (unblocks the build)

The turbovec dependency must build on RustyRed's pinned stable Rust. AVX-512 intrinsics stabilized in Rust 1.89 but RustyRed pins 1.85+, so sub-1.89 needs a cfg gate, not just the runtime `is_x86_feature_detected!` check turbovec already has. Either pin the workspace `[patch]` for turbovec to a specific rev or tag (NOT a branch: branch pins break on `cargo update` or force-push), or add the stable-Rust cfg gate. Pick the lower-risk option and make `cargo check --workspace` green.

## Step 2: lift the codec into the core as a per-class quantized vector MODE

In `rustyredcore_THG/crates/rustyred-thg-core/src/graph_store.rs`, next to the existing HNSW layer (instant-distance), add a quantized mode:

- Extend the per-class vector designation with `mode = hnsw` (default, full-dim, hot) | `quantized` (TurboQuant codes in a turbovec `IdMapIndex`).
- Add `GraphStore::vector_search_quantized` that calls turbovec with an allowlist of candidate node pks for the active tenant/scope.
- Contract: `IdMapIndex` keyed by RustyRed node pk (u64) so the compressed index and the graph share one id space; on upsert quantize and `add_with_ids(vector, node_pk)` with no train step; on delete `IdMapIndex.remove(node_pk)`; on query pass the tenant/scope allowlist so the kernel short-circuits non-allowed blocks.
- For a quantized class store ONLY the compressed codes (drop the f32); persist the `.tvim` alongside the tenant snapshot and load it on restart.
- HNSW stays unchanged for hot classes. This is additive, not a replacement.

## Tests

Add tests for the quantized designation and `vector_search_quantized`: add, delete, and allowlist short-circuit.

## Output

Open a PR against `main` and do NOT merge it. If the harness MCP is available to you, write a coordination record in the default-tenant room so this work shows up alongside Codex and Claude Code.
