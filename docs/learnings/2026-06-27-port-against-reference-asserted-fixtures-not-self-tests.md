# When porting a reference system's behavior, fetch its OWN asserted test fixtures as the oracle before writing your impl and your own tests; self-authored tests calcify a plausible-but-wrong guess

**Kind:** anti_pattern
**Captured:** 2026-06-27
**Session signature:** `claude-code:travisgilbert (DATAWAVE ingest+edge intake; Codex built the reconstruction half)`
**Domain tags:** reverse-engineering, parity, datawave, normalizers, oracle-grounding

## Trigger

Porting DATAWAVE's `NumberNormalizer` to Rust, my first pass was `normalize_number` returning `format!("{n}")` -- canonical shortest decimal, so `111 -> "111"`, `42.0 -> "42"`, `3.14 -> "3.14"`. It looked obviously correct, and the unit tests I wrote alongside it (asserting exactly those outputs) passed green. That impl was wrong. DATAWAVE's `NumberNormalizer` is `NumericalEncoder`: a sign char + exponent letter + mantissa whose ASCII sort order equals numeric order, so the asserted `NumberNormalizerTest` says `111 -> "+cE1.11"`, `1 -> "+aE1"`, `-1.0 -> "!ZE9"`, `0 -> "+AE0"`. My version would have shipped a number encoding that breaks both DATAWAVE digest parity and lexicographic range-sort, and my own tests would have certified it forever. The only thing that caught it: I had dispatched a scout to fetch DATAWAVE's *asserted* test outputs (the `my-nci.csv` -> `HEADER_NUMBER 111 -> +cE1.11` row, the `NumberNormalizerTest` table, the `JsonObjectFlattenerImplTest` 25-key/29-value counts) as the oracle before finalizing. The same scout corrected three more guesses: `DateType` emits millis (`...T12:01:47.000Z`, not `...Z`), `LcNoDiacritics` does NOT trim/collapse whitespace, and the JSON flattener puts array *primitives* under the shared parent key (index only for object/array members).

## Rule

When you port a reference system's behavior (a normalizer, a parser, a serializer, an encoder), the reference's own checked-in test fixtures -- input plus the literal expected output the upstream test asserts -- are the oracle. Fetch them FIRST (a research subagent over the upstream repo is cheap), encode them as parity tests, and only then write the impl. A test you author next to your own code asserts your guess; it cannot falsify it. The reverse-engineer rule "validate against an oracle" means the upstream's asserted outputs, not your restatement of what you think the output should be. Tag the fixture provenance as asserted-by-upstream-test vs documented-not-asserted so the trust level is explicit.

## Evidence

- `crates/rustyred-thg-datawave/src/field.rs` `numerical_encode` reproduces the asserted `NumberNormalizerTest` table; `tests/parity.rs` asserts `my-nci.csv` -> `HEADER_NUMBER 111 -> +cE1.11` and `flattener-test.json` -> 25 keys / 29 values.
- First-pass `format!("{n}")` passed self-authored tests; failed the DATAWAVE oracle (`111` should be `+cE1.11`, not `111`).
- Same pass corrected date (`.SSS` millis), lc-text (no trim/collapse), and JSON array-primitive flattening against the upstream fixtures.

## Encoded in

- `docs/learnings/2026-06-27-port-against-reference-asserted-fixtures-not-self-tests.md` (this file)
