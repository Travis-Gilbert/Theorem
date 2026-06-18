# Project-scope/memory recall is implemented TWICE (rustyred-thg-memory crate vs theorem-harness-runtime::memory); the MCP `recall` tool routes to harness-runtime, and shared graph keys between the two must match byte-for-byte or membership silently vanishes

**Kind:** gotcha
**Captured:** 2026-06-17
**Session signature:** `claude-code:travisgilbert (cuts 4+5 reconcile-with-codex / verifier)`
**Domain tags:** memory, project-scope, recall, mcp-routing, anchor-id, cross-crate, silent-mismatch

## Trigger

While verifying cut 5 (permeable project scope) I found `project_anchor_node_id` defined
in TWO crates with DIFFERENT normalization:
- `rustyred-thg-memory`: `mem:project:{tenant.trim()}:{project.trim()}` (trim only)
- `theorem-harness-runtime::memory`: `mem:project:{normalize_tenant_slug(tenant)}:{slugify(project).if_empty("unknown")}`

I almost flagged this as a live bug. It is not — but only because the MCP `recall` tool
routes to `theorem_harness_runtime::recall_memory` (write + recall both slugify, so they
agree), NOT the memory crate's plugin-op `rustyred_thg_memory::recall` (which trims). The
divergence is latent: point the memory-crate recall at harness-runtime-written data with a
slug carrying caps/spaces/punctuation and the computed anchor id won't equal the written
one, the membership edge's `to_id` falls outside recall's id_set, and the project bias
disappears with NO error.

## Rule

Before reasoning about memory or project-scope behavior, know there are two recall
implementations and confirm which the surface under test actually wires: the MCP `recall`
tool -> `theorem_harness_runtime::recall_memory` (grep `recall_memory(` in
`rustyred-thg-mcp/src/lib.rs`), not the memory crate's plugin op. Any graph key shared
across the two subsystems (anchor node id, membership edge endpoints) must be byte-identical
or one side's writes are invisible to the other's reads. A cross-crate parity test that
asserts the two id helpers agree across caps/spaces/punctuation/empty is the cheap, durable
guard against drift — cheaper than collapsing the two subsystems.

## Evidence

- `rustyred-thg-mcp/src/lib.rs:3693` calls `recall_memory(` (harness-runtime).
- `rustyred-thg-memory::recall` is registered as the `memory.recall` plugin op.
- Fix 21501c67 aligned the memory crate's `project_anchor_node_id` to the harness-runtime
  formula (slugify + lowercase) and added
  `theorem-harness-runtime/tests/project_anchor_parity.rs` asserting both crates' helpers
  produce identical ids across non-trivial slugs.
