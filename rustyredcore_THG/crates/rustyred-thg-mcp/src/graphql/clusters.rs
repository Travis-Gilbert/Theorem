//! Remaining-cluster domains (A6): the in-crate harness operations clusters as
//! TYPED GraphQL fields -- ensemble (capability packs), skills (skill packs),
//! jobs (the dispatch board), and harness-run. Each field lowers to the existing
//! flat-tool payload through a per-cluster operation-param invoker method
//! (`skill` / `ensemble` / `job`) or the single `harness_run` method; no cluster
//! logic is reimplemented.
//!
//! Each domain is modeled as a typed object with a defensive `from_value`
//! constructor, the same quality bar as `memory::MemoryDoc`: the constructor
//! reads across the live payload shapes (snake_case from the runtime types, with
//! camelCase fallbacks) and tolerates missing fields rather than nulling the
//! whole object. The connection tenant is never a field -- it is fixed on the
//! invoker, and (by the cluster-domain SDL contract) no typed field carries a
//! `tenant`-named property.
//!
//! Note: the web / browse / fractal-expansion clusters are server-only (their
//! flat arms punt to the async product server; there is no in-crate payload), so
//! they are intentionally NOT fields here -- wrapping them belongs to the
//! server-domain slice, like `hippoRetrieve`. See the coverage map in
//! `graphql/mod.rs`.

use async_graphql::{Object, Result as GqlResult, SimpleObject};
use serde_json::{json, Value};

use super::scalars::Json;
use super::{map_err, with_invoker};

// ---------------------------------------------------------------------------
// Defensive field extraction across the live cluster payload `Value` shapes.
// ---------------------------------------------------------------------------

/// The first present value among `keys`.
fn pick<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| value.get(*key))
}

/// A non-empty string at any of `keys`.
fn s(value: &Value, keys: &[&str]) -> Option<String> {
    pick(value, keys)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|found| !found.is_empty())
}

/// A string at any of `keys`, or empty.
fn s_or(value: &Value, keys: &[&str]) -> String {
    s(value, keys).unwrap_or_default()
}

fn b(value: &Value, keys: &[&str]) -> Option<bool> {
    pick(value, keys).and_then(Value::as_bool)
}

fn f(value: &Value, keys: &[&str]) -> Option<f64> {
    pick(value, keys).and_then(Value::as_f64)
}

fn u(value: &Value, keys: &[&str]) -> Option<u64> {
    pick(value, keys).and_then(Value::as_u64)
}

fn arr_s(value: &Value, keys: &[&str]) -> Vec<String> {
    pick(value, keys)
        .and_then(Value::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The array at any of `keys` (the elements, by reference).
fn arr<'a>(value: &'a Value, keys: &[&str]) -> Vec<&'a Value> {
    pick(value, keys)
        .and_then(Value::as_array)
        .map(|array| array.iter().collect())
        .unwrap_or_default()
}

/// The free-form JSON value at `key` (Null when absent), as the GraphQL scalar.
fn json_at(value: &Value, key: &str) -> Json {
    Json(value.get(key).cloned().unwrap_or(Value::Null))
}

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

// ---------------------------------------------------------------------------
// Skills domain: skill packs.
// ---------------------------------------------------------------------------

/// A skill pack, modeling the live `SkillPackState` shape. The active scope is
/// implicit on the invoker, so no scope-identifying field is exposed.
#[derive(Clone, SimpleObject)]
pub struct SkillPack {
    pub pack_id: String,
    pub pack_content_hash: String,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub source_content_hash: Option<String>,
    pub artifact_hashes: Vec<String>,
    pub published_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// The full pack body/instructions, as advertised by the flat skill tool. A
    /// GraphQL-only caller (flat tools hidden under graphql_default_surface) must
    /// still be able to read the contract it selected.
    pub pack: Json,
    /// The pack's validator contract.
    pub validators: Json,
    /// Declared artifacts payload.
    pub artifacts: Json,
    /// Free-form pack metadata.
    pub metadata: Json,
}

impl SkillPack {
    fn from_value(value: &Value) -> SkillPack {
        SkillPack {
            pack_id: s_or(value, &["pack_id", "packId"]),
            pack_content_hash: s_or(value, &["pack_content_hash", "packContentHash"]),
            kind: s(value, &["kind"]).unwrap_or_else(|| "skill_pack".to_string()),
            status: s_or(value, &["status"]),
            title: s_or(value, &["title", "name"]),
            description: s_or(value, &["description", "summary"]),
            capabilities: arr_s(value, &["capabilities"]),
            source_content_hash: s(value, &["source_content_hash", "sourceContentHash"]),
            artifact_hashes: arr_s(value, &["artifact_hashes", "artifactHashes"]),
            published_by: s(value, &["published_by", "publishedBy"]),
            created_at: s_or(value, &["created_at", "createdAt"]),
            updated_at: s_or(value, &["updated_at", "updatedAt"]),
            pack: json_at(value, "pack"),
            validators: json_at(value, "validators"),
            artifacts: json_at(value, "artifacts"),
            metadata: json_at(value, "metadata"),
        }
    }
}

/// The receipt returned by recording a skill-pack application.
#[derive(Clone, SimpleObject)]
pub struct SkillApplyReceipt {
    pub receipt_id: String,
    pub pack_id: String,
    pub pack_content_hash: String,
    pub status: String,
    pub validator_execution_mode: Option<String>,
    pub task: Option<String>,
    pub created_at: String,
}

impl SkillApplyReceipt {
    fn from_value(value: &Value) -> SkillApplyReceipt {
        SkillApplyReceipt {
            receipt_id: s_or(value, &["receipt_id", "receiptId"]),
            pack_id: s_or(value, &["pack_id", "packId"]),
            pack_content_hash: s_or(value, &["pack_content_hash", "packContentHash"]),
            status: s_or(value, &["status"]),
            validator_execution_mode: s(
                value,
                &["validator_execution_mode", "validatorExecutionMode"],
            ),
            task: s(value, &["task"]),
            created_at: s_or(value, &["created_at", "createdAt"]),
        }
    }
}

// ---------------------------------------------------------------------------
// Ensemble domain: capability packs + the replayable selection decision.
// ---------------------------------------------------------------------------

/// A capability pack as registered in the ensemble registry.
#[derive(Clone, SimpleObject)]
pub struct EnsemblePack {
    pub node_id: Option<String>,
    pub pack_content_hash: String,
    pub kind: Option<String>,
}

impl EnsemblePack {
    /// Build from the register envelope `{ node_id, pack }`.
    fn from_envelope(envelope: &Value) -> EnsemblePack {
        let pack = envelope.get("pack").cloned().unwrap_or(Value::Null);
        EnsemblePack {
            node_id: s(envelope, &["node_id", "nodeId"]),
            pack_content_hash: s_or(&pack, &["pack_content_hash", "packContentHash"]),
            kind: s(&pack, &["kind"]),
        }
    }
}

/// A capability the selector chose for a task.
#[derive(Clone, SimpleObject)]
pub struct SelectedCapability {
    pub kind: String,
    pub pack_content_hash: String,
    pub reason: Option<String>,
    pub score: Option<f64>,
    pub cost_units: Option<u64>,
}

impl SelectedCapability {
    fn from_value(value: &Value) -> SelectedCapability {
        SelectedCapability {
            kind: s_or(value, &["kind"]),
            pack_content_hash: s_or(value, &["pack_content_hash", "packContentHash"]),
            reason: s(value, &["reason"]),
            score: f(value, &["score"]),
            cost_units: u(value, &["cost_units", "costUnits"]),
        }
    }
}

/// A candidate the selector considered and rejected, with the reason.
#[derive(Clone, SimpleObject)]
pub struct RejectedCandidate {
    pub kind: String,
    pub pack_content_hash: String,
    pub reason: String,
}

impl RejectedCandidate {
    fn from_value(value: &Value) -> RejectedCandidate {
        RejectedCandidate {
            kind: s_or(value, &["kind"]),
            pack_content_hash: s_or(value, &["pack_content_hash", "packContentHash"]),
            reason: s_or(value, &["reason"]),
        }
    }
}

/// The replayable ensemble decision, with its content address and the selected /
/// rejected capabilities resolved as nested typed objects.
#[derive(Clone, SimpleObject)]
pub struct EnsembleSelection {
    pub decision_content_hash: Option<String>,
    pub task: String,
    pub budget_units: Option<u64>,
    pub spent_units: u64,
    pub risk: String,
    pub selected: Vec<SelectedCapability>,
    pub rejected: Vec<RejectedCandidate>,
}

impl EnsembleSelection {
    /// Build from the select envelope `{ decision_content_hash, decision }`.
    fn from_envelope(envelope: &Value) -> EnsembleSelection {
        let decision = envelope.get("decision").cloned().unwrap_or(Value::Null);
        EnsembleSelection {
            decision_content_hash: s(
                envelope,
                &["decision_content_hash", "decisionContentHash"],
            ),
            task: s_or(&decision, &["task"]),
            budget_units: u(&decision, &["budget_units", "budgetUnits"]),
            spent_units: u(&decision, &["spent_units", "spentUnits"]).unwrap_or(0),
            risk: s_or(&decision, &["risk"]),
            selected: arr(&decision, &["selected"])
                .into_iter()
                .map(SelectedCapability::from_value)
                .collect(),
            rejected: arr(&decision, &["rejected"])
                .into_iter()
                .map(RejectedCandidate::from_value)
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Jobs domain: the dispatch board.
// ---------------------------------------------------------------------------

/// An append-only receipt on a job thread.
#[derive(Clone, SimpleObject)]
pub struct JobReceipt {
    pub actor: String,
    pub at: String,
    pub text: String,
    pub refs: Vec<String>,
}

impl JobReceipt {
    fn from_value(value: &Value) -> JobReceipt {
        JobReceipt {
            actor: s_or(value, &["actor"]),
            at: s_or(value, &["at"]),
            text: s_or(value, &["text"]),
            refs: arr_s(value, &["refs"]),
        }
    }
}

/// A dispatch job. `state` is the board's derived state (pending / started /
/// archived); the runtime injects it on list/action shapes, and we recompute it
/// from `started_at` / `archived_at` on the bare submit shape.
#[derive(Clone, SimpleObject)]
pub struct Job {
    pub job_id: String,
    pub title: String,
    pub repo: String,
    pub state: String,
    pub priority: String,
    pub target_head: String,
    pub spec_ref: Option<String>,
    pub spec_inline: Option<String>,
    pub submitted_by: String,
    pub submitted_at: String,
    pub started_at: Option<String>,
    pub session_ref: Option<String>,
    pub archived_at: Option<String>,
    pub archived_reason: Option<String>,
    pub idempotency_key: String,
    pub receipts: Vec<JobReceipt>,
}

impl Job {
    fn from_value(value: &Value) -> Job {
        Job {
            job_id: s_or(value, &["job_id", "jobId"]),
            title: s_or(value, &["title"]),
            repo: s_or(value, &["repo"]),
            state: s(value, &["state"]).unwrap_or_else(|| derived_state(value)),
            priority: s_or(value, &["priority"]),
            target_head: s_or(value, &["target_head", "targetHead"]),
            spec_ref: s(value, &["spec_ref", "specRef"]),
            spec_inline: s(value, &["spec_inline", "specInline"]),
            submitted_by: s_or(value, &["submitted_by", "submittedBy"]),
            submitted_at: s_or(value, &["submitted_at", "submittedAt"]),
            started_at: s(value, &["started_at", "startedAt"]),
            session_ref: s(value, &["session_ref", "sessionRef"]),
            archived_at: s(value, &["archived_at", "archivedAt"]),
            archived_reason: s(value, &["archived_reason", "archivedReason"]),
            idempotency_key: s_or(value, &["idempotency_key", "idempotencyKey"]),
            receipts: arr(value, &["receipts"])
                .into_iter()
                .map(JobReceipt::from_value)
                .collect(),
        }
    }
}

/// Mirror of `Job::derived_state`, for the submit shape that omits `state`.
fn derived_state(value: &Value) -> String {
    if s(value, &["archived_at", "archivedAt"]).is_some() {
        "archived".to_string()
    } else if s(value, &["started_at", "startedAt"]).is_some() {
        "started".to_string()
    } else {
        "pending".to_string()
    }
}

/// The result of a job action (note / archive): whether the job was found, the
/// action applied, a message, and the updated job.
#[derive(Clone, SimpleObject)]
pub struct JobActionResult {
    pub job_id: String,
    pub found: bool,
    pub applied: Option<bool>,
    pub message: Option<String>,
    pub job: Option<Job>,
}

impl JobActionResult {
    /// Build from the action `result` object.
    fn from_value(value: &Value) -> JobActionResult {
        JobActionResult {
            job_id: s_or(value, &["job_id", "jobId"]),
            found: b(value, &["found"]).unwrap_or(false),
            applied: b(value, &["applied"]),
            message: s(value, &["message"]),
            job: value
                .get("job")
                .filter(|job| !job.is_null())
                .map(Job::from_value),
        }
    }
}

// ---------------------------------------------------------------------------
// Harness-run domain: a run and its event ledger.
// ---------------------------------------------------------------------------

/// One event in a harness run's append-only ledger.
#[derive(Clone, SimpleObject)]
pub struct HarnessEvent {
    pub event_id: String,
    pub seq: u64,
    pub event_type: String,
    pub created_at: String,
    pub state_hash_after: Option<String>,
    /// The transition payload. Required to replay/audit a run through GraphQL
    /// when the flat harness_run tool is hidden under graphql_default_surface.
    pub payload: Json,
    pub state_hash_before: Option<String>,
    pub idempotency_key: Option<String>,
}

impl HarnessEvent {
    fn from_value(value: &Value) -> HarnessEvent {
        HarnessEvent {
            event_id: s_or(value, &["event_id", "eventId"]),
            seq: u(value, &["seq"]).unwrap_or(0),
            // `EventState` serializes the discriminant under `type`.
            event_type: s_or(value, &["type", "event_type", "eventType"]),
            created_at: s_or(value, &["created_at", "createdAt"]),
            state_hash_after: s(value, &["state_hash_after", "stateHashAfter"]),
            payload: json_at(value, "payload"),
            state_hash_before: s(value, &["state_hash_before", "stateHashBefore"]),
            idempotency_key: s(value, &["idempotency_key", "idempotencyKey"]),
        }
    }
}

/// A harness run with its nested event ledger.
#[derive(Clone, SimpleObject)]
pub struct HarnessRun {
    pub run_id: String,
    pub task: String,
    pub actor: String,
    pub status: String,
    pub task_signature: Option<String>,
    pub last_event_seq: u64,
    pub created_at: String,
    pub updated_at: String,
    pub events: Vec<HarnessEvent>,
}

impl HarnessRun {
    /// Build from the `harness_run` envelope `{ run_id, found, detail: { run, events } }`.
    /// Returns `None` when the run was not found (mirrors the flat tool's `found:false`).
    fn from_envelope(envelope: &Value) -> Option<HarnessRun> {
        let found = b(envelope, &["found"]).unwrap_or(false);
        let detail = envelope.get("detail");
        if !found || detail.map(Value::is_null).unwrap_or(true) {
            return None;
        }
        let detail = detail.expect("detail present");
        let run = detail.get("run").cloned().unwrap_or(Value::Null);
        Some(HarnessRun {
            run_id: s(&run, &["run_id", "runId"])
                .or_else(|| s(envelope, &["run_id", "runId"]))
                .unwrap_or_default(),
            task: s_or(&run, &["task"]),
            actor: s_or(&run, &["actor"]),
            status: s_or(&run, &["status"]),
            task_signature: s(&run, &["task_signature", "taskSignature"]),
            last_event_seq: u(&run, &["last_event_seq", "lastEventSeq"]).unwrap_or(0),
            created_at: s_or(&run, &["created_at", "createdAt"]),
            updated_at: s_or(&run, &["updated_at", "updatedAt"]),
            events: arr(detail, &["events"])
                .into_iter()
                .map(HarnessEvent::from_value)
                .collect(),
        })
    }
}

// ---------------------------------------------------------------------------
// Resolvers: each wraps the existing payload handler via the scoped invoker and
// shapes the result into the typed object above.
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct ClustersQuery;

#[Object]
impl ClustersQuery {
    /// Fetch a harness run's detail (run + event ledger) by id (wraps
    /// `harness_run`). Null when the run is not found.
    async fn harness_run(&self, run_id: String) -> GqlResult<Option<HarnessRun>> {
        let args = json!({ "run_id": run_id });
        with_invoker(|inv| {
            let value = inv.harness_run(args.clone()).map_err(map_err)?;
            Ok(HarnessRun::from_envelope(&value))
        })
    }

    /// List skill packs (wraps `skill_list`).
    async fn skill_list(
        &self,
        status: Option<String>,
        include_retired: Option<bool>,
    ) -> GqlResult<Vec<SkillPack>> {
        let args = args_from(vec![
            ("status", status.map(Value::from)),
            ("include_retired", include_retired.map(Value::from)),
        ]);
        with_invoker(|inv| {
            let value = inv.skill("list", args.clone()).map_err(map_err)?;
            Ok(arr(&value, &["packs"])
                .into_iter()
                .map(SkillPack::from_value)
                .collect())
        })
    }

    /// Fetch a single skill pack (wraps `skill_get`). `input` carries the
    /// selector the tool expects (id / name / slug / content hash).
    async fn skill_get(&self, input: Json) -> GqlResult<Option<SkillPack>> {
        with_invoker(|inv| {
            let value = inv.skill("get", input.0.clone()).map_err(map_err)?;
            Ok(value
                .get("pack")
                .filter(|pack| !pack.is_null())
                .map(SkillPack::from_value))
        })
    }

    /// Select a capability pack for a task under budget (wraps `ensemble_select`).
    /// `input` carries the task/query plus budget and trust knobs.
    async fn ensemble_select(&self, input: Json) -> GqlResult<EnsembleSelection> {
        with_invoker(|inv| {
            let value = inv.ensemble("select", input.0.clone()).map_err(map_err)?;
            Ok(EnsembleSelection::from_envelope(&value))
        })
    }

    /// List dispatch-board jobs (wraps `job_list`).
    async fn job_list(&self, repo: Option<String>, state: Option<String>) -> GqlResult<Vec<Job>> {
        let args = args_from(vec![
            ("repo", repo.map(Value::from)),
            ("state", state.map(Value::from)),
        ]);
        with_invoker(|inv| {
            let value = inv.job("list", args.clone()).map_err(map_err)?;
            let jobs = value
                .get("result")
                .map(|result| arr(result, &["jobs"]))
                .unwrap_or_default();
            Ok(jobs.into_iter().map(Job::from_value).collect())
        })
    }
}

#[derive(Default)]
pub struct ClustersMutation;

#[Object]
impl ClustersMutation {
    /// Publish a skill pack (wraps `skill_publish`); returns the published pack.
    /// `input` carries the pack payload the tool expects.
    async fn skill_publish(&self, input: Json) -> GqlResult<SkillPack> {
        with_invoker(|inv| {
            let value = inv.skill("publish", input.0.clone()).map_err(map_err)?;
            let pack = value
                .get("published")
                .and_then(|published| published.get("pack"))
                .cloned()
                .ok_or_else(|| async_graphql::Error::new("skill_publish returned no pack"))?;
            Ok(SkillPack::from_value(&pack))
        })
    }

    /// Apply (record use of) a skill pack (wraps `skill_apply`).
    async fn skill_apply(&self, input: Json) -> GqlResult<SkillApplyReceipt> {
        with_invoker(|inv| {
            let value = inv.skill("apply", input.0.clone()).map_err(map_err)?;
            let receipt = value
                .get("receipt")
                .cloned()
                .ok_or_else(|| async_graphql::Error::new("skill_apply returned no receipt"))?;
            Ok(SkillApplyReceipt::from_value(&receipt))
        })
    }

    /// Register a capability pack in the ensemble registry (wraps
    /// `ensemble_register`). `input` carries the pack plus content/artifact hashes.
    async fn ensemble_register(&self, input: Json) -> GqlResult<EnsemblePack> {
        with_invoker(|inv| {
            let value = inv
                .ensemble("register", input.0.clone())
                .map_err(map_err)?;
            Ok(EnsemblePack::from_envelope(&value))
        })
    }

    /// Submit a job to the dispatch board (wraps `job_submit`); returns the job.
    /// `input` carries the submission (spec_ref / spec_inline, priority, etc.).
    async fn job_submit(&self, input: Json) -> GqlResult<Job> {
        with_invoker(|inv| {
            let value = inv.job("submit", input.0.clone()).map_err(map_err)?;
            let job = value
                .get("result")
                .and_then(|result| result.get("job"))
                .cloned()
                .ok_or_else(|| async_graphql::Error::new("job_submit returned no job"))?;
            Ok(Job::from_value(&job))
        })
    }

    /// Append a note/receipt to a job (wraps `job_note`).
    async fn job_note(
        &self,
        job_id: String,
        text: String,
        actor: Option<String>,
        refs: Option<Vec<String>>,
    ) -> GqlResult<JobActionResult> {
        let args = args_from(vec![
            ("job_id", Some(Value::from(job_id))),
            ("text", Some(Value::from(text))),
            ("actor", actor.map(Value::from)),
            ("refs", refs.map(|refs| json!(refs))),
        ]);
        with_invoker(|inv| {
            let value = inv.job("note", args.clone()).map_err(map_err)?;
            let result = value.get("result").cloned().unwrap_or(Value::Null);
            Ok(JobActionResult::from_value(&result))
        })
    }

    /// Archive a job with a reason (wraps `job_archive`).
    async fn job_archive(
        &self,
        job_id: String,
        reason: String,
        actor: Option<String>,
    ) -> GqlResult<JobActionResult> {
        let args = args_from(vec![
            ("job_id", Some(Value::from(job_id))),
            ("reason", Some(Value::from(reason))),
            ("actor", actor.map(Value::from)),
        ]);
        with_invoker(|inv| {
            let value = inv.job("archive", args.clone()).map_err(map_err)?;
            let result = value.get("result").cloned().unwrap_or(Value::Null);
            Ok(JobActionResult::from_value(&result))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SharedStore;
    use rustyred_thg_core::InMemoryGraphStore;
    use serde_json::json;

    /// Drive a GraphQL document over a fresh in-process backend handle, returning
    /// the serialized response. `SharedStore` is cloned per call so a sequence of
    /// operations runs against ONE persistent in-memory store.
    fn run(
        store: &SharedStore<InMemoryGraphStore>,
        query: &str,
        variables: Value,
        op: super::super::OpKind,
    ) -> Value {
        let arguments = json!({ "query": query, "variables": variables });
        super::super::execute_graphql("smoke", store.clone(), &arguments, op)
            .expect("graphql execution")
    }

    fn assert_no_errors(response: &Value) {
        assert!(
            response.get("errors").map(Value::is_null).unwrap_or(true)
                || response["errors"].as_array().map(|e| e.is_empty()).unwrap_or(false),
            "unexpected graphql errors: {response}"
        );
    }

    fn store() -> SharedStore<InMemoryGraphStore> {
        SharedStore::new(InMemoryGraphStore::new())
    }

    // ---- R3: typed `from_value` parity over the exact live payload shapes ----

    #[test]
    fn skill_pack_from_value_reads_skill_pack_state_shape() {
        // The shape `skill_list_payload` puts in `packs[i]` (a serialized SkillPackState).
        let value = json!({
            "tenant_slug": "smoke",
            "pack_id": "pack-1",
            "pack_content_hash": "sha256:abc",
            "kind": "skill_pack",
            "status": "draft",
            "title": "Rust Engineering",
            "description": "guidance",
            "capabilities": ["rust", "mcp"],
            "source_content_hash": "sha256:src",
            "artifact_hashes": ["sha256:art"],
            "published_by": "claude-code",
            "created_at": "t0",
            "updated_at": "t1",
            "pack": { "instructions": "use the graph store" },
            "validators": [ { "kind": "native", "id": "v1" } ],
            "artifacts": { "bundle": "sha256:bundle" },
            "metadata": { "source": "fixture" }
        });
        let pack = SkillPack::from_value(&value);
        assert_eq!(pack.pack_content_hash, "sha256:abc");
        assert_eq!(pack.status, "draft");
        assert_eq!(pack.title, "Rust Engineering");
        assert_eq!(pack.capabilities, vec!["rust", "mcp"]);
        assert_eq!(pack.source_content_hash.as_deref(), Some("sha256:src"));
        assert_eq!(pack.artifact_hashes, vec!["sha256:art"]);
        assert_eq!(pack.published_by.as_deref(), Some("claude-code"));
        // The pack contract must survive the typed projection (it is the only
        // surface under graphql_default_surface).
        assert_eq!(pack.pack.0["instructions"], json!("use the graph store"));
        assert_eq!(pack.validators.0[0]["id"], json!("v1"));
        assert_eq!(pack.artifacts.0["bundle"], json!("sha256:bundle"));
        assert_eq!(pack.metadata.0["source"], json!("fixture"));
    }

    #[test]
    fn ensemble_selection_from_envelope_reads_decision_shape() {
        // The shape `ensemble_select_payload` returns.
        let envelope = json!({
            "tenant": "smoke",
            "decision_content_hash": "sha256:dec",
            "decision": {
                "task": "use rust graph store",
                "budget_units": 100,
                "spent_units": 7,
                "risk": "low",
                "selected": [
                    { "kind": "skill_pack", "pack_content_hash": "sha256:p1", "reason": "fit", "score": 0.9, "cost_units": 3 }
                ],
                "rejected": [
                    { "kind": "skill_pack", "pack_content_hash": "sha256:p2", "reason": "over budget" }
                ]
            }
        });
        let selection = EnsembleSelection::from_envelope(&envelope);
        assert_eq!(selection.decision_content_hash.as_deref(), Some("sha256:dec"));
        assert_eq!(selection.task, "use rust graph store");
        assert_eq!(selection.budget_units, Some(100));
        assert_eq!(selection.spent_units, 7);
        assert_eq!(selection.selected.len(), 1);
        assert_eq!(selection.selected[0].pack_content_hash, "sha256:p1");
        assert_eq!(selection.selected[0].cost_units, Some(3));
        assert_eq!(selection.rejected.len(), 1);
        assert_eq!(selection.rejected[0].pack_content_hash, "sha256:p2");
    }

    #[test]
    fn job_from_value_reads_list_shape_and_recovers_submit_state() {
        // List shape: state is injected by the runtime.
        let listed = json!({
            "job_id": "job-1",
            "title": "T",
            "repo": "owner/repo",
            "state": "started",
            "priority": "normal",
            "target_head": "claude",
            "spec_inline": "do x",
            "submitted_by": "claude-code",
            "submitted_at": "t0",
            "started_at": "t1",
            "idempotency_key": "sha256:k",
            "receipts": [ { "actor": "receiver", "at": "t2", "text": "started", "refs": [] } ]
        });
        let job = Job::from_value(&listed);
        assert_eq!(job.job_id, "job-1");
        assert_eq!(job.state, "started");
        assert_eq!(job.spec_inline.as_deref(), Some("do x"));
        assert_eq!(job.receipts.len(), 1);
        assert_eq!(job.receipts[0].actor, "receiver");

        // Submit shape: no `state` field; must be recomputed to "pending".
        let submitted = json!({
            "job_id": "job-2",
            "title": "T2",
            "repo": "owner/repo",
            "priority": "high",
            "target_head": "codex",
            "spec_ref": "docs/x.md",
            "submitted_by": "claude-code",
            "submitted_at": "t0",
            "idempotency_key": "sha256:k2"
        });
        let job = Job::from_value(&submitted);
        assert_eq!(job.state, "pending", "submit shape must derive pending state");
        assert_eq!(job.spec_ref.as_deref(), Some("docs/x.md"));
    }

    #[test]
    fn harness_run_from_envelope_is_none_when_missing_and_reads_detail() {
        let missing = json!({ "tenant": "smoke", "run_id": "missing", "detail": null, "found": false });
        assert!(HarnessRun::from_envelope(&missing).is_none());

        let present = json!({
            "tenant": "smoke",
            "run_id": "run-1",
            "found": true,
            "detail": {
                "run": { "run_id": "run-1", "task": "ship", "actor": "claude-code", "status": "running", "last_event_seq": 2, "created_at": "t0", "updated_at": "t1" },
                "events": [ { "event_id": "e1", "run_id": "run-1", "seq": 1, "type": "created", "created_at": "t0", "state_hash_before": "h0", "state_hash_after": "h1", "idempotency_key": "idem-1", "payload": { "delta": 7 } } ]
            }
        });
        let run = HarnessRun::from_envelope(&present).expect("present run");
        assert_eq!(run.run_id, "run-1");
        assert_eq!(run.task, "ship");
        assert_eq!(run.last_event_seq, 2);
        assert_eq!(run.events.len(), 1);
        assert_eq!(run.events[0].event_type, "created");
        assert_eq!(run.events[0].seq, 1);
        // Replay fields must survive: payload, both state hashes, idempotency key.
        assert_eq!(run.events[0].payload.0["delta"], json!(7));
        assert_eq!(run.events[0].state_hash_before.as_deref(), Some("h0"));
        assert_eq!(run.events[0].idempotency_key.as_deref(), Some("idem-1"));
    }

    // ---- R3: round-trips through `execute_graphql` over a real in-process store ----

    #[test]
    fn skill_publish_then_list_round_trips_typed() {
        let store = store();
        let published = run(
            &store,
            "mutation($i:JSON!){ skillPublish(input:$i){ packContentHash status title } }",
            json!({ "i": { "pack": { "kind": "skill_pack", "title": "Rust Engineering", "capabilities": ["rust"] } } }),
            super::super::OpKind::Mutate,
        );
        assert_no_errors(&published);
        let hash = published["data"]["skillPublish"]["packContentHash"]
            .as_str()
            .expect("published pack hash")
            .to_string();
        assert!(!hash.is_empty());
        assert_eq!(published["data"]["skillPublish"]["title"], json!("Rust Engineering"));

        let listed = run(
            &store,
            "query{ skillList { packContentHash status title } }",
            Value::Null,
            super::super::OpKind::Query,
        );
        assert_no_errors(&listed);
        let packs = listed["data"]["skillList"].as_array().expect("packs array");
        assert!(
            packs.iter().any(|pack| pack["packContentHash"] == json!(hash)),
            "skillList must surface the typed published pack: {listed}"
        );
    }

    #[test]
    fn job_submit_list_note_archive_round_trips_typed() {
        let store = store();
        let submitted = run(
            &store,
            "mutation($i:JSON!){ jobSubmit(input:$i){ jobId state title } }",
            json!({ "i": { "title": "Ship A6", "repo": "owner/repo", "spec_inline": "do x" } }),
            super::super::OpKind::Mutate,
        );
        assert_no_errors(&submitted);
        let job_id = submitted["data"]["jobSubmit"]["jobId"]
            .as_str()
            .expect("job id")
            .to_string();
        assert_eq!(submitted["data"]["jobSubmit"]["state"], json!("pending"));

        let listed = run(
            &store,
            "query{ jobList { jobId state } }",
            Value::Null,
            super::super::OpKind::Query,
        );
        assert_no_errors(&listed);
        let jobs = listed["data"]["jobList"].as_array().expect("jobs array");
        assert!(
            jobs.iter().any(|job| job["jobId"] == json!(job_id) && job["state"] == json!("pending")),
            "jobList must surface the typed submitted job: {listed}"
        );

        let archived = run(
            &store,
            "mutation($id:String!){ jobArchive(jobId:$id, reason:\"done\"){ found applied job { state } } }",
            json!({ "id": job_id }),
            super::super::OpKind::Mutate,
        );
        assert_no_errors(&archived);
        assert_eq!(archived["data"]["jobArchive"]["found"], json!(true));
        assert_eq!(archived["data"]["jobArchive"]["job"]["state"], json!("archived"));
    }

    #[test]
    fn ensemble_register_then_select_round_trips_typed() {
        let store = store();
        let registered = run(
            &store,
            "mutation($i:JSON!){ ensembleRegister(input:$i){ packContentHash } }",
            json!({ "i": {
                "pack": { "kind": "skill_pack", "title": "Rust Engineering", "description": "rust + graph store", "capabilities": ["rust", "graph_store", "mcp"] },
                "source_content_hash": "hash-source",
                "artifact_hashes": ["hash-artifact"]
            } }),
            super::super::OpKind::Mutate,
        );
        assert_no_errors(&registered);
        let hash = registered["data"]["ensembleRegister"]["packContentHash"]
            .as_str()
            .expect("registered pack hash")
            .to_string();
        assert!(!hash.is_empty());

        let selected = run(
            &store,
            "query($i:JSON!){ ensembleSelect(input:$i){ decisionContentHash selected { packContentHash } } }",
            json!({ "i": { "task": "use rust graph store mcp code search", "kind": "skill_pack", "max_selected": 1 } }),
            super::super::OpKind::Query,
        );
        assert_no_errors(&selected);
        assert_eq!(
            selected["data"]["ensembleSelect"]["selected"][0]["packContentHash"],
            json!(hash),
            "ensembleSelect must select the registered pack via the typed field: {selected}"
        );
        assert!(
            selected["data"]["ensembleSelect"]["decisionContentHash"].is_string(),
            "ensembleSelect must surface the decision content hash: {selected}"
        );
    }

    #[test]
    fn harness_run_missing_resolves_null_typed() {
        let store = store();
        let response = run(
            &store,
            "query{ harnessRun(runId:\"missing\"){ runId status } }",
            Value::Null,
            super::super::OpKind::Query,
        );
        assert_no_errors(&response);
        assert!(
            response["data"]["harnessRun"].is_null(),
            "missing run must resolve to null: {response}"
        );
    }

    // ---- R3: SDL exposes the typed cluster domains, tenant-free ----

    #[test]
    fn introspect_exposes_typed_cluster_types_and_fields() {
        let sdl = super::super::introspect_sdl();
        let sdl = sdl.as_str().expect("sdl string");
        for fragment in [
            // typed object types
            "type SkillPack",
            "type EnsembleSelection",
            "type SelectedCapability",
            "type Job",
            "type JobReceipt",
            "type JobActionResult",
            "type HarnessRun",
            "type HarnessEvent",
            // fields
            "skillList",
            "skillGet",
            "ensembleSelect",
            "jobList",
            "harnessRun",
            "skillPublish",
            "skillApply",
            "ensembleRegister",
            "jobSubmit",
            "jobNote",
            "jobArchive",
        ] {
            assert!(sdl.contains(fragment), "SDL missing {fragment}");
        }
        assert!(
            !sdl.to_lowercase().contains("tenant"),
            "no cluster GraphQL field may carry a tenant property:\n{sdl}"
        );
    }
}
