//! RustyRed HTTP client.
//!
//! This is the contract seam. Every route + body shape here is pinned to the
//! real handlers in `rustyredcore_THG/crates/rustyred-thg-server/src/router.rs`:
//!
//!   POST /v1/tenants/{t}/graph/nodes        NodeWriteBody  -> {ok, node}
//!   POST /v1/tenants/{t}/graph/bulk/nodes   JSONL body     -> {ok, inserted, ...}
//!   POST /v1/tenants/{t}/graph/edges        EdgeWriteBody  -> {ok, edge}
//!   POST /v1/tenants/{t}/graph/bulk/edges   JSONL body     -> {ok, inserted, ...}
//!   POST /v1/tenants/{t}/graph/nodes/query  NodeQuery      -> {ok, nodes}
//!   GET  /v1/tenants/{t}/graph/nodes/{id}                  -> {ok, node}
//!   POST /v1/tenants/{t}/graph/vector/designate            -> {ok, ...}
//!   POST /v1/tenants/{t}/graph/vector/search VectorSearch  -> {ok, results:[{node_id,distance,node}]}
//!   POST /v1/tenants/{t}/graph/algorithms/ppr PprBody      -> {ok, scores:[{node_id,score}]}
//!   POST /v1/tenants/{t}/graph/algorithms/pagerank        -> {ok, scores:[{node_id,score}]}
//!   POST /v1/tenants/{t}/context/pack       {artifact_id,sections,token_ledger}
//!
//! Auth: `Authorization: Bearer <token>` (see server auth.rs::authenticate).

use std::collections::HashMap;

use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::Config;
use crate::error::{JobIntelError, Result};

/// A node to write, matching the server `NodeWriteBody` shape.
#[derive(Debug, Clone, Serialize)]
pub struct NodeSpec {
    pub id: String,
    pub labels: Vec<String>,
    pub properties: Value,
}

/// An edge to write, matching the server `EdgeWriteBody` shape (`type` is the
/// wire key; `edge_type` is the Rust field).
#[derive(Debug, Clone, Serialize)]
pub struct EdgeSpec {
    pub id: String,
    pub from_id: String,
    pub to_id: String,
    #[serde(rename = "type")]
    pub edge_type: String,
    pub properties: Value,
}

/// One `(node_id, score)` row from a PPR / PageRank response.
#[derive(Debug, Clone, Deserialize)]
pub struct ScoreRow {
    pub node_id: String,
    pub score: f64,
}

/// One vector-search hit: the matched node id, its distance, and the node body.
#[derive(Debug, Clone, Deserialize)]
pub struct VectorHit {
    pub node_id: String,
    pub distance: f32,
    /// The full node, when the server inlines it. jobintel reads roles via a
    /// separate query, so this is retained only to document the response shape.
    #[serde(default)]
    #[allow(dead_code)]
    pub node: Option<Value>,
}

#[derive(Deserialize)]
struct ScoresEnvelope {
    #[serde(default)]
    scores: Vec<ScoreRow>,
}

#[derive(Deserialize)]
struct VectorEnvelope {
    #[serde(default)]
    results: Vec<VectorHit>,
}

#[derive(Deserialize)]
struct NodesEnvelope {
    #[serde(default)]
    nodes: Vec<Value>,
}

#[derive(Deserialize)]
struct NodeEnvelope {
    #[serde(default)]
    node: Option<Value>,
}

#[derive(Deserialize)]
struct BulkEnvelope {
    #[serde(default)]
    pub inserted: usize,
    #[serde(default)]
    pub failed: usize,
    #[serde(default)]
    pub errors: Vec<Value>,
}

pub struct RustyRedClient {
    http: Client,
    base: String,
    tenant: String,
    token: String,
}

impl RustyRedClient {
    pub fn new(config: &Config) -> Result<Self> {
        let http = Client::builder().user_agent("jobintel/0.1").build()?;
        Ok(Self {
            http,
            base: config.rustyred_url.clone(),
            tenant: config.tenant.clone(),
            token: config.token.clone(),
        })
    }

    fn url(&self, suffix: &str) -> String {
        format!("{}/v1/tenants/{}/{}", self.base, self.tenant, suffix)
    }

    /// POST a JSON body and deserialize the response into `T`. Centralizes the
    /// bearer header and non-2xx -> typed-error mapping.
    fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        route: &str,
        suffix: &str,
        body: &B,
    ) -> Result<T> {
        let resp = self
            .http
            .post(self.url(suffix))
            .bearer_auth(&self.token)
            .json(body)
            .send()?;
        self.read(route, resp)
    }

    /// POST a raw JSONL body (bulk loader path; not application/json).
    fn post_jsonl(&self, route: &str, suffix: &str, body: String) -> Result<BulkEnvelope> {
        let resp = self
            .http
            .post(self.url(suffix))
            .bearer_auth(&self.token)
            .header(reqwest::header::CONTENT_TYPE, "application/jsonl")
            .body(body)
            .send()?;
        self.read(route, resp)
    }

    fn read<T: DeserializeOwned>(
        &self,
        route: &str,
        resp: reqwest::blocking::Response,
    ) -> Result<T> {
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(JobIntelError::Rustyred {
                route: route.to_string(),
                status: status.as_u16(),
                body: truncate(&body, 400),
            });
        }
        Ok(resp.json::<T>()?)
    }

    // ---- writes ------------------------------------------------------------

    pub fn upsert_node(&self, node: &NodeSpec) -> Result<Value> {
        self.post_json("graph/nodes", "graph/nodes", node)
    }

    pub fn upsert_edge(&self, edge: &EdgeSpec) -> Result<Value> {
        self.post_json("graph/edges", "graph/edges", edge)
    }

    /// Bulk-write nodes via the streaming JSONL loader. Returns (inserted, failed).
    pub fn bulk_nodes(&self, nodes: &[NodeSpec]) -> Result<(usize, usize)> {
        if nodes.is_empty() {
            return Ok((0, 0));
        }
        let body = to_jsonl(nodes)?;
        let env = self.post_jsonl("graph/bulk/nodes", "graph/bulk/nodes", body)?;
        if !env.errors.is_empty() {
            eprintln!(
                "  bulk/nodes: {} inserted, {} failed (first error: {})",
                env.inserted,
                env.failed,
                env.errors
                    .first()
                    .map(|e| e.to_string())
                    .unwrap_or_default()
            );
        }
        Ok((env.inserted, env.failed))
    }

    /// Bulk-write edges via the streaming JSONL loader. Returns (inserted, failed).
    pub fn bulk_edges(&self, edges: &[EdgeSpec]) -> Result<(usize, usize)> {
        if edges.is_empty() {
            return Ok((0, 0));
        }
        let body = to_jsonl(edges)?;
        let env = self.post_jsonl("graph/bulk/edges", "graph/bulk/edges", body)?;
        Ok((env.inserted, env.failed))
    }

    pub fn designate_vector(&self, label: &str, property: &str, dimension: usize) -> Result<Value> {
        let body = json!({ "label": label, "property": property, "dimension": dimension });
        self.post_json("graph/vector/designate", "graph/vector/designate", &body)
    }

    // ---- reads / algorithms ------------------------------------------------

    pub fn vector_search(
        &self,
        query: &[f32],
        k: usize,
        label: Option<&str>,
        property: &str,
    ) -> Result<Vec<VectorHit>> {
        let body = json!({ "query": query, "k": k, "label": label, "property": property });
        let env: VectorEnvelope =
            self.post_json("graph/vector/search", "graph/vector/search", &body)?;
        Ok(env.results)
    }

    /// Personalized PageRank seeded on `seeds` (node_id -> seed weight).
    pub fn ppr(&self, seeds: &HashMap<String, f64>, top_k: Option<usize>) -> Result<Vec<ScoreRow>> {
        let body = json!({ "seeds": seeds, "top_k": top_k });
        let env: ScoresEnvelope =
            self.post_json("graph/algorithms/ppr", "graph/algorithms/ppr", &body)?;
        Ok(env.scores)
    }

    /// Whole-graph PageRank (hiring-spike proxy). Takes no seeds.
    pub fn pagerank(&self, top_k: Option<usize>) -> Result<Vec<ScoreRow>> {
        let body = json!({ "top_k": top_k });
        let env: ScoresEnvelope = self.post_json(
            "graph/algorithms/pagerank",
            "graph/algorithms/pagerank",
            &body,
        )?;
        Ok(env.scores)
    }

    /// Read all nodes carrying `label` via the nodes/query route.
    pub fn query_nodes(&self, label: &str, limit: Option<usize>) -> Result<Vec<Value>> {
        let body = json!({ "label": label, "limit": limit });
        let env: NodesEnvelope = self.post_json("graph/nodes/query", "graph/nodes/query", &body)?;
        Ok(env.nodes)
    }

    /// Fetch a single node by id (`GET /graph/nodes/{id}` -> `{ok, node}`).
    /// Returns `None` on 404. This is the read half of the outreach state
    /// machine's read-modify-write: RustyRed's upsert REPLACES a node wholesale
    /// (it does not merge properties), so a status change must re-send the node's
    /// full property map, not just the changed keys.
    pub fn get_node(&self, id: &str) -> Result<Option<Value>> {
        let suffix = format!("graph/nodes/{}", urlencode_path(id));
        let resp = self
            .http
            .get(self.url(&suffix))
            .bearer_auth(&self.token)
            .send()?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let env: NodeEnvelope = self.read("graph/nodes/{id}", resp)?;
        Ok(env.node)
    }

    /// Store a context pack. Returns the server echo `{artifact_id, sections, ...}`.
    pub fn context_pack(
        &self,
        artifact_id: &str,
        sections: Value,
        token_ledger: Value,
    ) -> Result<Value> {
        let body = json!({
            "artifact_id": artifact_id,
            "sections": sections,
            "token_ledger": token_ledger,
        });
        self.post_json("context/pack", "context/pack", &body)
    }
}

/// Serialize a slice into newline-delimited JSON (one object per line). Pure so
/// it is unit-tested without a live server. Trailing newline omitted; the
/// server's LineSplitter flushes the final partial line.
pub fn to_jsonl<T: Serialize>(items: &[T]) -> Result<String> {
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&serde_json::to_string(item)?);
    }
    Ok(out)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// Percent-encode a node id for use as a single URL path segment. Node ids look
/// like `role:hn:123` (the `:` is path-legal) but ATS source ids can carry
/// arbitrary characters, so anything outside the RFC 3986 unreserved set plus
/// `:` is encoded. `/` is always encoded so an id never splits the segment.
fn urlencode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        let keep = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b':');
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[allow(dead_code)]
fn status_is_auth_error(status: StatusCode) -> bool {
    matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_jsonl_emits_one_object_per_line() {
        let nodes = vec![
            NodeSpec {
                id: "company:qdrant".into(),
                labels: vec!["Company".into()],
                properties: json!({ "name": "Qdrant" }),
            },
            NodeSpec {
                id: "role:hn:1".into(),
                labels: vec!["Role".into()],
                properties: json!({ "title": "Rust Engineer" }),
            },
        ];
        let jsonl = to_jsonl(&nodes).unwrap();
        let lines: Vec<&str> = jsonl.split('\n').collect();
        assert_eq!(lines.len(), 2);
        // Each line must independently parse as a node object with a string id.
        for line in lines {
            let v: Value = serde_json::from_str(line).unwrap();
            assert!(v.get("id").and_then(Value::as_str).is_some());
            assert!(v.get("labels").and_then(Value::as_array).is_some());
        }
    }

    #[test]
    fn edge_spec_serializes_type_key() {
        let edge = EdgeSpec {
            id: "edge:a|posts|b".into(),
            from_id: "a".into(),
            to_id: "b".into(),
            edge_type: "posts".into(),
            properties: json!({}),
        };
        let v = serde_json::to_value(&edge).unwrap();
        // Server EdgeWriteBody reads the wire key `type`, not `edge_type`.
        assert_eq!(v.get("type").and_then(Value::as_str), Some("posts"));
        assert!(v.get("edge_type").is_none());
    }

    #[test]
    fn truncate_caps_long_bodies() {
        assert_eq!(truncate("short", 400), "short");
        let long = "x".repeat(500);
        assert_eq!(truncate(&long, 400).len(), 403); // 400 + "..."
    }

    #[test]
    fn urlencode_path_keeps_node_id_colons_but_escapes_slashes() {
        assert_eq!(urlencode_path("role:hn:123"), "role:hn:123");
        assert_eq!(urlencode_path("role:greenhouse:a/b"), "role:greenhouse:a%2Fb");
        assert_eq!(urlencode_path("a b"), "a%20b");
    }
}
