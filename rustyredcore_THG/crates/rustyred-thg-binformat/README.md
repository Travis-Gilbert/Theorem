# rustyred-thg-binformat

Binary loader facts for Theorem reconstruction.

## What it is

Binary artifact loading for the Theorem reconstruction pipeline.

This crate owns observed binary facts only: artifact identity, file format,
architecture, sections, symbols, relocations when available, entrypoints,
and printable strings. Downstream crates decode, lift, infer, and compile
reconstruction obligations from these facts.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-binformat
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
