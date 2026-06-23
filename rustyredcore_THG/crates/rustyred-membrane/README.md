# rustyred-membrane

Shared context admission and eviction membrane for RustyRed graph-backed context.

## What it is

Shared context admission and eviction membrane.

The context window is treated as a cache over graph-resident nodes. Callers
generate candidates, a [`Scorer`] ranks them for the current arm, and the
shared gate admits a budgeted subset while converting overflow into
recoverable graph handles.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-membrane
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
