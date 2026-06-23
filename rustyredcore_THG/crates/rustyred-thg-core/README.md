# rustyred-thg-core

Theorem HotGraph core command executor and state machine.

## What it is

THG-Core: Theorem HotGraph command runtime.

This crate has no Django, Python, or network-server dependencies. Both
PyO3 in-process bindings and the standalone HTTP server call this same
executor.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-core
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
