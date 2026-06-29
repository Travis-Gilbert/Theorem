//! Cache-stable memory injection into model request bodies
//! (SPEC-LOCAL-PROXY-MVP D3).
//!
//! Relevant memory is appended to the LAST user message, so the `system` prefix and
//! the `tools` array -- the cached prefix -- are never touched. Re-serialization is
//! deterministic, so two identical client requests produce identical proxied bytes:
//! the second hits the provider prompt cache. That determinism is the cache-stability
//! guarantee, testable without a live call.
//!
//! Fail open: a body we cannot parse, one without messages, or one with no relevant
//! memory is returned unchanged.

use serde_json::{json, Value};

use crate::memory::{MemoryHit, MemorySource};

/// Inject relevant memory into `body` at the cache-stable suffix. Returns new body
/// bytes, or the original on any failure (fail open).
pub fn inject_memory(body: &[u8], source: &dyn MemorySource, max: usize) -> Vec<u8> {
    let Ok(mut request) = serde_json::from_slice::<Value>(body) else {
        return body.to_vec();
    };
    let query = match request.get("messages").and_then(Value::as_array) {
        Some(messages) => last_user_text(messages),
        None => return body.to_vec(),
    };
    if query.trim().is_empty() {
        return body.to_vec();
    }
    let hits = source.retrieve(&query, max);
    if hits.is_empty() {
        return body.to_vec();
    }
    if !append_to_last_user(&mut request, &render_block(&hits)) {
        return body.to_vec();
    }
    serde_json::to_vec(&request).unwrap_or_else(|_| body.to_vec())
}

/// Inject relevant memory into an OpenAI Responses request body. Codex uses the
/// Responses wire API; its cached prefix still lives before the latest user input,
/// so this follows the same suffix-only, fail-open rule as Anthropic Messages.
pub fn inject_openai_responses_memory(
    body: &[u8],
    source: &dyn MemorySource,
    max: usize,
) -> Vec<u8> {
    let Ok(mut request) = serde_json::from_slice::<Value>(body) else {
        return body.to_vec();
    };
    let Some(input) = request.get("input") else {
        return body.to_vec();
    };
    let query = responses_input_to_text(input);
    if query.trim().is_empty() {
        return body.to_vec();
    }
    let hits = source.retrieve(&query, max);
    if hits.is_empty() {
        return body.to_vec();
    }
    if !append_to_openai_input(&mut request, &render_block(&hits)) {
        return body.to_vec();
    }
    serde_json::to_vec(&request).unwrap_or_else(|_| body.to_vec())
}

fn last_user_text(messages: &[Value]) -> String {
    messages
        .iter()
        .rev()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .map(|message| content_to_text(message.get("content")))
        .unwrap_or_default()
}

fn content_to_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn responses_input_to_text(input: &Value) -> String {
    match input {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .rev()
            .find(|item| item.get("role").and_then(Value::as_str) == Some("user"))
            .map(|item| openai_content_to_text(item.get("content")))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn openai_content_to_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| {
                matches!(
                    block.get("type").and_then(Value::as_str),
                    Some("input_text") | Some("text")
                )
            })
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Append a text block to the last user message. Everything before it -- system,
/// tools, prior turns -- is left untouched.
fn append_to_last_user(request: &mut Value, block: &str) -> bool {
    let Some(messages) = request.get_mut("messages").and_then(Value::as_array_mut) else {
        return false;
    };
    for message in messages.iter_mut().rev() {
        if message.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let content = message
            .get_mut("content")
            .map(Value::take)
            .unwrap_or(Value::Null);
        let mut blocks = match content {
            Value::String(text) => vec![json!({"type": "text", "text": text})],
            Value::Array(existing) => existing,
            Value::Null => Vec::new(),
            other => vec![other],
        };
        blocks.push(json!({"type": "text", "text": block}));
        message["content"] = Value::Array(blocks);
        return true;
    }
    false
}

fn append_to_openai_input(request: &mut Value, block: &str) -> bool {
    let Some(input) = request.get_mut("input") else {
        return false;
    };
    match input {
        Value::String(text) => {
            text.push_str("\n\n");
            text.push_str(block);
            true
        }
        Value::Array(items) => {
            for item in items.iter_mut().rev() {
                if item.get("role").and_then(Value::as_str) != Some("user") {
                    continue;
                }
                let content = item
                    .get_mut("content")
                    .map(Value::take)
                    .unwrap_or(Value::Null);
                let mut blocks = match content {
                    Value::String(text) => vec![json!({"type": "input_text", "text": text})],
                    Value::Array(existing) => existing,
                    Value::Null => Vec::new(),
                    other => vec![other],
                };
                blocks.push(json!({"type": "input_text", "text": block}));
                item["content"] = Value::Array(blocks);
                return true;
            }
            false
        }
        _ => false,
    }
}

fn render_block(hits: &[MemoryHit]) -> String {
    let mut block = String::from(
        "<theorem-memory note=\"ambient context from the local proxy; verify against current code before relying on it\">\n",
    );
    for hit in hits {
        block.push_str("- ");
        block.push_str(&hit.title);
        block.push_str(": ");
        block.push_str(hit.body.trim());
        block.push('\n');
    }
    block.push_str("</theorem-memory>");
    block
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::VecMemorySource;

    fn request() -> Value {
        json!({
            "model": "claude",
            "system": [{"type": "text", "text": "SYSTEM", "cache_control": {"type": "ephemeral"}}],
            "tools": [{"name": "Read"}],
            "messages": [
                {"role": "user", "content": "first turn"},
                {"role": "assistant", "content": "ok"},
                {"role": "user", "content": "tell me about the planner pushdown"}
            ]
        })
    }

    #[test]
    fn injects_relevant_memory_without_touching_the_cached_prefix() {
        let body = serde_json::to_vec(&request()).unwrap();
        let source = VecMemorySource::new(vec![
            ("planner", "the planner does boolean pushdown in planner.rs"),
            ("cats", "cats are nice"),
        ]);
        let injected = inject_memory(&body, &source, 5);
        let out: Value = serde_json::from_slice(&injected).unwrap();
        let original = request();

        // Cached prefix is unchanged: system, tools, and all but the last turn.
        assert_eq!(out["system"], original["system"]);
        assert_eq!(out["tools"], original["tools"]);
        assert_eq!(out["messages"][0], original["messages"][0]);
        assert_eq!(out["messages"][1], original["messages"][1]);

        // The relevant memory landed in the last user message; the irrelevant did not.
        let last = serde_json::to_string(&out["messages"][2]).unwrap();
        assert!(last.contains("planner.rs"), "relevant memory injected");
        assert!(!last.contains("cats"), "irrelevant memory excluded");
        assert!(
            last.contains("tell me about the planner"),
            "user text preserved"
        );
        assert!(last.contains("theorem-memory"), "injection is delimited");
    }

    #[test]
    fn injection_is_deterministic_so_the_prefix_cache_holds() {
        let body = serde_json::to_vec(&request()).unwrap();
        let source = VecMemorySource::new(vec![("planner", "planner.rs pushdown")]);
        let first = inject_memory(&body, &source, 5);
        let second = inject_memory(&body, &source, 5);
        assert_eq!(
            first, second,
            "identical requests produce identical bytes -> the second hits the cache"
        );
    }

    #[test]
    fn fails_open_on_non_json_body() {
        let body = b"not json at all".to_vec();
        let source = VecMemorySource::new(vec![("planner", "planner.rs pushdown")]);
        assert_eq!(inject_memory(&body, &source, 5), body);
    }

    #[test]
    fn fails_open_when_no_memory_is_relevant() {
        let body = serde_json::to_vec(&request()).unwrap();
        let source = VecMemorySource::new(vec![("cats", "cats are nice")]);
        assert_eq!(inject_memory(&body, &source, 5), body);
    }

    #[test]
    fn preserves_tool_use_and_tool_result_blocks() {
        // D4 tool-call parity: injection appends to the last user TEXT turn and must
        // leave tool_use ids and tool_result blocks byte-identical.
        let original = json!({
            "model": "claude",
            "tools": [{"name": "Read"}],
            "messages": [
                {"role": "user", "content": "start"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {"path": "a"}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "file bytes"}
                ]},
                {"role": "user", "content": "now tell me about the planner pushdown"}
            ]
        });
        let body = serde_json::to_vec(&original).unwrap();
        let source = VecMemorySource::new(vec![("planner", "planner.rs pushdown")]);
        let out: Value = serde_json::from_slice(&inject_memory(&body, &source, 5)).unwrap();

        // The tool_use turn and the tool_result turn survive untouched.
        assert_eq!(
            out["messages"][1], original["messages"][1],
            "tool_use preserved"
        );
        assert_eq!(
            out["messages"][2], original["messages"][2],
            "tool_result preserved"
        );
        // Injection landed only in the last user text turn.
        let last = serde_json::to_string(&out["messages"][3]).unwrap();
        assert!(
            last.contains("planner.rs"),
            "memory injected into last user turn"
        );
        assert!(last.contains("theorem-memory"), "injection delimited");
    }

    #[test]
    fn injects_openai_responses_memory_at_latest_user_input() {
        let original = json!({
            "model": "gpt-5.5",
            "tools": [{"type": "function", "name": "shell"}],
            "input": [
                {"role": "system", "content": [{"type": "input_text", "text": "SYSTEM"}]},
                {"role": "user", "content": [{"type": "input_text", "text": "tell me about planner pushdown"}]}
            ]
        });
        let body = serde_json::to_vec(&original).unwrap();
        let source = VecMemorySource::new(vec![("planner", "planner.rs pushdown")]);
        let out: Value =
            serde_json::from_slice(&inject_openai_responses_memory(&body, &source, 5)).unwrap();

        assert_eq!(out["tools"], original["tools"]);
        assert_eq!(out["input"][0], original["input"][0]);
        let last = serde_json::to_string(&out["input"][1]).unwrap();
        assert!(last.contains("tell me about planner"));
        assert!(last.contains("planner.rs"));
        assert!(last.contains("theorem-memory"));
    }

    #[test]
    fn openai_responses_memory_fails_open_when_no_relevant_hit() {
        let body = serde_json::to_vec(&json!({
            "model": "gpt-5.5",
            "input": [{"role": "user", "content": "hello"}]
        }))
        .unwrap();
        let source = VecMemorySource::new(vec![("planner", "planner.rs pushdown")]);
        assert_eq!(inject_openai_responses_memory(&body, &source, 5), body);
    }
}
