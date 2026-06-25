# SPEC: Hypergraph Statement Model

Reified statements as first-class provenance-bearing, confidence-carrying hyperedges, semiring-annotated, sitting under the egglog saturation layer.

## Context and grounding

This is the schema layer the egglog saturation spec operates over. The saturation spec (SPEC-EGGLOG-SATURATION-LAYER.md) is the engine; the zero-copy spec (SPEC-ZERO-COPY-SUBSTRATE.md) is the physical serialization; this defines the logical data model in between.

It is grounded in surfaces read this session in `crates/rustyred-thg-core/src/`:

- `epistemic.rs`: `EpistemicShadow` nodes (id `epistemic:shadow:{stable_hash([content_id, engine_version])}`), `HasEpistemicShadow` edges, `field_provenance{source_kind: structural|learned|mixed, engine, version, computed_at}`, `quarantine: true` on derived structure, `EpistemicType::{Supports, Contradicts}`, `epistemic_egraph_dedup` writing `SameEClass` member to representative edges (non-destructive, reversible, removable).
- Record APIs: `NodeRecord::new(id, [labels], props_json)`; `EdgeRecord::new(id, from, edge_type, to, props_json).with_confidence(f64).with_epistemic_type(EpistemicType).with_provenance(Provenance{source_id, timestamp, method})`; `store.upsert_node/upsert_edge/get_node/neighbors(NeighborQuery::out(id).with_edge_type(...))/query_nodes(NodeQuery::label(...))`; `stable_hash(json)`.
- `symbolic.rs`: `derived_fact(...)` already content-addresses facts, computing `fact_id = stable_hash_value({rule_id, relation, subject_id, attributes, dependency_fact_ids})`.
- `versioned_graph.rs`: the base/derived separation (asserted facts versioned, derived layer recomputable).

The decision reached this session, carried forward: do not adopt RDF as the data model. Take RDF's one good idea (the statement as a first-class, provenance-bearing, globally-identified unit with inference) and implement it as property-graph reified-statement nodes. Drop URIs, literal-as-node, blank nodes, named graphs, and IRI ceremony. Literals live as properties, not nodes.

## 1. The statement as a reified hyperedge

A statement is a node (a hypernode) connecting its roles through incidence edges, rather than a single edge between two endpoints. This is what makes the store a hypergraph: a claim binds subject, predicate, object, provenance, and context together as one addressable unit.

New module `crates/rustyred-thg-core/src/statement.rs`.

Statement identity is content-addressed so the same claim derived along different paths converges on one node (idempotent derivation, reusing the existing content-addressing discipline):

```text
statement:{stable_hash([subject_ref, predicate_key, object_ref])}
```

where `subject_ref` and `object_ref` are the entity node id for entity endpoints, or `lit:{stable_hash(value)}` for literal endpoints (the literal value is carried as a property on the statement, the hash only participates in identity). `predicate_key` is the canonical predicate string.

Constructor surface:

```text
StatementRecord::assert(subject_ref, predicate_key, object_ref, props_json) -> NodeRecord
StatementRecord::derive(subject_ref, predicate_key, object_ref, provenance, props_json) -> NodeRecord
```

Both produce a `NodeRecord` with label `Statement` and the content-addressed id above. `assert` tags `asserted: true`; `derive` tags `derived: true` and `quarantine: true` (reusing the existing quarantine convention) and carries the provenance annotation from section 3.

Incidence edges (reusing `EdgeRecord`):

- `HasSubject`: statement to subject node.
- `HasObject`: statement to object node (entity objects only; literal objects stay as the `object_value` property on the statement).
- the predicate is carried as the statement property `relation = predicate_key` by default.

## 2. Predicate representation

Predicate as a property by default (`relation` on the statement). Predicate promoted to a node (`Predicate` label, id `predicate:{predicate_key}`) with a `HasPredicate` incidence edge only when predicate-level rules exist (for example subPropertyOf or domain/range inference). Promotion is additive: the `relation` property remains, and the `HasPredicate` edge is added alongside, so the flatten view (section 7) does not depend on whether a predicate was promoted. This keeps node count proportional to the predicates that actually carry rules, not to every triple.

## 3. Provenance as a first-class annotation

Provenance is a property of the statement, not a separate edge type. Every derived statement carries a provenance annotation that is a provenance-semiring monomial: the set of source facts it was derived from, plus the rule and the semiring it is read under.

Reuse and extend the existing structures. The annotation carried on a derived statement:

```text
provenance: {
  field_provenance: { source_kind: structural|learned|mixed, engine, version, computed_at },
  dependency_fact_ids: [FactId],   // the derivation set, the provenance monomial
  rule_id: RuleId,
  semiring: Boolean | Counting | Viterbi | Why
}
```

`dependency_fact_ids` is the same derivation set `derived_fact` already threads through `fact_id` in `symbolic.rs`, lifted to the statement level. This is the single mechanism that makes confidence propagation and provenance tracking the same machinery, parameterized by `semiring`:

- `Boolean`: does the statement hold.
- `Counting`: how many independent derivations support it (feeds the support-ratio and source-independence fitness traits).
- `Viterbi`: best-path confidence (section 4).
- `Why`: which source facts contributed (the monomial itself, for audit and for correct retraction).

Provenance flows through the incidence structure: reconstructing the contributors of a derived statement is a walk over `dependency_fact_ids`, and retraction of a non-final contributor recomputes the value from the surviving contributors rather than reversing a merge (the recompute-from-surviving-contributors path the saturation spec already specifies).

## 4. Confidence as a monotone lattice value

Confidence on a statement is a bounded lattice value in `[0, 1]`, carried via the existing `EdgeRecord::with_confidence` on incidence edges and as the `confidence` property on the statement.

For the saturating layer the join is `max` (a monotone lattice join, which is what keeps egglog saturation terminating and order-independent). For conjunctive derivation (a derived statement's confidence from its premises) the combinator is `min` scaled by a per-rule decay:

```text
confidence(derived) = decay(rule_id) * min(confidence(premise_i) for i)
```

This is the Viterbi semiring (max-product) read through the fuzzy-Datalog t-norm (`min` as the conjunction t-norm). The decay is a per-rule constant grounded in the rule's evidential strength, named on the rule, not a single global magic number applied to every hop.

Non-monotone fusion (Bayesian update, Dempster-Shafer combination, noisy-OR) does not run inside saturation, because non-monotone merges break the termination and determinism guarantees. It runs as a separate pass over the materialized closure, after the fixpoint, reading the `dependency_fact_ids` to know which independent sources to combine.

Module surface:

```text
Confidence(f64)                       // newtype, clamped to [0,1]
Confidence::join(a, b) -> Confidence  // max, the saturating-layer lattice join
Confidence::conjoin(premises: &[Confidence], decay: f64) -> Confidence  // decay * min
```

## 5. Entity resolution as confidence-weighted SAME_AS

Entity merge is a confidence-weighted edge, not a hard collapse, because shared keys (email, phone) are not safe merge keys. Reuse the `SameEClass` machinery from `epistemic.rs` (member to representative, non-destructive, reversible, removable):

- `SameAs` edge between candidate entities, carrying `confidence` and the `dependency_fact_ids` that support the match.
- hard collapse to a single canonical node happens only above a corroboration threshold and only with support from independent sources (the source-independence fitness trait gates the collapse, so a single shared key cannot trigger it).

Surface:

```text
propose_same_as(store, a_id, b_id, confidence, dependency_fact_ids) -> EdgeRecord
collapse_if_corroborated(store, cluster, threshold) -> Option<NodeRecord>  // canonical node, only above threshold with independent support
```

Below threshold the match stays a reversible edge; the collapse, when it happens, is itself recorded in the versioned base so it can be rolled back and recomputed.

## 6. EDB and IDB split

Asserted statements (the EDB, the base) carry `asserted: true` and live in the versioned base graph (`versioned_graph.rs`), where their assertion history is inspectable. Derived statements (the IDB, the saturation closure) carry `derived: true` and `quarantine: true`, are reproducible from the base plus the rules, and are never the source of truth. Retraction is base-fact retraction plus closure invalidation plus recompute, which is the materialized-view-invalidation pattern the saturation spec resolves through versioning. This is the seam that lets the monotone saturation layer coexist with belief revision: monotone forward closure lives here; non-monotone revision is recompute, not in-place un-merge.

## 7. Flatten-to-triples read view

A query surface reconstructing `(subject, predicate, object, confidence, provenance)` from the incidence edges, so consumers that expect triples do not need to know the reification:

```text
flatten_statements(store, query: StatementQuery) -> Vec<FlatTriple>
```

`FlatTriple` carries the subject id, the `relation` predicate, the object (entity id or literal value), the statement confidence, and a provenance handle. The view reads `relation` off the statement and does not depend on whether the predicate was promoted to a node.

## Deliverables

- `crates/rustyred-thg-core/src/statement.rs`: `StatementRecord::{assert, derive}`, the content-addressed id function, the incidence-edge writers over `EdgeRecord`, the provenance annotation type with the semiring tag and `dependency_fact_ids`, the `Confidence` lattice with `join` and `conjoin`, `propose_same_as`, `collapse_if_corroborated`, `flatten_statements`.
- Wiring into the egglog saturation layer so saturation reads statements as facts and writes derived statements through `StatementRecord::derive` with the carried `dependency_fact_ids`.
- A migration that re-expresses existing `EpistemicShadow` derived structure as statements where the shadow already encodes a claim, keeping the shadow ids stable.

## Acceptance criteria

- The same claim derived along two distinct rule paths produces exactly one `Statement` node, confirmed by the content-addressed id being identical for both derivations.
- Given a derived statement, its contributing source facts are enumerable by walking `dependency_fact_ids`, and the `Why` semiring returns that monomial.
- Retracting a non-final contributor to a statement's confidence yields the `max` over the surviving contributors, confirmed by recompute matching a from-scratch saturation over the reduced base.
- A `SameAs` match below the corroboration threshold leaves both entities present and the edge removable; a collapse occurs only with independent-source support above threshold and is itself rolled back by reverting the base.
- `flatten_statements` returns identical triples for a predicate whether or not that predicate was promoted to a node.
