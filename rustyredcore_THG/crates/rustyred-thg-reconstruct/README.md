# rustyred-thg-reconstruct

Semantic graph and reconstruction instruction compiler for binary artifacts.

## What it is

Semantic reconstruction compiler for binary artifacts.

This crate takes observed binary facts plus THIR and emits graph-backed
reconstruction obligations. It intentionally does not produce human-facing
decompiler text; it produces bounded tasks with evidence and validators.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-reconstruct
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
