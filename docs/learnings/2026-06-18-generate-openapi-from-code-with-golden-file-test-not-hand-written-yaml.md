# Don't hand-write an OpenAPI spec in the docs folder — generate it from a code function, serve it at `/openapi.json`, and golden-file-test the committed snapshot so it cannot drift

**Kind:** method
**Captured:** 2026-06-18
**Session signature:** `claude-code:travisgilbert (harness product docs + generated OpenAPI)`
**Domain tags:** openapi, docs-drift, golden-file-test, theorem-harness-server, json-macro, recursion-limit, axum

## Trigger

The task was "OpenAPI docs for the Harness, so the spec can't drift." My first delivery was a hand-written `docs/site/reference/openapi-harness.yaml`. That re-creates the exact problem the task existed to kill: a static doc in a folder lags the routes the moment anyone edits `main.rs`, with nothing to catch it. The repo already had the correct pattern — `rustyred-thg-server/src/openapi.rs` builds the doc with a `json!` literal and serves it at `GET /openapi.json`. Porting that to `theorem-harness-server` immediately hit `error: recursion limit reached while expanding $crate::json_internal!` — Rust's default 128 macro-recursion limit is too shallow for a large `json!` document.

## Rule

To document an HTTP surface in this repo so it can't drift, generate it from code and guard the published copy with a test:

1. **Single source of truth** = a function `openapi_document() -> serde_json::Value` in the crate's lib (not a YAML/JSON file someone hand-edits).
2. **Serve it**: `GET /openapi.json -> Json(openapi_document())`. A stateless handler is fine even on a stateful router.
3. **Golden-file test** the committed snapshot:
   ```rust
   let doc = openapi_document();
   let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/site/reference/openapi-harness.json");
   if std::env::var("UPDATE_OPENAPI").is_ok() { fs::write(&path, to_string_pretty(&doc)? + "\n"); return; }
   assert_eq!(doc, serde_json::from_str::<Value>(&fs::read_to_string(&path)?)?);  // drift => red test
   ```
   Compare parsed `Value`, not strings — serde_json's default `Map` is a `BTreeMap`, so `to_string_pretty` is deterministic, but `Value` equality is also key-order / whitespace independent.
4. **Large `json!` literal** => add `#![recursion_limit = "512"]` to the crate root. Use **512**, not 256 — it's the repo norm (`rustyred-thg-server/src/{lib,main}.rs` both set 512). The macro lives in the lib crate, so the attribute goes on `lib.rs`.
5. Retire any hand-written spec file in the same change; two specs is two drift surfaces.

The load-bearing part is the test, not the endpoint: a generated endpoint alone still lets the *committed* snapshot drift. The `assert_eq!` against the committed file is what turns drift into a `cargo test` failure.

## Evidence

- Commit `3e139495`. `apps/theorem-harness-server/src/openapi.rs` holds `openapi_document()` + two tests (`openapi_document_is_well_formed`, `committed_openapi_snapshot_matches_code`).
- `UPDATE_OPENAPI=1 cargo test` regenerated `docs/site/reference/openapi-harness.json` (18 paths, 10 schemas); `cargo test` (no env) then passed all 26 lib tests against it.
- The recursion error fired at `src/openapi.rs:22` until `#![recursion_limit = "512"]` was added to `lib.rs`; `grep -rn recursion_limit rustyred-thg-server/src` confirmed the 512 precedent on both `lib.rs:1` and `main.rs:1`.
- Grounding the generated doc against the live routes also caught a real drift the static YAML had missed: the `mentions` endpoint had gained `urgency`/`urgencies` query params (Codex commit `03dd7624`).
