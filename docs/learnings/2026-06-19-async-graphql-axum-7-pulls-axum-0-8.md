# `async-graphql-axum = "7"` resolves to axum 0.8, so pinning `axum = "0.7"` breaks the Handler trait bound

**Kind:** gotcha
**Captured:** 2026-06-19
**Session signature:** `claude-code:travisgilbert (CommonPlace consumer loop, apps/commonplace-api)`
**Domain tags:** rust, cargo, async-graphql, axum, version-skew, dependency-resolution

## Trigger

New crate `apps/commonplace-api` (F3 GraphQL gateway) declared `async-graphql = "7"`, `async-graphql-axum = "7"`, `axum = "0.7"`. It failed to compile with a wall of `Handler<T, S>` "trait bound not satisfied" errors on the `.post(graphql_handler)` route -- the handler signature looked correct (FromRequestParts extractors before the request-consuming `GraphQLRequest`). The real cause was a SILENT major-version skew: `async-graphql-axum = "7"` resolved to `7.2.1`, which depends on **axum 0.8**, while the crate pinned `axum = "0.7"`. The lockfile carried BOTH `axum 0.7.9` and `axum 0.8.9`; `GraphQLRequest`'s `FromRequest` impl is for axum 0.8's traits, so it did not satisfy axum 0.7's `Handler`. Fix: bump the direct dep to `axum = "0.8"` (no route-syntax change was needed since the routes had no path params). 8/8 then compiled and the live HTTP smoke passed.

## Rule

When wiring `async-graphql-axum` (or any framework-bridge crate) to axum, do NOT independently pin the axum major version. First check which axum the bridge pulls: `grep -A1 '^name = "axum"' <crate>/Cargo.lock` (or `cargo tree -i axum`). If two axum versions appear in the lock, that is the bug -- align your direct `axum` dep to the bridge's. A `Handler<T,S>` "trait bound not satisfied" on a handler whose signature looks right is the signature of an axum major-version mismatch between your dep and an extractor-providing dep, not a handler-shape error.

## Evidence

- `apps/commonplace-api/Cargo.lock` contained `axum 0.7.9` (direct) and `axum 0.8.9` (via `async-graphql-axum 7.2.1`).
- Changing `axum = "0.7"` -> `axum = "0.8"` in `apps/commonplace-api/Cargo.toml` resolved the `Handler` error with zero handler/route code changes (routes used `/graphql`, `/healthz` -- no `:param` -> `{param}` 0.8 syntax migration required).
- Note for the parent workspace: `theorem-gateway` also uses async-graphql 7 + axum; check its axum pin if touching it.
