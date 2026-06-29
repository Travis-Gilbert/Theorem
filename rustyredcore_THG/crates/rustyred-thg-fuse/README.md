# rustyred-thg-fuse

Read-only FUSE/path index over zero-copy RustyRed THG graph archives.

## What it is

Read-only graph archive filesystem core.

The default build is pure Rust and has no macFUSE/libfuse dependency. It
indexes zero-copy archive bytes from `CompiledGraphPack` into the path scheme
that a FUSE host can serve.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-fuse
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
