//! Remaining-cluster domains (A6): the in-crate harness operations clusters as
//! typed GraphQL fields -- ensemble (capability packs), skills (skill packs),
//! jobs (the dispatch board), and harness-run. Each field lowers to the existing
//! flat-tool payload through a per-cluster operation-param invoker method
//! (`skill` / `ensemble` / `job`) or the single `harness_run` method; no cluster
//! logic is reimplemented.
//!
//! Note: the web / browse / fractal-expansion clusters are server-only (their
//! flat arms punt to the async product server; there is no in-crate payload), so
//! they are intentionally NOT fields here -- wrapping them belongs to the
//! server-domain slice, like `hippoRetrieve`.

use async_graphql::{Object, Result as GqlResult};
use serde_json::{json, Value};

use super::scalars::Json;
use super::{map_err, with_invoker};

/// Merge optional typed key/values into a fresh args object (only set ones).
fn args_from(pairs: Vec<(&str, Option<Value>)>) -> Value {
    let mut args = json!({});
    let obj = args.as_object_mut().expect("json object");
    for (key, value) in pairs {
        if let Some(value) = value {
            obj.insert(key.to_string(), value);
        }
    }
    args
}

#[derive(Default)]
pub struct ClustersQuery;

#[Object]
impl ClustersQuery {
    /// Fetch a harness run's detail by id (wraps `harness_run`).
    async fn harness_run(&self, run_id: String) -> GqlResult<Json> {
        let args = json!({ "run_id": run_id });
        with_invoker(|inv| Ok(Json(inv.harness_run(args).map_err(map_err)?)))
    }

    /// List skill packs (wraps `skill_list`).
    async fn skill_list(
        &self,
        status: Option<String>,
        include_retired: Option<bool>,
    ) -> GqlResult<Json> {
        let args = args_from(vec![
            ("status", status.map(Value::from)),
            ("include_retired", include_retired.map(Value::from)),
        ]);
        with_invoker(|inv| Ok(Json(inv.skill("list", args).map_err(map_err)?)))
    }

    /// Fetch a single skill pack (wraps `skill_get`). `input` carries the
    /// selector the tool expects (id / name / slug / version).
    async fn skill_get(&self, input: Json) -> GqlResult<Json> {
        with_invoker(|inv| Ok(Json(inv.skill("get", input.0).map_err(map_err)?)))
    }

    /// Select a capability pack for a task under budget (wraps `ensemble_select`).
    /// `input` carries the task/query plus budget and trust knobs.
    async fn ensemble_select(&self, input: Json) -> GqlResult<Json> {
        with_invoker(|inv| Ok(Json(inv.ensemble("select", input.0).map_err(map_err)?)))
    }

    /// List dispatch-board jobs (wraps `job_list`).
    async fn job_list(&self, repo: Option<String>, state: Option<String>) -> GqlResult<Json> {
        let args = args_from(vec![
            ("repo", repo.map(Value::from)),
            ("state", state.map(Value::from)),
        ]);
        with_invoker(|inv| Ok(Json(inv.job("list", args).map_err(map_err)?)))
    }
}

#[derive(Default)]
pub struct ClustersMutation;

#[Object]
impl ClustersMutation {
    /// Publish a skill pack (wraps `skill_publish`). `input` carries the pack
    /// payload the tool expects.
    async fn skill_publish(&self, input: Json) -> GqlResult<Json> {
        with_invoker(|inv| Ok(Json(inv.skill("publish", input.0).map_err(map_err)?)))
    }

    /// Apply (record use of) a skill pack (wraps `skill_apply`).
    async fn skill_apply(&self, input: Json) -> GqlResult<Json> {
        with_invoker(|inv| Ok(Json(inv.skill("apply", input.0).map_err(map_err)?)))
    }

    /// Register a capability pack in the ensemble registry (wraps
    /// `ensemble_register`). `input` carries the pack plus content/artifact hashes.
    async fn ensemble_register(&self, input: Json) -> GqlResult<Json> {
        with_invoker(|inv| Ok(Json(inv.ensemble("register", input.0).map_err(map_err)?)))
    }

    /// Submit a job to the dispatch board (wraps `job_submit`). `input` carries
    /// the job submission (spec_ref / spec_inline, priority, target head, etc.).
    async fn job_submit(&self, input: Json) -> GqlResult<Json> {
        with_invoker(|inv| Ok(Json(inv.job("submit", input.0).map_err(map_err)?)))
    }

    /// Append a note/receipt to a job (wraps `job_note`).
    async fn job_note(
        &self,
        job_id: String,
        text: String,
        actor: Option<String>,
        refs: Option<Vec<String>>,
    ) -> GqlResult<Json> {
        let args = args_from(vec![
            ("job_id", Some(Value::from(job_id))),
            ("text", Some(Value::from(text))),
            ("actor", actor.map(Value::from)),
            ("refs", refs.map(|r| json!(r))),
        ]);
        with_invoker(|inv| Ok(Json(inv.job("note", args).map_err(map_err)?)))
    }

    /// Archive a job with a reason (wraps `job_archive`).
    async fn job_archive(
        &self,
        job_id: String,
        reason: String,
        actor: Option<String>,
    ) -> GqlResult<Json> {
        let args = args_from(vec![
            ("job_id", Some(Value::from(job_id))),
            ("reason", Some(Value::from(reason))),
            ("actor", actor.map(Value::from)),
        ]);
        with_invoker(|inv| Ok(Json(inv.job("archive", args).map_err(map_err)?)))
    }
}
