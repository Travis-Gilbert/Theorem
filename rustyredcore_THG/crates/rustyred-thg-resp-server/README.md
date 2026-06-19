# rustyred-thg-resp-server

RESP protocol server for RustyRedCore-THG ordered-index commands.

## What it is

`rustyred-thg-resp-server` is a small Tokio RESP command loop over `OrderedIndexRegistry`. It accepts Redis-shaped commands, parses both array and whitespace command forms, executes ordered-index protocol operations, and returns RESP-encoded responses for focused compatibility and smoke testing.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-resp-server
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
