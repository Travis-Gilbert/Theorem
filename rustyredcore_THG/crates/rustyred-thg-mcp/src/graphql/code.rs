//! Code domain (A5): the typed surface over the CodeCrawler `compute_code` tool.
//! Each field lowers to `code_search_payload` with a fixed `operation` (the same
//! handler the flat `compute_code` / `code_ingest` tools call), so a code query
//! returns exactly what the matching `compute_code` operation returns. Reads are
//! `CodeQuery`; ingest/reindex writes are `CodeMutation` (so they are reachable
//! only through `graphql_mutate`, which is read-only-gated at the transport).
//!
//! Note: the harness instant-KG fields (`harness_kg_*`) are a distinct
//! sub-domain (the instant-KG `view`) and are wrapped in their own follow-on
//! slice; this module is the CodeCrawler half of A5.

use async_graphql::{Object, Result as GqlResult};
use serde_json::json;

use super::scalars::Json;
use super::{map_err, with_invoker};

/// Build the compute_code argument object from the optional typed inputs. Only
/// set fields are inserted, so no spurious argument reaches an operation that
/// does not expect it. `repo` is the convenience key the payload normalizes into
/// repo_id / repo_url / repo_path.
fn code_args(
    query: Option<String>,
    repo: Option<String>,
    node_id: Option<String>,
    limit: Option<i32>,
    actor: Option<String>,
) -> serde_json::Value {
    let mut args = json!({});
    let obj = args.as_object_mut().expect("json object");
    if let Some(query) = query {
        obj.insert("query".to_string(), json!(query));
    }
    if let Some(repo) = repo {
        obj.insert("repo".to_string(), json!(repo));
    }
    if let Some(node_id) = node_id {
        obj.insert("node_id".to_string(), json!(node_id));
    }
    if let Some(limit) = limit {
        obj.insert("limit".to_string(), json!(limit));
    }
    if let Some(actor) = actor {
        obj.insert("actor".to_string(), json!(actor));
    }
    args
}

#[derive(Default)]
pub struct CodeQuery;

#[Object]
impl CodeQuery {
    /// Search the code graph (wraps `compute_code` operation `search`).
    async fn code_search(
        &self,
        query: String,
        repo: Option<String>,
        limit: Option<i32>,
    ) -> GqlResult<Json> {
        let args = code_args(Some(query), repo, None, limit, None);
        with_invoker(|inv| Ok(Json(inv.code("search", args.clone()).map_err(map_err)?)))
    }

    /// Assemble a context pack around a symbol or query (operation `context`).
    async fn code_context(
        &self,
        query: Option<String>,
        node_id: Option<String>,
        repo: Option<String>,
        limit: Option<i32>,
    ) -> GqlResult<Json> {
        let args = code_args(query, repo, node_id, limit, None);
        with_invoker(|inv| Ok(Json(inv.code("context", args.clone()).map_err(map_err)?)))
    }

    /// Explore the neighborhood of a code symbol (operation `explore`).
    async fn code_explore(
        &self,
        node_id: Option<String>,
        query: Option<String>,
        repo: Option<String>,
        limit: Option<i32>,
    ) -> GqlResult<Json> {
        let args = code_args(query, repo, node_id, limit, None);
        with_invoker(|inv| Ok(Json(inv.code("explore", args.clone()).map_err(map_err)?)))
    }

    /// Explain a code symbol or edge (operation `explain`).
    async fn code_explain(
        &self,
        node_id: Option<String>,
        query: Option<String>,
        repo: Option<String>,
    ) -> GqlResult<Json> {
        let args = code_args(query, repo, node_id, None, None);
        with_invoker(|inv| Ok(Json(inv.code("explain", args.clone()).map_err(map_err)?)))
    }

    /// Recognize a snippet against the indexed code (operation `recognize`).
    async fn code_recognize(&self, query: String, repo: Option<String>) -> GqlResult<Json> {
        let args = code_args(Some(query), repo, None, None, None);
        with_invoker(|inv| Ok(Json(inv.code("recognize", args.clone()).map_err(map_err)?)))
    }

    /// List the repositories ingested into the code graph (operation `list_repos`).
    async fn list_repos(&self) -> GqlResult<Json> {
        with_invoker(|inv| Ok(Json(inv.code("list_repos", json!({})).map_err(map_err)?)))
    }
}

#[derive(Default)]
pub struct CodeMutation;

#[Object]
impl CodeMutation {
    /// Ingest a codebase into the code graph (operation `ingest`). `repo` is a
    /// repo id, URL, or local path; the payload normalizes it.
    async fn ingest_codebase(&self, repo: String, actor: Option<String>) -> GqlResult<Json> {
        let args = code_args(None, Some(repo), None, None, actor);
        with_invoker(|inv| Ok(Json(inv.code("ingest", args.clone()).map_err(map_err)?)))
    }

    /// Re-index a codebase incrementally (operation `reindex`).
    async fn reindex_codebase(&self, repo: String, actor: Option<String>) -> GqlResult<Json> {
        let args = code_args(None, Some(repo), None, None, actor);
        with_invoker(|inv| Ok(Json(inv.code("reindex", args.clone()).map_err(map_err)?)))
    }
}
