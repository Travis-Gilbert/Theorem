use rustyred_thg_core::{
    EdgeRecord, GraphStore, GraphStoreError, GraphStoreResult, NodeQuery, NodeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use theorem_harness_core::stable_value_hash;

pub type SkillPackResult<T> = Result<T, SkillPackError>;

const DEFAULT_TENANT: &str = "default";
const DEFAULT_STATUS: &str = "draft";
const MAX_PACK_LIST_LIMIT: usize = 100;
const PACK_QUERY_LIMIT: usize = 10_000;
const MAX_DECLARATION_VALIDATORS_PER_APPLY: usize = 128;
const MAX_NATIVE_ARTIFACT_VALIDATORS_PER_APPLY: usize = 64;
const PACK_STATUSES: &[&str] = &[
    "draft",
    "shadow",
    "advisory",
    "validated",
    "canonical",
    "retired",
];

pub trait SkillPackGraphStore {
    fn skill_pack_upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<()>;
    fn skill_pack_upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<()>;
    fn skill_pack_get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>>;
    fn skill_pack_query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>>;
}

impl<T: GraphStore> SkillPackGraphStore for T {
    fn skill_pack_upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<()> {
        self.upsert_node(node).map(|_| ())
    }

    fn skill_pack_upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<()> {
        self.upsert_edge(edge).map(|_| ())
    }

    fn skill_pack_get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        Ok(self.get_node(id).cloned())
    }

    fn skill_pack_query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        Ok(self.query_nodes(query))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SkillPackError {
    Store(GraphStoreError),
    Serialization(String),
    Deserialization(String),
    InvalidInput { field: String, message: String },
    NotFound { kind: String, id: String },
}

impl fmt::Display for SkillPackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{}: {}", error.code, error.message),
            Self::Serialization(error) => write!(f, "serialization failed: {error}"),
            Self::Deserialization(error) => write!(f, "deserialization failed: {error}"),
            Self::InvalidInput { field, message } => {
                write!(f, "invalid skill pack input {field}: {message}")
            }
            Self::NotFound { kind, id } => write!(f, "{kind} not found: {id}"),
        }
    }
}

impl Error for SkillPackError {}

impl From<GraphStoreError> for SkillPackError {
    fn from(value: GraphStoreError) -> Self {
        Self::Store(value)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct SkillPackPublishInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub pack_content_hash: String,
    #[serde(default)]
    pub source_content_hash: String,
    #[serde(default)]
    pub artifact_hashes: Vec<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    #[serde(default)]
    pub created_at: String,
    pub pack: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct SkillPackListInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub include_retired: bool,
    #[serde(default)]
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct SkillPackGetInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub pack_id: String,
    #[serde(default)]
    pub pack_content_hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct SkillPackApplyInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub pack_id: String,
    #[serde(default)]
    pub pack_content_hash: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub context: Value,
    #[serde(default)]
    pub outcome: Value,
    #[serde(default)]
    pub allow_retired: bool,
    #[serde(default)]
    pub receipt_id: String,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct SkillPackState {
    pub tenant_slug: String,
    pub pack_id: String,
    pub pack_content_hash: String,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub validators: Value,
    pub artifacts: Value,
    pub artifact_hashes: Vec<String>,
    pub source_content_hash: String,
    pub pack: Value,
    pub metadata: Map<String, Value>,
    pub published_by: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct SkillPackApplyReceipt {
    pub tenant_slug: String,
    pub receipt_id: String,
    pub pack_id: String,
    pub pack_content_hash: String,
    pub actor_id: String,
    pub run_id: String,
    pub task: String,
    pub status: String,
    pub validator_execution_mode: String,
    pub validators: Vec<SkillPackValidatorReceipt>,
    pub outcome: Value,
    pub metadata: Map<String, Value>,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct SkillPackValidatorReceipt {
    pub validator_id: String,
    pub kind: String,
    pub status: String,
    pub message: String,
    pub artifact_hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct SkillPackPublishReceipt {
    pub pack: SkillPackState,
    pub source_edge_id: String,
    pub artifact_edge_ids: Vec<String>,
}

pub fn skill_pack_node_id(tenant: &str, pack_content_hash: &str) -> String {
    format!(
        "skill_pack:{}:{}",
        normalize_tenant(tenant),
        pack_content_hash.trim()
    )
}

pub fn skill_pack_source_node_id(tenant: &str, source_content_hash: &str) -> String {
    format!(
        "skill_pack_source:{}:{}",
        normalize_tenant(tenant),
        source_content_hash.trim()
    )
}

pub fn skill_pack_artifact_node_id(tenant: &str, artifact_hash: &str) -> String {
    format!(
        "skill_pack_artifact:{}:{}",
        normalize_tenant(tenant),
        artifact_hash.trim()
    )
}

pub fn skill_pack_use_receipt_node_id(tenant: &str, receipt_id: &str) -> String {
    format!(
        "skill_pack_use:{}:{}",
        normalize_tenant(tenant),
        receipt_id.trim()
    )
}

pub fn publish_skill_pack<S: SkillPackGraphStore>(
    store: &mut S,
    input: SkillPackPublishInput,
) -> SkillPackResult<SkillPackPublishReceipt> {
    let tenant = normalize_tenant(&input.tenant_slug);
    let pack = normalize_pack(input.pack)?;
    let metadata = merged_metadata(&pack, input.metadata);
    let pack_content_hash = resolve_pack_content_hash(&pack, &metadata, &input.pack_content_hash)?;
    let status = normalize_status(&input.status, &metadata)?;
    let now = timestamp_or_now(&input.created_at);
    let source_content_hash = resolve_source_content_hash(&metadata, &input.source_content_hash);
    let artifact_hashes = resolve_artifact_hashes(&pack, &metadata, input.artifact_hashes);
    let state = SkillPackState {
        tenant_slug: tenant.clone(),
        pack_id: pack_id(&pack, &pack_content_hash),
        pack_content_hash: pack_content_hash.clone(),
        kind: pack
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("skill_pack")
            .trim()
            .to_string(),
        status,
        title: text_at(&pack, &["title", "name"]).unwrap_or_default(),
        description: text_at(&pack, &["description", "summary"]).unwrap_or_default(),
        capabilities: string_array_at(&pack, "capabilities"),
        validators: pack.get("validators").cloned().unwrap_or_else(|| json!([])),
        artifacts: metadata
            .get("artifacts")
            .cloned()
            .or_else(|| pack.get("artifacts").cloned())
            .unwrap_or_else(|| json!({})),
        artifact_hashes,
        source_content_hash,
        pack,
        metadata,
        published_by: input.actor_id.trim().to_string(),
        created_at: now.clone(),
        updated_at: now,
    };
    validate_pack_state(&state)?;

    store.skill_pack_upsert_node(skill_pack_node(&state)?)?;

    let source_edge_id = if state.source_content_hash.trim().is_empty() {
        String::new()
    } else {
        persist_hash_node_and_edge(
            store,
            &tenant,
            &state.pack_content_hash,
            &state.source_content_hash,
            HashNodeKind::Source,
        )?
    };
    let mut artifact_edge_ids = Vec::new();
    for artifact_hash in &state.artifact_hashes {
        artifact_edge_ids.push(persist_hash_node_and_edge(
            store,
            &tenant,
            &state.pack_content_hash,
            artifact_hash,
            HashNodeKind::Artifact,
        )?);
    }

    Ok(SkillPackPublishReceipt {
        pack: state,
        source_edge_id,
        artifact_edge_ids,
    })
}

pub fn list_skill_packs<S: SkillPackGraphStore>(
    store: &S,
    input: SkillPackListInput,
) -> SkillPackResult<Vec<SkillPackState>> {
    let tenant = normalize_tenant(&input.tenant_slug);
    let status = input.status.trim().to_lowercase();
    let limit = input.limit.clamp(1, MAX_PACK_LIST_LIMIT);
    let mut packs = store
        .skill_pack_query_nodes(NodeQuery::label("SkillPack").with_limit(PACK_QUERY_LIMIT))?
        .into_iter()
        .map(skill_pack_from_node)
        .collect::<SkillPackResult<Vec<_>>>()?;
    packs.retain(|pack| pack.tenant_slug == tenant);
    if !status.is_empty() {
        packs.retain(|pack| pack.status == status);
    } else if !input.include_retired {
        packs.retain(|pack| pack.status != "retired");
    }
    packs.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.pack_id.cmp(&right.pack_id))
    });
    packs.truncate(limit);
    Ok(packs)
}

pub fn get_skill_pack<S: SkillPackGraphStore>(
    store: &S,
    input: SkillPackGetInput,
) -> SkillPackResult<SkillPackState> {
    let tenant = normalize_tenant(&input.tenant_slug);
    let key = input
        .pack_content_hash
        .trim()
        .to_string()
        .if_empty(input.pack_id.trim().to_string());
    if key.trim().is_empty() {
        return Err(SkillPackError::InvalidInput {
            field: "pack_id".to_string(),
            message: "skill_get requires pack_id or pack_content_hash".to_string(),
        });
    }
    if let Some(node) = store.skill_pack_get_node(&skill_pack_node_id(&tenant, &key))? {
        return skill_pack_from_node(node);
    }
    let packs = list_skill_packs(
        store,
        SkillPackListInput {
            tenant_slug: tenant.clone(),
            include_retired: true,
            limit: MAX_PACK_LIST_LIMIT,
            ..SkillPackListInput::default()
        },
    )?;
    packs
        .into_iter()
        .find(|pack| pack.pack_id == key || pack.pack_content_hash == key)
        .ok_or_else(|| SkillPackError::NotFound {
            kind: "skill_pack".to_string(),
            id: key,
        })
}

pub fn apply_skill_pack<S: SkillPackGraphStore>(
    store: &mut S,
    input: SkillPackApplyInput,
) -> SkillPackResult<SkillPackApplyReceipt> {
    let tenant = normalize_tenant(&input.tenant_slug);
    let pack = get_skill_pack(
        store,
        SkillPackGetInput {
            tenant_slug: tenant.clone(),
            pack_id: input.pack_id.clone(),
            pack_content_hash: input.pack_content_hash.clone(),
        },
    )?;
    if pack.status == "retired" && !input.allow_retired {
        return Err(SkillPackError::InvalidInput {
            field: "status".to_string(),
            message: "skill_apply refuses retired packs unless allow_retired is true".to_string(),
        });
    }
    let now = timestamp_or_now(&input.created_at);
    let validator_run = run_skill_pack_validators(&pack, &input);
    let status = if validator_run
        .receipts
        .iter()
        .any(|receipt| receipt.status == "failed")
    {
        "failed"
    } else {
        "applied"
    };
    let receipt_id = if input.receipt_id.trim().is_empty() {
        stable_value_hash(&json!({
            "tenant_slug": tenant,
            "pack_content_hash": pack.pack_content_hash,
            "actor_id": input.actor_id,
            "run_id": input.run_id,
            "task": input.task,
            "created_at": now,
        }))
    } else {
        input.receipt_id.trim().to_string()
    };
    let receipt = SkillPackApplyReceipt {
        tenant_slug: tenant.clone(),
        receipt_id,
        pack_id: pack.pack_id.clone(),
        pack_content_hash: pack.pack_content_hash.clone(),
        actor_id: input.actor_id.trim().to_string(),
        run_id: input.run_id.trim().to_string(),
        task: input.task.trim().to_string(),
        status: status.to_string(),
        validator_execution_mode: validator_run.execution_mode,
        validators: validator_run.receipts,
        outcome: input.outcome,
        metadata: input.metadata,
        created_at: now,
    };
    store.skill_pack_upsert_node(skill_pack_use_receipt_node(&receipt)?)?;
    store.skill_pack_upsert_edge(skill_pack_use_receipt_edge(&pack, &receipt)?)?;
    Ok(receipt)
}

fn skill_pack_node(state: &SkillPackState) -> SkillPackResult<NodeRecord> {
    Ok(NodeRecord::new(
        skill_pack_node_id(&state.tenant_slug, &state.pack_content_hash),
        ["SkillPack", "CapabilityPack"],
        serialize_value(state)?,
    ))
}

fn skill_pack_use_receipt_node(receipt: &SkillPackApplyReceipt) -> SkillPackResult<NodeRecord> {
    Ok(NodeRecord::new(
        skill_pack_use_receipt_node_id(&receipt.tenant_slug, &receipt.receipt_id),
        ["SkillPackUseReceipt", "UseReceipt"],
        serialize_value(receipt)?,
    ))
}

fn skill_pack_use_receipt_edge(
    pack: &SkillPackState,
    receipt: &SkillPackApplyReceipt,
) -> SkillPackResult<EdgeRecord> {
    Ok(EdgeRecord::new(
        format!(
            "skill_pack_applied:{}:{}:{}",
            pack.tenant_slug, pack.pack_content_hash, receipt.receipt_id
        ),
        skill_pack_node_id(&pack.tenant_slug, &pack.pack_content_hash),
        "SKILL_PACK_APPLIED",
        skill_pack_use_receipt_node_id(&receipt.tenant_slug, &receipt.receipt_id),
        json!({
            "tenant_slug": pack.tenant_slug,
            "status": receipt.status,
            "created_at": receipt.created_at
        }),
    ))
}

fn skill_pack_from_node(node: NodeRecord) -> SkillPackResult<SkillPackState> {
    serde_json::from_value::<SkillPackState>(node.properties)
        .map_err(|error| SkillPackError::Deserialization(error.to_string()))
}

fn persist_hash_node_and_edge<S: SkillPackGraphStore>(
    store: &mut S,
    tenant: &str,
    pack_content_hash: &str,
    hash: &str,
    kind: HashNodeKind,
) -> SkillPackResult<String> {
    let hash = hash.trim();
    if hash.is_empty() {
        return Ok(String::new());
    }
    let (node_id, labels, edge_type, edge_id) = match kind {
        HashNodeKind::Source => (
            skill_pack_source_node_id(tenant, hash),
            ["SkillPackSource", "ContentHash"],
            "SKILL_PACK_SOURCE",
            format!("skill_pack_source:{tenant}:{pack_content_hash}:{hash}"),
        ),
        HashNodeKind::Artifact => (
            skill_pack_artifact_node_id(tenant, hash),
            ["SkillPackArtifact", "ContentHash"],
            "SKILL_PACK_ARTIFACT",
            format!("skill_pack_artifact:{tenant}:{pack_content_hash}:{hash}"),
        ),
    };
    store.skill_pack_upsert_node(NodeRecord::new(
        node_id.clone(),
        labels,
        json!({
            "tenant_slug": tenant,
            "content_hash": hash,
            "kind": kind.as_str(),
        }),
    ))?;
    store.skill_pack_upsert_edge(EdgeRecord::new(
        edge_id.clone(),
        skill_pack_node_id(tenant, pack_content_hash),
        edge_type,
        node_id,
        json!({
            "tenant_slug": tenant,
            "pack_content_hash": pack_content_hash,
            "content_hash": hash,
        }),
    ))?;
    Ok(edge_id)
}

#[derive(Clone, Copy)]
enum HashNodeKind {
    Source,
    Artifact,
}

impl HashNodeKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Artifact => "artifact",
        }
    }
}

fn validate_pack_state(state: &SkillPackState) -> SkillPackResult<()> {
    if state.pack_content_hash.trim().is_empty() {
        return Err(SkillPackError::InvalidInput {
            field: "pack_content_hash".to_string(),
            message: "pack_content_hash must not be empty".to_string(),
        });
    }
    if state.kind != "skill_pack" {
        return Err(SkillPackError::InvalidInput {
            field: "kind".to_string(),
            message: "CapabilityPackSpec kind must be skill_pack".to_string(),
        });
    }
    Ok(())
}

struct SkillPackValidatorRun {
    execution_mode: String,
    receipts: Vec<SkillPackValidatorReceipt>,
}

fn run_skill_pack_validators(
    pack: &SkillPackState,
    input: &SkillPackApplyInput,
) -> SkillPackValidatorRun {
    let mut receipts = Vec::new();
    let mut native_artifact_hashes = BTreeSet::new();
    let validators = pack.validators.as_array().cloned().unwrap_or_default();
    if !validators.is_empty() {
        for (index, validator) in validators
            .iter()
            .take(MAX_DECLARATION_VALIDATORS_PER_APPLY)
            .enumerate()
        {
            receipts.push(run_validator_declaration(pack, input, index, validator));
        }
        if validators.len() > MAX_DECLARATION_VALIDATORS_PER_APPLY {
            receipts.push(SkillPackValidatorReceipt {
                validator_id: "validator-budget".to_string(),
                kind: "bounded_validator_set".to_string(),
                status: "failed".to_string(),
                message: format!(
                    "validator declaration count {} exceeds bounded limit {}",
                    validators.len(),
                    MAX_DECLARATION_VALIDATORS_PER_APPLY
                ),
                artifact_hash: String::new(),
            });
        }
    }

    let artifact_receipts =
        run_native_artifact_validators(pack, input, &mut native_artifact_hashes);
    let native_artifact_attempted = !artifact_receipts.is_empty();
    receipts.extend(artifact_receipts);

    if receipts.is_empty() {
        receipts.push(SkillPackValidatorReceipt {
            validator_id: "pack-present".to_string(),
            kind: "required_field".to_string(),
            status: "passed".to_string(),
            message: "pack is present and content-addressed".to_string(),
            artifact_hash: String::new(),
        });
    }

    SkillPackValidatorRun {
        execution_mode: if native_artifact_attempted {
            "native_artifact_sandbox".to_string()
        } else {
            "safe_declaration".to_string()
        },
        receipts,
    }
}

fn run_validator_declaration(
    pack: &SkillPackState,
    input: &SkillPackApplyInput,
    index: usize,
    validator: &Value,
) -> SkillPackValidatorReceipt {
    let validator_id =
        text_at(validator, &["id", "name"]).unwrap_or_else(|| format!("validator-{index}"));
    let kind = text_at(validator, &["kind", "type"]).unwrap_or_else(|| "declaration".to_string());
    let artifact_hash = text_at(
        validator,
        &[
            "artifact_hash",
            "artifactHash",
            "content_hash",
            "contentHash",
            "hash",
        ],
    )
    .unwrap_or_default();
    match kind.as_str() {
        "required_field" | "pack_required_field" => {
            let field = text_at(validator, &["field", "path"]).unwrap_or_default();
            let passed = !field.is_empty() && value_path_exists(&pack.pack, &field);
            SkillPackValidatorReceipt {
                validator_id,
                kind,
                status: if passed { "passed" } else { "failed" }.to_string(),
                message: if passed {
                    format!("required pack field {field} is present")
                } else {
                    format!("required pack field {field} is missing")
                },
                artifact_hash,
            }
        }
        "context_required_field" => {
            let field = text_at(validator, &["field", "path"]).unwrap_or_default();
            let passed = !field.is_empty() && value_path_exists(&input.context, &field);
            SkillPackValidatorReceipt {
                validator_id,
                kind,
                status: if passed { "passed" } else { "failed" }.to_string(),
                message: if passed {
                    format!("required context field {field} is present")
                } else {
                    format!("required context field {field} is missing")
                },
                artifact_hash,
            }
        }
        "artifact_hash_present" => {
            let passed = !artifact_hash.is_empty()
                || !pack.artifact_hashes.is_empty()
                || !pack
                    .artifacts
                    .as_object()
                    .map(Map::is_empty)
                    .unwrap_or(true);
            SkillPackValidatorReceipt {
                validator_id,
                kind,
                status: if passed { "passed" } else { "failed" }.to_string(),
                message: if passed {
                    "validator artifact hash is present".to_string()
                } else {
                    "validator artifact hash is missing".to_string()
                },
                artifact_hash,
            }
        }
        "always_pass" | "declaration" => SkillPackValidatorReceipt {
            validator_id,
            kind,
            status: "passed".to_string(),
            message: "validator declaration passed".to_string(),
            artifact_hash,
        },
        other => SkillPackValidatorReceipt {
            validator_id,
            kind: other.to_string(),
            status: "registered".to_string(),
            message: "validator is registered; sandboxed artifact execution is a follow-up"
                .to_string(),
            artifact_hash,
        },
    }
}

fn run_native_artifact_validators(
    pack: &SkillPackState,
    input: &SkillPackApplyInput,
    executed_hashes: &mut BTreeSet<String>,
) -> Vec<SkillPackValidatorReceipt> {
    let mut artifacts = Vec::new();
    collect_native_validator_artifacts(&pack.artifacts, &mut artifacts);
    artifacts
        .iter()
        .take(MAX_NATIVE_ARTIFACT_VALIDATORS_PER_APPLY)
        .enumerate()
        .filter_map(|(index, artifact)| {
            let artifact_hash = artifact_content_hash(artifact);
            if !artifact_hash.is_empty() && !executed_hashes.insert(artifact_hash.clone()) {
                return None;
            }
            Some(run_native_validator_artifact(input, index, artifact))
        })
        .chain(
            (artifacts.len() > MAX_NATIVE_ARTIFACT_VALIDATORS_PER_APPLY).then(|| {
                SkillPackValidatorReceipt {
                    validator_id: "native-artifact-budget".to_string(),
                    kind: "native_artifact_sandbox".to_string(),
                    status: "failed".to_string(),
                    message: format!(
                        "native validator artifact count {} exceeds bounded limit {}",
                        artifacts.len(),
                        MAX_NATIVE_ARTIFACT_VALIDATORS_PER_APPLY
                    ),
                    artifact_hash: String::new(),
                }
            }),
        )
        .collect()
}

fn collect_native_validator_artifacts(value: &Value, artifacts: &mut Vec<Value>) {
    if artifacts.len() > MAX_NATIVE_ARTIFACT_VALIDATORS_PER_APPLY {
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                collect_native_validator_artifacts(item, artifacts);
                if artifacts.len() > MAX_NATIVE_ARTIFACT_VALIDATORS_PER_APPLY {
                    return;
                }
            }
        }
        Value::Object(map) if is_native_validator_artifact(value) => {
            artifacts.push(Value::Object(map.clone()));
        }
        Value::Object(map) => {
            for key in ["artifacts", "validators", "items"] {
                if let Some(child) = map.get(key) {
                    collect_native_validator_artifacts(child, artifacts);
                    if artifacts.len() > MAX_NATIVE_ARTIFACT_VALIDATORS_PER_APPLY {
                        return;
                    }
                }
            }
        }
        _ => {}
    }
}

fn is_native_validator_artifact(value: &Value) -> bool {
    let Some(kind) = text_at(value, &["kind", "artifact_kind", "artifactKind"]) else {
        return false;
    };
    matches!(
        kind.as_str(),
        "native_validator_candidate" | "rust_validator_artifact" | "native_validator_artifact"
    )
}

fn run_native_validator_artifact(
    input: &SkillPackApplyInput,
    index: usize,
    artifact: &Value,
) -> SkillPackValidatorReceipt {
    let artifact_hash = artifact_content_hash(artifact);
    let validator_id = text_at(artifact, &["artifact_id", "artifactId", "id"])
        .unwrap_or_else(|| format!("native-validator-artifact-{index}"));
    let kind = text_at(artifact, &["kind", "artifact_kind", "artifactKind"])
        .unwrap_or_else(|| "native_validator_candidate".to_string());
    if artifact_hash.is_empty() {
        return SkillPackValidatorReceipt {
            validator_id,
            kind,
            status: "failed".to_string(),
            message: "native validator artifact is missing content_hash".to_string(),
            artifact_hash,
        };
    }

    let Some(body) = artifact.get("body").filter(|value| value.is_object()) else {
        return SkillPackValidatorReceipt {
            validator_id,
            kind,
            status: "failed".to_string(),
            message: "native validator artifact is missing body".to_string(),
            artifact_hash,
        };
    };
    let source_atom_type =
        text_at(body, &["source_atom_type", "sourceAtomType"]).unwrap_or_default();
    match source_atom_type.as_str() {
        "code_member" => {
            run_code_member_validator_artifact(input, validator_id, kind, artifact_hash, body)
        }
        "" => SkillPackValidatorReceipt {
            validator_id,
            kind,
            status: "failed".to_string(),
            message: "native validator artifact is missing source_atom_type".to_string(),
            artifact_hash,
        },
        other => SkillPackValidatorReceipt {
            validator_id,
            kind,
            status: "registered".to_string(),
            message: format!(
                "native artifact sandbox registered unsupported source_atom_type {other}"
            ),
            artifact_hash,
        },
    }
}

fn run_code_member_validator_artifact(
    input: &SkillPackApplyInput,
    validator_id: String,
    kind: String,
    artifact_hash: String,
    body: &Value,
) -> SkillPackValidatorReceipt {
    let expected = text_at(body, &["entrypoint", "name"]).unwrap_or_default();
    if expected.is_empty() {
        return SkillPackValidatorReceipt {
            validator_id,
            kind,
            status: "failed".to_string(),
            message: "code_member validator artifact is missing entrypoint".to_string(),
            artifact_hash,
        };
    }
    let actual = candidate_code_member_name(input);
    let passed = actual.as_deref() == Some(expected.as_str());
    SkillPackValidatorReceipt {
        validator_id,
        kind,
        status: if passed { "passed" } else { "failed" }.to_string(),
        message: match actual {
            Some(actual) if passed => {
                format!("native artifact sandbox matched code_member entrypoint {actual}")
            }
            Some(actual) => {
                format!(
                    "native artifact sandbox expected code_member entrypoint {expected}, got {actual}"
                )
            }
            None => format!(
                "native artifact sandbox expected code_member entrypoint {expected}, but no candidate name was provided"
            ),
        },
        artifact_hash,
    }
}

fn artifact_content_hash(artifact: &Value) -> String {
    text_at(
        artifact,
        &[
            "content_hash",
            "contentHash",
            "artifact_hash",
            "artifactHash",
            "hash",
        ],
    )
    .unwrap_or_default()
}

fn candidate_code_member_name(input: &SkillPackApplyInput) -> Option<String> {
    first_text_path(
        &input.context,
        &[
            "code_member.name",
            "candidate.name",
            "signature.name",
            "member.name",
            "function.name",
            "name",
            "entrypoint",
        ],
    )
    .or_else(|| {
        first_text_path(
            &input.outcome,
            &[
                "code_member.name",
                "candidate.name",
                "signature.name",
                "member.name",
                "function.name",
                "name",
                "entrypoint",
            ],
        )
    })
}

fn normalize_pack(pack: Value) -> SkillPackResult<Value> {
    if !pack.is_object() {
        return Err(SkillPackError::InvalidInput {
            field: "pack".to_string(),
            message: "skill_publish requires pack object".to_string(),
        });
    }
    Ok(pack)
}

fn merged_metadata(pack: &Value, mut explicit: Map<String, Value>) -> Map<String, Value> {
    if let Some(pack_metadata) = pack.get("metadata").and_then(Value::as_object) {
        for (key, value) in pack_metadata {
            explicit.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    explicit
}

fn resolve_pack_content_hash(
    pack: &Value,
    metadata: &Map<String, Value>,
    explicit: &str,
) -> SkillPackResult<String> {
    let hash = explicit
        .trim()
        .to_string()
        .if_empty(text_from_map(
            metadata,
            &[
                "pack_content_hash",
                "packContentHash",
                "content_hash",
                "contentHash",
            ],
        ))
        .if_empty(stable_value_hash(pack));
    if hash.trim().is_empty() {
        Err(SkillPackError::InvalidInput {
            field: "pack_content_hash".to_string(),
            message: "pack_content_hash must not be empty".to_string(),
        })
    } else {
        Ok(hash)
    }
}

fn normalize_status(explicit: &str, metadata: &Map<String, Value>) -> SkillPackResult<String> {
    let status = explicit
        .trim()
        .to_lowercase()
        .if_empty(text_from_map(
            metadata,
            &["status", "promotion_state", "promotionState"],
        ))
        .if_empty(DEFAULT_STATUS.to_string());
    if PACK_STATUSES.contains(&status.as_str()) {
        Ok(status)
    } else {
        Err(SkillPackError::InvalidInput {
            field: "status".to_string(),
            message: format!("status must be one of {}", PACK_STATUSES.join(", ")),
        })
    }
}

fn resolve_source_content_hash(metadata: &Map<String, Value>, explicit: &str) -> String {
    explicit.trim().to_string().if_empty(text_from_map(
        metadata,
        &[
            "source_content_hash",
            "sourceContentHash",
            "source_hash",
            "sourceHash",
        ],
    ))
}

fn resolve_artifact_hashes(
    pack: &Value,
    metadata: &Map<String, Value>,
    explicit: Vec<String>,
) -> Vec<String> {
    let mut hashes = BTreeSet::new();
    for hash in explicit {
        insert_hash(&mut hashes, &hash);
    }
    for key in ["artifact_hashes", "artifactHashes"] {
        if let Some(items) = metadata.get(key).and_then(Value::as_array) {
            for item in items {
                if let Some(hash) = item.as_str() {
                    insert_hash(&mut hashes, hash);
                }
            }
        }
    }
    if let Some(artifacts) = metadata.get("artifacts").or_else(|| pack.get("artifacts")) {
        collect_hash_strings(artifacts, &mut hashes);
    }
    if let Some(validators) = pack.get("validators") {
        collect_hash_strings(validators, &mut hashes);
    }
    hashes.into_iter().collect()
}

fn collect_hash_strings(value: &Value, hashes: &mut BTreeSet<String>) {
    collect_hash_strings_inner(value, hashes, false);
}

fn collect_hash_strings_inner(value: &Value, hashes: &mut BTreeSet<String>, hash_field: bool) {
    match value {
        Value::String(text) if hash_field => insert_hash(hashes, text),
        Value::String(_) => {}
        Value::Array(items) => {
            for item in items {
                collect_hash_strings_inner(item, hashes, hash_field);
            }
        }
        Value::Object(map) => {
            for (key, value) in map {
                collect_hash_strings_inner(
                    value,
                    hashes,
                    key.to_ascii_lowercase().contains("hash"),
                );
            }
        }
        _ => {}
    }
}

fn insert_hash(hashes: &mut BTreeSet<String>, hash: &str) {
    let hash = hash.trim();
    if !hash.is_empty() {
        hashes.insert(hash.to_string());
    }
}

fn pack_id(pack: &Value, pack_content_hash: &str) -> String {
    text_at(pack, &["id", "pack_id", "packId", "name"])
        .unwrap_or_else(|| pack_content_hash.to_string())
}

fn text_at(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn text_from_map(map: &Map<String, Value>, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| map.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_default()
}

fn string_array_at(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            let mut values = BTreeSet::new();
            for item in items {
                if let Some(text) = item.as_str().map(str::trim).filter(|text| !text.is_empty()) {
                    values.insert(text.to_string());
                }
            }
            values.into_iter().collect()
        })
        .unwrap_or_default()
}

fn value_path_exists(value: &Value, path: &str) -> bool {
    let mut current = value;
    for segment in path.split('.') {
        let segment = segment.trim();
        if segment.is_empty() {
            return false;
        }
        match current {
            Value::Object(map) => match map.get(segment) {
                Some(next) => current = next,
                None => return false,
            },
            _ => return false,
        }
    }
    !current.is_null()
}

fn first_text_path(value: &Value, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| text_path(value, path))
}

fn text_path(value: &Value, path: &str) -> Option<String> {
    let mut current = value;
    for segment in path.split('.') {
        let segment = segment.trim();
        if segment.is_empty() {
            return None;
        }
        current = current.as_object()?.get(segment)?;
    }
    current
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn serialize_value<T: Serialize>(value: &T) -> SkillPackResult<Value> {
    serde_json::to_value(value).map_err(|error| SkillPackError::Serialization(error.to_string()))
}

fn normalize_tenant(tenant: &str) -> String {
    let tenant = tenant.trim();
    if tenant.is_empty() {
        DEFAULT_TENANT.to_string()
    } else {
        tenant.to_string()
    }
}

fn timestamp_or_now(value: &str) -> String {
    let value = value.trim();
    if !value.is_empty() {
        return value.to_string();
    }
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("unix_ms:{millis}")
}

trait IfEmpty {
    fn if_empty(self, fallback: String) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, fallback: String) -> String {
        if self.trim().is_empty() {
            fallback
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::InMemoryGraphStore;

    fn sample_pack() -> Value {
        json!({
            "id": "rustyred-rust-skill",
            "kind": "skill_pack",
            "title": "RustyRed Rust skill",
            "capabilities": ["rust_refactor", "graph_store"],
            "validators": [
                { "id": "has-kind", "kind": "required_field", "field": "kind" },
                { "id": "has-artifact", "kind": "artifact_hash_present", "artifact_hash": "hash-validator" }
            ],
            "metadata": {
                "pack_content_hash": "hash-pack",
                "source_content_hash": "hash-source",
                "artifacts": {
                    "validator": { "content_hash": "hash-validator" }
                }
            }
        })
    }

    fn native_artifact_pack() -> Value {
        json!({
            "id": "rust-native-artifact-skill",
            "kind": "skill_pack",
            "title": "Rust native artifact skill",
            "capabilities": ["rust_refactor"],
            "validators": [],
            "metadata": {
                "pack_content_hash": "hash-native-pack",
                "artifacts": [
                    {
                        "artifact_id": "artifact:compile-pack",
                        "kind": "native_validator_candidate",
                        "target": "validator_contract",
                        "language": "rust",
                        "content_hash": "sha256:compile-pack-validator",
                        "parent_hashes": ["sha256:source"],
                        "body": {
                            "source_atom_type": "code_member",
                            "entrypoint": "compile_pack",
                            "dialect": "rust",
                            "executable": false
                        }
                    }
                ]
            }
        })
    }

    #[test]
    fn publish_get_and_list_skill_pack() {
        let mut store = InMemoryGraphStore::new();
        let receipt = publish_skill_pack(
            &mut store,
            SkillPackPublishInput {
                tenant_slug: "tenant-a".to_string(),
                actor_id: "codex".to_string(),
                status: "validated".to_string(),
                pack: sample_pack(),
                created_at: "t1".to_string(),
                ..SkillPackPublishInput::default()
            },
        )
        .unwrap();

        assert_eq!(receipt.pack.pack_content_hash, "hash-pack");
        assert_eq!(receipt.pack.status, "validated");
        assert_eq!(
            receipt.source_edge_id,
            "skill_pack_source:tenant-a:hash-pack:hash-source"
        );
        assert_eq!(
            receipt.artifact_edge_ids,
            vec!["skill_pack_artifact:tenant-a:hash-pack:hash-validator"]
        );

        let loaded = get_skill_pack(
            &store,
            SkillPackGetInput {
                tenant_slug: "tenant-a".to_string(),
                pack_id: "rustyred-rust-skill".to_string(),
                ..SkillPackGetInput::default()
            },
        )
        .unwrap();
        assert_eq!(loaded.pack_content_hash, "hash-pack");

        let packs = list_skill_packs(
            &store,
            SkillPackListInput {
                tenant_slug: "tenant-a".to_string(),
                limit: 10,
                ..SkillPackListInput::default()
            },
        )
        .unwrap();
        assert_eq!(packs.len(), 1);
    }

    #[test]
    fn apply_skill_pack_writes_use_receipt() {
        let mut store = InMemoryGraphStore::new();
        publish_skill_pack(
            &mut store,
            SkillPackPublishInput {
                tenant_slug: "tenant-a".to_string(),
                status: "validated".to_string(),
                pack: sample_pack(),
                ..SkillPackPublishInput::default()
            },
        )
        .unwrap();

        let receipt = apply_skill_pack(
            &mut store,
            SkillPackApplyInput {
                tenant_slug: "tenant-a".to_string(),
                pack_content_hash: "hash-pack".to_string(),
                actor_id: "codex".to_string(),
                run_id: "run-1".to_string(),
                task: "refactor GraphStore".to_string(),
                created_at: "t2".to_string(),
                ..SkillPackApplyInput::default()
            },
        )
        .unwrap();

        assert_eq!(receipt.status, "applied");
        assert_eq!(receipt.validators.len(), 2);
        assert!(store
            .get_node(&skill_pack_use_receipt_node_id(
                "tenant-a",
                &receipt.receipt_id
            ))
            .is_some());
    }

    #[test]
    fn apply_skill_pack_fails_missing_required_field_validator() {
        let mut store = InMemoryGraphStore::new();
        let mut pack = sample_pack();
        pack["validators"] = json!([
            { "id": "missing", "kind": "required_field", "field": "metadata.nope" }
        ]);
        publish_skill_pack(
            &mut store,
            SkillPackPublishInput {
                tenant_slug: "tenant-a".to_string(),
                pack,
                ..SkillPackPublishInput::default()
            },
        )
        .unwrap();

        let receipt = apply_skill_pack(
            &mut store,
            SkillPackApplyInput {
                tenant_slug: "tenant-a".to_string(),
                pack_content_hash: "hash-pack".to_string(),
                actor_id: "codex".to_string(),
                ..SkillPackApplyInput::default()
            },
        )
        .unwrap();

        assert_eq!(receipt.status, "failed");
        assert_eq!(receipt.validators[0].status, "failed");
    }

    #[test]
    fn apply_skill_pack_runs_native_code_member_artifact_validator() {
        let mut store = InMemoryGraphStore::new();
        publish_skill_pack(
            &mut store,
            SkillPackPublishInput {
                tenant_slug: "tenant-a".to_string(),
                status: "validated".to_string(),
                pack: native_artifact_pack(),
                ..SkillPackPublishInput::default()
            },
        )
        .unwrap();

        let receipt = apply_skill_pack(
            &mut store,
            SkillPackApplyInput {
                tenant_slug: "tenant-a".to_string(),
                pack_content_hash: "hash-native-pack".to_string(),
                actor_id: "codex".to_string(),
                context: json!({
                    "code_member": {
                        "name": "compile_pack",
                        "params": ["plan", "result"],
                        "return_type": "CapabilityPackSpec"
                    }
                }),
                ..SkillPackApplyInput::default()
            },
        )
        .unwrap();

        assert_eq!(receipt.status, "applied");
        assert_eq!(receipt.validator_execution_mode, "native_artifact_sandbox");
        assert_eq!(receipt.validators.len(), 1);
        assert_eq!(receipt.validators[0].status, "passed");
        assert_eq!(
            receipt.validators[0].artifact_hash,
            "sha256:compile-pack-validator"
        );
    }

    #[test]
    fn apply_skill_pack_fails_native_artifact_validator_on_wrong_candidate() {
        let mut store = InMemoryGraphStore::new();
        publish_skill_pack(
            &mut store,
            SkillPackPublishInput {
                tenant_slug: "tenant-a".to_string(),
                pack: native_artifact_pack(),
                ..SkillPackPublishInput::default()
            },
        )
        .unwrap();

        let receipt = apply_skill_pack(
            &mut store,
            SkillPackApplyInput {
                tenant_slug: "tenant-a".to_string(),
                pack_content_hash: "hash-native-pack".to_string(),
                actor_id: "codex".to_string(),
                context: json!({ "code_member": { "name": "other_function" } }),
                ..SkillPackApplyInput::default()
            },
        )
        .unwrap();

        assert_eq!(receipt.status, "failed");
        assert_eq!(receipt.validator_execution_mode, "native_artifact_sandbox");
        assert_eq!(receipt.validators[0].status, "failed");
        assert!(receipt.validators[0]
            .message
            .contains("expected code_member entrypoint compile_pack"));
    }

    #[test]
    fn apply_skill_pack_fails_native_artifact_missing_content_hash() {
        let mut store = InMemoryGraphStore::new();
        let mut pack = native_artifact_pack();
        pack["metadata"]["artifacts"][0]
            .as_object_mut()
            .unwrap()
            .remove("content_hash");
        publish_skill_pack(
            &mut store,
            SkillPackPublishInput {
                tenant_slug: "tenant-a".to_string(),
                pack,
                ..SkillPackPublishInput::default()
            },
        )
        .unwrap();

        let receipt = apply_skill_pack(
            &mut store,
            SkillPackApplyInput {
                tenant_slug: "tenant-a".to_string(),
                pack_content_hash: "hash-native-pack".to_string(),
                actor_id: "codex".to_string(),
                context: json!({ "code_member": { "name": "compile_pack" } }),
                ..SkillPackApplyInput::default()
            },
        )
        .unwrap();

        assert_eq!(receipt.status, "failed");
        assert_eq!(receipt.validator_execution_mode, "native_artifact_sandbox");
        assert_eq!(
            receipt.validators[0].message,
            "native validator artifact is missing content_hash"
        );
    }

    #[test]
    fn apply_skill_pack_blocks_retired_pack_unless_allowed() {
        let mut store = InMemoryGraphStore::new();
        publish_skill_pack(
            &mut store,
            SkillPackPublishInput {
                tenant_slug: "tenant-a".to_string(),
                status: "retired".to_string(),
                pack: sample_pack(),
                ..SkillPackPublishInput::default()
            },
        )
        .unwrap();

        let blocked = apply_skill_pack(
            &mut store,
            SkillPackApplyInput {
                tenant_slug: "tenant-a".to_string(),
                pack_content_hash: "hash-pack".to_string(),
                actor_id: "codex".to_string(),
                ..SkillPackApplyInput::default()
            },
        )
        .unwrap_err();
        assert!(blocked
            .to_string()
            .contains("refuses retired packs unless allow_retired is true"));

        let allowed = apply_skill_pack(
            &mut store,
            SkillPackApplyInput {
                tenant_slug: "tenant-a".to_string(),
                pack_content_hash: "hash-pack".to_string(),
                actor_id: "codex".to_string(),
                allow_retired: true,
                ..SkillPackApplyInput::default()
            },
        )
        .unwrap();
        assert_eq!(allowed.status, "applied");
    }
}
