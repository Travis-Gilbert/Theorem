//! DIAGNOSTIC: does a Value carrying an f32 embedding re-serialize byte-stably
//! across a JSON round-trip? RedCore's AOF checksum is sha256(to_vec(mutation));
//! if the embedding Value is unstable, recover's checksum check fails -> truncate.

use commonplace::{DeterministicEmbedder, Embedder};
use serde_json::json;

#[test]
fn embedding_value_roundtrips_byte_stable() {
    let emb = DeterministicEmbedder::default()
        .embed_text("rust ownership and borrowing govern memory safety")
        .unwrap();
    let value = json!({ "embedding": emb, "title": "x", "kind": "doc" });

    let a = serde_json::to_vec(&value).unwrap();
    let reparsed: serde_json::Value = serde_json::from_slice(&a).unwrap();
    let b = serde_json::to_vec(&reparsed).unwrap();

    eprintln!("emb sample = {:?}", &emb[..emb.len().min(6)]);
    eprintln!("a == b ? {}", a == b);
    if a != b {
        eprintln!("WRITE  = {}", String::from_utf8_lossy(&a));
        eprintln!("REPARSE= {}", String::from_utf8_lossy(&b));
    }
    assert_eq!(a, b, "embedding Value not byte-stable across JSON round-trip");
}
