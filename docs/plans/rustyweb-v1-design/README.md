# RustyWeb V1 Design (substrate-native search engine)

**Status:** Design + schema. Claude-primary (commonplace plan ownership table). Design-heavy, sanctioned to run ahead of the Phase 0 live deploy. No Rust here: this is the contract the V1 build implements.
**Date:** 2026-05-29
**Spec source:** `Theseus/CommonPlace/Commonplaces needs/rustyweb-substrate-search-reframe-2026-05-27.md` (the reframe + the three shippability pushbacks). Phase 3 of `docs/plans/commonplace/implementation-plan.md`.
**Co-ownership:** the AnswerDraft schema and any imported-edge property that becomes a RustyRed graph contract are co-owned with Codex (see `answerdraft-schema.md`). Codex redline welcome async.

---

## What RustyWeb is (the reframe, accepted)

RustyWeb is not a crawler; it is the first substrate-native search engine. The crawler is the input pipeline. Google's product is "find URLs that answer this query." RustyWeb's product is "make this knowledge part of my substrate so I can reason over it forever." The result of a search is graph state, not a list of links.

V0 (Codex lane, Phase 2) is the boring-and-lethal crawler kernel: `link` tenant, URL frontier, robots/politeness, streaming parse, content-hash dedup, gRPC, local graph search. V1 (this doc) is what it grows into. The reframe does not bloat V0; V1 is additive.

Dual framing (Claude pushback 2.1, accepted): one binary, two audiences. OSS-developer framing = "the fastest Rust crawler whose output is a queryable graph database" (standalone, no substrate vision required). Substrate framing = "the crawl input layer of your personal search substrate." The moat is the system; the adoption path is the separable tool. Do not force adopters to buy the whole substrate vision to use the crawler.

---

## The three Claude-owned V1 pieces

### 1. AnswerDraft: two-fidelity answer-as-graph-state

The product returns synthesis with citations, persisted as queryable graph state the user can fork, contradict, and follow up on (the Perplexity move, but with a durable substrate underneath so answers become part of the user's knowledge instead of evaporating).

The load-bearing constraint (Claude pushback 2.2, accepted): the substrate stays CPU-cheap. An LLM-produced answer on the substrate's critical path reintroduces the GPU cost the substrate exists to escape and couples search latency to model availability. So AnswerDraft is two-fidelity:

- **Tier A (extractive, substrate-native, CPU, always):** the default. Pull the highest-PageRank `Claim` nodes matching the query, assemble them with `GROUNDED_IN` citations, rank by epistemic confidence. No LLM. This is a graph algorithm, exactly what the substrate is good at. Per Codex: Tier A is "compose-engine output persisted as graph state," not a new synthesis engine. The existing compose/ask path already retrieves + ranks + produces grounded snippets; the V1 work is the adapter + schema that persists that result as an `AnswerDraft(synthesis_tier="extractive")` with `GROUNDED_IN` edges.
- **Tier B (enriched, GPU, opt-in, consumer budget):** a downstream LLM refines the Tier A draft into prose. Runs on the consumer's inference budget (the browser's model, Claude, the 31B), never on the substrate. Added as a `REFINED_INTO` edge by the consuming participant, opt-in per query.

Full node + edge schema in `answerdraft-schema.md` (co-owned with Codex).

### 2. Graph-yielding discovery broker (edges, not URLs)

The current spec's discovery broker reduces providers to URL lists, discarding their graph structure. The sharpest reframe insight: pull EDGES, not URLs. The first crawl on a topic then imports graph structure from several external knowledge graphs at once, and the substrate-self-seeding property compounds immediately instead of after months. A "Tudor Revival" query pulls Wikipedia's architectural-style category graph, OpenAlex citations, LoC records, OSM building locations, and entity resolution links them.

Per-source adapters and their licenses:

| Source | Graph it yields | License | Federable by default |
|---|---|---|---|
| Wikidata | full KG via SPARQL | CC0 | yes |
| OpenAlex | citation graph + author affiliations | CC0 | yes |
| Common Crawl | link graph (implicit in WARC) | permissive | yes |
| Semantic Scholar (S2AG) | citation graph | own research-permissive terms | conditional |
| Wikipedia (MediaWiki) | link graph + category tree | CC-BY-SA 4.0 | NO (share-alike) |
| OpenStreetMap (Overpass) | geospatial topology | ODbL | NO (share-alike) |

The broker contract: each adapter emits typed edges with `source_graph` + `source_license` + `federable` properties (schema in `answerdraft-schema.md`), not URL lists. Entity resolution links imported edges to existing substrate Objects.

### 3. License-tiering and share-alike quarantine

The graph-yielding move carries a license-propagation risk nobody in the source thread except this pushback (2.3) named: ODbL (OSM) and CC-BY-SA (Wikipedia) are share-alike. If share-alike EDGES land in a federated graph that is shared with peers, the share-alike clauses can propagate license obligations to the entire federated graph. That is invisible until launch and then expensive.

The model:
- Tag every imported edge with `source_license` and `source_graph`.
- CC0 / permissive edges (Wikidata, OpenAlex, Common Crawl) flow freely into the federated `link` / `episteme` tenants.
- Share-alike edges (OSM ODbL, Wikipedia CC-BY-SA) carry `federable=false` and are excluded from default federation. They are usable locally (single-user, self-hosted) but never cross the federation boundary unless the federation manifest explicitly opts in and declares the inherited obligation.
- The federation manifest declares which license tiers a peer shares, so a subscriber knows what obligations it inherits before subscribing.

Open question for Codex (federation owner): is quarantine best as a separate non-federated tenant/namespace for share-alike edges, OR property-gated federation (`federable=false` filter on the delta-sync)? This is a RustyRed federation-shape decision; recommendation leans property-gated (one tenant, a federation filter) for simplicity, but Codex owns the libp2p delta-sync shape.

---

## Checklist (RW-V1)

Each item backrefs the reframe section it implements. Design + schema only; Rust impl is the V1 build (Codex-heavy where it touches the crawl engine, Claude on broker + schema).

- **RW-V1.1** AnswerDraft node + edge schema finalized and Codex-redlined (`answerdraft-schema.md`). Backref: reframe 1.3 + §3.
- **RW-V1.2** Tier A extractive adapter contract: compose/ask output -> `AnswerDraft(extractive)` + `GROUNDED_IN` edges. Confirm with Codex whether the existing compose engine covers it or needs a thin persistence adapter. Backref: reframe 2.2 + §5 Q1.
- **RW-V1.3** Graph-yielding broker contract: per-source edge-emitting adapter interface + the 6 source adapters' edge shapes. Backref: reframe 1.2.
- **RW-V1.4** License-tiering: `source_license` / `source_graph` / `federable` edge properties + the quarantine model + federation manifest license-tier declaration. Backref: reframe 2.3 + §3.
- **RW-V1.5** PPR frontier scoring contract (which edges/nodes seed the crawl frontier; reuse the inline PPR already shipped). Backref: reframe §4 V1.
- **RW-V1.6** Resolve the 5 open questions for Codex (below) so the co-owned contracts are settled before the V1 build starts.

## Open questions for Codex (co-owned contract decisions)

From the reframe §5, the ones that become storage/federation semantics:

1. **Tier A source:** does the existing compose engine cover extractive synthesis, or does AnswerDraft Tier A need new logic? (Codex earlier: compose-output persisted; confirm the adapter is thin.)
2. **License quarantine shape:** separate non-federated tenant vs property-gated federation (`federable=false`)?
3. **Graph-yielding import cadence:** one-time pull at crawl time, or an ongoing federation relationship with a re-sync policy?
4. **Standalone-and-surface:** one binary with a feature flag, or a workspace split, so the OSS crawler binary stays lean without the substrate-fusion code paths?
5. **AnswerDraft Git-versioning:** same Prolly-tree kernel store as the rest of the coordination kernel (Part 3 three-component kernel), confirmed?

## Sequencing

This is Phase 3 of the commonplace plan. It does not ship before Phase 0 (reliability) closes and Phase 2 (RustyWeb V0 crawler) exists. This design runs ahead so the V1 contracts are settled when the build reaches them. V2 (enriched Tier B at scale, episteme enrichment, HTTP/3, browser embedding) is out of scope here.

## Files

- `README.md` (this file): the V1 design spine.
- `answerdraft-schema.md`: the co-owned AnswerDraft node/edge + imported-edge-property schema (Codex redline surface).
