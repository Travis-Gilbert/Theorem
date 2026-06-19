//! X2 acceptance: import and export (no lock-in).
//!
//! Plan acceptance (COMMONPLACE-CONSUMER-LOOP.md, X2):
//! "a markdown or JSON export reimports without loss."
//!
//! Exports an instance to JSON, imports into a fresh instance, and asserts the
//! items round-trip without loss (kind/title/body/tags/collections), plus a
//! human-readable markdown export.

use std::collections::BTreeMap;
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

/// A comparable signature of the store's items (id -> normalized fields).
async fn item_signatures(schema: &ConsumerSchema) -> BTreeMap<String, serde_json::Value> {
    let data = exec(
        schema,
        r#"query { items { id kind title bodyText tags collections } }"#,
    )
    .await;
    let mut map = BTreeMap::new();
    for item in data["items"].as_array().unwrap() {
        let id = item["id"].as_str().unwrap().to_string();
        let mut tags: Vec<String> = item["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap().to_string())
            .collect();
        tags.sort();
        let mut collections: Vec<String> = item["collections"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap().to_string())
            .collect();
        collections.sort();
        map.insert(
            id,
            serde_json::json!({
                "kind": item["kind"], "title": item["title"],
                "bodyText": item["bodyText"], "tags": tags, "collections": collections,
            }),
        );
    }
    map
}

#[tokio::test]
async fn json_export_reimports_without_loss() {
    let source = instance();

    // Populate: auto-structured docs (collections + similarity), a tagged note,
    // and a note filed into a manual collection.
    exec(&source, r#"mutation { ingest(input: { title: "Rust ownership", text: "rust ownership and borrowing govern memory safety", kind: "doc" }) { id } }"#).await;
    exec(&source, r#"mutation { ingest(input: { title: "Rust borrowing", text: "rust ownership and borrowing rules govern memory safety in rust", kind: "doc" }) { id } }"#).await;
    exec(&source, r#"mutation { putNote(title: "A loose thought", text: "sort later", tags: ["open"]) { id } }"#).await;
    let coll = exec(
        &source,
        r#"mutation { createCollection(name: "Manual") { id } }"#,
    )
    .await;
    let coll_id = coll["createCollection"]["id"].as_str().unwrap().to_string();
    let filed = exec(
        &source,
        r#"mutation { putNote(title: "Filed note", text: "lives in manual") { id } }"#,
    )
    .await;
    let filed_id = filed["putNote"]["id"].as_str().unwrap().to_string();
    exec(
        &source,
        format!(
            r#"mutation {{ addToCollection(itemId: "{filed_id}", collectionId: "{coll_id}") }}"#
        ),
    )
    .await;

    let before = item_signatures(&source).await;
    assert!(before.len() >= 4, "source has items");

    // Export JSON.
    let export = exec(&source, r#"query { export(format: JSON) }"#).await;
    let json = export["export"].as_str().unwrap().to_string();
    assert!(json.contains("Rust ownership"));

    // Import into a fresh instance.
    let target = instance();
    assert!(
        item_signatures(&target).await.is_empty(),
        "target starts empty"
    );
    let import = exec(
        &target,
        format!(
            r#"mutation {{ importItems(data: {}) {{ imported collections }} }}"#,
            serde_json::Value::String(json.clone())
        ),
    )
    .await;
    assert!(import["importItems"]["imported"].as_i64().unwrap() >= 4);
    assert!(import["importItems"]["collections"].as_i64().unwrap() >= 1);

    // The target's items match the source's exactly (id-keyed, no loss).
    let after = item_signatures(&target).await;
    assert_eq!(after, before, "every item round-trips without loss");

    // Memberships survived (the filed note is still in its collection).
    assert!(
        after[&filed_id]["collections"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == &serde_json::json!(coll_id)),
        "collection membership preserved across export/import"
    );
}

#[tokio::test]
async fn markdown_export_is_human_readable() {
    let schema = instance();
    exec(&schema, r#"mutation { ingest(input: { title: "Coffee notes", text: "pour over technique and grind size", kind: "doc" }) { id } }"#).await;
    let export = exec(&schema, r#"query { export(format: MARKDOWN) }"#).await;
    let markdown = export["export"].as_str().unwrap();
    assert!(
        markdown.starts_with("# CommonPlace export"),
        "markdown header: {markdown}"
    );
    assert!(
        markdown.contains("Coffee notes"),
        "markdown includes the item: {markdown}"
    );
}
