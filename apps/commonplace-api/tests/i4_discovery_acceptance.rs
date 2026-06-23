//! I4 acceptance: discovery.
//!
//! Plan acceptance (COMMONPLACE-CONSUMER-LOOP.md, I4):
//! "a discovery call returns candidate links between items that are not yet
//! connected, ranked."

use std::sync::Arc;

use async_graphql::Request;
use commonplace::{DeterministicEmbedder, IngestInput, IngestPipeline};
use commonplace_api::{
    build_schema, in_memory_store, ApiKeyRegistry, ApiKeyToken, ConsumerSchema, InMemoryShared,
};

const KEY: &str = "key";

async fn discover(schema: &ConsumerSchema, min: f64) -> serde_json::Value {
    let query = format!(
        r#"query {{ discover(minSimilarity: {min}, maxResults: 20) {{ a {{ id title }} b {{ id title }} similarity reason }} }}"#
    );
    let response = schema
        .execute(Request::new(query).data(ApiKeyToken(KEY.to_string())))
        .await;
    assert!(
        response.errors.is_empty(),
        "discover errors: {:?}",
        response.errors
    );
    response.data.into_json().unwrap()
}

fn is_rust_pair(link: &serde_json::Value, a: &str, b: &str) -> bool {
    let la = link["a"]["id"].as_str().unwrap();
    let lb = link["b"]["id"].as_str().unwrap();
    (la == a && lb == b) || (la == b && lb == a)
}

#[tokio::test]
async fn discover_proposes_similar_but_unlinked_pairs_then_excludes_connected() {
    let store: InMemoryShared = in_memory_store();

    // Pre-populate with an artificially HIGH similarity threshold so the items
    // are embedded and indexed but the auto-linker writes NO SIMILAR_TO edges:
    // a latent connection for discovery to surface.
    let (rust_a, rust_b);
    {
        let mut cp = store.lock().unwrap();
        let pipeline =
            IngestPipeline::new(DeterministicEmbedder::default()).with_similarity_threshold(0.99);
        let a = pipeline
            .ingest(
                &mut cp,
                IngestInput::document(
                    "Rust ownership",
                    "rust ownership and borrowing govern memory safety",
                ),
            )
            .unwrap();
        let b = pipeline
            .ingest(
                &mut cp,
                IngestInput::document(
                    "Rust borrowing",
                    "rust ownership and borrowing rules govern memory safety in rust",
                ),
            )
            .unwrap();
        pipeline
            .ingest(
                &mut cp,
                IngestInput::document(
                    "Postgres indexing",
                    "btree and gin indexes speed up postgres query planning",
                ),
            )
            .unwrap();
        rust_a = a.item.id.clone();
        rust_b = b.item.id.clone();
    }

    let registry = Arc::new(ApiKeyRegistry::new().with_key(KEY, "instance"));
    let schema = build_schema(store.clone(), registry);

    // The two rust docs are similar but not yet linked -> proposed, ranked.
    let data = discover(&schema, 0.3).await;
    let links = data["discover"].as_array().unwrap();
    assert!(!links.is_empty(), "discovery returns candidate links");
    assert!(
        links
            .iter()
            .any(|link| is_rust_pair(link, &rust_a, &rust_b)),
        "the similar-but-unlinked rust pair is proposed"
    );
    // Ranked by similarity, each above the floor, each with a reason.
    let mut previous = f64::INFINITY;
    for link in links {
        let similarity = link["similarity"].as_f64().unwrap();
        assert!(similarity >= 0.3, "respects the min-similarity floor");
        assert!(
            similarity <= previous + 1e-9,
            "results are ranked by similarity desc"
        );
        previous = similarity;
        assert!(!link["reason"].as_str().unwrap().is_empty());
    }

    // Connect the pair directly; discovery must no longer propose it.
    {
        let mut cp = store.lock().unwrap();
        cp.add_similarity(&rust_a, &rust_b, 1.0).unwrap();
    }
    let data = discover(&schema, 0.3).await;
    let links = data["discover"].as_array().unwrap();
    assert!(
        !links
            .iter()
            .any(|link| is_rust_pair(link, &rust_a, &rust_b)),
        "a now-connected pair is no longer proposed"
    );
}
