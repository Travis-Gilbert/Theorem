# rustyred-thg-intake

CommonPlace source intake: the ingestion-spoke framework, a scoped incremental sync driver, curated and universal sources, and the NeedsYou-residue act seam. Sync and tokio-free, generic over the `GraphStore` the `Commonplace` store wraps. It is the intake-side sibling of `rustyred-thg-connectors`, keeping `commonplace` a pure data layer.

## Key API

- Spoke contract: `SourceSpoke` (`source_id`, `fetch`, `to_ingest_input`, `record_container`), `SourceRecord`, `SourceCursor`, `SourcePage`, `SourceScope` (containers/since_ms/max_records/filters; scoped by construction), `SourceError`.
- Driver: `sync_source(commonplace, spoke, scope, cursor, pipeline) -> SyncReport`. Fetches page by page over a `RecordTransport`, maps each record, ingests the page through the batch path, dedupes by source ref, advances and returns the cursor. A mapping error skips one record, not the sync.
- Transport seam: `RecordTransport`, `FixtureTransport` (in-memory recorded responses; honors scope filters, resumable cursor).
- Universal path: `MappedSpoke` plus `SourceContract` (`FieldMap` / `Schema` / `Mcp`), so any describable source becomes a spoke with no bespoke Rust. `McpRecordTransport` pulls records from a live MCP read-tool via the connectors transport.
- Curated spokes: `GmailSpoke`, `GSuiteSpoke`, `OutlookSpoke`, `NotionSpoke`, `LinearSpoke` (Linear maps an issue to a `Task` with state/priority/due on the scalars).
- Live REST (`rest.rs`): one injected HTTP seam (`HttpRequest`, `ureq_fetch`) with `GSuiteDriveTransport`, `GmailHttpTransport`, `OutlookHttpTransport`, `NotionHttpTransport`, `LinearHttpTransport`. Each has `with_token` (real) and `with_http` (injected/fixture) constructors. Bodies are metadata/snippet only; full-content fetch is a follow-up.
- Act seam: `ActSeam`, `AgentSuggestion`, `SuggestionAction` (`Draft`/`Delegate`/`Develop`), `accept_suggestion`, `absorb_residue`, `ConnectorActSeam` (wraps `invoke_affordance` with `InvokePolicy::FireAllowlist`).

Path deps: `commonplace`, `rustyred-thg-core`, `rustyred-thg-connectors`, `rustyred-thg-affordances`, `ureq`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-intake
```

Fixture-only by default. `tests/intake_acceptance.rs` plus inline unit tests; 5 `#[ignore]` live REST smokes (each needs a real per-source token, e.g. `GMAIL_TOKEN`, `NOTION_TOKEN` plus `NOTION_DATABASE_ID`, `LINEAR_API_KEY`).

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
