# Servo Browser-Use, build step three: abilities only the substrate can do (job-009)

**Repo:** Travis-Gilbert/theorem
**Audience:** Claude Code + Codex, building as one agent
**Plan home:** docs/plans/servo-browser-use-agent/
**Builds on:** step one (parity, job-007) and step two (engine abilities, job-008).
**License posture:** likely NOT open source. This is the OSS-plugin-over-closed-substrate layer: the value is the graph, the epistemics, and the background compute, which a CDP agent or a bare engine cannot have. Keep this layer behind the substrate boundary.
**Job linkage:** job-009, kind Feature, priority P1, target_head Either.

## North star

A browser that drives itself is a commodity in 2026. A browser whose every action lands in a grounded epistemic graph, whose extractions are checked before they are believed, and whose slow, careful work happens in the background because correctness matters more than latency, is not. The premise the user named: browsers run many background processes that do not need to be fast. The substrate is exactly the place to put the slow, valuable work that Browser Use cannot, because it has no graph to put it in.

## Deliverables

### D1: browsing IS ingestion (the in-process payoff, made real)
- Every observe()/extract() admits candidates to the graph in the same step, not as a later sync. A WebDoc, the claims on it, its source, and its provenance become nodes and edges as the agent reads, because the loop runs in-process with RustyRed.
- Admission is quarantined: open_web_unverified tier, confidence ceiling 0.35, promoted only through the existing eleven-stage epistemic filter. The agent reads the open web; the substrate decides what becomes belief.
- A task that browses ten sources leaves a connected subgraph (sources, claims, agreements, contradictions), not ten transcripts.

### D2: graph-grounded perception (the agent already knows things)
- Before fetching, PerceptionBundle resolves candidates against the in-process graph: what does the substrate already know about this entity, claim, or source. CoverageDiagnosis.needs_web fires only when the graph is genuinely thin.
- The agent skips fetches it does not need (known-and-fresh in the graph), and prioritizes sources the graph already rates as high quality. This is recall-as-PPR over the memory graph applied to live browsing: a Browser Use agent always starts from zero; this one starts from everything it has read before.
- Source reputation from accumulated outcomes: a domain the substrate has found reliable is weighted up; a noisy one down, per stored UseReceipts and the source-reliability signal.

### D3: epistemic actions in the action rail (verbs no scraper has)
The action_rail catalog already names these; this slice makes them substrate-backed, not frontend stubs:
- compare_to_graph: does this page agree or conflict with what we already believe; emit the agreement/contradiction edges.
- find_counterevidence: actively seek sources that contradict the current claim, using the graph to know what would count as counterevidence.
- verify_claim: run the claim through the epistemic filter and symbolic checks, return a grounded verdict with provenance, not a vibe.
- inspect_source_quality: surface the substrate's accumulated judgment of this source.
These turn "I scraped a page" into "I checked a claim against a belief graph," which is the Theseus differentiator at the browsing layer.

### D4: background processes (slow because correct beats fast)
The agent emits work that does not block the user and does not need to be quick. Reuse the dispatch queue (Job) and the harness scheduled lane; do not build a new scheduler.
- Consolidation: after a browsing session, a background pass merges duplicate WebDocs, compresses a source cluster into a semantic summary node, and links derived claims to their sources. (This is the memory-suite consolidation idea, applied to browse output.)
- Monitor: monitor_page as a recurring background Job that re-fetches on an interval, diffs against the stored snapshot, and raises a typed record on a meaningful change, all bi-temporally (what was true when), not just what changed.
- Deferred deep verification: a claim admitted at the 0.35 ceiling can be queued for slow, thorough cross-source verification later; promotion happens when the background check clears, not at fetch time. Latency does not matter because belief is not due on the user's clock.
- Background enrichment: re-run fractal/gap-frontier expansion on the freshly ingested subgraph to surface what is missing, as a non-blocking Job.

### D5: cross-agent, cross-session grounded continuity
- A browsing run's grounded result is shared substrate: another head (Codex, a desktop session, the composed agent) reads it through recall and builds on it, with provenance. Browser Use memory is per-session; this is the harness wedge applied to browsing.
- The composed theorems agent can browse as one identity while many models contribute, all writing to one grounded graph with one receipt stream (agent binding).

## Acceptance criteria

1. A browse_for_me run over multiple sources leaves a connected, queryable subgraph with provenance, not flat transcripts.
2. The agent demonstrably skips a fetch for an entity the graph already holds fresh, and the run log shows the skip decision.
3. compare_to_graph on a page that contradicts a stored belief emits a contradiction edge with both sources attached.
4. verify_claim returns a grounded verdict with provenance through the epistemic filter, not a model opinion.
5. A monitor_page Job runs on its interval as a background job, diffs against the stored snapshot, and raises a typed record on change.
6. A claim admitted at the quarantine ceiling is later promoted by a background verification pass, with the promotion receipted.
7. A second agent recalls a prior run's grounded result and continues from it with provenance intact.

## Fences

- This layer is not open source; it lives behind the substrate boundary as an OSS-plugin-over-closed-core boundary.
- No new scheduler and no new graph store; reuse the Job queue, the harness scheduled lane, and the existing quarantine/promotion/filter machinery.
- Quarantine is mandatory: open-web extraction never enters grounded belief without passing the filter.
- Standing no-graph-view fence holds (graph-grounded behavior, not a graph UI).

## Where it rides

The perceive/govern/afford stack in core (graph-grounded perception, epistemic actions), the dispatch Job queue and harness scheduled lane (background processes), the existing quarantine/filter/promotion path (admission and verification). rustyred-web only supplies the raw observe/extract; the value added here is all substrate-side.
