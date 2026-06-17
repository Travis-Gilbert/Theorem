# rustyred-thg-code

Code parsing plugin/runtime for RustyRedCore-THG tenant graph stores.

## What it is

Code parsing plugin/runtime over RedCore.

This crate owns the graph shape for code search. It is intentionally
independent of tonic/MCP/HTTP so every transport can call the same parser
and write into the caller's `RedCoreGraphStore`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-code
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
