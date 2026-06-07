use std::error::Error;
use std::fmt;

use rustyred_thg_core::{EdgeRecord, GraphStoreError, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use theorem_harness_core::stable_value_hash;
use theorem_harness_core::work_graph::TaskNode;

use crate::skill_pack::{
    get_skill_pack, skill_pack_node_id, SkillPackError, SkillPackGetInput, SkillPackGraphStore,
    SkillPackState,
};

pub type NodeTypeBindingResult<T> = Result<T, NodeTypeBindingError>;

pub const NODE_TYPE_BINDING_LABEL: &str = "NodeTypeSkillPackBinding";
pub const EDGE_NODE_TYPE_USES_SKILL_PACK: &str = "NODE_TYPE_USES_SKILL_PACK";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct NodeTypeSkillPackRef {
    #[serde(default)]
    pub pack_id: String,
    #[serde(default)]
    pub pack_content_hash: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub priority: i64,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct BindNodeTypeSkillPacksInput {
    #[serde(default)]
    pub tenant_slug: String,
    pub node_type: String,
    #[serde(default)]
    pub pack_refs: Vec<NodeTypeSkillPackRef>,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub allow_retired: bool,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ResolveNodeTypeSkillPacksInput {
    #[serde(default)]
    pub tenant_slug: String,
    pub node_type: String,
    #[serde(default)]
    pub include_retired: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct NodeTypeSkillPackBindingState {
    pub tenant_slug: String,
    pub node_type: String,
    pub status: String,
    pub pack_refs: Vec<NodeTypeSkillPackRef>,
    pub metadata: Map<String, Value>,
    pub updated_by: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct NodeTypeSkillPackBindingReceipt {
    pub binding: NodeTypeSkillPackBindingState,
    pub edge_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedNodeTypeSkillPack {
    pub role: String,
    pub priority: i64,
    pub required: bool,
    pub pack: SkillPackState,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct NodeTypeSkillPackResolution {
    pub tenant_slug: String,
    pub node_type: String,
    pub packs: Vec<ResolvedNodeTypeSkillPack>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeTypeBindingError {
    InvalidInput { field: String, message: String },
    NotFound { kind: String, id: String },
    Store(String),
    Serialization(String),
    Deserialization(String),
    SkillPack(SkillPackError),
}

impl fmt::Display for NodeTypeBindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput { field, message } => {
                write!(f, "invalid node-type binding input {field}: {message}")
            }
            Self::NotFound { kind, id } => write!(f, "{kind} not found: {id}"),
            Self::Store(message) => write!(f, "store error: {message}"),
            Self::Serialization(message) => write!(f, "serialization error: {message}"),
            Self::Deserialization(message) => write!(f, "deserialization error: {message}"),
            Self::SkillPack(error) => write!(f, "{error}"),
        }
    }
}

impl Error for NodeTypeBindingError {}

impl From<GraphStoreError> for NodeTypeBindingError {
    fn from(value: GraphStoreError) -> Self {
        Self::Store(format!("{}: {}", value.code, value.message))
    }
}

impl From<SkillPackError> for NodeTypeBindingError {
    fn from(value: SkillPackError) -> Self {
        Self::SkillPack(value)
    }
}

pub fn node_type_binding_node_id(tenant: &str, node_type: &str) -> String {
    format!(
        "node_type_skill_pack:{}:{}",
        normalize_tenant(tenant),
        slugify(node_type)
    )
}

pub fn node_type_skill_pack_edge_id(
    tenant: &str,
    node_type: &str,
    pack_content_hash: &str,
) -> String {
    format!(
        "node_type_skill_pack:{}:{}:{}",
        normalize_tenant(tenant),
        slugify(node_type),
        slugify(pack_content_hash)
    )
}

pub fn bind_node_type_skill_packs<S: SkillPackGraphStore>(
    store: &mut S,
    input: BindNodeTypeSkillPacksInput,
) -> NodeTypeBindingResult<NodeTypeSkillPackBindingReceipt> {
    let tenant = normalize_tenant(&input.tenant_slug);
    let node_type = normalize_node_type(&input.node_type)?;
    let status = normalize_status(&input.status)?;
    if input.pack_refs.is_empty() {
        return Err(NodeTypeBindingError::InvalidInput {
            field: "pack_refs".to_string(),
            message: "at least one skill pack ref is required".to_string(),
        });
    }

    let mut pack_refs = Vec::new();
    let mut seen_hashes = std::collections::BTreeSet::new();
    for (index, pack_ref) in input.pack_refs.into_iter().enumerate() {
        let pack = get_skill_pack(
            store,
            SkillPackGetInput {
                tenant_slug: tenant.clone(),
                pack_id: pack_ref.pack_id.clone(),
                pack_content_hash: pack_ref.pack_content_hash.clone(),
            },
        )?;
        if pack.status == "retired" && !input.allow_retired {
            return Err(NodeTypeBindingError::InvalidInput {
                field: "pack_refs".to_string(),
                message: format!(
                    "pack {} is retired; set allow_retired to bind it",
                    pack.pack_id
                ),
            });
        }
        if !seen_hashes.insert(pack.pack_content_hash.clone()) {
            continue;
        }
        pack_refs.push(NodeTypeSkillPackRef {
            pack_id: pack.pack_id,
            pack_content_hash: pack.pack_content_hash,
            role: normalize_role(&pack_ref.role, index),
            priority: pack_ref.priority,
            required: pack_ref.required,
            metadata: pack_ref.metadata,
        });
    }
    pack_refs.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.role.cmp(&right.role))
            .then_with(|| left.pack_id.cmp(&right.pack_id))
    });

    let state = NodeTypeSkillPackBindingState {
        tenant_slug: tenant.clone(),
        node_type: node_type.clone(),
        status,
        pack_refs,
        metadata: input.metadata,
        updated_by: input.actor_id.trim().to_string(),
        updated_at: timestamp_or_now(&input.updated_at),
    };
    store.skill_pack_upsert_node(binding_node(&state)?)?;
    let mut edge_ids = Vec::new();
    for pack_ref in &state.pack_refs {
        let edge = binding_edge(&state, pack_ref);
        edge_ids.push(edge.id.clone());
        store.skill_pack_upsert_edge(edge)?;
    }
    Ok(NodeTypeSkillPackBindingReceipt {
        binding: state,
        edge_ids,
    })
}

pub fn load_node_type_skill_pack_binding<S: SkillPackGraphStore>(
    store: &S,
    tenant_slug: &str,
    node_type: &str,
) -> NodeTypeBindingResult<Option<NodeTypeSkillPackBindingState>> {
    let tenant = normalize_tenant(tenant_slug);
    let node_type = normalize_node_type(node_type)?;
    store
        .skill_pack_get_node(&node_type_binding_node_id(&tenant, &node_type))?
        .map(|node| {
            serde_json::from_value::<NodeTypeSkillPackBindingState>(node.properties)
                .map_err(|error| NodeTypeBindingError::Deserialization(error.to_string()))
        })
        .transpose()
}

pub fn resolve_node_type_skill_packs<S: SkillPackGraphStore>(
    store: &S,
    input: ResolveNodeTypeSkillPacksInput,
) -> NodeTypeBindingResult<NodeTypeSkillPackResolution> {
    let tenant = normalize_tenant(&input.tenant_slug);
    let node_type = normalize_node_type(&input.node_type)?;
    let binding = load_node_type_skill_pack_binding(store, &tenant, &node_type)?.ok_or_else(|| {
        NodeTypeBindingError::NotFound {
            kind: "node_type_skill_pack_binding".to_string(),
            id: node_type_binding_node_id(&tenant, &node_type),
        }
    })?;
    if binding.status != "active" {
        return Err(NodeTypeBindingError::InvalidInput {
            field: "status".to_string(),
            message: format!("binding for {node_type} is {}", binding.status),
        });
    }

    let mut packs = Vec::new();
    for pack_ref in &binding.pack_refs {
        let pack = get_skill_pack(
            store,
            SkillPackGetInput {
                tenant_slug: tenant.clone(),
                pack_id: pack_ref.pack_id.clone(),
                pack_content_hash: pack_ref.pack_content_hash.clone(),
            },
        )?;
        if pack.status == "retired" && !input.include_retired {
            return Err(NodeTypeBindingError::InvalidInput {
                field: "pack_refs".to_string(),
                message: format!("pack {} is retired", pack.pack_id),
            });
        }
        packs.push(ResolvedNodeTypeSkillPack {
            role: pack_ref.role.clone(),
            priority: pack_ref.priority,
            required: pack_ref.required,
            pack,
            metadata: pack_ref.metadata.clone(),
        });
    }
    Ok(NodeTypeSkillPackResolution {
        tenant_slug: tenant,
        node_type,
        packs,
    })
}

pub fn resolve_task_node_skill_packs<S: SkillPackGraphStore>(
    store: &S,
    tenant_slug: &str,
    node: &TaskNode,
) -> NodeTypeBindingResult<NodeTypeSkillPackResolution> {
    resolve_node_type_skill_packs(
        store,
        ResolveNodeTypeSkillPacksInput {
            tenant_slug: tenant_slug.to_string(),
            node_type: node.node_type.clone(),
            include_retired: false,
        },
    )
}

fn binding_node(state: &NodeTypeSkillPackBindingState) -> NodeTypeBindingResult<NodeRecord> {
    let properties = serde_json::to_value(state)
        .map_err(|error| NodeTypeBindingError::Serialization(error.to_string()))?;
    Ok(NodeRecord::new(
        node_type_binding_node_id(&state.tenant_slug, &state.node_type),
        [NODE_TYPE_BINDING_LABEL],
        properties,
    ))
}

fn binding_edge(
    state: &NodeTypeSkillPackBindingState,
    pack_ref: &NodeTypeSkillPackRef,
) -> EdgeRecord {
    EdgeRecord::new(
        node_type_skill_pack_edge_id(
            &state.tenant_slug,
            &state.node_type,
            &pack_ref.pack_content_hash,
        ),
        node_type_binding_node_id(&state.tenant_slug, &state.node_type),
        EDGE_NODE_TYPE_USES_SKILL_PACK,
        skill_pack_node_id(&state.tenant_slug, &pack_ref.pack_content_hash),
        json!({
            "tenant_slug": state.tenant_slug,
            "node_type": state.node_type,
            "pack_content_hash": pack_ref.pack_content_hash,
            "pack_id": pack_ref.pack_id,
            "role": pack_ref.role,
            "priority": pack_ref.priority,
            "required": pack_ref.required,
        }),
    )
}

fn normalize_tenant(tenant: &str) -> String {
    let trimmed = tenant.trim();
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_node_type(node_type: &str) -> NodeTypeBindingResult<String> {
    let trimmed = node_type.trim();
    if trimmed.is_empty() {
        return Err(NodeTypeBindingError::InvalidInput {
            field: "node_type".to_string(),
            message: "node_type is required".to_string(),
        });
    }
    Ok(trimmed.to_string())
}

fn normalize_status(status: &str) -> NodeTypeBindingResult<String> {
    let normalized = status.trim().to_lowercase();
    if normalized.is_empty() {
        return Ok("active".to_string());
    }
    match normalized.as_str() {
        "active" | "retired" => Ok(normalized),
        _ => Err(NodeTypeBindingError::InvalidInput {
            field: "status".to_string(),
            message: "status must be active or retired".to_string(),
        }),
    }
}

fn normalize_role(role: &str, index: usize) -> String {
    let trimmed = role.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    if index == 0 {
        "primary".to_string()
    } else {
        "support".to_string()
    }
}

fn timestamp_or_now(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "unix_ms:0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn slugify(value: &str) -> String {
    let mut out = String::new();
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string().if_empty("unknown")
}

trait IfEmpty {
    fn if_empty(self, fallback: &str) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.trim().is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}

#[allow(dead_code)]
fn binding_hash(state: &NodeTypeSkillPackBindingState) -> NodeTypeBindingResult<String> {
    let value = serde_json::to_value(state)
        .map_err(|error| NodeTypeBindingError::Serialization(error.to_string()))?;
    Ok(stable_value_hash(&value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill_pack::{publish_skill_pack, SkillPackPublishInput};
    use rustyred_thg_core::InMemoryGraphStore;

    fn sample_pack(id: &str, hash: &str, capability: &str, status: &str) -> SkillPackPublishInput {
        SkillPackPublishInput {
            tenant_slug: "tenant-a".to_string(),
            actor_id: "codex".to_string(),
            status: status.to_string(),
            created_at: "t1".to_string(),
            pack: json!({
                "id": id,
                "kind": "skill_pack",
                "title": id,
                "capabilities": [capability],
                "validators": [],
                "metadata": {
                    "pack_content_hash": hash
                }
            }),
            ..SkillPackPublishInput::default()
        }
    }

    #[test]
    fn binding_resolves_node_type_to_ordered_skill_packs() {
        let mut store = InMemoryGraphStore::new();
        publish_skill_pack(
            &mut store,
            sample_pack("rust-engineering", "hash-rust", "rust_impl", "validated"),
        )
        .unwrap();
        publish_skill_pack(
            &mut store,
            sample_pack("fixture-oracle", "hash-oracle", "fixture_check", "validated"),
        )
        .unwrap();

        let receipt = bind_node_type_skill_packs(
            &mut store,
            BindNodeTypeSkillPacksInput {
                tenant_slug: "tenant-a".to_string(),
                node_type: "rust_contract_parity".to_string(),
                actor_id: "codex".to_string(),
                updated_at: "t2".to_string(),
                pack_refs: vec![
                    NodeTypeSkillPackRef {
                        pack_id: "fixture-oracle".to_string(),
                        role: "oracle".to_string(),
                        priority: 20,
                        required: true,
                        ..NodeTypeSkillPackRef::default()
                    },
                    NodeTypeSkillPackRef {
                        pack_content_hash: "hash-rust".to_string(),
                        role: "primary".to_string(),
                        priority: 10,
                        required: true,
                        ..NodeTypeSkillPackRef::default()
                    },
                ],
                ..BindNodeTypeSkillPacksInput::default()
            },
        )
        .unwrap();

        assert_eq!(receipt.binding.pack_refs[0].pack_id, "rust-engineering");
        assert_eq!(receipt.binding.pack_refs[1].pack_id, "fixture-oracle");
        assert_eq!(receipt.edge_ids.len(), 2);
        assert!(store
            .get_node(&node_type_binding_node_id(
                "tenant-a",
                "rust_contract_parity"
            ))
            .is_some());

        let resolution = resolve_node_type_skill_packs(
            &store,
            ResolveNodeTypeSkillPacksInput {
                tenant_slug: "tenant-a".to_string(),
                node_type: "rust_contract_parity".to_string(),
                ..ResolveNodeTypeSkillPacksInput::default()
            },
        )
        .unwrap();
        assert_eq!(resolution.packs.len(), 2);
        assert_eq!(resolution.packs[0].role, "primary");
        assert_eq!(resolution.packs[0].pack.pack_id, "rust-engineering");
        assert_eq!(resolution.packs[1].role, "oracle");
    }

    #[test]
    fn task_node_resolution_uses_work_graph_node_type() {
        let mut store = InMemoryGraphStore::new();
        publish_skill_pack(
            &mut store,
            sample_pack("browser-playbook", "hash-browser", "browser_use", "validated"),
        )
        .unwrap();
        bind_node_type_skill_packs(
            &mut store,
            BindNodeTypeSkillPacksInput {
                tenant_slug: "tenant-a".to_string(),
                node_type: "browser_use_surface".to_string(),
                pack_refs: vec![NodeTypeSkillPackRef {
                    pack_id: "browser-playbook".to_string(),
                    required: true,
                    ..NodeTypeSkillPackRef::default()
                }],
                ..BindNodeTypeSkillPacksInput::default()
            },
        )
        .unwrap();

        let node = TaskNode::open(
            "n1",
            "run-1",
            "browser_use_surface",
            "validate browser surface",
            "seed",
        );
        let resolution = resolve_task_node_skill_packs(&store, "tenant-a", &node).unwrap();
        assert_eq!(resolution.node_type, "browser_use_surface");
        assert_eq!(resolution.packs[0].pack.pack_id, "browser-playbook");
    }

    #[test]
    fn binding_rejects_missing_pack_and_retired_pack_by_default() {
        let mut store = InMemoryGraphStore::new();
        let missing = bind_node_type_skill_packs(
            &mut store,
            BindNodeTypeSkillPacksInput {
                tenant_slug: "tenant-a".to_string(),
                node_type: "peer_refute_patch".to_string(),
                pack_refs: vec![NodeTypeSkillPackRef {
                    pack_id: "missing".to_string(),
                    ..NodeTypeSkillPackRef::default()
                }],
                ..BindNodeTypeSkillPacksInput::default()
            },
        );
        assert!(matches!(
            missing,
            Err(NodeTypeBindingError::SkillPack(SkillPackError::NotFound { .. }))
        ));

        publish_skill_pack(
            &mut store,
            sample_pack("old-review", "hash-review", "review", "retired"),
        )
        .unwrap();
        let retired = bind_node_type_skill_packs(
            &mut store,
            BindNodeTypeSkillPacksInput {
                tenant_slug: "tenant-a".to_string(),
                node_type: "peer_refute_patch".to_string(),
                pack_refs: vec![NodeTypeSkillPackRef {
                    pack_id: "old-review".to_string(),
                    ..NodeTypeSkillPackRef::default()
                }],
                ..BindNodeTypeSkillPacksInput::default()
            },
        );
        assert!(matches!(
            retired,
            Err(NodeTypeBindingError::InvalidInput { field, .. }) if field == "pack_refs"
        ));
    }

    #[test]
    fn inactive_binding_does_not_resolve() {
        let mut store = InMemoryGraphStore::new();
        publish_skill_pack(
            &mut store,
            sample_pack("review", "hash-review", "review", "validated"),
        )
        .unwrap();
        bind_node_type_skill_packs(
            &mut store,
            BindNodeTypeSkillPacksInput {
                tenant_slug: "tenant-a".to_string(),
                node_type: "peer_refute_patch".to_string(),
                status: "retired".to_string(),
                pack_refs: vec![NodeTypeSkillPackRef {
                    pack_id: "review".to_string(),
                    ..NodeTypeSkillPackRef::default()
                }],
                ..BindNodeTypeSkillPacksInput::default()
            },
        )
        .unwrap();

        let resolution = resolve_node_type_skill_packs(
            &store,
            ResolveNodeTypeSkillPacksInput {
                tenant_slug: "tenant-a".to_string(),
                node_type: "peer_refute_patch".to_string(),
                ..ResolveNodeTypeSkillPacksInput::default()
            },
        );
        assert!(matches!(
            resolution,
            Err(NodeTypeBindingError::InvalidInput { field, .. }) if field == "status"
        ));
    }
}
