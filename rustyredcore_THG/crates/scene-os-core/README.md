# scene-os-core

Rust SceneOS atom, catalog, selection, and package compiler contracts for Theorem.

## What it is

SceneOS core contracts for Theorem.

This crate is the Rust-native director seam for the browser: atoms and
relations use the same JSON contract as Index-API/Theseus-UI, catalogs
describe trusted projections/chromes, and the compiler selects a package
without crossing a Python API boundary.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p scene-os-core
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
