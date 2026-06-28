//! Local-first commonplace DATA routes, exercised end-to-end through a real
//! loopback bind with `reqwest`.
//!
//! The governing claim of this slice: a same-machine desktop webapp connects to
//! THIS local instance for its commonplace data. So the control endpoint serves
//! items / collections / search over the SAME durable graph the ambient watcher
//! maintains (one process, one graph, no second AOF handle), behind the existing
//! `DeviceAuth`.
//!
//! Coverage (the mandatory matrix for this slice):
//! * ingest a real file through the ambient watcher, then `GET /v1/items` returns
//!   it over HTTP with a valid token, and `GET /v1/search` finds it -- proving the
//!   watcher's write is visible to the read because they share one graph;
//! * `/v1/items` is `401` without a token;
//! * the auto-provisioned local-access token authorizes the data routes over HTTP
//!   (no pairing-code dance) and is revocable like any device.

use std::sync::Arc;
use std::time::{Duration, Instant};

use commonplace_desktop_runtime::{
    serve_control, spawn_ambient_with_data, ControlServer, ControlState, DevicePairing,
    MockExecutor, RunRegistry, SharedSink, WatchConfig, PAIRING_CODE_HEADER,
};

const PAIRING_CODE: &str = "test-pairing-code";

/// Bring up a control server over a given shared sink and a SHARED device
/// registry on an ephemeral loopback port. The caller passes the `DevicePairing`
/// it also holds, so revocations it makes are visible to the server (one registry
/// instance; a second `DevicePairing::open` on the same dir would have its own
/// in-memory mirror and would not see the revoke).
async fn serve_with_data(pairing: DevicePairing, data: SharedSink) -> (ControlServer, String) {
    let runs = RunRegistry::new(Arc::new(MockExecutor::new()));
    let state = ControlState::new(pairing, PAIRING_CODE, runs).with_data(data);
    let server = serve_control(state, 0).await.expect("bind loopback");
    let base = format!("http://{}", server.local_addr());
    (server, base)
}

/// Pair a device over the wire and return its bearer token (the QR/code path).
async fn pair(client: &reqwest::Client, base: &str) -> String {
    let response = client
        .post(format!("{base}/pair"))
        .header(PAIRING_CODE_HEADER, PAIRING_CODE)
        .json(&serde_json::json!({ "label": "Webapp" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json::<serde_json::Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The headline acceptance: an ambient file write lands in the durable graph and
/// is then readable + searchable over the local HTTP data routes with a token.
#[tokio::test]
async fn ambient_ingest_is_readable_and_searchable_over_http() {
    // A watched working tree (the sidecar lives under it, always ignored).
    let workdir = tempfile::tempdir().unwrap();
    let config = WatchConfig::new(workdir.path());

    // Start the ambient watcher AND get a read handle onto the SAME graph.
    let (handle, data) = spawn_ambient_with_data(config).expect("spawn ambient");

    // Serve the data routes over that shared graph. The device registry lives in
    // its own tempdir (a separate concern from the watched tree).
    let registry_dir = tempfile::tempdir().unwrap();
    let pairing = DevicePairing::open(registry_dir.path()).unwrap();
    let (server, base) = serve_with_data(pairing, data.clone()).await;
    let client = reqwest::Client::new();
    let token = pair(&client, &base).await;

    // Give the spawned watch thread a moment to establish the recursive watch
    // before writing, so the create event is not raced by a just-started watcher
    // (the proven timing from the slice-4 spawn_watcher acceptance).
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Write a real file into the watched tree; the watcher ingests it.
    let needle = "the local-first connection path ships in this slice";
    std::fs::write(workdir.path().join("roadmap.md"), needle).unwrap();

    // Poll the local HTTP list route until the ambient ingest has landed (the
    // debounce + ingest is async on the watch thread). This is the real
    // end-to-end: watcher write -> shared graph -> HTTP read.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut found = None;
    while Instant::now() < deadline {
        let response = client
            .get(format!("{base}/v1/items"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body = response.json::<serde_json::Value>().await.unwrap();
        let items = body["items"].as_array().cloned().unwrap_or_default();
        if let Some(item) = items
            .into_iter()
            .find(|item| item["title"] == serde_json::json!("roadmap.md"))
        {
            found = Some(item);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let item = found.expect("the ambient-ingested file appears over /v1/items");
    let id = item["id"].as_str().unwrap().to_string();
    assert_eq!(item["kind"], "doc", "a watched text file ingests as a doc");

    // GET /v1/items/{id} returns the same item over HTTP.
    let detail = client
        .get(format!("{base}/v1/items/{id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(detail.status(), reqwest::StatusCode::OK);
    let detail_body = detail.json::<serde_json::Value>().await.unwrap();
    assert_eq!(detail_body["item"]["id"], serde_json::json!(id));

    // GET /v1/search finds the ingested item (real commonplace vector search over
    // the embedding the ambient ingest wrote).
    let search = client
        .get(format!("{base}/v1/search"))
        .query(&[("q", "local-first connection"), ("k", "5")])
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(search.status(), reqwest::StatusCode::OK);
    let search_body = search.json::<serde_json::Value>().await.unwrap();
    let hits = search_body["hits"].as_array().unwrap();
    assert!(
        hits.iter().any(|hit| hit["item"]["id"] == serde_json::json!(id)),
        "search surfaces the ambient-ingested item: {search_body}"
    );

    // /v1/items without a token is 401 over the wire.
    let unauth = client
        .get(format!("{base}/v1/items"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    server.shutdown().await.expect("clean shutdown");
    handle.stop().expect("clean watcher stop");
}

/// The local-access path: a same-machine webapp uses the auto-provisioned
/// local-access token (no pairing-code dance), and it is revocable like any
/// device.
#[tokio::test]
async fn local_access_token_authorizes_data_routes_and_is_revocable() {
    let workdir = tempfile::tempdir().unwrap();
    let config = WatchConfig::new(workdir.path());
    let data = SharedSink::open(&config).expect("open shared sink");
    // Seed one item directly so a read has something to return.
    {
        use commonplace::{IngestInput, IngestPipeline};
        let mut sink = data.lock();
        IngestPipeline::default()
            .ingest(
                sink.commonplace_mut(),
                IngestInput::document("note.md", "a local note"),
            )
            .unwrap();
    }

    // The instance provisions a local-access device on startup and the Tauri shell
    // reads its token. We model that here: open the SAME registry the control
    // endpoint uses and ensure local access.
    let registry_dir = tempfile::tempdir().unwrap();
    let provisioning = DevicePairing::open(registry_dir.path()).unwrap();
    let local = provisioning.ensure_local_access().expect("provision local access");
    // The Tauri shell would read the token back from the sidecar:
    let token_for_webapp = provisioning
        .local_access_token()
        .unwrap()
        .expect("a local-access token is persisted for the shell");
    assert_eq!(token_for_webapp, local.token);

    // Serve over the SAME `DevicePairing` instance, so the local-access device the
    // control endpoint verifies against is the one we just provisioned -- and a
    // revoke through `provisioning` is immediately seen by the server (one registry
    // instance shared via its cheap `Arc`-backed clone).
    let (server, base) = serve_with_data(provisioning.clone(), data.clone()).await;
    let client = reqwest::Client::new();

    // The webapp uses the local-access token directly -- no /pair call.
    let listed = client
        .get(format!("{base}/v1/items"))
        .bearer_auth(&token_for_webapp)
        .send()
        .await
        .unwrap();
    assert_eq!(
        listed.status(),
        reqwest::StatusCode::OK,
        "the local-access token authorizes the data routes with no pairing dance"
    );
    assert_eq!(
        listed.json::<serde_json::Value>().await.unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Revoke the local-access device (like any device): the token stops working.
    assert!(provisioning.revoke_device(&local.device_id).unwrap());
    let after = client
        .get(format!("{base}/v1/items"))
        .bearer_auth(&token_for_webapp)
        .send()
        .await
        .unwrap();
    assert_eq!(
        after.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "a revoked local-access token no longer authorizes (revocable like any device)"
    );

    server.shutdown().await.expect("clean shutdown");
}
