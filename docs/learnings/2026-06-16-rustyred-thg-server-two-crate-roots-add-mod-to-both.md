# `rustyred-thg-server` has TWO crate roots (`lib.rs` AND `main.rs`) that both declare the full `mod` list: a new module must be added to BOTH or the bin target fails with E0433

**Kind:** gotcha
**Captured:** 2026-06-16
**Session signature:** `claude:travisgilbert (agent-space-viewport / transport)`
**Domain tags:** rust, cargo, crate-roots, rustyred-thg-server, E0433

## Trigger

Adding the new `agent_space` module, I put `mod agent_space;` in
`rustyred-thg-server/src/lib.rs` (next to `mod auth;` etc.) and wired the routes
in `router.rs` via `crate::agent_space::agent_space_stream`. The library compiled
fine. The build then failed only on the binary:

```
error[E0433]: cannot find `agent_space` in `crate`
  --> crates/rustyred-thg-server/src/router.rs:410:24
   | get(crate::agent_space::agent_space_stream),
warning: `rustyred-thg-server` (bin "rustyred-thg-server") generated 1 warning (1 duplicate)
```

The "(1 duplicate)" warning was the tell: this package has both a `lib.rs` and a
`main.rs`, and **`main.rs` re-declares the same `mod auth; mod bulk; ... mod router;`
list as its own crate root** (so `router.rs` is compiled twice, once per root).
`router.rs` resolves `crate::agent_space` against whichever root is compiling it;
the bin root had no `mod agent_space;`, so it failed. Adding `mod agent_space;` to
`main.rs` as well fixed it immediately.

## Rule

In `rustyred-thg-server`, treat `lib.rs` and `main.rs` as two parallel crate roots
that BOTH enumerate the module list. When you add a `mod foo;`, add it to BOTH
files. A green `cargo build --lib` (or lib-only test run) does NOT prove the bin
compiles. If you see `E0433 cannot find <mod> in crate` from a shared file like
`router.rs` plus a `bin ... (1 duplicate)` warning, the missing `mod` is in
`main.rs`, not `lib.rs`.

## Evidence

- `lib.rs` and `main.rs` both start with `#![recursion_limit = "512"]` then the
  identical `mod auth; mod bulk; ... mod ttl_sweep;` block.
- Fix was a one-line `mod agent_space;` added to `main.rs` mirroring `lib.rs`.

## Encoded in

- `docs/learnings/2026-06-16-rustyred-thg-server-two-crate-roots-add-mod-to-both.md` (this file)
