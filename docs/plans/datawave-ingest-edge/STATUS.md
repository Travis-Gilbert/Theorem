# DATAWAVE intake + edge layer: status

This plan lands the DATAWAVE-style intake half of the reconstruction North Star:
turn any source record into typed normalized field-facts plus declared
entity-edges in the RustyRed graph, so a crawled URL, a parsed repo, an ingested
document, and a reconstructed binary all intersect in one self-describing corpus.

- North Star register: [NORTH-STAR-RECONSTRUCTION-ENGINE.md](NORTH-STAR-RECONSTRUCTION-ENGINE.md)
- Reference scout: [DATAWAVE-SCOUT-INGEST-AND-EDGE.md](DATAWAVE-SCOUT-INGEST-AND-EDGE.md)
- Oracle: `NationalSecurityAgency/datawave` @ `integration` (the repo's default branch).

The Ghidra / binary-reconstruction half (loader, disasm, lift, reconstruct,
reconstruct-harness, engineering-compiler, program-analysis, native-loader,
Ghidra-oracle) is built and lives in `rustyred-thg-{binformat,disasm,lift,
reconstruct,reconstruct-harness,code}`. This plan is the complementary write-side
intake; the two compose because they share the GraphStore.

## What landed

Crate: `rustyredcore_THG/crates/rustyred-thg-datawave` (workspace member, sync,
trait-generic over `GraphStore`; deps = `rustyred-thg-core` + serde + sha2). All
nine DATAWAVE build phases, lint-clean, 29 tests green (26 unit + 2 parity + 1
doc-test).

| Spec build phase | Module | Key types / fns | Tests |
|---|---|---|---|
| 1. Raw-record + data-type model | `record.rs` | `RawRecord`, `RecordBody`, `TypeRegistry` | record::tests |
| 2. Record -> field-facts (the keystone ingest-helper) | `helper.rs` | `IngestHelper`, `derive_fields`, `CsvHelper`, `JsonHelper`, `MappedHelper` | helper::tests + parity |
| 3. Field config + normalizers | `field.rs` | `FieldConfig`, `IndexPolicy`, `FieldType` (lc-text, NumericalEncoder number, date, ip, geo) | field::tests |
| 4. Composite / virtual / aliased fields | `field.rs` + `helper.rs` | `CompositeDef`, `VirtualDef`/`VirtualTransform`, alias map | field::tests |
| 5. Materialization to the index | `materialize.rs` | `materialize_event` writes `IngestEvent`/`FieldFact`/`FieldEntity`; `vf="field=value"` rides the property index | materialize::tests |
| 6. Edge model | `edge.rs` | `EdgeDef`, `EdgeCondition` (present/equals/not-equals + all/any/not boolean gate), `version`, `derive_edges` | edge::tests |
| 7. Dictionary | `dictionary.rs` | `data_dictionary`, `edge_dictionary`, `write_dictionary` (self-describing nodes) | dictionary::tests |
| 8. Visibility + masking | `field.rs` + `materialize.rs` | `MaskRule`, per-fact `visibility`, masked alternate carried as properties | field::tests |
| 9. Content + fuzzy hashing | `hash.rs` | `content_hash` (sha256 dedup), `fuzzy_hash`/`fuzzy_compare` (ssdeep via `fuzzyhash` crate) | hash::tests |
| Driver | `lib.rs` | `DatawaveIngest::{ingest_record, ingest_batch, write_dictionary}` | tests |
| Next Build Cut: ingest parity lane | `tests/parity.rs` | DATAWAVE `my-nci.csv` + `flattener-test.json` as oracle | parity (2) |
| Tiered retrieval (global+field index) | `tiered.rs` | `TieredIndex` (value+field->fragment, cardinality threshold, `intersect`/`union` pushdown) | tiered::tests |
| Parity receipts -> training stream | `training.rs` | `ParityReceipt`, `write_parity_receipt`, `export_parity_receipts` (JSONL into the shared stream) | training::tests |
| Agent surface (harness pack) | crate `rustyred-thg-datawave-harness` | `DatawaveIngestPlugin` / `theorem.ingest.datawave`: describe/record/batch/lookup/intersect | harness tests (RedCore) |

## Parity oracle (the grounding)

`tests/parity.rs` runs DATAWAVE's own checked-in fixtures through these contracts:

- CSV (`NormalizedContentInterfaceTest` / `my-nci.csv` / `norm-content-interface.xml`):
  asserts the six field-facts' raw + normalized values and index policy, including
  the asserted `NumberType` encoding `111 -> +cE1.11`, `DateType`
  `2024-02-29 12:01:47 -> 2024-02-29T12:01:47.000Z`, and the TEXT-field
  index-disallowlist.
- JSON (`JsonObjectFlattenerImplTest` / `flattener-test.json`): asserts the
  NORMAL-mode flatten counts (25 distinct keys, 29 values, `DATE` x3, array
  primitives sharing the parent key), including the nested inner-array leaves.

The normalizer unit tests in `field.rs` additionally pin the asserted
`NumericalEncoder` table (`1->+aE1`, `-1.0->!ZE9`, `0->+AE0`, `2147483647->
+jE2.147483647`) and the `DateNormalizer` outputs (compact, pipe, micros-truncated).

## How it composes with the reconstruction half

- One graph: `FieldFact` / `FieldEntity` / `IngestEvent` nodes land beside the
  binary-reconstruction facts via the same `GraphStore` write path and the same
  `sha256:<hex>` content-address convention as `rustyred-thg-binformat`.
- Shared similarity: `hash::fuzzy_hash` is the content-similarity primitive the
  spec earmarks for binary similarity in the reconstruction corpus.
- Universal front door: `MappedHelper` (a config-driven field map) makes "point at
  a URL / repo / API and ingest by configuration" real with no bespoke Rust per
  source, which is DATAWAVE's "no loader per source" thesis.

## Completed in the floor pass (the four named deferrals are now built)

- Agent-facing harness surface: `rustyred-thg-datawave-harness`, the
  `theorem.ingest.datawave` capability pack (a `RustyRedPlugin` mirroring
  `rustyred-thg-reconstruct-harness`) with five operations -- `ingest.describe`,
  `ingest.record`, `ingest.batch` (optional dictionary), `ingest.lookup`,
  `ingest.intersect`. Fully data-driven: a serializable `HelperSpec`
  (csv/json/mapped) configures the data-type in the request, so "point at a source
  and ingest by configuration" works over the plugin bus. Lookup/intersect read
  persisted `FieldFact` nodes via the value+field property index. Proven end to end
  over a real `RedCoreGraphStore` (record -> lookup -> intersect, plus dry-run).
- Dedicated cardinality-tiered global/field index: `tiered.rs`. A small global
  index (value+field -> fragment) for low-cardinality fields, a per-fragment field
  index resolving events, a cardinality threshold that keeps the global tier small,
  and boolean `intersect`/`union` pushdown. Tested: a high-cardinality field stays
  out of the global tier (the boundedness guarantee).
- ssdeep byte-parity: `hash.rs` now uses the `fuzzyhash` crate, a pure-Rust
  ssdeep / spamsum implementation producing ssdeep-compatible `blocksize:h1:h2`
  digests (no C dependency). The hand-rolled CTPH was deleted.
- Parity-receipts -> training stream: `training.rs`. A `ParityReceipt` persists as
  a `DatawaveParityReceipt` node (mirroring the reconstruction `ValidationReceipt`
  pattern) and converts into the shared `LabeledTrainingRun` ->
  `TrainingExportRecord` JSONL stream (`rustyred_thg_core::{labeled_training_run,
  training_export}`), the same surface the reconstruction lane feeds. Tested:
  receipts export to JSONL with `validator_policy` / `parity_pass` labels.

## Remaining optional upgrades (beyond the nine phases; not blocking)

Each is a fidelity upgrade within an already-satisfied phase, gated on a real
corpus need or a dependency decision:

- Regex edge preconditions (JEXL `=~`): the boolean gate now covers
  `&&`/`||`/`!`/`!=` (`EdgeCondition::All`/`Any`/`Not`/`FieldNotEquals`); regex
  leaves would add a `regex` dependency.
- `NumericalEncoder` BigDecimal-exact long negative mantissas and `GeoNormalizer`
  geohash/S2 form: f64 + fixed-precision reproduce every asserted case; the exact
  long forms ride a typed ordered / `spatial_s2` index designation.
- Full Unicode NFD diacritic stripping (currently Latin-1 fold) and the textual
  `EEE MMM dd ... yyyy` date form: the asserted fixtures pass with the current set.

## Validation receipts

```
cargo test   -p rustyred-thg-datawave           # unit + 2 parity + 1 doc-test green
cargo test   -p rustyred-thg-datawave-harness   # plugin-bus round-trip over RedCore
cargo clippy -p rustyred-thg-datawave         --all-targets --no-deps -- -D warnings  # clean
cargo clippy -p rustyred-thg-datawave-harness --all-targets --no-deps -- -D warnings  # clean
```
