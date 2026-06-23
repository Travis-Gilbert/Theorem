//! I1 acceptance: ask over your store.
//!
//! Plan acceptance (COMMONPLACE-CONSUMER-LOOP.md, I1):
//! "a natural-language question returns an answer grounded in the user's items,
//! each claim traceable to the item it came from."
//!
//! Exercised through the live GraphQL schema with the default (no-model) answer
//! seam, so the answer is the honest extractive fallback over fused
//! graph+vector+lexical retrieval, with provenance back to the source items.

use std::sync::Arc;

use async_graphql::Request;
use commonplace_api::{build_schema, in_memory_store, ApiKeyRegistry, ApiKeyToken, ConsumerSchema};

fn instance(key: &str) -> ConsumerSchema {
    let registry = Arc::new(ApiKeyRegistry::new().with_key(key, "instance"));
    build_schema(in_memory_store(), registry)
}

async fn ingest(schema: &ConsumerSchema, key: &str, title: &str, text: &str) {
    let mutation = format!(
        r#"mutation {{ ingest(input: {{ title: "{title}", text: "{text}", kind: "doc" }}) {{ id }} }}"#
    );
    let response = schema
        .execute(Request::new(mutation).data(ApiKeyToken(key.to_string())))
        .await;
    assert!(
        response.errors.is_empty(),
        "ingest errors: {:?}",
        response.errors
    );
}

#[tokio::test]
async fn ask_returns_a_grounded_answer_with_traceable_provenance() {
    let key = "key";
    let schema = instance(key);

    // Populate the store: three related rust items plus an unrelated distractor.
    ingest(
        &schema,
        key,
        "Rust ownership",
        "rust ownership and borrowing rules govern memory safety",
    )
    .await;
    ingest(
        &schema,
        key,
        "Rust borrow checker",
        "the borrow checker enforces ownership and borrowing in rust programs",
    )
    .await;
    ingest(
        &schema,
        key,
        "Rust lifetimes",
        "lifetimes annotate how long references borrow in rust code",
    )
    .await;
    ingest(
        &schema,
        key,
        "Postgres indexing",
        "btree and gin indexes speed up postgres query planning",
    )
    .await;

    let query = r#"query {
        ask(question: "how does rust ownership and borrowing work", k: 3) {
            answer
            answerKind
            provenance { item { id title } score arms }
        }
    }"#;
    let response = schema
        .execute(Request::new(query).data(ApiKeyToken(key.to_string())))
        .await;
    assert!(
        response.errors.is_empty(),
        "ask errors: {:?}",
        response.errors
    );
    let data = response.data.into_json().unwrap();
    let ask = &data["ask"];

    // An answer was produced, extractively (no generative model configured).
    assert!(
        !ask["answer"].as_str().unwrap().is_empty(),
        "non-empty answer"
    );
    assert_eq!(ask["answerKind"], "EXTRACTIVE");

    // It is grounded in items, each traceable by id.
    let provenance = ask["provenance"].as_array().unwrap();
    assert!(
        !provenance.is_empty(),
        "answer is grounded in at least one item"
    );
    for entry in provenance {
        assert!(
            !entry["item"]["id"].as_str().unwrap().is_empty(),
            "each provenance entry traces to an item id"
        );
        assert!(
            !entry["arms"].as_array().unwrap().is_empty(),
            "each provenance entry records which retrieval arms surfaced it"
        );
    }

    // The top grounding item is a rust item, not the unrelated distractor.
    let top_title = provenance[0]["item"]["title"].as_str().unwrap();
    assert!(
        top_title.starts_with("Rust"),
        "top provenance should be a rust item, got {top_title}"
    );
    assert!(
        provenance
            .iter()
            .any(|entry| entry["item"]["title"].as_str().unwrap().starts_with("Rust")),
        "rust items are surfaced for a rust question"
    );
}

#[tokio::test]
async fn ask_requires_a_valid_key() {
    let schema = instance("good");
    let query = r#"query { ask(question: "anything") { answer } }"#;

    let response = schema.execute(Request::new(query)).await;
    assert!(!response.errors.is_empty(), "ask without a key is rejected");

    let response = schema
        .execute(Request::new(query).data(ApiKeyToken("bad".to_string())))
        .await;
    assert!(
        !response.errors.is_empty(),
        "ask with an invalid key is rejected"
    );
}
