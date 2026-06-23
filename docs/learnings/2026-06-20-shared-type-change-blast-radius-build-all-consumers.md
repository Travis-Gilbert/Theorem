# Changing a shared `pub` type (struct field, `QueryResult` row shape) breaks every consumer crate, not just the one you build — verify with `cargo check --workspace`

**Kind:** postmortem
**Captured:** 2026-06-20
**Session signature:** `claude-code:travisgilbert (review + fix multimodal-planner-unify)`
**Domain tags:** cargo, workspace, blast-radius, serde, regression, rustyred-thg-pg-server, relational-planner

## Trigger

The `multimodal-planner-unify` branch added a `fusion` field to `QueryIr` and changed `QueryResult.rows` from `Vec<BTreeMap<String, ScalarValue>>` to `Vec<QueryOutputRow>` (a flatten+score wrapper). It was validated with `cargo test -p rustyred-thg-core` + `cargo check -p rustyred-thg-mcp` + one filtered mcp test. `rustyred-thg-pg-server` — a *separate* consumer of `execute_query`/`QueryResult`/`QueryIr` — failed to compile the entire time and nobody noticed: `E0063 missing field 'fusion'` at `relational_server.rs:462` and `lib.rs:386`, plus three `E0308`s where `shape_result` passed `&Vec<QueryOutputRow>` into helpers typed `&[BTreeMap<String, ScalarValue>]`. A review that claimed "no regression: scalar/time/join unchanged" was wrong because it only built core + mcp.

## Rule

After changing any `pub` type that crosses crate boundaries — a struct field, an enum variant, a `fn` signature, or the shape of a re-exported alias — run `cargo check --workspace` (and for standalone Cargo roots under `apps/`, grep them too: `grep -rn 'execute_query\|QueryResult\|QueryIr' crates apps --include='*.rs'`). Adding a `#[serde(default)]` field does NOT save Rust struct *literals* — serde defaults only affect deserialization, so every `QueryIr { ... }` literal still needs the new field (`..QueryIr::default()` is the cheap fix). "Tests pass in the two crates I touched" is not "the workspace builds."

## Evidence

- `cargo check --workspace` surfaced 5 errors in `rustyred-thg-pg-server` AFTER `rustyred-thg-core` (211 tests) and `rustyred-thg-mcp` were both green.
- Fixes: `..QueryIr::default()` on the two `QueryIr` literals; at the pg-wire boundary, `let raw_rows: Vec<BTreeMap<String, ScalarValue>> = result.rows.into_iter().map(|row| row.values).collect();` (pg-wire serves scalar/time views, so dropping the score is correct).
- After the fix: pg-server 18 + 10 tests green (incl. `pg_wire_acceptance`), full workspace `cargo check` clean.
