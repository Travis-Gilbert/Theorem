# All acceptance tests passing is NOT "spec met" when the ACs assert behavior-on-seeded-data but the deliverables demand a specific mechanism

**Kind:** anti_pattern
**Captured:** 2026-06-20
**Session signature:** `claude-code:travisgilbert (review + fix multimodal-planner-unify)`
**Domain tags:** spec-review, acceptance-criteria, verification-gap, relational-planner, modality-resolver

## Trigger

Reviewing `SPEC-MULTIMODAL-PLANNER-UNIFY`, all 7 acceptance tests were green — yet the implementation built private in-memory shadow indexes (`VectorState`/`TextState`/`GeoState`, fed from relational row properties via `on_write`) instead of resolving from the live TurboVec / full-text / graph subsystems, which deliverables 2 and 5 demanded four separate times ("resolves vectors from the same index that backs `vector_search` ... It does not read vectors from relational columns ... No vectors ... copied into the relational store"). The ACs all passed because every test seeded modality data THROUGH relational rows, and not one AC asserted WHICH index answered. A shadow-copy implementation satisfies every AC while silently diverging from the siloed `vector_search`/`fulltext_search` tools (different tokenizer, ANN recall, cell math) — the exact opposite of the spec's "one consistent entry point" intent.

## Rule

When reviewing against a spec, treat deliverables and acceptance criteria as TWO separate checklists. If the ACs only constrain output-given-seeded-input and never assert the data source / mechanism the deliverables require, "all green" is necessary but not sufficient — read the deliverable prose as load-bearing. Smell test: if you can satisfy every AC with a parallel shadow implementation that never touches the system the spec names, the ACs under-specify and the review must check the mechanism directly (grep the call path, confirm it reaches the named subsystem).

## Evidence

- 8/8 ACs green on the shadow-copy implementation; `from_graph_snapshot` was copying `node.properties` (incl. embeddings) onto rows, the precise thing the spec forbade.
- The fix — a per-call `ModalityResolver` resolving by node id against the live `McpGraphBackend` (`vector_search`/`fulltext_search`/`neighbors`) — changed zero AC pass/fail outcomes but corrected the architecture; the rewritten in-memory test now seeds the RESOLVER (the "subsystem"), proving by-id resolution rather than column copying.
