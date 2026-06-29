//! Native-tool membrane (SPEC-LOCAL-PROXY-MVP D2): sample/defer oversized tool_result
//! content so one giant tool output does not tax every turn.
//!
//! Cache-safe by construction: it truncates tool_result blocks in the LAST message only
//! -- never an earlier turn -- so the cached prefix (system, tools, prior messages) stays
//! byte-identical. The latest tool_result is exactly the block not yet in the prompt
//! cache, i.e. the one being charged this turn, so that is the one worth shrinking. The
//! full content is preserved out-of-band (returned to the caller to store and serve at
//! `GET /tool_result/{id}`); the inline stub keeps a head+tail sample so the model still
//! sees the gist.
//!
//! ponytail: model-facing auto-fetch (wiring the stub's id to an MCP `tool_result_fetch`
//! the model can call) is the named follow-up; today the full output is retrievable
//! out-of-band over HTTP and the membrane is opt-in (threshold 0 = off), so nothing is
//! lost or surprising by default.

use serde_json::Value;

/// Truncate oversized tool_result content in the last message. `threshold` is the max
/// content length kept inline (0 disables the membrane). Returns the new body plus
/// `(id, full_content)` pairs for every elided block, for the caller to store and serve.
pub fn apply_membrane(body: &[u8], threshold: usize) -> (Vec<u8>, Vec<(String, String)>) {
    if threshold == 0 {
        return (body.to_vec(), Vec::new());
    }
    let Ok(mut request) = serde_json::from_slice::<Value>(body) else {
        return (body.to_vec(), Vec::new());
    };
    let mut stored = Vec::new();
    let Some(last) = request
        .get_mut("messages")
        .and_then(Value::as_array_mut)
        .and_then(|messages| messages.last_mut())
    else {
        return (body.to_vec(), Vec::new());
    };
    if let Some(blocks) = last.get_mut("content").and_then(Value::as_array_mut) {
        for block in blocks.iter_mut() {
            if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                truncate_tool_result(block, threshold, &mut stored);
            }
        }
    }
    (
        serde_json::to_vec(&request).unwrap_or_else(|_| body.to_vec()),
        stored,
    )
}

fn truncate_tool_result(block: &mut Value, threshold: usize, stored: &mut Vec<(String, String)>) {
    let full = match block.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return,
    };
    if full.len() <= threshold {
        return;
    }
    let id = content_id(&full);
    let half = threshold / 2;
    let head = safe_prefix(&full, half);
    let tail = safe_suffix(&full, half);
    let elided = full.len() - head.len() - tail.len();
    let stub = format!(
        "{head}\n[theorem-membrane: elided {elided} bytes; full tool output at GET /tool_result/{id}]\n{tail}"
    );
    block["content"] = Value::String(stub);
    stored.push((id, full));
}

/// Session-scoped retrieval id. Not cryptographic: the proxy stores and serves within one
/// run, so a fast in-process hash is sufficient (and avoids a new dependency).
fn content_id(text: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn safe_prefix(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn safe_suffix(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body_with_tool_result(content: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "model": "claude",
            "messages": [
                {"role": "user", "content": "go"},
                {"role": "assistant", "content": [{"type": "tool_use", "id": "t1", "name": "Read", "input": {}}]},
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": content}]}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn truncates_oversized_tool_result_and_returns_full_content() {
        let big = "X".repeat(5000);
        let (out, stored) = apply_membrane(&body_with_tool_result(&big), 1000);
        assert_eq!(stored.len(), 1, "one block stored");
        assert_eq!(stored[0].1, big, "full content preserved for retrieval");
        let value: Value = serde_json::from_slice(&out).unwrap();
        let new = value["messages"][2]["content"][0]["content"]
            .as_str()
            .unwrap();
        assert!(new.len() < big.len(), "inline content shrank");
        assert!(new.contains("theorem-membrane"), "stub marker present");
        assert!(
            new.contains(&stored[0].0),
            "stub references the retrieval id"
        );
    }

    #[test]
    fn leaves_small_results_untouched() {
        let original = body_with_tool_result("ok");
        let (out, stored) = apply_membrane(&original, 1000);
        assert!(stored.is_empty());
        assert_eq!(out, original, "small result: body unchanged");
    }

    #[test]
    fn disabled_when_threshold_zero() {
        let original = body_with_tool_result(&"X".repeat(5000));
        let (out, stored) = apply_membrane(&original, 0);
        assert!(stored.is_empty());
        assert_eq!(out, original);
    }

    #[test]
    fn only_touches_the_last_message_so_the_cached_prefix_is_safe() {
        // A big tool_result in an EARLIER message (already in the cached prefix) must not
        // be touched -- changing it would break the provider prompt cache.
        let big = "Y".repeat(5000);
        let original = serde_json::to_vec(&json!({
            "model": "claude",
            "messages": [
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "old", "content": big}]},
                {"role": "user", "content": "now a normal turn"}
            ]
        }))
        .unwrap();
        let (out, stored) = apply_membrane(&original, 1000);
        assert!(stored.is_empty(), "earlier-turn tool_result left alone");
        assert_eq!(out, original);
    }
}
