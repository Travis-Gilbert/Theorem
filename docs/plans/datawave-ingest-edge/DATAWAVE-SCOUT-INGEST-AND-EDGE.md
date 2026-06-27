# DATAWAVE Reference Scout: Ingest and Edge Layer

Reference source: `NationalSecurityAgency/datawave`, the `warehouse` modules. Primary reads: `warehouse/ingest-core`, `warehouse/ingest-configuration`, `warehouse/data-dictionary-core`, `warehouse/edge-dictionary-core`, `warehouse/edge-model-configuration-core`, with the concrete data-types `warehouse/ingest-csv`, `warehouse/ingest-json`, and `warehouse/ingest-ssdeep` as worked examples. Pin the scout commit at checkout. (The repo's default branch is `integration`; there is no `main`/`master`.)

## Read

The earlier DATAWAVE pass took the read path: the tiered global and field index, the boolean pushdown planner, and the external-merge iterator. That is the query side. The interesting thing on the write side is a pluggable intake that turns arbitrary records into typed normalized field-facts, derives an entity-edge graph from those facts by declared rules, publishes a self-describing dictionary of what exists, and carries cell-level visibility through all of it, with no bespoke loader written per source. A new source is a registered data-type plus an ingest helper, and everything downstream, the index, the edges, the dictionary, follows from the normalized fields it emits.

The subsystems worth absorbing:

- a raw-record container as the single intake abstraction
- a data-type registry where each source registers its parsing and field configuration
- an ingest-helper contract that turns one record into typed normalized field-and-value units
- a normalizer set that gives each field a typed, index-ready form
- composite and virtual field derivation that builds new fields from extracted ones
- field aliasing that maps external field names to internal ones, which is the root of query-side field expansion
- per-field index policy that decides indexed, reverse-indexed, tokenized, and index-only
- a data-type handler layer that materializes normalized fields into the shard, global-index, and field-index tables
- an edge handler that derives entity-to-entity edges from declared edge definitions with conditional evaluation and edge-key versioning
- a data dictionary and an edge dictionary that describe the fields and edges present, their types, and cardinality
- visibility markings and field masking applied per field
- content hashing and fuzzy hashing for dedup and similarity

Theorem already has the storage, the tiered index target, the graph, and the epistemic edges. The absorb is the intake itself: the data-type model, the record-to-field-facts derivation, the edge-derivation rules, the dictionary, and the visibility carry. RustyRed becomes the system that turns any source into field-facts and edges at scale, and the reconstruction facts compose with them because they share the graph.

## Evidence Map

| DATAWAVE reference | Observed shape | Theorem implication |
| --- | --- | --- |
| `warehouse/ingest-core/.../data/RawRecordContainer.java` | One raw-record abstraction carrying raw bytes, data-type, timestamp, visibility, and errors, used as the intake unit for every source. | Model a raw-record contract as the single ingest entry point, carrying source bytes, declared type, time, and visibility. |
| `warehouse/ingest-core/.../data/Type.java` and `TypeRegistry.java` | Each source registers as a data-type with its helper classes and handlers, and the registry resolves a record to its type. | A data-type registry where a source declares its ingest configuration, resolved per record. |
| `warehouse/ingest-core/.../data/config/ingest/IngestHelperInterface.java` and `BaseIngestHelper.java` | The central contract that turns one record into a multimap of normalized field-and-value units, with index policy per field. | The record-to-field-facts derivation contract: one record in, typed normalized field-facts out. |
| `warehouse/ingest-core/.../data/config/NormalizedContentInterface.java` and `NormalizedFieldAndValue.java` | A field carries its name, original value, normalized value, grouping, and markings. | The normalized field-fact node, carrying field, raw value, normalized value, group, and visibility. |
| `warehouse/ingest-core/.../data/normalizer/` | A normalizer set gives each field a typed index-ready form for lowercase-text, numeric, date, IP, and geo-style values. | A normalizer set keyed by field type that produces the index-ready normalized value. |
| `warehouse/ingest-core/.../data/config/ingest/CompositeIngest.java` and `VirtualIngest.java` | Composite fields concatenate extracted fields into compound keys, and virtual fields derive new fields from extracted ones. | Composite and virtual field-fact derivation built on the extracted fields. |
| `warehouse/ingest-core/.../data/config/ingest/FieldNameAliaserNormalizer.java` | External field names alias to internal names during ingest, which the query side expands against. | A field-alias map written at ingest that seeds query-side field expansion. |
| `warehouse/ingest-core/.../data/config/FieldConfigHelper.java` and `XMLFieldConfigHelper.java` | Per-field configuration decides indexed, reverse-indexed, tokenized, and index-only. | A per-field index policy attached to each field-fact. |
| `warehouse/ingest-core/.../mapreduce/handler/DataTypeHandler.java` and `handler/shard/` | Handlers materialize normalized fields into the shard, global-index, and field-index tables. | The materialization pass that writes field-facts into the tiered index already specced. |
| `warehouse/ingest-core/.../mapreduce/handler/edge/ProtobufEdgeDataTypeHandler.java`, `edge/define/`, `edge/evaluation/` | Edge definitions declare which field pairs become entity-edges, an evaluation layer gates edges conditionally, and an edge-key versioning cache tracks edge definition versions. | An edge-derivation contract: declared rules produce graph edges from field-facts, with conditional evaluation and definition versioning. |
| `warehouse/data-dictionary-core` and `warehouse/edge-dictionary-core` | A data dictionary describes fields, their types, and cardinality, and an edge dictionary describes edge types, derived from ingest metadata. | A dictionary node set that makes the corpus self-describing for fields and edges. |
| `warehouse/ingest-core/.../data/config/MarkingsHelper.java` and `MaskedFieldHelper.java` | Visibility markings attach per field, and masked fields carry a restricted alternate value. | Per-fact visibility labels and field masking layered over tenant-scoping. |
| `warehouse/ingest-core/.../data/hash/` and `warehouse/ingest-ssdeep` | Content hashing supports dedup, and ssdeep fuzzy hashing supports similarity over content. | Content-addressed dedup plus fuzzy-similarity facts, which also serve binary similarity in the reconstruction corpus. |

## Build

1. Raw-record and data-type model
   - Land a raw-record contract carrying source bytes, declared data-type, timestamp, and visibility as the single ingest entry point.
   - Land a data-type registry where a source registers its parsing and field configuration, resolved per record.

2. Record-to-field-facts derivation
   - Land the ingest-helper contract that turns one record into typed normalized field-facts, each carrying field name, raw value, normalized value, group, and visibility.
   - Land the normalized field-fact node and its edges to the source record and to the data-type.

3. Field configuration and normalizers
   - Land a per-field index policy of indexed, reverse-indexed, tokenized, and index-only attached to each field-fact.
   - Land a normalizer set keyed by field type for lowercase-text, numeric, date, IP, and geo-style values.

4. Composite, virtual, and aliased fields
   - Land composite field-facts that concatenate extracted fields into compound keys.
   - Land virtual field-facts derived from extracted fields.
   - Land a field-alias map written at ingest that the query side expands against.

5. Materialization to the tiered index
   - Land the materialization pass that writes field-facts into the global index, the field index, and the shard entries defined in the earlier DATAWAVE retrieval spec.

6. Edge model
   - Land an edge-definition contract that declares which field pairs become entity-edges, with a conditional evaluation gate and edge-definition versioning.
   - Land the edge-derivation pass that produces graph edges from field-facts by those definitions, writing into the existing graph and epistemic edges.

7. Dictionary
   - Land a data-dictionary node set describing fields, types, and cardinality from ingest metadata.
   - Land an edge-dictionary node set describing edge types, so the corpus is self-describing.

8. Visibility and masking
   - Land per-fact visibility labels layered over tenant-scoping.
   - Land field masking that carries a restricted alternate value for masked fields.

9. Content and fuzzy hashing
   - Land content-addressed dedup over ingested content.
   - Land fuzzy-similarity facts over content, reused for binary similarity in the reconstruction corpus.

## Next Build Cut

Build an ingest parity lane before expanding source coverage, using DATAWAVE's own test fixtures as the oracle:

1. Take the checked-in input fixtures and expected outputs in the concrete data-type modules, starting with `warehouse/ingest-csv` and `warehouse/ingest-json` test resources.
2. Run the same input records through the RustyRed ingest contracts.
3. Add parity tests comparing the RustyRed normalized field-facts, index policy, and derived edges against DATAWAVE's expected outputs for those fixtures.
4. Feed successful parity receipts into the same learned-scorer training stream the reconstruction lane feeds.

That cut grounds the intake against the reference the way the Ghidra oracle lane grounds reconstruction, so the field-facts and edges are checked against real expected outputs rather than asserted.

## Product Implication

The reconstruction engine produces typed facts, and this layer is how any source becomes field-facts and edges at scale without a loader written per source. The data-type model is the general intake, the ingest helper turns records into normalized field-facts, the edge model turns facts into a queryable relationship graph, the dictionary makes the corpus self-describing, visibility carries auth, and content and fuzzy hashing give dedup and similarity. Together with the tiered index already specced, this completes DATAWAVE's intake-to-corpus path on RustyRed, and it composes with the reconstruction facts and the NiFi dataflow because all of them land in the same graph.

---

## Theorem implementation

This scout is implemented in `rustyredcore_THG/crates/rustyred-thg-datawave`. The
parity lane (Next Build Cut) is `tests/parity.rs`, grounded against DATAWAVE's
asserted `my-nci.csv` (CSV) and `flattener-test.json` (JSON) fixtures. See
[STATUS.md](STATUS.md) for the phase -> module -> test map and the
deferred-by-composition list.
