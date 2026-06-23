# North-Star: The Organizing Engine

**Register**: north-star | **Home**: Theorem (rustyredcore_THG) | **Date**: 2026-06-20

## What this is

The organizing engine is what makes "a multi-modal self-organizing personal database" a true claim rather than a tagline. It is the standing capability that takes anything entering RustyRed and, across every modality the store holds, extracts what it contains, resolves it against what already exists, classifies and files it, and links it to what it relates to. The Auto-Organizer is its face. This is the engine beneath the face.

## Two halves, one engine

The capability already exists in two disconnected pieces.

`commonplace/src/ingest.rs` runs the four moves once, at write time, on a single item: it sets the embedding, a classification, and tags. `reflexive.rs` runs a subset continuously, after the fact: representations and edge densification. But the standing piece is graph-shaped, it links by embedding similarity, and it files nothing it cannot express as a node or an edge.

These are not two systems. They are the ingestion pass and the standing pass of one organizing engine. The north star is to make them one: a single bounded candidate engine that both the ingestion path and the standing path feed, producing the same output shape, a structural candidate with a confidence and an admission tier, governed by the same policy.

## The invariant

The engine ranks and proposes within a bounded, enumerated space. It does not author free-form mutation. This is the contract `reflexive.rs` already states, and it is what keeps a self-organizing system from being a self-corrupting one.

Every structural change the engine wants to make is a candidate with a confidence, and one dial decides the rest. Above a ceiling, the candidate is applied automatically, which is the "no button, it just gets organized" behavior. Below it, the candidate becomes a glance-able suggestion in the stream, never a backlog to clear. The `admission_tier` and `confidence_ceiling` fields are already that dial.

## Across modalities

Today the engine is graph-shaped. The north star is the four moves in each modality's own terms:

- Vector clusters propose membership and representatives.
- Spatial proximity (H3 and S2) proposes co-location links.
- Temporal order (the bi-temporal validity already on every record) proposes precedence and succession.
- Relational structure proposes a missing value or a row link.
- Document structure proposes section and passage links.

The mechanism is the extension already sketched for the reflexive organ: widen its representation targets past Node and Edge, feed geo and time into its feature extraction and candidate generation, and widen its outputs past the inferred edge to modality-native candidates. All of it inside the bounded, quarantined contract above.

## Where it triggers

The standing pass fires from the existing hook system: the `HookRegistration` seam in `hooks.rs` that plugins already use, and the dispatcher that runs on store mutation. A write to any modality is a mutation, and a hook turns that mutation into an organize trigger. The same hook seam is the changefeed the UI subscribes to, so one mechanism both organizes the write and surfaces it. The ingestion pass stays at write time and is reconciled to emit the same candidate type as the standing pass, so there is a single downstream.

## Where the intelligence comes from

The hard, model-driven parts of the four moves, understanding what an arbitrary input means and resolving it against existing entities, are Theseus's competence, not the substrate's. Theorem holds and links; Theseus understands and structures. In the engine, Theseus supplies extraction, entity resolution, and classification as organizing passes over RustyRed Items, while the substrate supplies storage, the reflexive representations, and the continuous densification. The division is by role, not by which system the user looks at, and neither shows the user a control panel.

## The surface it powers

The Auto-Organizer renders the engine working. A dropped input resolves into its kind, files into its collection, and draws its links, with the high-confidence candidates applied and the rest offered as a stream. The drag-and-drop animation is the legibility of the engine, not decoration: it is the moment the user sees the database organize itself.

## First execution slice

Extend the reflexive organ's reach inside its existing contract: geo and time into the feature extractor and candidate generators, representation targets past Node and Edge, outputs past the inferred edge. This is the smallest step that makes the multi-modal claim begin to come true, and it is backend-only, in Theorem, with no dependency on the UI or the desktop work. It is the natural execution handoff that follows this north star.

## Open forks, held here

- Where Theseus's passes run: native in Theorem for the ingest hot path, or in the Django service for the heavier epistemic reasoning, reached through the GraphQL and MCP contract. The boundary principle from the PG-wire decision applies: native for hot and load-bearing, service for the rest.
- Default aggressiveness of auto-apply: a high ceiling that files little and suggests much, against a lower ceiling that files confidently. Likely per-modality and per-user, tuned by the observed correction rate.
- One generalized organ against per-modality organs sharing the contract: whether the engine is a single candidate generator that reads every modality, or a set of modality organs feeding one admission policy and one stream.
