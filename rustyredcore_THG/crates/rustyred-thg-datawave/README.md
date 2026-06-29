# rustyred-thg-datawave

DATAWAVE-style intake: turn any source record into typed normalized field-facts plus declared entity-edges in a GraphStore, with per-field index policy, a self-describing dictionary, cell visibility/masking, and content + fuzzy hashing. The general front door the binary-reconstruction facts compose with, because they share the graph.

## What it is

# rustyred-thg-datawave

DATAWAVE-style intake for the RustyRed substrate: turn any source record into
typed normalized field-facts plus declared entity-edges in a `GraphStore`,
with per-field index policy, a self-describing dictionary, cell-level
visibility/masking, and content + fuzzy hashing.

Reference: `NationalSecurityAgency/datawave`, the `warehouse` ingest modules.
This is the *write* side (record -> field-facts -> corpus); the read-side
tiered index and boolean-pushdown planner are a sibling concern that composes
over the same facts.

The thesis, borrowed from DATAWAVE, is "no bespoke loader per source": a new
source is a registered data-type (an [`IngestHelper`]) plus a [`FieldConfig`],
and everything downstream -- normalization, index policy, derived edges, the
dictionary, dedup, similarity -- follows from the normalized fields it emits.
Because field-facts land in the same graph as the binary-reconstruction facts
([`rustyred_thg_core`] + the `rustyred-thg-binformat`/`-reconstruct` crates),
a reconstructed binary, an ingested document, a crawled URL, and a parsed repo
intersect in one corpus.

## Pipeline

```text
RawRecord --(IngestHelper::event_fields)--> [NormalizedField]
          --(derive_edges)--------------->  [DerivedEdge]
          --(materialize_event)---------->  IngestEvent + FieldFact + FieldEntity nodes/edges
          --(write_dictionary)----------->  DataDictionaryField + EdgeDictionaryType nodes
```

## Example

```
use rustyred_thg_datawave::{
    CsvHelper, DatawaveIngest, EdgeDef, FieldConfig, FieldType, IndexPolicy,
    IngestStats, MaterializeConfig, RawRecord,
};
use rustyred_thg_core::InMemoryGraphStore;

let config = FieldConfig::new()

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-datawave
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
