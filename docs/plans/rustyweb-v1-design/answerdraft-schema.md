# AnswerDraft Schema (co-owned: Claude + Codex)

**Status:** Proposed contract. Co-owned because it becomes a RustyRed graph contract, not just product prose. Codex redline welcome on every `<!-- codex: -->` marker.
**Date:** 2026-05-29
**Backref:** `rustyweb-substrate-search-reframe-2026-05-27.md` §3; commonplace plan Phase 3; kernel-object-model.md §3d (AnswerDraft named as the first concrete Artifact subtype).

This is the storage shape for "answer as durable, forkable graph state." It slots into the kernel object model: AnswerDraft is the first concrete `Artifact` subtype, versioned by the Git state machine (the three-component kernel, Part 3 §1.3).

---

## Node: AnswerDraft

```
AnswerDraft:
  id: uuid
  query: string                       # the question this answers
  query_embedding: float[D]           # for FOLLOWS_FROM / related-draft retrieval
  synthesis_tier: enum { "extractive", "enriched" }
                                       # extractive = substrate-native CPU (Tier A, default)
                                       # enriched   = LLM-refined prose (Tier B, opt-in)
  body: string                        # extractive: assembled claims + citations
                                       # enriched:   prose
  confidence: float                   # epistemic confidence of the assembled answer
  session_id: string                  # provenance: which session produced it
  room_id: string                     # coordination room scope (nullable for solo)
  created_at: datetime
  superseded_by: uuid | null          # Git-versioned; a fork/contradiction creates a new draft
```

State model: append-only identity, supersedable (same as `Decision` in the kernel object model). A fork or contradiction does not mutate the draft; it creates a new AnswerDraft and sets the prior draft's `superseded_by`. This keeps the Git state machine's time-traversal/blame intact.

<!-- codex: confirm this lives in the same Prolly-tree kernel store as the coordination primitives (open Q5), and that `superseded_by` is the right supersession mechanism vs a SUPERSEDES edge. The coordination Decision kind is append-only-supersedable; AnswerDraft should match whatever pattern you settled there. -->

---

## Edges

```
GROUNDED_IN    : AnswerDraft -> Claim | Page     # provenance to every source synthesized
REFINED_INTO   : AnswerDraft -> AnswerDraft       # Tier A extractive -> Tier B enriched
                                                  # added by the CONSUMING participant, not the substrate
CONTRADICTS    : AnswerDraft -> AnswerDraft | Claim
FOLLOWS_FROM   : AnswerDraft -> AnswerDraft        # a follow-up query traverses the prior draft
```

The substrate produces Tier A (`AnswerDraft(extractive)` + `GROUNDED_IN`) always and for free. Tier B is never substrate-produced: a consuming participant (browser model, Claude, the 31B) reads the Tier A draft, refines it on its own inference budget, and writes the enriched draft + a `REFINED_INTO` edge back. This is the boundary that keeps the substrate CPU-cheap (reframe pushback 2.2).

<!-- codex: GROUNDED_IN targets Claim | Page. In the current graph, "Page" maps to the WebDoc node (HAS_WEBDOC) and Claim is the epistemic Claim. Confirm the target node labels match the live RustyRed/THG schema, or tell me the canonical labels. -->

---

## Imported-edge properties (graph-yielding broker + license-tiering)

Every edge imported by the graph-yielding discovery broker (Wikidata, OpenAlex, Semantic Scholar, Wikipedia, OSM, Common Crawl) carries:

```
source_graph   : enum { "wikidata" | "openalex" | "semantic_scholar"
                       | "wikipedia" | "osm" | "common_crawl" }
source_license : enum { "CC0" | "CC-BY-SA" | "ODbL" | "permissive" | "research-terms" }
federable      : bool   # false for share-alike (CC-BY-SA, ODbL) unless the
                        # federation contract explicitly opts in and declares the obligation
```

License-tier rule:
- CC0 / permissive (`wikidata`, `openalex`, `common_crawl`): `federable=true`, flow freely into federated tenants.
- Share-alike (`wikipedia` CC-BY-SA, `osm` ODbL): `federable=false` by default. Usable locally; excluded from federation unless the manifest opts in and states the inherited share-alike obligation.
- `semantic_scholar` (research-terms): `federable` per the source's terms; default `false` pending a terms review.

The federation manifest declares which `source_license` tiers a peer shares, so a subscriber knows the obligations it inherits before subscribing.

<!-- codex: this is the federation-shape decision (open Q2). Two implementations: (a) a separate non-federated tenant/namespace for `federable=false` edges, or (b) one tenant with a `federable=false` filter on the libp2p delta-sync. I lean (b) for simplicity; you own the delta-sync shape, so this property contract should match whichever you pick. The property `federable` works for both; (a) would additionally route by tenant. -->

---

## Open contract decisions (gate RW-V1.6)

1. Same Prolly-tree kernel store for AnswerDraft as the coordination primitives? (Q5)
2. `superseded_by` field vs a SUPERSEDES edge for fork/contradiction supersession?
3. GROUNDED_IN target labels: confirm `Claim` + `Page`/`WebDoc` against the live schema. (Q from edges section)
4. License quarantine: separate tenant vs property-gated federation. (Q2)
5. Graph-yielding import cadence: one-time pull vs ongoing federation re-sync. (Q3) Affects whether imported edges need a `synced_at` / `source_revision` property for staleness.

When these five are settled with Codex, RW-V1.1 (schema finalized) closes and the V1 build can implement against a stable contract.
