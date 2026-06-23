# pyo3 root cdylib fails `cargo build --workspace` at the libpython link step

**Kind:** gotcha
**Captured:** 2026-06-07
**Session signature:** `claude:travisgilbert@Traviss-Laptop:b944c683`
**Domain tags:** rust, pyo3, maturin, build, rustyredcore_thg

## Trigger

During the dispatch-queue build, `cargo build --workspace` in `rustyredcore_THG`
failed with:

```
ld: symbol(s) not found for architecture arm64: __Py_NoneStruct, __Py_TrueStruct
error: could not compile `rustyredcore_thg` (lib) due to 1 previous error
```

It read as if the new crates had broken the build. They had not. The workspace
root crate `rustyredcore_thg` is a pyo3 `extension-module` cdylib
(`crate-type = ["cdylib"]`, `features = ["extension-module"]`). That feature
intentionally leaves the libpython symbols unresolved at link time — `maturin`
(or `-undefined dynamic_lookup` rustflags) supplies them. A plain
`cargo build --workspace` links the cdylib without libpython and fails. Every
library crate had already compiled fine.

## Rule

Validate workspace library crates with
`cargo build --workspace --exclude rustyredcore_thg`. Build the root only via
`maturin develop`. A `__Py_*`/libpython link error on `rustyredcore_thg` is
never your regression — it is the extension-module link step, not your code.

## Evidence

- `rustyredcore_THG/Cargo.toml`: `[lib] crate-type = ["cdylib"]`,
  `pyo3 = { ... features = ["abi3-py312", "extension-module"] }`
- CLAUDE.md documents `maturin develop` for the root but does not warn that
  `cargo build --workspace` fails at the cdylib link step.
- Session: `cargo build --workspace --exclude rustyredcore_thg` finished clean
  while `cargo build --workspace` failed only on the root link.

## Encoded in

- `docs/learnings/2026-06-07-pyo3-cdylib-workspace-build-link-failure.md` (this file)
