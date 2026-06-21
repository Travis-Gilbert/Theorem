# Spec 1: Extend the reflexive organ across geo, time, and properties

**Register**: execution | **Home**: Theorem, `rustyredcore_THG/crates/rustyred-thg-adapters/src/reflexive.rs` | **Date**: 2026-06-20

Follows the organizing-engine north star. This is the first slice: make the reflexive organ reach modalities it is currently blind to, inside the bounded contract it already enforces. Today the organ densifies edges over node embeddings by two-hop composition and Pairformer link prediction, and it reads only `embedding` and `features` off a record. It is blind to geography and time, and its only output is an inferred edge. This slice adds spatial and temporal candidate generation, geo and time features, and a property-level candidate output, all flowing through the existing admission and quarantine path.

## What changed from the north star, and why

The north star named three levers: geo and time into features and generators, representation targets past Node and Edge, and outputs past the inferred edge. Reading the code closes one of them. Representation sidecars already attach to any node, and CommonPlace Items, documents, and relational rows are all nodes, so the modality objects are already representable as `RepresentationTargetKind::Node`. Widening the target kind is not the lever. The three real levers are spatial and temporal candidate generators, geo and time features, and a property-level candidate output. This spec builds those.

## The invariant to preserve

`reflexive.rs` states it at the top of the file: the organ ranks and proposes within a bounded, enumerated space and does not author free-form mutation. Every new generator and output in this slice produces a candidate carrying a confidence, a `confidence_ceiling`, and an `admission_tier`, and flows through `quarantine_densification_candidates` or its property equivalent. Above the ceiling a candidate is applied; below it the candidate is a suggestion. No new path writes the graph directly.

## Deliverables

### 1. Geo features and a spatial candidate generator

- Extend `node_features` and `edge_features` to encode geography when present: the latitude and longitude properties registered through spatial designation, emitted as bounded normalized features, and the S2 or H3 cell token folded in as a categorical feature. A record with no geo adds nothing and does not error.
- Add `rank_spatial_candidates(snapshot, request)` paralleling `rank_densification_candidates`. For each seed it finds spatially near nodes through the core spatial index (`rustyred-thg-core/src/spatial.rs` and `spatial_s2.rs`, the radius and cell queries; confirm the exact query API on the branch) and proposes co-location candidates with a `proposed_edge_type` of `NEAR` or `CO_LOCATED`. Distance maps to confidence, clamped by `confidence_ceiling`. Candidates feed the existing `quarantine_densification_candidates`.

### 2. Time features and a temporal candidate generator

- Extend the feature extractors to encode time when present: recency and age derived from the record's valid-time and creation time (confirm the bi-temporal field names against the record types in `rustyred-thg-core`), emitted as bounded features.
- Add `rank_temporal_candidates(snapshot, request)`. From the bi-temporal validity on records, propose `PRECEDES` candidates for ordered pairs inside a window and `CONCURRENT` candidates for overlapping validity, scored by proximity and clamped by the ceiling, feeding the same quarantine path.

### 3. Property-level candidate output

- Add `InferredPropertyCandidate` beside `InferredEdgeCandidate`: target node id, property key, proposed value, confidence, `confidence_ceiling`, `admission_tier`, model id, and the support that produced it. This is the output past the inferred edge.
- Add a quarantine writeback for property candidates paralleling `quarantine_densification_candidates`: it writes a `ReflexivePropertyCandidate` node and the support edges and never mutates the target property below the ceiling. Above the ceiling, application is a single property upsert through the bounded mutation path, recorded with adapter provenance.
- Two callers seed it: a relational missing-value proposal, a node missing a property its neighbors or its kind imply, and a classification proposal, a `classification` scalar. Collection and tag filing stay edge candidates, since `IN_COLLECTION` and `HAS_TAG` are edges; keep this slice's property writeback to the scalar property case.

### 4. One generation entry point

- Add a generator that runs the existing graph generators plus the new spatial and temporal generators over a request and returns the merged, deduplicated, admission-sorted candidate set, so a caller gets one organizing pass across the modalities rather than calling each generator by hand. `rank_densification_candidates` and `rank_pairformer_densification_candidates` stay as they are and are called by this entry point.

## Acceptance criteria

1. A spatial generator over a snapshot with geo proposes `NEAR` candidates between co-located nodes, each carrying a confidence at or below the ceiling and an admission tier, all materialized through the quarantine path; a snapshot with no geo yields no spatial candidates and does not error.
2. A temporal generator proposes `PRECEDES` and `CONCURRENT` candidates from validity adjacency and overlap, honoring the same admission, and ignores records without temporal fields.
3. `node_features` and `edge_features` include geo and time derived features when present and are unchanged when absent, with no panic on missing fields.
4. A property candidate proposes a missing scalar value, quarantines below the ceiling as a suggestion node, and on a forced above-ceiling case applies exactly one property upsert through the bounded path with adapter provenance.
5. The bounded-contract invariant holds across every new path: no direct graph write outside the quarantine and admission flow, dry-run respected, and a candidate always carries confidence and admission tier.
6. The one generation entry point returns graph, spatial, and temporal candidates merged and admission-sorted, and the existing two-hop and Pairformer generators return identical results to before when called directly.
7. The existing `reflexive_test.rs` suite passes unchanged.

## Assumptions

- The spatial index query API in `spatial.rs` and `spatial_s2.rs` and the bi-temporal field names on the record types are confirmed on the working branch before binding. The generators read geography and time the same way the spatial index and the versioned graph already do, not from reinvented fields.
- Geo lives as designated node properties and time as bi-temporal validity on records, both already written by the substrate. This slice reads them; it does not change how they are written.
- The model id on the new candidates follows the existing reflexive-composition convention, and the admission tier defaults to the advisory inferred tier already used by densification.

## Implementation Notes

- The reflexive layer implements the SPEC-1 feature and ranker surface in `rustyred-thg-adapters/src/reflexive.rs`: spatial, temporal, geo/time features, property candidates, quarantine, dry-run, and above-ceiling property application.
- The standing-pass organizer wraps the property surface as `Spec1PropertyStandingGenerator`, alongside the spatial and temporal rule generators, so the default background pass now covers the full SPEC-1 rule generator set.
