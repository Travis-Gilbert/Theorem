# rustyred-thg-datawave-harness

Harness capability pack theorem.ingest.datawave: exposes the DATAWAVE-style intake (ingest a record/batch, write the dictionary, look up and intersect facts by value+field) to agents as RustyRed plugin operations, mirroring rustyred-thg-reconstruct-harness.

## What it is

Harness capability pack `theorem.ingest.datawave`: the agent-facing surface
over the DATAWAVE-style intake, mirroring `rustyred-thg-reconstruct-harness`.

Agents call data-driven operations rather than linking the data layer:
- `ingest.describe`  : list the pack's operations (no graph write).
- `ingest.record`    : ingest one record into normalized field-facts + edges.
- `ingest.batch`     : ingest many records; optionally write the dictionary.
- `ingest.lookup`    : event ids matching one value+field predicate.
- `ingest.intersect` : event ids matching an AND of value+field predicates.

Every operation is fully data-driven (a serializable `HelperSpec` selects and
configures the CSV/JSON/Mapped data-type), so "point at a source and ingest by
configuration" is reachable over the plugin bus with no bespoke Rust. The
lookup/intersect operations read the persisted `FieldFact` nodes through the
GraphStore's value+field property index.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-datawave-harness
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
