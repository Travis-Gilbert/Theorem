# rustyred-thg-fractal

Native Rust fractal expansion pipeline over RustyRed and RustyWeb.

## What it is

Native Rust fractal expansion over RustyRed and RustyWeb.

A fractal expansion run is corpus growth, not graph-only retrieval. The
public runner in this crate always builds a web crawl request and ingests
admitted web graph state as a lower-trust, quarantined tier.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-fractal
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
