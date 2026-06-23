# `cargo build` at the `rustyredcore_THG` workspace root fails at the PyO3 link step — validate members with `-p`, never the root

**Kind:** gotcha
**Captured:** 2026-06-16
**Session signature:** `claude:travisgilbert (graph-hook-primitive)`
**Domain tags:** rust, cargo, pyo3, maturin, rustyredcore_THG, linking, validation

## Trigger

After additive changes to `rustyred-thg-core`, I ran `cargo build` at the
`rustyredcore_THG` workspace root to confirm nothing downstream broke. It failed:

```
error: linking with `cc` failed: exit status: 1
  ... Undefined symbols ...
  pyo3::types::capsule::name_ptr_ignore_error ...
  pyo3::err::panic_after_error ...
  -o .../librustyredcore_THG.dylib
```

This looked like I'd broken the build, but it is unrelated to any member-crate
change: the root crate `rustyredcore_THG` is the PyO3/maturin `#[pymodule]`
`cdylib` (exported to Python as `theseus_native`). Its undefined `pyo3::*`
symbols are only resolved under `maturin develop` (which supplies the
`extension-module` / Python link flags); a plain `cargo build` of that cdylib
always fails to link locally. Building the affected members with `-p` instead
(`cargo build -p rustyred-thg-server -p rustyred-thg-mcp ...`) compiled cleanly.

(Related: a multi-crate `cargo test -p core -p code -p web -p server` unifies
features across the selected packages, which turns on `redis-store` on core via
the server and surfaces 2 pre-existing `graph_store::tests::redis_keyspace_*`
failures that single-crate / default-feature runs hide. Those are not yours —
they predate the session in commit `6f383275`'s injective tenant encoding.)

## Rule

Never validate via `cargo build` / `cargo test` at the `rustyredcore_THG`
workspace ROOT — the root package is a PyO3 cdylib that cannot link without
maturin's Python flags, and the link error is unrelated to your code. Build/test
specific members with `-p <crate>` (and `--manifest-path apps/.../Cargo.toml` for
the standalone `apps/*` crates). Build the wheel only via `maturin develop`. If a
multi-crate `-p` run shows failures in a crate you didn't touch, check whether a
feature got unified on (e.g. `redis-store`) before assuming you broke it.

## Evidence

- `CLAUDE.md`: "Root lib (`src/`) is the PyO3 module exported to Python as
  `theseus_native` (maturin)"; build commands use `maturin develop` or `-p`.
- Link error names only `pyo3::*` symbols on `librustyredcore_THG.dylib`, never a
  member crate.
