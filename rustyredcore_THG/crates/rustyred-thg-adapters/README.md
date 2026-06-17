# rustyred-thg-adapters

LoRA adapter catalog and routing over RustyRedCore-THG graph records.

## What it is

LoRA adapter catalog over RustyRedCore-THG graph records.

The crate stays above `rustyred-thg-core`: it reuses core graph records,
stores, and PPR, while keeping adapter-specific routing and fitness logic
out of the core executor.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-adapters
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
