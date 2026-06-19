# A green spec-acceptance suite passed over three silent-wrong-data bugs; treating "acceptance green" as push-ready is the anti-pattern

**Kind:** anti_pattern
**Captured:** 2026-06-19
**Session signature:** `claude-code:travisgilbert (PG-WIRE + DOCUMENT-TIER engine tiers)`
**Domain tags:** testing, adversarial-review, silent-wrong-data, data-path, pg-wire, result-shaping

## Trigger

`rustyred-thg-pg-server` had **17 unit + 10 live tokio-postgres acceptance tests GREEN** against every numbered PG-WIRE spec criterion (AC1-AC7), plus clippy-clean. By the "spec is the bar" reading this looked push-ready. A 4-lens adversarial review, run anyway before pushing, found **three silent-wrong-DATA bugs** the green suite never exercised:

1. Result rows were stored in a `BTreeMap` keyed by output **display name**, so `SELECT * FROM a JOIN b` collapsed the two `id` columns -- one overwrote the other (the join returned 6 descriptors but both `id` cells carried the same value).
2. `count(DISTINCT col)` silently returned the **non-distinct** count (`FunctionArgumentList.duplicate_treatment` was dropped).
3. `knn` / `geo_within` / `text_match` modality predicates returned **all rows unfiltered** (the planner stubs them with `row_matches => true`).

Each query SUCCEEDED and returned plausible-but-wrong rows -- the worst failure class. The happy-path acceptance tests couldn't see them because none of them constructed a colliding-column join, a DISTINCT aggregate, or a stubbed predicate and asserted exact counts.

## Rule

Spec-acceptance tests prove the happy path; they systematically MISS silent-wrong-data (a query that succeeds and returns incorrect rows). Before pushing a data-path surface, run an adversarial pass that builds the corners acceptance didn't -- duplicate/colliding output names, DISTINCT, predicates the lower layer stubs, type mismatches -- and asserts exact row counts/values, not just "no error". Do not treat "acceptance green" as sufficient to push a data path. Two concrete, reusable sub-rules this produced: (a) key output rows by POSITION, never by display name (duplicate column names are legal SQL); (b) if a lower layer STUBS a predicate (returns all rows), the surface above must REFUSE that predicate, not expose it and silently return unfiltered data.

## Evidence

- All three fixed + regression-tested before push: positional `Vec<Vec<Option<ScalarValue>>>` rows (`select_star_join_keeps_both_id_columns`), `duplicate_treatment` honored + deduped (`count_distinct`), modality stubs refused with 0A000 (`rejects_unindexed_modality_predicates`). Shipped in `0d574f4`.
- The review verdict moved the work from "17 green, ship" to "3 wrong-data blockers fixed, then ship" -- i.e. the green suite alone would have shipped wrong data.
- A separate later peer review (pre-push) found a fourth class the same way: a SQL-injection seam in the param path (fixed in `069c6c7`) -- again invisible to the green acceptance suite because no test sent a hostile param.
