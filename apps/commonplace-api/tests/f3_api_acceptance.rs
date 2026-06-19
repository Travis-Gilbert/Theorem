//! F3 acceptance: the interoperability API seam.
//!
//! Plan acceptance (COMMONPLACE-CONSUMER-LOOP.md, F3):
//! "an external client with a URL and a key reads and writes items; pointing it
//! at a different instance URL connects to that instance's data; an invalid key
//! is rejected."
//!
//! Verified in-process against the live schema (the HTTP layer in main.rs is a
//! thin header -> request-data shim over the same schema.execute path).

use std::sync::Arc;

use async_graphql::Request;
use commonplace_api::{build_schema, in_memory_store, ApiKeyRegistry, ApiKeyToken, ConsumerSchema};

fn instance_with_key(key: &str) -> ConsumerSchema {
    let registry = Arc::new(ApiKeyRegistry::new().with_key(key, "instance"));
    build_schema(in_memory_store(), registry)
}

#[tokio::test]
async fn client_with_key_reads_and_writes_items() {
    let key = "valid-key";
    let schema = instance_with_key(key);

    // Write: auto-structuring ingest through the API.
    let mutation = r#"mutation {
        ingest(input: { title: "Ownership", text: "rust ownership and borrowing", kind: "doc" }) {
            id
            kind
            title
            collections
            path
        }
    }"#;
    let response = schema
        .execute(Request::new(mutation).data(ApiKeyToken(key.to_string())))
        .await;
    assert!(
        response.errors.is_empty(),
        "ingest errors: {:?}",
        response.errors
    );
    let data = response.data.into_json().unwrap();
    let id = data["ingest"]["id"].as_str().unwrap().to_string();
    assert!(!id.is_empty());
    assert_eq!(data["ingest"]["kind"], "doc");
    assert!(!data["ingest"]["collections"].as_array().unwrap().is_empty());

    // Read it back by id.
    let query = format!(r#"query {{ item(id: "{id}") {{ id title kind }} }}"#);
    let response = schema
        .execute(Request::new(query).data(ApiKeyToken(key.to_string())))
        .await;
    assert!(
        response.errors.is_empty(),
        "read errors: {:?}",
        response.errors
    );
    let data = response.data.into_json().unwrap();
    assert_eq!(data["item"]["title"], "Ownership");

    // Similarity search returns the item.
    let search =
        r#"query { search(query: "rust ownership borrowing", k: 5) { item { id } score } }"#;
    let response = schema
        .execute(Request::new(search).data(ApiKeyToken(key.to_string())))
        .await;
    assert!(
        response.errors.is_empty(),
        "search errors: {:?}",
        response.errors
    );
    let data = response.data.into_json().unwrap();
    let hits = data["search"].as_array().unwrap();
    assert!(
        hits.iter()
            .any(|hit| hit["item"]["id"] == serde_json::json!(id)),
        "ingested item is searchable through the API"
    );
}

#[tokio::test]
async fn invalid_or_missing_key_is_rejected() {
    let schema = instance_with_key("good-key");
    let query = r#"query { items { id } }"#;

    // Wrong key.
    let response = schema
        .execute(Request::new(query).data(ApiKeyToken("wrong-key".to_string())))
        .await;
    assert!(
        !response.errors.is_empty(),
        "an invalid key must be rejected"
    );

    // No key at all.
    let response = schema.execute(Request::new(query)).await;
    assert!(
        !response.errors.is_empty(),
        "a missing key must be rejected"
    );

    // A write with a bad key must also be rejected and must not mutate.
    let mutation = r#"mutation { putNote(title: "sneaky", text: "x") { id } }"#;
    let response = schema
        .execute(Request::new(mutation).data(ApiKeyToken("wrong-key".to_string())))
        .await;
    assert!(
        !response.errors.is_empty(),
        "an unauthorized write must be rejected"
    );
}

#[tokio::test]
async fn different_instances_have_separate_data() {
    let key = "shared-key";
    let instance_a = instance_with_key(key);
    let instance_b = instance_with_key(key);

    // Write to instance A only.
    let mutation = r#"mutation { putNote(title: "A only", text: "lives in A") { id } }"#;
    let response = instance_a
        .execute(Request::new(mutation).data(ApiKeyToken(key.to_string())))
        .await;
    assert!(response.errors.is_empty(), "{:?}", response.errors);

    // Instance B (a different instance URL) has its own, empty dataset.
    let query = r#"query { items { id title } }"#;
    let response = instance_b
        .execute(Request::new(query).data(ApiKeyToken(key.to_string())))
        .await;
    let data = response.data.into_json().unwrap();
    assert_eq!(
        data["items"].as_array().unwrap().len(),
        0,
        "instance B does not see instance A's data"
    );

    // Instance A has exactly the one item.
    let response = instance_a
        .execute(Request::new(query).data(ApiKeyToken(key.to_string())))
        .await;
    let data = response.data.into_json().unwrap();
    let items = data["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "A only");
}
