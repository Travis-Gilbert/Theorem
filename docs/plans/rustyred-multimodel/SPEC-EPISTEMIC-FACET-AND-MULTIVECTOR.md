# Epistemic Facet And Multi-Vector Retrieval Plan

Status: implementation started, first GraphQL facet slice landed 2026-06-23.

Source inputs:

- Current THG code in `rustyredcore_THG`, especially the shadow epistemic graph,
  GraphQL MCP surface, native relational planner, memory recall, and pg-wire
  views.
- Travis and Claude design notes from 2026-06-23 on hot graph, cold storage,
  epistemics as content relationships, SQL as projection, and shadow standing as
  a cached readout.
- Vespa documentation on binary vectors with Hamming distance and binarized
  vector tradeoffs:
  `https://github.com/vespa-engine/documentation/blob/master/en/querying/nearest-neighbor-search.md`
  and
  `https://github.com/vespa-engine/documentation/blob/master/en/rag/binarizing-vectors.md`.
- Candle's Rust ColPali example:
  `https://github.com/huggingface/candle/blob/main/candle-examples/examples/colpali/main.rs`.

## Decision

The caller-facing model is a content node with an epistemic facet, not a
separate epistemic substrate.

Canonical writes go to the graph only: lean content nodes, canonical epistemic
edges, and reified epistemic assertion nodes only when a relationship needs its
own identity. SQL views, pg-wire relations, and the epistemic shadow are
read-only projections over that graph. Large bodies and high-cardinality vector
payloads live in cold/object storage and are hydrated only when selected.

The GraphQL surface should make this feel like:

```graphql
node(id: "doc:123") {
  id
  label
  summary
  epistemic(asOf: "2026-06-23T17:00:00Z") {
    standing {
      status
      supportMass
      attackMass
      sourceReliability
      chokepointScore
      stale
    }
    relationships(limit: 20, types: [SUPPORTS, CONTRADICTS]) {
      type
      direction
      confidence
      validTime { from to }
      transactionTime { recordedAt supersededAt }
      target { id summary }
      evidenceRef
      assertionId
      sourceKind
    }
  }
}
```

The resolver may use graph traversal, a relational projection, the shadow cache,
and cold storage internally. The caller should not have to know which one
answered.

## Non-Negotiable Invariants

1. **One write source of truth.** Content nodes and epistemic relationships are
   written to the canonical graph. SQL and shadow projections are rebuildable and
   read-only. Existing mutation surfaces that apply shadow enrichment must be
   treated as internal/admin projection rebuild APIs, not ordinary user writes.
2. **Lean content nodes.** A content node stores routing metadata, a salient
   summary, hashes, object references, and hot retrieval fields. It does not
   store whole documents, evidence bodies, transcripts, or full multi-vector
   arrays.
3. **Raw relation as edge.** Simple epistemic facts are graph edges on content:
   supports, contradicts, cites, derives, tension. Edges carry confidence,
   provenance, source kind, valid time, transaction time, and evidence refs.
4. **Shadow as cached standing.** The shadow graph is not a second canonical
   ontology. It is the derived structural standing projection: propagated
   support/attack mass, grounded-extension status, source reliability,
   chokepoints, morphology, and other expensive readouts. It has bounded
   staleness through the dirty frontier and graph-version receipts.
5. **Standing is propagated, not just local degree.** One-hop support and attack
   degree may be cheap fallback fields, but the first-class `standing` result
   should come from shadow/PPR-style propagation when available.
6. **Bi-temporal epistemics.** Epistemic relationships need valid time and
   transaction time. `standing(asOf: t)` answers belief state at that valid-time
   cutoff, while transaction-time receipts explain when Theorem learned or
   superseded the relationship.
7. **Cost tiers are explicit.** Standing is cheap cached projection read.
   Relationships are medium-cost traversal/projection. Evidence bodies and exact
   multi-vector reranks are expensive cold hydrations, selected explicitly and
   bounded by limits.
8. **No unbounded fan-out.** Every facet resolver takes limits, type filters, and
   cost controls. Defaults must not hydrate cold bodies or walk arbitrarily deep.
9. **Multi-vector storage is tiered.** ColPali-style page/token/patch vectors do
   not live directly on content nodes. Hot storage gets compact binary/index
   material; cold storage keeps exact vectors and manifests for bounded rerank or
   rebuild.

## Existing Code Reality

What already exists:

- `EpistemicType` and `EdgeRecord.epistemic_type` in `rustyred-thg-core`.
- `EpistemicShadow`, `HasEpistemicShadow`, structural pass, cron pass,
  `epistemic_shadow_ppr`, and e-graph dedup in `rustyred-thg-core::epistemic`.
- `include_epistemic` on memory recall, attaching `epistemic_shadow` to recall
  provenance when requested.
- REST, gRPC, and MCP GraphQL epistemic-neighbor surfaces over the shadow/edge
  traversal path.
- Native relational planner support for joining content-shaped and
  epistemic-shaped relations, plus pg-wire demo/native views.
- A GraphQL MCP surface whose current graph node field returns raw JSON rather
  than a typed content node with an epistemic facet.

What is missing:

- A typed GraphQL content node facet that normalizes direct epistemic edges,
  reified assertions, and shadow standing into one caller-facing shape.
- A write barrier that makes SQL/shadow projection writes impossible outside
  explicit internal rebuild/admin paths.
- A first-class `EpistemicAssertion` node type and promotion rule.
- A projection contract that treats SQL and shadow as siblings downstream of the
  graph.
- A multi-vector retrieval tier that avoids keeping ColPali-scale vector payloads
  hot in graph node properties.

## Canonical Model

### Content Node

A content node is the stable identity for a memory, document chunk, task, claim,
source item, code symbol, image page, or multimodal object.

Hot fields:

- `id`
- labels/kind
- short summary or salient text
- `content_hash`
- `object_ref` or document-store pointer
- tenant/project scope
- small routing features
- optional single-vector or compact retrieval designation

Cold fields:

- full body
- full evidence text
- transcripts
- image/page binaries
- exact multi-vector arrays
- model traces

### Epistemic Relationship Edge

A simple relationship remains an edge:

```text
content:a --SUPPORTS { confidence, provenance, valid_from, valid_to, recorded_at, evidence_ref }--> content:b
```

Required edge properties:

- `epistemic_type`: supports, contradicts, tension, derives, cites
- `confidence`
- `source_kind`: human, structural, learned, imported, mixed
- `provenance`
- `valid_from_ms`, `valid_to_ms`
- `recorded_at_ms`, `superseded_at_ms`
- `evidence_ref`
- `projection_generation` only for derived/projection edges, never for canonical
  user writes

### Epistemic Assertion Node

Promote an edge into an `EpistemicAssertion` node when the relationship itself
needs identity.

Promotion triggers:

- The relation has its own body or explanation.
- Multiple evidence objects attach to the relation.
- The relation has a lifecycle: proposed, accepted, retracted, superseded.
- Another model/human makes claims about the relation.
- The relation needs independent scoring, review, permissions, or provenance.
- The relation participates as an argument object in another epistemic relation.

Promotion rule:

1. Create `EpistemicAssertion` with a stable id derived from the original logical
   relation key.
2. Move relation fields onto the assertion node.
3. Tombstone or mark the direct edge as `promoted_to = assertion_id`.
4. Rewire through the assertion:

```text
content:a --ASSERTION_SUBJECT--> assertion:x
assertion:x --ASSERTION_OBJECT { relation_type: "supports" }--> content:b
assertion:x --EVIDENCED_BY--> evidence:...
```

Facet resolvers must return the same `EpistemicRelationship` shape for both a
direct edge and a promoted assertion. The caller should not branch on storage
form unless it asks for `assertionId`.

## Projections

### SQL Projection

SQL is an access path, not an ontology. It exposes read-only relations such as:

- `epistemic_relationships`
- `epistemic_standing`
- `epistemic_evidence_refs`
- `content_index`
- `multivector_embeddings`

Example `epistemic_relationships` columns:

- `relationship_id`
- `source_id`
- `target_id`
- `assertion_id`
- `relation_type`
- `confidence`
- `source_kind`
- `valid_from_ms`
- `valid_to_ms`
- `recorded_at_ms`
- `superseded_at_ms`
- `evidence_ref`
- `graph_version`

Write attempts through pg-wire or relational GraphQL against these views must
fail with a projection/read-only error.

### Shadow Projection

The shadow stores expensive standing readouts keyed by content node, graph
version, projection version, and optional `as_of` bucket.

Standing fields:

- `grounded_extension_status`
- `support_mass`
- `attack_mass`
- `support_in_degree` and `attack_in_degree` as fallback/diagnostic only
- `source_reliability`
- `chokepoint_score`
- `community_id`
- `same_eclass`
- morphology signals such as articulation/chokepoint fragility or line-graph
  betweenness where available
- `computed_at_ms`
- `input_graph_version`
- `stale`

The dirty frontier is the invalidation contract. A content or epistemic edge
write marks affected shadows stale. Cron/full rebuild refreshes the cache.

## GraphQL Surface

Add a typed content-node path beside the existing raw `graphNode` JSON path.
Possible naming:

```graphql
type Query {
  node(id: ID!): ContentNode
}

type ContentNode {
  id: ID!
  labels: [String!]!
  summary: String
  contentRef: ContentRef
  epistemic(asOf: DateTime, includeStale: Boolean = false): EpistemicFacet!
}
```

`EpistemicFacet`:

- `standing`: cheap cached read, with stale marker.
- `relationships`: medium-cost bounded traversal/projection.
- `evidence`: explicit cold-hydrate path by ref, not default fan-out.
- `debug`: optional projection receipts and graph versions for operators.

Backcompat:

- Keep existing `epistemicNeighbors` while the facet lands.
- Internally lower it to the same projection/traversal code used by
  `ContentNode.epistemic.relationships`.
- Mark the standalone epistemic fields as expert/legacy once the facet is
  complete.

Current implementation slice, 2026-06-23:

- MCP GraphQL now has `contentNode(id)` returning the raw node plus an
  `epistemic` facet.
- `contentNode.epistemic.relationships` defaults to epistemic edge kinds only:
  supports, contradicts, tension, derives, cites.
- Direct canonical content edges are preferred. If none exist yet, the resolver
  bridges through `HasEpistemicShadow` to keep current shadow-backed enrichment
  readable during migration.
- `contentNode.epistemic.standing` returns shadow PPR scores when present and a
  direct support/attack degree fallback when the shadow has not caught up.
- `bulkEdges` accepts `epistemic_type`, so GraphQL can write a canonical
  epistemic edge and immediately read it through the content-node facet.
- Canonical epistemic edge writes mark their content endpoints
  `epistemic_shadow_dirty` for projection refresh.

## Multi-Vector Retrieval

The ColPali/Vespa note fits this plan because multimodal embeddings are another
case where hot nodes must stay lean.

Problem:

ColPali-style retrieval stores many vectors per page or document region and
scores with MaxSim: each query vector finds its best matching document vector,
then the per-query maxima are aggregated. Storing every exact float vector in
hot graph properties would explode memory before multi-user scale.

Planned tiering:

1. **Candle inference path.** Add a Rust-native ColPali encoder behind an
   optional feature in `rustyred-thg-ml` or a narrow sibling module. Use the
   Candle example as the first reference implementation, but measure local CPU,
   Metal, and CUDA behavior before making it the default.
2. **Cold exact vectors.** Store exact page/token/patch vectors in the cold
   object or array tier, referenced by a `MultiVectorEmbeddingSet` graph node or
   manifest edge. Use content hash, model id, model version, page/chunk id, and
   vector count for rebuildability.
3. **Hot binary projection.** Store bit-packed or `int8` binary vectors in a
   dedicated multi-vector access method, not as arbitrary node properties. This
   is the Vespa-style cost tier: Hamming MaxSim for candidate generation.
4. **Exact bounded rerank.** For the top N candidates only, hydrate exact vectors
   from cold storage and compute float MaxSim. N is a query budget, not an
   implementation accident.
5. **Graph-aware merge.** Feed multi-vector scores into the same planner/ranking
   system as text, PPR, source reliability, and epistemic standing. The
   multi-vector arm is a signal, not a replacement for graph retrieval.

Acceptance for the study gate:

- Implement a reference MaxSim scorer over small in-memory fixtures.
- Implement a binary Hamming MaxSim scorer and compare recall against exact
  float MaxSim on a fixed fixture.
- Measure memory bytes per page for exact float, fp16/bf16 if used, and binary
  projection.
- Prove the default GraphQL query does not hydrate cold exact vectors.
- Record a local benchmark with Candle ColPali inference before committing to a
  production ingestion path.

## Implementation Phases

### Phase 0: Contract And Audit

- Confirm whether any `EpistemicAssertion` type already exists. If not, reserve
  the label and edge names in this plan.
- Write the projection write invariant into tests or docs before changing
  behavior.
- Inventory public mutation paths that can currently write shadow or relational
  projection state.
- Confirm current `epistemic_enrich_apply` exposure and decide which operator
  scope keeps it available.

Acceptance:

- A document lists every current write path into graph, SQL projection, shadow,
  and cold object storage.
- There is one explicit rule for each path: canonical write, projection rebuild,
  or forbidden.

### Phase 1: Projection Read Model

- Add a normalized internal `EpistemicRelationshipView`.
- Add `EpistemicStandingView` sourced from shadow when fresh and degree fallback
  when absent.
- Normalize direct edges and promoted assertion nodes into the same relationship
  output.
- Add projection receipts: graph version, projection version, stale status,
  computed time.

Acceptance:

- Direct `SUPPORTS` edge and reified assertion both return the same relationship
  shape.
- Stale shadow standing is visible as stale, not silently treated as current.
- Unit tests cover no-shadow fallback and fresh-shadow propagation.

### Phase 2: GraphQL Epistemic Facet

- Add typed `node(id)` or equivalent `ContentNode` surface.
- Add `epistemic(asOf, includeStale)` with `standing` and bounded
  `relationships`.
- Add explicit evidence hydration by `evidenceRef`.
- Keep existing raw `graphNode` and standalone `epistemicNeighbors` during
  migration.

Acceptance:

- One GraphQL query fetches content summary, standing, and relationships without
  the caller naming the shadow graph.
- Selecting no evidence body performs no cold hydrate.
- Selecting evidence body hydrates only the selected refs and respects limits.

### Phase 3: Write Barriers And Projection Rebuild

- Make SQL/native projection relations read-only to external callers.
- Move shadow enrichment/apply paths behind internal/admin projection rebuild
  scope.
- Add dirty-frontier invalidation on canonical epistemic writes.
- Add deterministic rebuild from canonical graph for shadow and SQL projection.

Acceptance:

- A caller cannot mutate `epistemic_standing`, `epistemic_relationships`, or
  shadow nodes through normal GraphQL/pg-wire tools.
- Rebuilding projections from the same graph version produces the same rows and
  shadow readouts.

### Phase 4: Bi-Temporal Standing

- Add or standardize valid-time and transaction-time fields on epistemic edges
  and assertions.
- Teach standing projection to respect `asOf`.
- Add retraction/supersession semantics.

Acceptance:

- A relationship valid in 2024 and retracted in 2026 is included by
  `standing(asOf: 2024-...)` and excluded or marked superseded by
  `standing(asOf: 2026-...)`.
- Transaction-time receipts still show when Theorem learned and superseded it.

### Phase 5: Multi-Vector Study And Spine

- Add a small `MaxSim` fixture module in the planned ML/multi-vector home.
- Implement exact float MaxSim and binary Hamming MaxSim reference paths.
- Define `MultiVectorEmbeddingSet` manifests and cold exact-vector references.
- Prototype Candle ColPali ingestion on a tiny fixture.

Acceptance:

- Binary candidate generation has measured recall versus exact MaxSim.
- Memory per page/chunk is reported for exact and binary tiers.
- Exact rerank hydrates cold vectors only for bounded top N.

## Open Questions

- Should the promoted assertion rewire use `ASSERTION_SUBJECT` /
  `ASSERTION_OBJECT`, or a more domain-specific pair such as `CLAIM_SUBJECT` /
  `CLAIM_OBJECT`?
- Should the direct edge be tombstoned immediately on promotion, or retained as
  a derived traversal projection with `canonical_assertion_id`? The invariant
  prefers tombstone-plus-view to avoid two canonical facts.
- What is the right `asOf` cache granularity for shadow standing: exact query
  time, day bucket, graph-version bucket, or explicit snapshot?
- Should `epistemicNeighbors` remain in GraphQL indefinitely as an expert field,
  or become a thin deprecated alias after `ContentNode.epistemic` ships?
- Which binary layout should the multi-vector projection use first:
  bit-packed u64 blocks, Vespa-like `int8` packed bytes, or both behind a trait?
- Does the ColPali path belong directly in `rustyred-thg-ml`, or in a
  feature-gated `rustyred-thg-multivector` crate that consumes the shared ML
  runtime?

## Verification Commands

Initial implementation slices should prefer narrow oracles:

```bash
cd rustyredcore_THG
cargo test -p rustyred-thg-mcp graphql_content_node_epistemic_facet_reads_canonical_edges
cargo test -p rustyred-thg-mcp graphql_epistemic_domain_matches_flat_tools
cargo test -p rustyred-thg-core epistemic
cargo test -p rustyred-thg-memory recall_include_epistemic
cargo test -p rustyred-thg-mcp graphql_epistemic
cargo test -p rustyred-thg-core relational_core_acceptance
```

Add new tests beside the slice that changes behavior. Do not claim the projection
contract is enforced until mutation attempts against SQL/shadow surfaces fail in
tests.
