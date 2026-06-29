//! Memory source seam for ambient injection (SPEC-LOCAL-PROXY-MVP D3 +
//! SPEC-PROXY-PROVE-AND-PRUNE D1: relevance-ranked, not wholesale).
//!
//! The proxy retrieves over a `MemorySource` and injects the top hits at the
//! cache-stable suffix. The default ranks by query token overlap -- a real, if
//! simple, relevance signal. The substrate retrieval (`hippo_retrieve` /
//! index-context, with embeddings + PPR) is the production impl that plugs in behind
//! this trait without the proxy changing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

/// A ranked memory retrieval surface.
pub trait MemorySource: Send + Sync {
    /// Up to `limit` memories relevant to `query`, most relevant first.
    fn retrieve(&self, query: &str, limit: usize) -> Vec<MemoryHit>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct MemoryHit {
    pub title: String,
    pub body: String,
    pub score: f64,
}

/// Lowercased alphanumeric tokens of length >= 3 (drops trivial connective words
/// that would inflate overlap scores).
fn tokens(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.len() >= 3)
        .map(str::to_string)
        .collect()
}

/// Relevance: how many distinct query tokens appear in the memory text.
fn relevance(query_tokens: &BTreeSet<String>, memory_text: &str) -> f64 {
    let memory_tokens = tokens(memory_text);
    query_tokens
        .iter()
        .filter(|token| memory_tokens.contains(*token))
        .count() as f64
}

fn rank(items: impl Iterator<Item = MemoryHit>, query: &str, limit: usize) -> Vec<MemoryHit> {
    if limit == 0 {
        return Vec::new();
    }
    let query_tokens = tokens(query);
    if query_tokens.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<MemoryHit> = items
        .map(|mut hit| {
            hit.score = relevance(&query_tokens, &format!("{} {}", hit.title, hit.body));
            hit
        })
        .filter(|hit| hit.score > 0.0)
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.title.cmp(&b.title))
    });
    scored.truncate(limit);
    scored
}

/// In-memory source (tests, small static sets).
pub struct VecMemorySource {
    items: Vec<MemoryHit>,
}

impl VecMemorySource {
    pub fn new(items: Vec<(&str, &str)>) -> Self {
        Self {
            items: items
                .into_iter()
                .map(|(title, body)| MemoryHit {
                    title: title.to_string(),
                    body: body.to_string(),
                    score: 0.0,
                })
                .collect(),
        }
    }
}

impl MemorySource for VecMemorySource {
    fn retrieve(&self, query: &str, limit: usize) -> Vec<MemoryHit> {
        rank(self.items.iter().cloned(), query, limit)
    }
}

/// Directory source: one `*.md` file per memory (title = file stem, body = file
/// contents). The simplest durable real source; the substrate retrieval replaces it.
pub struct DirectoryMemorySource {
    dir: PathBuf,
}

impl DirectoryMemorySource {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
        }
    }

    fn load(&self) -> Vec<MemoryHit> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                continue;
            }
            let title = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string();
            if let Ok(body) = std::fs::read_to_string(&path) {
                out.push(MemoryHit {
                    title,
                    body: body.trim().to_string(),
                    score: 0.0,
                });
            }
        }
        out
    }
}

impl MemorySource for DirectoryMemorySource {
    fn retrieve(&self, query: &str, limit: usize) -> Vec<MemoryHit> {
        rank(self.load().into_iter(), query, limit)
    }
}

/// Substrate-backed source: the live local Theorem node's graph memory, reached over
/// its MCP endpoint (`hippo_retrieve` -- the spec's named retrieval path: query-specific
/// PPR over the memory graph). This is the production memory the proxy injects --
/// relevance-ranked by the substrate, not a static directory.
///
/// Fail open by construction: a node that is down, slow, or returns garbage yields no
/// hits, so the turn is forwarded unchanged. The agent's short read timeout guarantees
/// a hung node can never delay a model turn.
pub struct HttpMemorySource {
    /// The node's MCP endpoint, e.g. `http://127.0.0.1:8380/mcp`.
    endpoint: String,
    /// Optional tenant slug; omitted uses the node's default tenant.
    tenant: Option<String>,
    agent: ureq::Agent,
}

impl HttpMemorySource {
    pub fn new(endpoint: impl Into<String>, tenant: Option<String>) -> Self {
        // Node retrieval over a real corpus (PPR + a multi-KB response) takes seconds, not
        // milliseconds, so the read timeout is generous; it still bounds a hung node. The
        // fast, low-latency ambient path is `DirectoryMemorySource` -- this HTTP source is
        // opt-in for graph retrieval.
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_millis(300))
            .timeout_read(Duration::from_millis(3000))
            .build();
        Self {
            endpoint: endpoint.into(),
            tenant,
            agent,
        }
    }

    /// JSON-RPC `tools/call` for `hippo_retrieve` against the node. `None` on any
    /// transport/parse failure (the caller maps that to no hits -> passthrough).
    fn query(&self, query: &str, limit: usize) -> Option<Vec<MemoryHit>> {
        if limit == 0 {
            return Some(Vec::new());
        }
        // auto_index_memory=false keeps this a pure read: indexing the corpus re-scans
        // every memory and far exceeds the agent's read timeout, so the hot path must
        // query the already-warm index. Building/warming the index is the seed step's job
        // (scripts/seed-node.py), not the per-turn retrieval's.
        let mut arguments = json!({ "query": query, "top_k": limit, "auto_index_memory": false });
        if let Some(tenant) = &self.tenant {
            arguments["tenant"] = json!(tenant);
        }
        let request = json!({
            "jsonrpc": "2.0",
            "id": "theorem-proxy-recall",
            "method": "tools/call",
            "params": { "name": "hippo_retrieve", "arguments": arguments },
        });
        let body = serde_json::to_string(&request).ok()?;
        let response = self
            .agent
            .post(&self.endpoint)
            .set("content-type", "application/json")
            .send_string(&body)
            .ok()?;
        let text = response.into_string().ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        Some(parse_candidates(&value, limit))
    }
}

impl MemorySource for HttpMemorySource {
    fn retrieve(&self, query: &str, limit: usize) -> Vec<MemoryHit> {
        if query.trim().is_empty() || limit == 0 {
            return Vec::new();
        }
        self.query(query, limit).unwrap_or_default()
    }
}

/// Map a `hippo_retrieve` JSON-RPC response to ranked hits. The real MCP `tools/call`
/// envelope puts the payload under `result.structuredContent`; older/flat shapes put it
/// at `result.candidates` or the top level. All are tolerated. A candidate without text
/// is skipped.
fn parse_candidates(value: &Value, limit: usize) -> Vec<MemoryHit> {
    if limit == 0 {
        return Vec::new();
    }
    let result = value.get("result");
    let candidates = result
        .and_then(|result| result.get("structuredContent"))
        .and_then(|structured| structured.get("candidates"))
        .or_else(|| result.and_then(|result| result.get("candidates")))
        .or_else(|| value.get("candidates"))
        .and_then(Value::as_array);
    let Some(candidates) = candidates else {
        return Vec::new();
    };
    let mut hits: Vec<MemoryHit> = candidates.iter().filter_map(candidate_to_hit).collect();
    hits.truncate(limit);
    hits
}

fn candidate_to_hit(candidate: &Value) -> Option<MemoryHit> {
    let body = candidate
        .get("text")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if body.is_empty() {
        return None;
    }
    let title = candidate
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or("memory")
        .to_string();
    let score = candidate
        .get("ppr_proximity")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    Some(MemoryHit { title, body, score })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_relevant_above_irrelevant_and_drops_zero() {
        let source = VecMemorySource::new(vec![
            (
                "planner",
                "the planner lives in planner.rs and does boolean pushdown",
            ),
            ("cats", "cats are nice"),
        ]);
        let hits = source.retrieve("tell me about the planner pushdown", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "planner");
    }

    #[test]
    fn empty_query_returns_nothing() {
        let source = VecMemorySource::new(vec![("a", "b")]);
        assert!(source.retrieve("", 5).is_empty());
    }

    #[test]
    fn zero_limit_returns_nothing() {
        let source = VecMemorySource::new(vec![("planner", "planner pushdown")]);
        assert!(source
            .retrieve("tell me about planner pushdown", 0)
            .is_empty());
        let value = json!({"result": {"candidates": [{"node_id": "mem:1", "text": "hit"}]}});
        assert!(parse_candidates(&value, 0).is_empty());
    }

    #[test]
    fn parses_hippo_retrieve_candidates_nested_under_result() {
        let value = json!({
            "result": {
                "candidates": [
                    {"node_id": "mem:1", "text": "planner.rs does boolean pushdown", "ppr_proximity": 0.9},
                    {"node_id": "mem:2", "text": "  ", "ppr_proximity": 0.5},
                    {"node_id": "mem:3", "ppr_proximity": 0.4}
                ]
            }
        });
        let hits = parse_candidates(&value, 5);
        assert_eq!(
            hits.len(),
            1,
            "empty-text and text-less candidates are skipped"
        );
        assert_eq!(hits[0].title, "mem:1");
        assert!(hits[0].body.contains("pushdown"));
        assert_eq!(hits[0].score, 0.9);
    }

    #[test]
    fn parses_real_mcp_structured_content_envelope() {
        // The shape rustyred-thg-server /mcp actually returns: payload under
        // result.structuredContent (caught against the live node, 2026-06-28).
        let value = json!({
            "result": {
                "structuredContent": {
                    "candidates": [
                        {"node_id": "hippo:page:memory:sha256:abc", "text": "live node hit", "ppr_proximity": 0.7}
                    ]
                }
            }
        });
        let hits = parse_candidates(&value, 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "hippo:page:memory:sha256:abc");
        assert!(hits[0].body.contains("live node hit"));
        assert_eq!(hits[0].score, 0.7);
    }

    #[test]
    fn garbage_response_yields_no_hits() {
        assert!(parse_candidates(&json!({"error": "boom"}), 5).is_empty());
        assert!(parse_candidates(&json!("not even an object"), 5).is_empty());
    }
}
