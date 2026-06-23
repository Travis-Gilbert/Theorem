# theorem-dispatch

Postgres hot execution queue for Theorem Dispatch v2.

## What it is

Postgres hot execution queue for Dispatch v2.

The canonical coordination thread stays in the THG Dispatch v2 board. This
crate owns only hot execution state: claim leases, retries, completion, and
dead-letter visibility.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p theorem-dispatch
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
