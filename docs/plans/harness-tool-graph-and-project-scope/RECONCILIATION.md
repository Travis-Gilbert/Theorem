# Harness Cuts 4 + 5: Reconciliation and Implementation Plan

Reconciles `theorem-harness-handoff-4-unify-tool-graph.md`,
`theorem-harness-handoff-5-project-scope.md`, and
`theorem-harness-revisions-cache-and-storage.md` against the live code
(2026-06-17), and against Codex's lane.

## Lane reconciliation (with Codex)

- Read at turn start: room (`room:ungrouped`, tenant `rustyredcore-theorem-production`)
  + `git status`. The HippoRAG/rerank work is shipped (`a35fa73c`); working tree
  is clean of `.rs` changes; nobody holds an intent on cuts 4 or 5.
- Codex's forward staging is `docs/reference/egglog-main.zip` +
  `rust-postgres-master.zip` (untracked) -> cuts **6 (storage spine)** and
  **9 (egglog)**. Complementary to 4/5, no source overlap.
- Dependency specs already landed in `3bd2f250`: Spec 1 (incremental Prolly
  commit), Spec 2 (native ordered index + RESP ZSET), Valkey cache-aside.
- **Decision: Claude Code owns the full 4+5 implementation** (open-lane case, not
  the HippoRAG overlap/verifier case). Footprint posted via `coordination_intent`.
  Files: `ensemble/{registry,outcomes(new),toolgraph(new)}.rs`,
  `rustyred-thg-affordances/src/registry.rs`, `rustyred-thg-memory/src/lib.rs`,
  `theorem-harness-runtime/src/memory.rs`. Stay off Codex's storage/egglog files.

## Code-grounded facts that shape the build

- `select_affordances` (affordances/selection.rs) already does task-seeded PPR x
  fitness; `record_invocation` (outcomes.rs) writes SERVED_TASK / PRODUCED_OUTCOME
  / SEQUENCED_WITH + a half-life fitness EWMA. Cut 4 is **pack-side parity +
  unification**, not rebuilding affordance selection.
- `ensemble::select` is PURE (replayable `content_address`); `select_from_store`
  fills candidates then calls it. Registry writes only PACK_SOURCE / PACK_ARTIFACT.
  No pack fitness, no pack learning edges, no live PPR. Ensemble does NOT yet
  depend on affordances.
- Affordance node ids: `affordance:{tenant}:{id}`, task type `task_type:{tenant}:{tt}`
  (affordances/types.rs). Reuse these as PPR node ids in the unified graph.
- `CsrGraph::personalized_pagerank` exists (graph_csr.rs) but uses power-iteration
  with `alpha`=walk-continuation; `graph.rs::personalized_pagerank` is forward-push
  with `alpha`=absorption. Inverted convention, different algorithm -> NOT a
  byte-parity drop-in. CSR is confined to the cache-miss compute path behind a
  ranking-agreement test, not a blind swap of the public selectors.
- Memory: write path is `theorem-harness-runtime::memory`
  (`remember_memory`/`encode_memory`/`upsert_note` -> `persist_memory_{document,node}`),
  nodes labeled `MemoryDocument`. PPR recall is `rustyred-thg-memory::recall`
  over the same label. `project_slug` already persists on the node; what's missing
  is the anchor node, the membership edge, and recall reading them.
  `harness-runtime -> rustyred-thg-memory` is acyclic, so the anchor-id + edge
  constants live in the memory crate as the single source of truth.

## Cut 5 - Permeable project scope (memory)

1. `rustyred-thg-memory`: add `MEMBER_OF` edge const, `MEMORY_PROJECT_ANCHOR_LABEL`,
   `project_anchor_node_id(tenant, project_slug)`. Extend `MemoryRecallInput` with
   `project_slug: String` + `project_permeability: Option<f64>` (serde default, so
   the field threads through the plugin-op JSON surface automatically).
2. `recall`: when `project_slug` set, fetch the anchor, add it to the id set and to
   the PPR seeds with weight = permeability. Anchor is structural only - never
   added to the ranked candidate set (different label).
3. `memory_adjacency`: special-case `MEMBER_OF` as bidirectional (anchor <-> member)
   so seeding the anchor lifts the member cluster; all other edge types stay
   directed exactly as today (no recall-semantics drift). Restructured to accumulate
   instead of insert-overwrite.
4. Write path (`persist_memory_{document,node}`): when `project_slug` non-empty,
   upsert the anchor node (idempotent), then the `MEMBER_OF` edge doc -> anchor.
   No-project memories untouched.
5. `decay`/consolidation/export already read `project_slug` off the node properties;
   confirm grouping works, leave no-project behavior unchanged.
6. Anchor-prior caching delta: the stable single-seed anchor PPR is the cacheable
   unit (scoped PPR cache); volatile activation stays applied fresh post-PPR.

## Cut 4 - Unify the learned tool graph (ensemble + affordances)

1. New pack/domain edges (consts in ensemble/registry.rs):
   `PACK_EXPOSES_AFFORDANCE` (pack -> affordance), `PACK_SERVED_TASK`,
   `PACK_SEQUENCED_WITH`, `PACK_IN_DOMAIN`, `TASK_IN_DOMAIN`. Domain node ids reuse
   the `domain` pack-kind / DomainMap ids (no parallel domain entity).
2. `PACK_EXPOSES_AFFORDANCE` written at pack registration from the pack spec
   (`spec.exposes_affordances` / `affordances` / `tools`); plus a public
   `link_pack_affordance` helper for connector-time linking. Ensemble gains a path
   dep on affordances for the id helper (acyclic: affordances does not dep ensemble).
3. New `ensemble/outcomes.rs`: `record_pack_invocation` mirroring `record_invocation`
   - writes PACK_SERVED_TASK + PACK_SEQUENCED_WITH, updates a pack fitness EWMA with
   the same half-life decay + fitness-neutral outcome rules. `effective_pack_fitness_from_node`.
4. New `ensemble/toolgraph.rs`: unified store-backed selection. ONE task-seeded PPR
   over the merged adjacency (affordance edges + pack/domain edges). Derive
   `pack_scores` for candidate packs from that vector, blend with any offline prior
   under prior_weight/lexical_weight, pass as `priors` into the PURE `select` (select
   stays pure - PPR runs in the wrapper). Affordances ranked from the same vector x
   fitness. Domain in scope biases the seeds toward that domain's packs/tasks.
   Returns affordances + packs in one ordering.
5. Per-task-type structural-prior cache delta: the single-seed task-type PPR is the
   cacheable unit; fitness applied fresh. (Scoped PPR cache.)

## Shared - Scoped PPR cache (new spec)

- Module: `rustyred-thg-core` (shared by affordances, memory, ensemble).
  Cache-aside over PPR result vectors. Key = scope + seed set + PPR params + graph
  version. Belief-revision correctness: key carries graph version, so a contradiction
  that bumps the version misses the stale entry. Stable-seed decomposition: cache the
  single stable-seed contribution; combine live query seeds by linearity; volatile
  post-PPR multipliers (fitness, activation) never cached.
- In-process default (interface identical to a future Valkey backing; Valkey is
  deploy-gated for Travis-present, consistent with the existing cache.rs posture).
- CSR routing is the cache-miss compute path, guarded by a ranking-agreement test
  vs the reference forward-push PPR on the acceptance fixtures.

## Out of scope (named, not cut)

- Cut 6 (storage spine) + its revisions deltas, cut 9 (egglog), cut 11 (invocation
  hub surface) and its disclosure-protocol question: separate handoffs; cut 6/9 are
  Codex's staged lane. The unified ranked entry (cut 4 deliverable 4) is built; the
  HTTP `search_tools`/`invoke` transport (cut 11) is explicitly a later handoff.
- Live Valkey backing of the scoped PPR cache: deploy-gated; in-process interface
  ships now.

## Acceptance criteria (proof targets)

Cut 4: (1) recorded pack outcome + empty offline priors lifts that pack via live PPR;
(2) pack fitness decays like affordance fitness; (3) pure `select` content_address
unchanged given the live-computed prior; (4) one call ranks affordances+packs,
domain reorders toward its packs; (5) trust/budget/cap gates still reject.
Cut 5: (1) high permeability -> project memories first, sibling strongly-connected
memory still appears, permeability sweeps project-only<->tenant-wide; (2) hard tenant
wall unchanged; (3) anchor seed lifts members at equal lexical; (4) in-project memory
carries project_slug + MEMBER_OF edge, no-project unchanged; (5) decay/consolidation
group by project_slug with no-project behavior unchanged.

## OUTCOME (2026-06-17): Codex implemented, Claude Code verified

The plan above was written as a Claude-Code implementation plan. Within minutes of
posting the lane intent, Codex was already solo-sprinting the full scope (it had the
same handoffs) across every file: ensemble/{registry,selector,outcomes,lib}.rs (cut 4),
rustyred-thg-memory/src/lib.rs (cut 5 recall), theorem-harness-runtime/src/memory.rs
(cut 5 write path), and the scoped-PPR-cache delta in the memory crate. Per the
Travis-confirmed division (no text-CRDT on a shared git tree; when Codex solo-sprints,
Claude Code verifies + integration-tests, never co-writes the same .rs), Claude Code
pivoted to **verifier**.

Edge-type and helper names landed slightly differently from this plan (Codex's, now
canonical): membership edge is `MEMORY_IN_PROJECT` (not `MEMBER_OF`); project scope is
implemented in BOTH the memory crate and harness-runtime rather than via a shared
helper; `record_pack_invocation` + `select_unified_from_store` + `UnifiedSelectionEntry`
match the plan.

Claude-Code deliverables (NEW files, zero edits to Codex source):
- `rustyredcore_THG/crates/ensemble/tests/cut4_acceptance.rs` (3 tests, green).
- `rustyredcore_THG/crates/rustyred-thg-memory/tests/cut5_acceptance.rs` (4 tests, green).

Verification: ensemble 24 unit + 3 acceptance, rustyred-thg-memory 10 unit + 4
acceptance, theorem-harness-runtime 107 - all green. Findings recorded to the
coordination room (record_a5dde043b4f99057):
- FINDING 1 (real, found mid-sprint, fixed by Codex): `memory_adjacency` clobbered the
  reverse anchor->member edge via `insert` after accumulating it; members sort before
  the anchor so the bias silently no-oped. Now `extend`. cut5_acceptance::c3 guards it.
- FINDING 2 (latent, not live): divergent `project_anchor_node_id` (memory crate trims;
  harness-runtime slugifies). Live MCP recall is harness-runtime end-to-end so consistent
  today; recommend one shared helper to prevent drift on non-trivial slugs.
