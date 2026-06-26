# RustyRed Adaptive Index Spine Checklist

Source spec: `/Users/travisgilbert/.codex/attachments/929849fe-ac48-425a-a364-920305dda883/pasted-text.txt`

Scope: solo Codex implementation pass for `rustyredcore_THG`, starting with Phase 0 foundations. RustyRed / `RedCoreGraphStore` remains canonical. No Memgraph, Redis, FalkorDB, pgvector, HNSW, or external graph/vector DB is required by this work.

## Grounding

- [x] Pulled latest `main`.
- [x] Read project instructions and `CLAUDE.md` navigation context.
- [x] Read the full attached spec.
- [x] Confirmed the Rust workspace root is `rustyredcore_THG/`, not the repo root.
- [x] Confirmed existing core primitives: `PlanTrace`, ordered indexes, full-text, exact/TurboVec vector fallback, H3 spatial, cold indexes, graph versions.
- [x] Keep this checklist updated as slices land.

## Phase 0: Manifest And Receipt Foundation

- [x] Add `index_manifest.rs` with `IndexManifest`, kind/backend/scope/status enums, cost/quality fields, stable state hash, and lifecycle helpers.
- [x] Add `index_registry.rs` with register/list/get/retire behavior.
- [x] Add `query_receipt.rs` with `QueryReceipt`, query kind/outcome enums, query signature, scan/hydration/candidate/timing/token fields.
- [x] Convert `PlanTrace` into a `QueryReceipt` for relational planner queries.
- [x] Add `index_proposal.rs` with proposal lifecycle, risk level, promotion thresholds, and supporting receipt IDs.
- [x] Add an EXPLAIN skeleton that reports selected access paths, scans, candidate counts, rankers, and advisor notes.
- [x] Add focused tests for manifest lifecycle, registry behavior, receipt recording, and planner trace conversion.
- [x] Validate with focused `cargo test -p rustyred-thg-core ...` from `rustyredcore_THG/`.

## Phase 1: Identity And Ordered Indexes

- [x] Add identity index registry over exact keys and hydration handles.
- [x] Enforce ambiguous identity collisions as explicit problem records.
- [x] Add scoped ordered index manifest metadata over existing `OrderedIndex`.
- [x] Prove frontier and training queues pop without scanning.
- [ ] Prove TTL queue pop behavior through the Phase 1 scoped queue surface.
- [x] Add acceptance tests for URL re-ingest, run replay, symbol identity, and collision handling.

## Phase 2: Composite, Partial, Covering

- [x] Add scope-first composite property index metadata.
- [x] Add partial predicate declarations and predicate drift invalidation.
- [x] Add covering/read-model index rows with source object version.
- [x] Prove artifact/run/card list paths avoid full graph hydration.

## Phase 3: Full-Text Upgrade

- [x] Extend full-text manifests over existing BM25 backend.
- [x] Add fielded search, code-aware tokenization, snippets/offsets, phrase/prefix/fuzzy paths as needed.
- [x] Keep Tantivy optional and non-required.
- [x] Prove exact identifier, URL/domain, phrase, and postmortem lookup behavior.

## Phase 4: Vector And TurboVec

- [x] Add vector index manifests over existing exact normalized cosine baseline.
- [x] Keep exact search as oracle.
- [x] Add TurboVec recall evaluation hooks behind the existing feature.
- [x] Enforce scope, policy, tombstone, and TTL filters.

## Phase 5: Multi-Vector Late Interaction

- [x] Extend `rustyred-thg-ml` multi-vector metadata with manifests/source offsets.
- [x] Add MaxSim rerank provenance and binary projection recall reporting.
- [x] Prove rerank improves precision while preserving source identity.

## Phase 6: Structural, Temporal, Spatial, Cold

- [ ] Add graph structural manifest support for adjacency, PPR, motifs, support/attack, bridge indexes.
- [ ] Add temporal/bitemporal manifest support and past-context reconstruction tests.
- [ ] Add H3-first spatial manifests; keep S2 optional.
- [ ] Add cold fragment skip metadata and no-false-negative tests.

## Phase 7: Index Advisor

- [x] First slice: add `index_advisor.rs` with repeated full-scan pain detection and proposal construction.
- [ ] Cluster query receipts by signature/task/scope/access path.
- [ ] Detect pain: scans, latency, candidate waste, token cost, poor recall, cold reads.
- [ ] Generate proposals with cost/risk estimates and shadow validation plans.
- [ ] Promote only after replay validation and EXPLAIN evidence.
- [ ] Retire unused/harmful indexes.

## Phase 8: Context Views, Maps, Training Runs

- [x] Add `context_view.rs` preserving source artifacts/runs/maps, included and excluded atom IDs, labels, summary, handles, graph version, freshness.
- [x] Add `map_artifact.rs` with stable section IDs, hydration handles, freshness, usage/outcome labels.
- [x] Add `labeled_training_run.rs` with positive and negative labels for context/tool/adapter/validator/memory/map/artifact outcomes.
- [x] Add `training_export.rs` JSONL export helpers with graph version, redaction status, and atom/map identity.
- [ ] Add map diffs and refresh/regeneration behavior.

## Phase 9: Inspection Surface

- [ ] Add product-server routes for index manifests, receipts, advisor proposals, context views, maps, training runs, and export validation.
- [ ] Add dashboard/explorer surfaces after core APIs are stable.

## Verification Matrix

- [ ] Index lookup equals scan baseline.
- [x] TurboVec recall measured against exact.
- [x] Multi-vector rerank preserves source identity.
- [ ] Graph version/predicate/TTL/tombstone invalidation works.
- [x] Repeated full scan produces an advisor proposal.
- [ ] Bad proposal is rejected.
- [x] Included/cited atoms become positive labels.
- [x] Dismissed/wasted atoms become negative labels.
- [x] Scope and policy filters cannot be bypassed.
- [ ] Redaction blocks unsafe exports.
