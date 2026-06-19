# A real Postgres driver introspects `pg_catalog.pg_type` (with a LEFT JOIN) to resolve an unknown type OID -- a minimal pg-wire server must report well-known OIDs or the driver sends catalog SQL it cannot serve

**Kind:** gotcha
**Captured:** 2026-06-19
**Session signature:** `claude-code:travisgilbert (PG-WIRE + DOCUMENT-TIER engine tiers)`
**Domain tags:** pg-wire, postgres-protocol, tokio-postgres, pg_catalog, type-oid, rustyred-thg-pg-server

## Trigger

Building `rustyred-thg-pg-server` (a minimal Postgres-wire server that lowers a SQL subset to the native planner). A parameterized query over a live `tokio-postgres` client:

```rust
client.query("SELECT id FROM memory WHERE topic = $1", &[&"planning"]).await
```

failed with: `ERROR ... only INNER JOIN is supported` (SQLSTATE 0A000) -- an error from MY join lowering, for a query that has no join. A `THEOREM_PG_DEBUG` trace of every SQL the server lowered showed the cause:

```
[pg-wire] execute: SELECT id FROM memory                       <- my Describe probe (fine)
[pg-wire] execute: SELECT t.typname, t.typtype, t.typelem, r.rngsubtype, t.typbasetype, n.nspname, t.typrelid
                   FROM pg_catalog.pg_type t
                   LEFT OUTER JOIN pg_catalog.pg_range r ON r.rngtypid = t.oid
                   INNER JOIN pg_catalog.pg_namespace n ON t.typnamespace = n.oid
```

tokio-postgres prepared the statement, saw the server report the `$1` parameter's type OID as `0` (unspecified) in ParameterDescription, and -- to resolve OID 0 -- issued a `pg_catalog.pg_type` introspection query with a LEFT OUTER JOIN. The minimal surface only lowers INNER JOIN over native views, so it rejected the catalog query and the whole parameterized call failed. Non-parameterized queries passed because their result OIDs (TEXT=25, INT8=20) are built-ins tokio-postgres already knows, so no introspection fired.

## Rule

When building a minimal Postgres-wire server, NEVER report type OID `0` (or any OID outside the driver's built-in type cache) in `ParameterDescription`/`RowDescription`. The driver resolves an unknown OID by querying `pg_catalog.pg_type` (a multi-JOIN catalog query your surface almost certainly cannot lower), and that round-trip fails the user's query. Default undeclared parameter types to a well-known built-in OID (TEXT = 25); for text params the binary encoding IS the UTF-8 bytes, so this "just works". Honor any declared (non-zero) OIDs as-is. Report exactly one parameter entry per `$N` placeholder (Postgres infers types the client did not declare). Related extended-protocol facts proven the same session: tokio-postgres requests results in BINARY format (the server must honor the Bind result-format codes, not always send text), and `Describe('S')` must emit `ParameterDescription` BEFORE `RowDescription` or `prepare()` desyncs.

## Evidence

- Fix: in the statement-`Describe` path, build the ParameterDescription OIDs as `declared.get(i).filter(|o| *o != 0).unwrap_or(Type::TEXT.oid())` (one per placeholder). After that, `parameterized_query_over_the_wire` passed and the full live suite is 10/10 tokio-postgres integration green.
- The trace above is the literal driver behavior; the fix removes the trigger (no OID-0 in the descriptor -> no `pg_type` introspection).
- Earlier in the same build, the analogous "honor Bind binary result format" + "emit ParameterDescription" were required to make `client.prepare()` + binary `row.get::<i64>()` work at all.
