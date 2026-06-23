//! Gated live smoke for the multimodal (vision) input path.
//!
//! This test is `#[ignore]` so it never runs in CI: it requires a running
//! llama-server started with the Gemma vision projector, e.g.
//!
//! ```text
//! llama-server -m gemma-4-12b-it-q4.gguf --mmproj mmproj-gemma-4-12b-it.gguf -c 4096
//! ```
//!
//! Run it by hand after that server is up:
//!
//! ```text
//! AGENTD_SMOKE_BASE_URL=http://127.0.0.1:8080/v1 \
//!   cargo test -p theorem-agentd --test multimodal_live_smoke -- --ignored --nocapture
//! ```
//!
//! It sends exactly one `ChatMessage::user_with_images` turn carrying a small
//! solid-red PNG and asserts the live model returned a completion. The point is
//! to prove two things the unit tests cannot: that the projector is loaded
//! (otherwise the server errors or ignores the image) and that our `image_url`
//! content-array body is accepted on the wire. It does not assert exact caption
//! text, because vision output is nondeterministic; the operator reads the
//! printed completion to confirm it reflects a red image.

use theorem_agentd::config::ModelConfig;
use theorem_agentd::model::{ChatMessage, InputImage, ModelClient, ModelDecision};
use theorem_agentd::tools::ToolCatalog;

/// A valid 8x8 solid-red PNG, base64-encoded with no `data:` prefix, generated
/// from Python stdlib (zlib + struct) so it decodes in any image stack.
const RED_PNG_BASE64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEUlEQVR42mP4z8CAFTEMLQkAKP8/wc53yE8AAAAASUVORK5CYII=";

#[test]
#[ignore = "live: needs a llama-server started with --mmproj (Gemma vision)"]
fn image_turn_returns_a_completion_from_live_server() {
    let base_url = std::env::var("AGENTD_SMOKE_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8080/v1".to_string());
    let model =
        std::env::var("AGENTD_SMOKE_MODEL").unwrap_or_else(|_| "gemma-4-12b-it-q4".to_string());

    // Build a real openai-compatible client. ModelConfig has serde defaults on
    // every field, so a minimal TOML stays robust if new fields are added.
    let config: ModelConfig = toml::from_str(&format!(
        "provider = \"openai-compatible\"\nbase_url = \"{base_url}\"\nmodel = \"{model}\"\n"
    ))
    .expect("smoke ModelConfig should deserialize");

    let client = ModelClient::from_config(
        config,
        "repo:theorem:branch:main".to_string(),
        "theorem-agentd".to_string(),
    )
    .expect("openai-compatible model client");

    let image = InputImage {
        media_type: "image/png".to_string(),
        data_base64: RED_PNG_BASE64.to_string(),
    };
    let messages = vec![
        ChatMessage::system(
            "You are a vision assistant. Look at the image and answer in one short sentence.",
        ),
        ChatMessage::user_with_images("What is the dominant color of this image?", vec![image]),
    ];

    // The catalog only offers the standard tools; a direct visual question
    // normally yields final text. Either outcome proves the image body was
    // accepted, which is what this smoke guards.
    let catalog = ToolCatalog::default_catalog();
    let output = client
        .decide(&messages, &catalog, "")
        .expect("decide() against a live --mmproj llama-server");

    assert!(
        !output.raw_content.trim().is_empty(),
        "live server returned an empty completion for an image turn",
    );

    match &output.decision {
        ModelDecision::Final { text } => {
            assert!(!text.trim().is_empty(), "vision completion text was empty");
            eprintln!("[smoke] vision completion: {text}");
        }
        ModelDecision::ToolCall { name, arguments } => {
            eprintln!(
                "[smoke] model chose a tool call ({name}, args={arguments}) rather than \
                 describing the image; the multimodal request was still accepted and \
                 produced a completion. Re-run with a more direct prompt to see a caption.",
            );
        }
    }
}
