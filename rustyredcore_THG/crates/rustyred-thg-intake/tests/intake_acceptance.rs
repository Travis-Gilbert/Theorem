//! Acceptance for the intake-crate deliverables of
//! SPEC-COMMONPLACE-SOURCE-INTAKE-AND-ROUTING: A1 (driver), A2 (scope), A3
//! (incremental idempotency), A5 (MappedSpoke universal path), A6 (curated
//! spokes), B1 (routing composed with a spoke), and C2 (the residue act seam).
//! No live network: every spoke runs over a recorded-response fixture.

use commonplace::{
    route, Commonplace, IngestInput, IngestPipeline, InMemoryBlobStore, Item, ItemKind, RoutingRule,
    SourceRef, SIMILAR_TO_EDGE,
};
use rustyred_thg_core::{InMemoryGraphStore, NeighborQuery};
use rustyred_thg_intake::{
    absorb_residue, accept_suggestion, sync_source, ActSeam, AgentSuggestion, FixtureTransport,
    GSuiteSpoke, GmailSpoke, IngestFieldMap, LinearSpoke, MappedSpoke, NotionSpoke, OutlookSpoke,
    SourceContract, SourceCursor, SourceRecord, SourceScope, SourceSpoke, SuggestionAction,
};
use serde_json::{json, Value};

type Cp = Commonplace<InMemoryGraphStore, InMemoryBlobStore>;

fn fresh() -> Cp {
    Commonplace::new(InMemoryGraphStore::new(), InMemoryBlobStore::new())
}

fn record(id: &str, raw: Value, fetched_at_ms: i64) -> SourceRecord {
    SourceRecord::new(id, raw, fetched_at_ms)
}

/// Total SIMILAR_TO edges in the store (out-degree summed over all items).
fn total_similar(cp: &Cp) -> usize {
    cp.all_items()
        .unwrap()
        .iter()
        .map(|item| {
            cp.store()
                .neighbors(NeighborQuery::out(&item.id).with_edge_type(SIMILAR_TO_EDGE))
                .len()
        })
        .sum()
}

// ---- A1: the driver ---------------------------------------------------------

#[test]
fn a1_driver_fetches_maps_files_and_reruns_idempotently() {
    let mut cp = fresh();
    let pipeline = IngestPipeline::default();
    let records = vec![
        record(
            "m1",
            json!({"subject":"Acme contract","body":"Client: Acme Corp. Indemnity and venue review."}),
            1000,
        ),
        record("m2", json!({"subject":"Lunch","body":"want tacos?"}), 1001),
    ];
    // page_size 1 forces multi-page pagination through the driver loop.
    let spoke = GmailSpoke::new(Box::new(FixtureTransport::new(records).page_size(1)));

    let report = sync_source(
        &mut cp,
        &spoke,
        &SourceScope::default(),
        SourceCursor::default(),
        &pipeline,
    )
    .unwrap();
    assert_eq!(report.source_id, "gmail");
    assert_eq!((report.fetched, report.ingested, report.updated), (2, 2, 0));
    assert_eq!(report.receipts.len(), 2);

    let items = cp.all_items().unwrap();
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|i| i.source.as_deref() == Some("gmail")));
    assert!(
        items.iter().all(|i| !i.collections.is_empty()),
        "every record is filed through the real classifier"
    );

    // Re-running with the same cursor ingests zero and updates in place.
    let rerun = sync_source(
        &mut cp,
        &spoke,
        &SourceScope::default(),
        SourceCursor::default(),
        &pipeline,
    )
    .unwrap();
    assert_eq!((rerun.ingested, rerun.updated), (0, 2));
    assert_eq!(cp.all_items().unwrap().len(), 2, "no duplicates on re-run");
}

// ---- A2: scoped fetch -------------------------------------------------------

#[test]
fn a2_scope_filters_containers_since_and_max_records() {
    let pipeline = IngestPipeline::default();

    // Container + since filtering: only the Inbox record after t=200 survives.
    let mut cp = fresh();
    let entries = vec![
        (record("a", json!({"subject":"A","body":"x"}), 100), Some("Inbox".to_string())),
        (record("b", json!({"subject":"B","body":"y"}), 300), Some("Inbox".to_string())),
        (record("c", json!({"subject":"C","body":"z"}), 300), Some("Archive".to_string())),
        (record("d", json!({"subject":"D","body":"w"}), 50), Some("Receipts".to_string())),
    ];
    let spoke = GmailSpoke::new(Box::new(FixtureTransport::with_containers(entries)));
    let scope = SourceScope::containers(["Inbox", "Receipts"]).since(200);
    let report = sync_source(&mut cp, &spoke, &scope, SourceCursor::default(), &pipeline).unwrap();
    assert_eq!(report.fetched, 1);
    let titles: Vec<String> = cp.all_items().unwrap().iter().map(|i| i.title.clone()).collect();
    assert_eq!(titles, vec!["B".to_string()]);

    // max_records caps the sync and the cursor resumes where it stopped.
    let mut cp = fresh();
    let entries: Vec<_> = (0..5)
        .map(|i| {
            (
                record(&format!("r{i}"), json!({"subject":format!("S{i}"),"body":"b"}), 100 + i),
                Some("Inbox".to_string()),
            )
        })
        .collect();
    let spoke = GmailSpoke::new(Box::new(FixtureTransport::with_containers(entries).page_size(2)));
    let capped = SourceScope::containers(["Inbox"]).max_records(3);
    let first = sync_source(&mut cp, &spoke, &capped, SourceCursor::default(), &pipeline).unwrap();
    assert_eq!(first.fetched, 3, "max_records caps the sync");
    assert_eq!(first.next_cursor.token, "3", "cursor resumes where it stopped");

    let uncapped = SourceScope::containers(["Inbox"]);
    let second = sync_source(&mut cp, &spoke, &uncapped, first.next_cursor.clone(), &pipeline).unwrap();
    assert_eq!((second.fetched, second.ingested), (2, 2), "resume picks up r3, r4");
    assert_eq!(cp.all_items().unwrap().len(), 5);
}

// ---- A3: incremental idempotency --------------------------------------------

#[test]
fn a3_changed_record_updates_in_place_without_doubling() {
    let mut cp = fresh();
    let pipeline = IngestPipeline::default();

    // Two related legal docs (so a SIMILAR_TO edge forms).
    let v1 = vec![
        record(
            "doc-1",
            json!({"subject":"Lease","body":"Client: Acme Corp. Lease indemnity venue clause."}),
            1,
        ),
        record(
            "doc-2",
            json!({"subject":"Lease follow-up","body":"Client: Acme Corp. Follow-up indemnity lease terms."}),
            2,
        ),
    ];
    let spoke1 = GmailSpoke::new(Box::new(FixtureTransport::new(v1)));
    sync_source(&mut cp, &spoke1, &SourceScope::default(), SourceCursor::default(), &pipeline).unwrap();
    let doc1_id = cp.item_by_source_ref("gmail", "doc-1").unwrap().unwrap().id;

    // Re-fetch doc-1 with a changed body (same external id).
    let v2 = vec![record(
        "doc-1",
        json!({"subject":"Lease","body":"Client: Acme Corp. Lease indemnity venue and termination."}),
        3,
    )];
    let spoke2 = GmailSpoke::new(Box::new(FixtureTransport::new(v2)));
    let report = sync_source(&mut cp, &spoke2, &SourceScope::default(), SourceCursor::default(), &pipeline).unwrap();
    assert_eq!((report.ingested, report.updated), (0, 1));

    // Exactly one item for doc-1, same id, updated in place.
    assert_eq!(cp.all_items().unwrap().len(), 2, "no duplicate item");
    let doc1_again = cp.item_by_source_ref("gmail", "doc-1").unwrap().unwrap();
    assert_eq!(doc1_again.id, doc1_id, "updated in place under the same id");
    assert!(matches!(
        doc1_again.body,
        commonplace::ItemBody::Inline { text } if text.contains("termination")
    ));

    // Edges are reconciled, not doubled: a second identical re-fetch is a no-op
    // on the graph shape.
    let edges_after_change = total_similar(&cp);
    let spoke3 = GmailSpoke::new(Box::new(FixtureTransport::new(vec![record(
        "doc-1",
        json!({"subject":"Lease","body":"Client: Acme Corp. Lease indemnity venue and termination."}),
        4,
    )])));
    sync_source(&mut cp, &spoke3, &SourceScope::default(), SourceCursor::default(), &pipeline).unwrap();
    assert_eq!(cp.all_items().unwrap().len(), 2);
    assert_eq!(total_similar(&cp), edges_after_change, "edges reconciled, not doubled");
}

// ---- A5: the universal MappedSpoke path -------------------------------------

#[test]
fn a5_mapped_spoke_files_like_a_curated_one_with_no_bespoke_rust() {
    let pipeline = IngestPipeline::default();
    let raw = json!({"heading":"Quarterly plan","content":"Goals for Q3 across the teams","label":"work"});

    // Universal path: a FieldMap over the JSON, no source-specific Rust.
    let field_map = IngestFieldMap::text("heading", "content", ItemKind::Doc).with_tags(["label"]);
    let mapped = MappedSpoke::new(
        "custom",
        SourceContract::FieldMap(field_map),
        Box::new(FixtureTransport::new(vec![record("rec-1", raw.clone(), 10)])),
    );
    let mut cp_mapped = fresh();
    sync_source(&mut cp_mapped, &mapped, &SourceScope::default(), SourceCursor::default(), &pipeline).unwrap();
    let mapped_item = cp_mapped.all_items().unwrap().remove(0);
    assert_eq!(mapped_item.title, "Quarterly plan");
    assert_eq!(mapped_item.kind, ItemKind::Doc);
    assert_eq!(mapped_item.source.as_deref(), Some("custom"));
    assert!(mapped_item.tags.contains(&"work".to_string()));
    assert!(!mapped_item.collections.is_empty());

    // Indistinguishable from a curated-style direct ingest of the same content:
    // organizing is source-agnostic, so it files into the same content collection.
    let mut cp_direct = fresh();
    let direct = pipeline
        .ingest(
            &mut cp_direct,
            IngestInput::text("Quarterly plan", "Goals for Q3 across the teams", ItemKind::Doc)
                .with_tags(["work"]),
        )
        .unwrap();
    assert_eq!(mapped_item.classification, Some(direct.collection.name));
}

// ---- A6: the five curated spokes --------------------------------------------

fn sync_one(spoke: &dyn SourceSpoke, rec: SourceRecord) -> (Cp, Item) {
    let pipeline = IngestPipeline::default();
    let mut cp = fresh();
    let report = sync_source(&mut cp, spoke, &SourceScope::default(), SourceCursor::default(), &pipeline).unwrap();
    assert_eq!(report.ingested, 1, "the scoped page was filed");
    let item = cp.get_item(&report.receipts[0].item.id).unwrap().unwrap();
    let _ = rec; // the spoke already holds its fixture
    (cp, item)
}

#[test]
fn a6_curated_spokes_fetch_map_and_file() {
    // Gmail -> Note from subject/body.
    let gmail = GmailSpoke::new(Box::new(FixtureTransport::new(vec![record(
        "g1",
        json!({"subject":"Hi","body":"hello there","labelIds":["Inbox"]}),
        1,
    )])));
    let (_, item) = sync_one(&gmail, record("g1", json!({}), 1));
    assert_eq!(item.source.as_deref(), Some("gmail"));
    assert_eq!(item.kind, ItemKind::Note);
    assert!(!item.collections.is_empty());

    // GSuite -> Doc for a Google document mimeType.
    let gsuite = GSuiteSpoke::new(Box::new(FixtureTransport::new(vec![record(
        "f1",
        json!({"name":"Spec.gdoc","mimeType":"application/vnd.google-apps.document","text":"the spec body"}),
        1,
    )])));
    let (_, item) = sync_one(&gsuite, record("f1", json!({}), 1));
    assert_eq!(item.source.as_deref(), Some("gsuite"));
    assert_eq!(item.kind, ItemKind::Doc);

    // Outlook -> Note from subject/bodyPreview.
    let outlook = OutlookSpoke::new(Box::new(FixtureTransport::new(vec![record(
        "o1",
        json!({"subject":"Standup","bodyPreview":"notes","parentFolderId":"Inbox"}),
        1,
    )])));
    let (_, item) = sync_one(&outlook, record("o1", json!({}), 1));
    assert_eq!(item.source.as_deref(), Some("outlook"));
    assert_eq!(item.kind, ItemKind::Note);

    // Notion -> Doc.
    let notion = NotionSpoke::new(Box::new(FixtureTransport::new(vec![record(
        "n1",
        json!({"title":"Roadmap","text":"Q3 plan","database_id":"db-7"}),
        1,
    )])));
    let (_, item) = sync_one(&notion, record("n1", json!({}), 1));
    assert_eq!(item.source.as_deref(), Some("notion"));
    assert_eq!(item.kind, ItemKind::Doc);

    // Linear -> Task with state/priority/due riding the scalars (Layer C).
    let linear = LinearSpoke::new(Box::new(FixtureTransport::new(vec![record(
        "L-1",
        json!({"title":"Fix the bug","description":"it breaks","state":"todo","priority":"2","dueAtMs":1_700_000_000_000_i64,"teamId":"ENG"}),
        1,
    )])));
    let (_, item) = sync_one(&linear, record("L-1", json!({}), 1));
    assert_eq!(item.source.as_deref(), Some("linear"));
    assert_eq!(item.kind, ItemKind::Task);
    assert_eq!(item.status.as_deref(), Some("todo"));
    assert_eq!(item.priority.as_deref(), Some("2"));
    assert_eq!(item.due_at_ms, Some(1_700_000_000_000));
}

// ---- A5 (Mcp variant): a live MCP server as an ingestion source -------------

/// A fake MCP server transport: answers `initialize`, ignores the
/// `notifications/initialized` notify, and returns a canned `tools/call` result.
/// Mirrors how `rustyred-thg-connectors` tests its real transport (which covers
/// the stdio/HTTP framing), so this exercises the connect -> handshake -> call ->
/// parse path without spawning a process.
struct FakeMcpServer {
    tool_result_json: String,
}

impl rustyred_thg_connectors::McpTransport for FakeMcpServer {
    fn request(
        &mut self,
        method: &str,
        _params: Value,
    ) -> rustyred_thg_connectors::ConnectorResult<Value> {
        match method {
            "initialize" => Ok(json!({"serverInfo":{"name":"fake"},"protocolVersion":"2025-06-18"})),
            "tools/call" => Ok(json!({
                "content": [{"type":"text","text": self.tool_result_json}],
                "isError": false,
            })),
            other => Err(rustyred_thg_connectors::ConnectorError::Protocol(format!(
                "unexpected method {other}"
            ))),
        }
    }

    fn notify(
        &mut self,
        _method: &str,
        _params: Value,
    ) -> rustyred_thg_connectors::ConnectorResult<()> {
        Ok(())
    }
}

#[test]
fn a5_mcp_contract_pulls_records_from_an_mcp_server_read_tool() {
    use rustyred_thg_intake::{McpRecordTransport, McpResourceDescriptor};

    let pipeline = IngestPipeline::default();
    let mut cp = fresh();

    // The read-tool returns a page object whose records carry an `id` plus the
    // mapped fields. No new MCP protocol: this is a `tools/call` result.
    let tool_result = json!({
        "records": [
            {"id":"page-1","heading":"Launch checklist","content":"ship the thing"},
            {"id":"page-2","heading":"Retro notes","content":"what went well"}
        ],
        "exhausted": true
    })
    .to_string();

    let transport = McpRecordTransport::with_connector("list_pages", move || {
        Ok(Box::new(FakeMcpServer {
            tool_result_json: tool_result.clone(),
        }) as Box<dyn rustyred_thg_connectors::McpTransport>)
    });

    // The Mcp contract variant shapes each record; the transport pulls them.
    let field_map = IngestFieldMap::text("heading", "content", ItemKind::Doc);
    let spoke = MappedSpoke::new(
        "notion-mcp",
        SourceContract::Mcp(McpResourceDescriptor {
            server_id: "notion".into(),
            resource: "list_pages".into(),
            field_map,
        }),
        Box::new(transport),
    );

    let report = sync_source(&mut cp, &spoke, &SourceScope::default(), SourceCursor::default(), &pipeline).unwrap();
    assert_eq!(report.fetched, 2);
    assert_eq!(report.ingested, 2);

    let items = cp.all_items().unwrap();
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|i| i.source.as_deref() == Some("notion-mcp")));
    assert!(items.iter().all(|i| i.kind == ItemKind::Doc && !i.collections.is_empty()));
    let mut titles: Vec<String> = items.iter().map(|i| i.title.clone()).collect();
    titles.sort();
    assert_eq!(titles, vec!["Launch checklist".to_string(), "Retro notes".to_string()]);

    // Idempotency rides the source ref through the MCP path too.
    let rerun = sync_source(&mut cp, &spoke, &SourceScope::default(), SourceCursor::default(), &pipeline).unwrap();
    assert_eq!((rerun.ingested, rerun.updated), (0, 2));
    assert_eq!(cp.all_items().unwrap().len(), 2);
}

// ---- A6 (GSuite live transport): Drive files.list over real HTTP ------------

fn auth_header(req: &rustyred_thg_intake::HttpRequest) -> String {
    req.headers
        .iter()
        .find(|(k, _)| k == "Authorization")
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

#[test]
fn a6_gsuite_drive_http_transport_files_records() {
    use rustyred_thg_intake::GSuiteDriveTransport;

    let pipeline = IngestPipeline::default();
    let mut cp = fresh();

    // A recorded Drive files.list response: a Google Doc and a binary file.
    let drive_response = json!({
        "files": [
            {"id":"f1","name":"Q3 Plan","mimeType":"application/vnd.google-apps.document","modifiedTime":"2024-06-01T12:00:00Z","parents":["folder-A"]},
            {"id":"f2","name":"logo.png","mimeType":"image/png","modifiedTime":"2024-06-02T12:00:00Z","parents":["folder-A"]}
        ]
    })
    .to_string();

    // Inject the HTTP layer: assert the bearer + Drive query shape, return the page.
    let transport =
        GSuiteDriveTransport::with_http("test-token", "https://drive.test", move |req| {
            assert_eq!(auth_header(req), "Bearer test-token", "bearer reaches HTTP");
            assert!(req.url.starts_with("https://drive.test/files?"), "hits files.list");
            assert!(req.url.contains("folder-A"), "scoped to the folder");
            Ok(drive_response.clone())
        });

    let spoke = GSuiteSpoke::new(Box::new(transport));
    let scope = SourceScope::containers(["folder-A"]);
    let report = sync_source(&mut cp, &spoke, &scope, SourceCursor::default(), &pipeline).unwrap();
    assert_eq!((report.fetched, report.ingested), (2, 2));

    let items = cp.all_items().unwrap();
    assert!(items.iter().all(|i| i.source.as_deref() == Some("gsuite")));
    assert_eq!(items.iter().find(|i| i.title == "Q3 Plan").unwrap().kind, ItemKind::Doc);
    assert_eq!(items.iter().find(|i| i.title == "logo.png").unwrap().kind, ItemKind::File);
    assert!(items.iter().all(|i| !i.collections.is_empty()), "filed by the classifier");

    // Idempotent re-pull (the source ref rides through the live transport too).
    let rerun = sync_source(&mut cp, &spoke, &scope, SourceCursor::default(), &pipeline).unwrap();
    assert_eq!((rerun.ingested, rerun.updated), (0, 2));
    assert_eq!(cp.all_items().unwrap().len(), 2);
}

#[test]
fn a6_max_records_caps_total_across_paginated_pages() {
    use rustyred_thg_intake::GSuiteDriveTransport;
    use std::cell::RefCell;
    use std::rc::Rc;

    let pipeline = IngestPipeline::default();
    let mut cp = fresh();
    // A source that always returns a full page plus a next token, ignoring
    // max_records (the realistic case: max is the per-request page size). Without
    // the driver's total cap this would paginate forever.
    let calls = Rc::new(RefCell::new(0usize));
    let counter = calls.clone();
    let transport = GSuiteDriveTransport::with_http("tok", "https://drive.test", move |_req| {
        let mut n = counter.borrow_mut();
        let base = *n * 4;
        *n += 1;
        let files: Vec<Value> = (0..4)
            .map(|i| json!({"id": format!("f{}", base + i), "name":"x","mimeType":"text/plain","parents":["folder-A"]}))
            .collect();
        Ok(json!({"files": files, "nextPageToken": format!("p{n}")}).to_string())
    });
    let spoke = GSuiteSpoke::new(Box::new(transport));
    let scope = SourceScope::containers(["folder-A"]).max_records(5);
    let report = sync_source(&mut cp, &spoke, &scope, SourceCursor::default(), &pipeline).unwrap();
    assert_eq!(report.fetched, 5, "total cap stops the sync, not just the page size");
    assert!(*calls.borrow() <= 2, "did not run away paginating");
    assert_eq!(cp.all_items().unwrap().len(), 5);
}

#[test]
fn a6_high_water_cursor_advances_and_is_consumed_next_sync() {
    use rustyred_thg_intake::GSuiteDriveTransport;
    use std::cell::RefCell;
    use std::rc::Rc;

    let pipeline = IngestPipeline::default();
    let mut cp = fresh();
    let response = json!({
        "files":[{"id":"f1","name":"Doc","mimeType":"text/plain","modifiedTime":"2024-06-01T12:00:00Z","parents":["folder-A"]}]
    })
    .to_string();

    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    let captured = seen.clone();
    let transport = GSuiteDriveTransport::with_http("tok", "https://drive.test", move |req| {
        captured.borrow_mut().push(req.url.clone());
        Ok(response.clone())
    });
    let spoke = GSuiteSpoke::new(Box::new(transport));
    let scope = SourceScope::containers(["folder-A"]);

    // First sync: no since filter; the high-water advances to the record instant.
    let first = sync_source(&mut cp, &spoke, &scope, SourceCursor::default(), &pipeline).unwrap();
    assert!(first.next_cursor.updated_at_ms > 0, "high-water advances past the record");

    // Second sync passing that cursor back: the query now carries a modifiedTime
    // filter derived from the high-water (so it does not re-pull everything).
    sync_source(&mut cp, &spoke, &scope, first.next_cursor.clone(), &pipeline).unwrap();
    let urls = seen.borrow();
    assert_eq!(urls.len(), 2);
    assert!(!urls[0].contains("modifiedTime%20%3E"), "first sync: no since filter");
    assert!(urls[1].contains("modifiedTime%20%3E"), "second sync: filters since the high-water");
}

#[test]
fn a6_gmail_http_transport_lists_then_gets_messages() {
    use rustyred_thg_intake::GmailHttpTransport;

    let pipeline = IngestPipeline::default();
    let mut cp = fresh();

    // Gmail is a two-call pull: list returns ids, a metadata get returns the
    // subject header + snippet + labels.
    let list = json!({"messages":[{"id":"m1"},{"id":"m2"}]}).to_string();
    let msg1 = json!({"id":"m1","snippet":"lunch?","labelIds":["INBOX"],"payload":{"headers":[{"name":"Subject","value":"Lunch"}]}}).to_string();
    let msg2 = json!({"id":"m2","snippet":"the report","labelIds":["INBOX"],"payload":{"headers":[{"name":"Subject","value":"Report"}]}}).to_string();

    let transport = GmailHttpTransport::with_http("tok", "https://gmail.test", move |req| {
        assert_eq!(auth_header(req), "Bearer tok");
        if req.url.contains("/messages/m1") {
            Ok(msg1.clone())
        } else if req.url.contains("/messages/m2") {
            Ok(msg2.clone())
        } else if req.url.contains("/messages?") {
            assert!(req.url.contains("labelIds=INBOX"), "label scope reaches the list query");
            Ok(list.clone())
        } else {
            Err(rustyred_thg_intake::SourceError::Transport(req.url.clone()))
        }
    });

    let spoke = GmailSpoke::new(Box::new(transport));
    let scope = SourceScope::containers(["INBOX"]);
    let report = sync_source(&mut cp, &spoke, &scope, SourceCursor::default(), &pipeline).unwrap();
    assert_eq!((report.fetched, report.ingested), (2, 2));
    let titles: Vec<String> = {
        let mut t: Vec<String> = cp.all_items().unwrap().iter().map(|i| i.title.clone()).collect();
        t.sort();
        t
    };
    assert_eq!(titles, vec!["Lunch".to_string(), "Report".to_string()]);
    assert!(cp.all_items().unwrap().iter().all(|i| i.source.as_deref() == Some("gmail")));
}

#[test]
fn a6_outlook_http_transport_reads_graph_messages() {
    use rustyred_thg_intake::OutlookHttpTransport;

    let pipeline = IngestPipeline::default();
    let mut cp = fresh();
    let response = json!({
        "value": [
            {"id":"o1","subject":"Standup","bodyPreview":"notes","parentFolderId":"Inbox"},
            {"id":"o2","subject":"Invoice","bodyPreview":"due friday","parentFolderId":"Inbox"}
        ]
    })
    .to_string();

    let transport = OutlookHttpTransport::with_http("tok", "https://graph.test", move |req| {
        assert_eq!(auth_header(req), "Bearer tok");
        assert!(req.url.contains("/me/mailFolders/Inbox/messages"), "folder scope");
        Ok(response.clone())
    });
    let spoke = OutlookSpoke::new(Box::new(transport));
    let scope = SourceScope::containers(["Inbox"]);
    let report = sync_source(&mut cp, &spoke, &scope, SourceCursor::default(), &pipeline).unwrap();
    assert_eq!((report.fetched, report.ingested), (2, 2));
    assert!(cp.all_items().unwrap().iter().all(|i| i.source.as_deref() == Some("outlook")));
}

#[test]
fn a6_notion_http_transport_queries_a_database() {
    use rustyred_thg_intake::NotionHttpTransport;

    let pipeline = IngestPipeline::default();
    let mut cp = fresh();
    let response = json!({
        "results": [
            {"id":"p1","properties":{"Name":{"title":[{"plain_text":"Roadmap"}]}}},
            {"id":"p2","properties":{"Name":{"title":[{"plain_text":"Retro"}]}}}
        ],
        "has_more": false,
        "next_cursor": null
    })
    .to_string();

    let transport = NotionHttpTransport::with_http("tok", "https://notion.test", move |req| {
        assert_eq!(req.method, "POST", "notion query is a POST");
        assert_eq!(auth_header(req), "Bearer tok");
        assert!(req.headers.iter().any(|(k, _)| k == "Notion-Version"), "version header");
        assert!(req.url.contains("/databases/db-7/query"), "queries the scoped database");
        Ok(response.clone())
    });
    let spoke = NotionSpoke::new(Box::new(transport));
    let scope = SourceScope::containers(["db-7"]);
    let report = sync_source(&mut cp, &spoke, &scope, SourceCursor::default(), &pipeline).unwrap();
    assert_eq!((report.fetched, report.ingested), (2, 2));
    let items = cp.all_items().unwrap();
    assert!(items.iter().all(|i| i.kind == ItemKind::Doc && i.source.as_deref() == Some("notion")));
}

#[test]
fn a6_linear_http_transport_pulls_issues_as_tasks() {
    use rustyred_thg_intake::LinearHttpTransport;

    let pipeline = IngestPipeline::default();
    let mut cp = fresh();
    let response = json!({
        "data": {"issues": {
            "nodes": [
                {"id":"L1","title":"Fix bug","description":"it breaks","priority":2,"dueDate":"2023-11-14","state":{"name":"todo"},"team":{"id":"ENG"}}
            ],
            "pageInfo": {"hasNextPage": false, "endCursor": "c1"}
        }}
    })
    .to_string();

    let transport = LinearHttpTransport::with_http("lin_api_key", "https://linear.test", move |req| {
        assert_eq!(req.method, "POST", "linear is GraphQL POST");
        assert_eq!(auth_header(req), "lin_api_key", "raw API key, no Bearer prefix");
        assert!(req.url.ends_with("/graphql"));
        assert!(req.body.as_deref().unwrap_or("").contains("\"ENG\""), "team filter in the query");
        Ok(response.clone())
    });
    let spoke = LinearSpoke::new(Box::new(transport));
    let scope = SourceScope::containers(["ENG"]);
    let report = sync_source(&mut cp, &spoke, &scope, SourceCursor::default(), &pipeline).unwrap();
    assert_eq!((report.fetched, report.ingested), (1, 1));
    let task = cp.get_item(&report.receipts[0].item.id).unwrap().unwrap();
    assert_eq!(task.kind, ItemKind::Task);
    assert_eq!(task.source.as_deref(), Some("linear"));
    assert_eq!(task.status.as_deref(), Some("todo"));
    assert_eq!(task.priority.as_deref(), Some("2"));
    assert_eq!(task.due_at_ms, Some(1_699_920_000_000)); // 2023-11-14T00:00:00Z
}

// Live smokes against the real services. Ignored by default; run with a real
// token, e.g.:
//   GOOGLE_DRIVE_TOKEN=ya29... cargo test -p rustyred-thg-intake gsuite_live -- --ignored
#[test]
#[ignore = "needs GOOGLE_DRIVE_TOKEN"]
fn gsuite_live_smoke() {
    use rustyred_thg_intake::GSuiteDriveTransport;
    let token = std::env::var("GOOGLE_DRIVE_TOKEN").expect("GOOGLE_DRIVE_TOKEN");
    let mut cp = fresh();
    let spoke = GSuiteSpoke::new(Box::new(GSuiteDriveTransport::with_token(token)));
    let report = sync_source(&mut cp, &spoke, &SourceScope::default().max_records(5), SourceCursor::default(), &IngestPipeline::default()).unwrap();
    assert!(report.fetched <= 5);
}

#[test]
#[ignore = "needs GMAIL_TOKEN"]
fn gmail_live_smoke() {
    use rustyred_thg_intake::GmailHttpTransport;
    let token = std::env::var("GMAIL_TOKEN").expect("GMAIL_TOKEN");
    let mut cp = fresh();
    let spoke = GmailSpoke::new(Box::new(GmailHttpTransport::with_token(token)));
    sync_source(&mut cp, &spoke, &SourceScope::default().max_records(3), SourceCursor::default(), &IngestPipeline::default()).unwrap();
}

#[test]
#[ignore = "needs OUTLOOK_TOKEN"]
fn outlook_live_smoke() {
    use rustyred_thg_intake::OutlookHttpTransport;
    let token = std::env::var("OUTLOOK_TOKEN").expect("OUTLOOK_TOKEN");
    let mut cp = fresh();
    let spoke = OutlookSpoke::new(Box::new(OutlookHttpTransport::with_token(token)));
    sync_source(&mut cp, &spoke, &SourceScope::default().max_records(3), SourceCursor::default(), &IngestPipeline::default()).unwrap();
}

#[test]
#[ignore = "needs NOTION_TOKEN and NOTION_DATABASE_ID"]
fn notion_live_smoke() {
    use rustyred_thg_intake::NotionHttpTransport;
    let token = std::env::var("NOTION_TOKEN").expect("NOTION_TOKEN");
    let db = std::env::var("NOTION_DATABASE_ID").expect("NOTION_DATABASE_ID");
    let mut cp = fresh();
    let spoke = NotionSpoke::new(Box::new(NotionHttpTransport::with_token(token)));
    sync_source(&mut cp, &spoke, &SourceScope::containers([db]), SourceCursor::default(), &IngestPipeline::default()).unwrap();
}

#[test]
#[ignore = "needs LINEAR_API_KEY"]
fn linear_live_smoke() {
    use rustyred_thg_intake::LinearHttpTransport;
    let token = std::env::var("LINEAR_API_KEY").expect("LINEAR_API_KEY");
    let mut cp = fresh();
    let spoke = LinearSpoke::new(Box::new(LinearHttpTransport::with_token(token)));
    sync_source(&mut cp, &spoke, &SourceScope::default().max_records(5), SourceCursor::default(), &IngestPipeline::default()).unwrap();
}

// ---- B1: routing composed with a spoke --------------------------------------

#[test]
fn b1_route_then_ingest_routed_through_a_spoke() {
    let mut cp = fresh();
    let pipeline = IngestPipeline::default();
    let rules = vec![RoutingRule::new("gmail", Some("Receipts".into()), "Finance")];

    let rec = record("r1", json!({"subject":"Invoice","body":"amount due","labelIds":["Receipts"]}), 1);
    let spoke = GmailSpoke::new(Box::new(FixtureTransport::new(vec![])));
    let container = spoke.record_container(&rec);
    let input = spoke
        .to_ingest_input(&rec)
        .unwrap()
        .with_source_ref(SourceRef::new("gmail", "r1"));

    let rule = route(&rules, spoke.source_id(), container.as_deref()).expect("rule matches");
    let receipt = pipeline.ingest_routed(&mut cp, input, &rule.collection).unwrap();
    assert_eq!(receipt.collection.name, "Finance");
    assert!(receipt.item.collections.contains(&receipt.collection.id));
    assert_eq!(receipt.item.source.as_deref(), Some("gmail"));
}

// ---- C2: the residue act seam -----------------------------------------------

struct MockSeam {
    fired: Vec<String>,
    result: Value,
}

impl ActSeam for MockSeam {
    fn act(&mut self, suggestion: &AgentSuggestion) -> rustyred_thg_intake::SourceResult<Value> {
        self.fired.push(suggestion.affordance_id.clone());
        Ok(self.result.clone())
    }
}

#[test]
fn c2_accept_suggestion_fires_affordance_and_writes_back() {
    let mut cp = fresh();
    let item = cp
        .put_item(Item::new(ItemKind::Note, "Reply needed").with_source("gmail"))
        .unwrap();
    let suggestion = AgentSuggestion {
        action: SuggestionAction::Draft,
        task_type: "draft_email".into(),
        affordance_id: "gmail.draft".into(),
        arguments: json!({"to":"a@b.com"}),
        auto_absorb: true,
    };
    let mut seam = MockSeam {
        fired: Vec::new(),
        result: json!({"draft_id":"d-1"}),
    };
    let updated = accept_suggestion(&mut cp, &item.id, &suggestion, &mut seam).unwrap();

    assert_eq!(seam.fired, vec!["gmail.draft".to_string()], "the correct affordance fired");
    assert_eq!(
        updated.extra.get("agent_action").and_then(Value::as_str),
        Some("draft")
    );
    // The result is readable on the object after a reload.
    let reloaded = cp.get_item(&item.id).unwrap().unwrap();
    assert_eq!(reloaded.extra.get("agent_result"), Some(&json!({"draft_id":"d-1"})));
}

#[test]
fn c2_absorb_residue_clears_auto_and_leaves_humans() {
    let mut cp = fresh();
    let auto1 = cp.put_item(Item::new(ItemKind::Note, "auto1")).unwrap();
    let auto2 = cp.put_item(Item::new(ItemKind::Note, "auto2")).unwrap();
    let human = cp.put_item(Item::new(ItemKind::Note, "human")).unwrap();
    let bare = cp.put_item(Item::new(ItemKind::Note, "bare")).unwrap();

    let suggest = |aff: &str, auto: bool| AgentSuggestion {
        action: SuggestionAction::Delegate,
        task_type: "delegate".into(),
        affordance_id: aff.into(),
        arguments: json!({}),
        auto_absorb: auto,
    };
    let residue = vec![
        (auto1.id.clone(), Some(suggest("aff1", true))),
        (auto2.id.clone(), Some(suggest("aff2", true))),
        (human.id.clone(), Some(suggest("aff3", false))), // suggested, but needs a human
        (bare.id.clone(), None),                           // no suggestion at all
    ];
    let mut seam = MockSeam {
        fired: Vec::new(),
        result: json!({"ok":true}),
    };
    let report = absorb_residue(&mut cp, residue, &mut seam).unwrap();

    assert_eq!(report.absorbed, vec![auto1.id.clone(), auto2.id.clone()]);
    let mut needs = report.needs_human.clone();
    needs.sort();
    let mut want = vec![human.id.clone(), bare.id.clone()];
    want.sort();
    assert_eq!(needs, want, "only the human-needed items remain");
    assert_eq!(seam.fired, vec!["aff1".to_string(), "aff2".to_string()]);

    // Absorbed items carry their result; the human-needed ones are untouched.
    assert!(cp.get_item(&auto1.id).unwrap().unwrap().extra.contains_key("agent_result"));
    assert!(!cp.get_item(&human.id).unwrap().unwrap().extra.contains_key("agent_result"));
}
