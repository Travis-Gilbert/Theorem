//! I2 acceptance: proactive surfacing (briefing).
//!
//! Plan acceptance (COMMONPLACE-CONSUMER-LOOP.md, I2):
//! "a briefing call returns recent and newly-connected items and open threads
//! drawn from the user's store."

use std::sync::Arc;

use async_graphql::Request;
use commonplace_api::{build_schema, in_memory_store, ApiKeyRegistry, ApiKeyToken, ConsumerSchema};

const KEY: &str = "key";

fn instance() -> ConsumerSchema {
    let registry = Arc::new(ApiKeyRegistry::new().with_key(KEY, "instance"));
    build_schema(in_memory_store(), registry)
}

async fn exec(schema: &ConsumerSchema, query: impl Into<String>) -> serde_json::Value {
    let response = schema
        .execute(Request::new(query).data(ApiKeyToken(KEY.to_string())))
        .await;
    assert!(
        response.errors.is_empty(),
        "gql errors: {:?}",
        response.errors
    );
    response.data.into_json().unwrap()
}

#[tokio::test]
async fn briefing_surfaces_recent_connected_and_open_threads() {
    let schema = instance();

    // Two closely-related docs (auto-structured + linked via SIMILAR_TO).
    exec(
        &schema,
        r#"mutation { ingest(input: { title: "Rust ownership", text: "rust ownership and borrowing govern memory safety", kind: "doc" }) { id } }"#,
    )
    .await;
    exec(
        &schema,
        r#"mutation { ingest(input: { title: "Rust borrowing", text: "rust ownership and borrowing rules govern memory safety in rust", kind: "doc" }) { id } }"#,
    )
    .await;

    // A raw capture in no collection -> an open thread by being unfiled.
    exec(
        &schema,
        r#"mutation { putNote(title: "Quick capture", text: "a raw thought to sort later") { id } }"#,
    )
    .await;

    // An open-tagged note that we then FILE into a collection: still an open
    // thread by tag, proving the tag path is independent of being unfiled.
    let todo = exec(
        &schema,
        r#"mutation { putNote(title: "Todo item", text: "finish the report", tags: ["open"]) { id } }"#,
    )
    .await;
    let todo_id = todo["putNote"]["id"].as_str().unwrap().to_string();
    let tasks = exec(
        &schema,
        r#"mutation { createCollection(name: "Tasks") { id } }"#,
    )
    .await;
    let tasks_id = tasks["createCollection"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    exec(
        &schema,
        format!(
            r#"mutation {{ addToCollection(itemId: "{todo_id}", collectionId: "{tasks_id}") }}"#
        ),
    )
    .await;

    let data = exec(
        &schema,
        r#"query {
            briefing(recentLimit: 10, connectedLimit: 10, openLimit: 10) {
                recent { id title }
                newlyConnected { item { id title } connections related { id } }
                openThreads { id title collections }
            }
        }"#,
    )
    .await;
    let briefing = &data["briefing"];

    // Recent: non-empty, drawn from the store.
    let recent = briefing["recent"].as_array().unwrap();
    assert!(!recent.is_empty(), "briefing surfaces recent items");

    // Newly connected: the linked rust docs appear with a connection count.
    let connected = briefing["newlyConnected"].as_array().unwrap();
    assert!(
        !connected.is_empty(),
        "briefing surfaces newly-connected items"
    );
    let top = &connected[0];
    assert!(
        top["item"]["title"].as_str().unwrap().starts_with("Rust"),
        "the most-connected item is one of the linked rust docs"
    );
    assert!(top["connections"].as_i64().unwrap() >= 1);
    assert!(
        !top["related"].as_array().unwrap().is_empty(),
        "connected item lists its neighbors"
    );

    // Open threads: the unfiled capture AND the open-tagged (but filed) todo;
    // the filed-and-untagged rust docs are NOT open threads.
    let open_titles: Vec<String> = briefing["openThreads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["title"].as_str().unwrap().to_string())
        .collect();
    assert!(
        open_titles.contains(&"Quick capture".to_string()),
        "unfiled capture is an open thread"
    );
    assert!(
        open_titles.contains(&"Todo item".to_string()),
        "open-tagged item is an open thread"
    );
    assert!(
        !open_titles.contains(&"Rust ownership".to_string()),
        "filed, untagged items are not open threads"
    );

    // The open-tagged todo is genuinely filed (open by tag, not by being unfiled).
    let todo_entry = briefing["openThreads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["title"] == "Todo item")
        .unwrap();
    assert!(
        !todo_entry["collections"].as_array().unwrap().is_empty(),
        "the open-tagged item is filed yet still surfaced as open"
    );
}
