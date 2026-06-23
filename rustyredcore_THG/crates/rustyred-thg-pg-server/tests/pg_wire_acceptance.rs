//! Spec-anchored acceptance suite for SPEC-RUSTYRED-PG-WIRE.
//!
//! These tests stand up the planner-backed native-views wire surface
//! (`serve_relational`) on a real TCP socket and drive it with `tokio-postgres`,
//! a production Postgres client. They are the load-bearing proof that "the entire
//! pg client and tooling ecosystem connects to RustyRed as if it were Postgres":
//! a real pg driver completes the handshake, runs simple and extended queries,
//! and decodes typed rows by OID.
//!
//! Each numbered test traces a spec acceptance criterion.

use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use rustyred_thg_pg_server::{demo_native_store, serve_relational};
use tokio_postgres::types::Type;
use tokio_postgres::{Client, NoTls};

/// Bind an ephemeral port and serve the native-views surface in a background
/// thread. Returns the chosen port.
fn spawn_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().unwrap().port();
    let store = Arc::new(Mutex::new(demo_native_store()));
    std::thread::spawn(move || {
        let _ = serve_relational(listener, store);
    });
    port
}

/// Connect a real `tokio-postgres` client (trust auth, TLS disabled) and drive
/// its connection task in the background.
async fn connect(port: u16) -> Client {
    let conn_str = format!("host=127.0.0.1 port={port} user=postgres dbname=postgres sslmode=disable");
    let (client, connection) = tokio_postgres::connect(&conn_str, NoTls)
        .await
        .expect("tokio-postgres connects to the rustyred pg-wire surface");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

/// AC #1: a pg driver connects, runs a SELECT over a native view, and gets typed
/// rows back. (The handshake completing at all is also part of AC #2.)
#[tokio::test]
async fn ac1_driver_connects_and_selects_native_view() {
    let client = connect(spawn_server()).await;
    let rows = client
        .query("SELECT id, topic FROM memory WHERE topic = 'planning'", &[])
        .await
        .expect("select over native memory view");
    assert_eq!(rows.len(), 2, "two planning memories in the demo view");
    let topics: Vec<String> = rows.iter().map(|row| row.get::<_, String>("topic")).collect();
    assert!(topics.iter().all(|topic| topic == "planning"));
}

/// AC #2: both a simple query and an extended Parse/Bind/Execute query return a
/// correct RowDescription, DataRow rows, and CommandComplete.
#[tokio::test]
async fn ac2_simple_and_extended_protocols() {
    let client = connect(spawn_server()).await;

    // Simple protocol (psql-style 'Q'): all-text result rows.
    let simple = client
        .simple_query("SELECT id FROM memory")
        .await
        .expect("simple query");
    let simple_rows = simple
        .iter()
        .filter(|message| matches!(message, tokio_postgres::SimpleQueryMessage::Row(_)))
        .count();
    assert_eq!(simple_rows, 3, "simple query returns all three memory rows");

    // Extended protocol: prepare (Parse + Describe + ParameterDescription +
    // RowDescription) then query (Bind + Execute). If ParameterDescription were
    // missing, prepare() would desync and this would error.
    let statement = client
        .prepare("SELECT id FROM memory")
        .await
        .expect("extended-protocol prepare");
    let rows = client.query(&statement, &[]).await.expect("extended query");
    assert_eq!(rows.len(), 3, "extended query returns all three memory rows");
}

/// AC #3: a query joining two native views lowers to the planner IR and executes
/// through the access-method seam.
#[tokio::test]
async fn ac3_join_of_two_native_views() {
    let client = connect(spawn_server()).await;
    let rows = client
        .query(
            "SELECT m.topic, e.stance FROM memory m JOIN epistemic e ON m.id = e.content_id",
            &[],
        )
        .await
        .expect("join across memory and epistemic views");
    assert!(!rows.is_empty(), "the join must return matched rows");
    // mem:1 (planning) -> supports, mem:3 (planning) -> undercuts.
    let stances: Vec<String> = rows.iter().map(|row| row.get::<_, String>("stance")).collect();
    assert!(stances.contains(&"supports".to_string()));
    assert!(stances.contains(&"undercuts".to_string()));
}

/// AC #4: a modality predicate expressed in the SQL subset executes against the
/// matching index rather than a scan. `time_range(col, lo, hi)` must route to the
/// time-series access method, observable via EXPLAIN.
#[tokio::test]
async fn ac4_modality_predicate_uses_index_not_scan() {
    let client = connect(spawn_server()).await;
    let plan = client
        .simple_query("EXPLAIN SELECT id FROM memory WHERE time_range(created_ms, 500, 2500)")
        .await
        .expect("explain modality predicate");
    let plan_text: String = plan
        .iter()
        .filter_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => row.get(0).map(ToString::to_string),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plan_text.contains("time_series"),
        "time_range must route to the time-series access method; plan was:\n{plan_text}"
    );
    assert!(
        plan_text.contains("full_relation_scans=0"),
        "the modality predicate must not fall back to a full scan; plan was:\n{plan_text}"
    );

    // And the predicate actually filters: ts in (500, 2500] -> mem:1 (1000), mem:2 (2000).
    let rows = client
        .query(
            "SELECT id FROM memory WHERE time_range(created_ms, 500, 2500)",
            &[],
        )
        .await
        .expect("time_range filter");
    assert_eq!(rows.len(), 2);
}

/// AC #5: column types arrive at the client as the correct pg type OIDs.
#[tokio::test]
async fn ac5_column_types_are_correct_oids() {
    let client = connect(spawn_server()).await;
    let rows = client
        .query("SELECT id, created_ms FROM memory ORDER BY created_ms", &[])
        .await
        .expect("typed select");

    // The driver decoded RowDescription OIDs: id is text, created_ms is int8.
    assert_eq!(rows[0].columns()[0].type_(), &Type::TEXT);
    assert_eq!(rows[0].columns()[1].type_(), &Type::INT8);

    // And the typed binary decode works end to end (would panic on OID mismatch).
    let id: String = rows[0].get("id");
    let created: i64 = rows[0].get("created_ms");
    assert_eq!(id, "mem:1");
    assert_eq!(created, 1000_i64);
}

/// AC #6: the native memory hot path makes no pg-wire/network call; the wire
/// surface serves external/application reads only.
///
/// Direct cross-crate proof: `rustyred-thg-memory` (which owns eviction and
/// rehydration) does not depend on this crate, so those paths cannot route
/// through the wire. Observable proof here: the wire surface is read-only -- it
/// refuses writes -- so it can never be on a mutation/eviction path.
#[tokio::test]
async fn ac6_wire_surface_is_read_only_and_off_the_hot_path() {
    let client = connect(spawn_server()).await;
    let write = client.simple_query("DELETE FROM memory").await;
    assert!(
        write.is_err(),
        "the wire surface must reject writes; eviction/rehydration never traverse it"
    );
    // Reads still work after the rejected write (connection stays usable).
    let rows = client.simple_query("SELECT id FROM memory").await.unwrap();
    let count = rows
        .iter()
        .filter(|message| matches!(message, tokio_postgres::SimpleQueryMessage::Row(_)))
        .count();
    assert_eq!(count, 3);
}

/// AC #7: epistemics and memory are served by the native engine; auth/billing are
/// the real-Postgres escape hatch and are NOT exposed as wire views.
#[tokio::test]
async fn ac7_boundary_native_views_yes_auth_billing_no() {
    let client = connect(spawn_server()).await;

    // Native epistemic + memory views resolve.
    assert!(client.query("SELECT stance FROM epistemic", &[]).await.is_ok());
    assert!(client.query("SELECT topic FROM memory", &[]).await.is_ok());

    // Auth/billing are not native wire views -> undefined relation. The app
    // connects to real Postgres (Neon) for those, per the corrected boundary.
    let billing = client.query("SELECT * FROM billing_accounts", &[]).await;
    assert!(billing.is_err(), "billing must not be a native wire view");
    let auth = client.query("SELECT * FROM auth_principals", &[]).await;
    assert!(auth.is_err(), "auth must not be a native wire view");
}

// --- regression coverage for the adversarial-review findings ---

/// Parameterized `$1` queries work end to end (Parse param decl -> Bind value ->
/// substitution). Spec section 6 names SeaORM; ORMs parameterize by default.
#[tokio::test]
async fn parameterized_query_over_the_wire() {
    let client = connect(spawn_server()).await;
    let topic = "planning";
    let rows = client
        .query("SELECT id FROM memory WHERE topic = $1", &[&topic])
        .await
        .expect("parameterized query");
    assert_eq!(rows.len(), 2, "both planning memories match via $1");
}

/// `SELECT *` over a join keeps both `id` columns distinct (no display-name
/// collapse): the memory id and the epistemic id both survive in each row.
#[tokio::test]
async fn select_star_join_keeps_both_ids_over_the_wire() {
    let client = connect(spawn_server()).await;
    let messages = client
        .simple_query("SELECT * FROM memory m JOIN epistemic e ON m.id = e.content_id")
        .await
        .expect("star join");
    let row = messages
        .iter()
        .find_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => Some(row),
            _ => None,
        })
        .expect("at least one joined row");
    let values: Vec<String> = (0..row.columns().len())
        .filter_map(|i| row.get(i).map(ToString::to_string))
        .collect();
    assert!(values.iter().any(|v| v.starts_with("mem:")), "memory id survived: {values:?}");
    assert!(values.iter().any(|v| v.starts_with("ep:")), "epistemic id survived: {values:?}");
}

/// `count(DISTINCT topic)` returns the distinct count (2), not the row count (3).
#[tokio::test]
async fn count_distinct_over_the_wire() {
    let client = connect(spawn_server()).await;
    let rows = client
        .query("SELECT count(DISTINCT topic) AS n FROM memory", &[])
        .await
        .expect("count distinct");
    let n: i64 = rows[0].get("n");
    assert_eq!(n, 2, "two distinct topics");
}
