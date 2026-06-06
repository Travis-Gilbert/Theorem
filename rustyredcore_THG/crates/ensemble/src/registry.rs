//! The capability-pack registry: content-addressed storage of `CapabilityPack`s of all
//! kinds, generalizing the `theorem-harness-runtime::skill_pack` storage pattern beyond the
//! single `skill_pack` kind.

use rustyred_thg_core::{EdgeRecord, GraphStore, GraphStoreError, NodeQuery, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use theorem_harness_core::stable_value_hash;

pub const PACK_LABEL: &str = "CapabilityPack";
pub const PACK_SOURCE_EDGE: &str = "PACK_SOURCE";
pub const PACK_ARTIFACT_EDGE: &str = "PACK_ARTIFACT";
const PACK_QUERY_LIMIT: usize = 10_000;

/// The kind discriminator carried by a `CapabilityPackSpec` JSON contract. `CapabilityPackSpec`
/// is a JSON shape (not a Rust struct) produced by the offline encode pipeline; the registry
/// stores the spec verbatim and keys behaviour off this kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackKind {
    Skill,
    Agent,
    Tool,
    Validator,
    Renderer,
    Compute,
    Policy,
    Domain,
    Context,
}

impl PackKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PackKind::Skill => "skill",
            PackKind::Agent => "agent",
            PackKind::Tool => "tool",
            PackKind::Validator => "validator",
            PackKind::Renderer => "renderer",
            PackKind::Compute => "compute",
            PackKind::Policy => "policy",
            PackKind::Domain => "domain",
            PackKind::Context => "context",
        }
    }

    /// Accepts the snake_case kind plus the `skill_pack` alias used by the existing
    /// skill-pack serving slice.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "skill" | "skill_pack" => Some(PackKind::Skill),
            "agent" => Some(PackKind::Agent),
            "tool" => Some(PackKind::Tool),
            "validator" => Some(PackKind::Validator),
            "renderer" => Some(PackKind::Renderer),
            "compute" => Some(PackKind::Compute),
            "policy" => Some(PackKind::Policy),
            "domain" => Some(PackKind::Domain),
            "context" => Some(PackKind::Context),
            _ => None,
        }
    }
}

/// Trust ladder. Packs start `Unverified` and climb to `FirstParty` with a passport id.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tier", rename_all = "snake_case")]
pub enum TrustTier {
    #[default]
    Unverified,
    FirstParty {
        passport_id: String,
    },
}

/// Exposure flag: packs stay hidden behind the single Ensemble surface
/// (`visible_to_agent = false`, `exposed_through = "ensemble"`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PackExposure {
    #[serde(default)]
    pub visible_to_agent: bool,
    #[serde(default = "default_exposed_through")]
    pub exposed_through: String,
}

fn default_exposed_through() -> String {
    "ensemble".to_string()
}

impl Default for PackExposure {
    fn default() -> Self {
        Self {
            visible_to_agent: false,
            exposed_through: default_exposed_through(),
        }
    }
}

/// A registered capability pack: the content-addressed registry entry persisted to the graph.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityPack {
    pub tenant_slug: String,
    #[serde(default)]
    pub pack_content_hash: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// The full `CapabilityPackSpec` JSON contract, stored verbatim.
    pub spec: Value,
    #[serde(default)]
    pub trust: TrustTier,
    #[serde(default)]
    pub exposure: PackExposure,
    #[serde(default)]
    pub source_content_hash: String,
    #[serde(default)]
    pub artifact_hashes: Vec<String>,
}

/// Registry errors.
#[derive(Debug)]
pub enum EnsembleError {
    Store(GraphStoreError),
    InvalidPack(String),
}

impl std::fmt::Display for EnsembleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnsembleError::Store(e) => write!(f, "ensemble graph store error: {e:?}"),
            EnsembleError::InvalidPack(m) => write!(f, "invalid capability pack: {m}"),
        }
    }
}

impl std::error::Error for EnsembleError {}

impl From<GraphStoreError> for EnsembleError {
    fn from(value: GraphStoreError) -> Self {
        EnsembleError::Store(value)
    }
}

pub type EnsembleResult<T> = Result<T, EnsembleError>;

/// Blanket trait over any `GraphStore`, mirroring `theorem-harness-runtime::SkillPackGraphStore`
/// so the registry stays decoupled from the concrete store impl.
pub trait EnsembleGraphStore {
    fn pack_upsert_node(&mut self, node: NodeRecord) -> EnsembleResult<()>;
    fn pack_upsert_edge(&mut self, edge: EdgeRecord) -> EnsembleResult<()>;
    fn pack_get_node(&self, id: &str) -> EnsembleResult<Option<NodeRecord>>;
    fn pack_query_nodes(&self, query: NodeQuery) -> EnsembleResult<Vec<NodeRecord>>;
}

impl<T: GraphStore> EnsembleGraphStore for T {
    fn pack_upsert_node(&mut self, node: NodeRecord) -> EnsembleResult<()> {
        self.upsert_node(node)
            .map(|_| ())
            .map_err(EnsembleError::from)
    }

    fn pack_upsert_edge(&mut self, edge: EdgeRecord) -> EnsembleResult<()> {
        self.upsert_edge(edge)
            .map(|_| ())
            .map_err(EnsembleError::from)
    }

    fn pack_get_node(&self, id: &str) -> EnsembleResult<Option<NodeRecord>> {
        Ok(self.get_node(id).cloned())
    }

    fn pack_query_nodes(&self, query: NodeQuery) -> EnsembleResult<Vec<NodeRecord>> {
        Ok(self.query_nodes(query))
    }
}

fn normalize_tenant(tenant: &str) -> String {
    let trimmed = tenant.trim();
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Deterministic node id for a registered pack.
pub fn pack_node_id(tenant: &str, pack_content_hash: &str) -> String {
    format!(
        "capability_pack:{}:{}",
        normalize_tenant(tenant),
        pack_content_hash.trim()
    )
}

/// Register a capability pack of any kind. Content-addresses the spec via `stable_value_hash`
/// when no hash is supplied, derives the `kind` from the spec, persists the pack node plus
/// `PACK_SOURCE` / `PACK_ARTIFACT` hash nodes and edges, and returns the normalized pack.
/// Idempotent on content hash.
pub fn register_pack<S: EnsembleGraphStore>(
    store: &mut S,
    mut pack: CapabilityPack,
) -> EnsembleResult<CapabilityPack> {
    if pack.kind.trim().is_empty() {
        pack.kind = pack
            .spec
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
    }
    let Some(kind) = PackKind::parse(&pack.kind) else {
        return Err(EnsembleError::InvalidPack(format!(
            "unknown or missing pack kind: {:?}",
            pack.kind
        )));
    };
    pack.kind = kind.as_str().to_string();
    if pack.pack_content_hash.trim().is_empty() {
        pack.pack_content_hash = stable_value_hash(&pack.spec);
    }
    let tenant = normalize_tenant(&pack.tenant_slug);
    pack.tenant_slug = tenant.clone();
    if pack.title.trim().is_empty() {
        pack.title = text_at(&pack.spec, &["title", "name"]);
    }
    if pack.description.trim().is_empty() {
        pack.description = text_at(&pack.spec, &["description", "summary"]);
    }

    let node_id = pack_node_id(&tenant, &pack.pack_content_hash);
    let properties =
        serde_json::to_value(&pack).map_err(|e| EnsembleError::InvalidPack(e.to_string()))?;
    let labels = vec![PACK_LABEL.to_string(), format!("PackKind:{}", pack.kind)];
    store.pack_upsert_node(NodeRecord::new(node_id.clone(), labels, properties))?;

    if !pack.source_content_hash.trim().is_empty() {
        let src_id = format!(
            "capability_pack_source:{}:{}",
            tenant,
            pack.source_content_hash.trim()
        );
        store.pack_upsert_node(NodeRecord::new(
            src_id.clone(),
            vec!["CapabilityPackSource".to_string()],
            json!({ "content_hash": pack.source_content_hash }),
        ))?;
        store.pack_upsert_edge(EdgeRecord::new(
            format!("{node_id}|{PACK_SOURCE_EDGE}|{src_id}"),
            node_id.clone(),
            PACK_SOURCE_EDGE,
            src_id,
            json!({}),
        ))?;
    }

    for artifact in &pack.artifact_hashes {
        let art = artifact.trim();
        if art.is_empty() {
            continue;
        }
        let art_id = format!("capability_pack_artifact:{tenant}:{art}");
        store.pack_upsert_node(NodeRecord::new(
            art_id.clone(),
            vec!["CapabilityPackArtifact".to_string()],
            json!({ "content_hash": art }),
        ))?;
        store.pack_upsert_edge(EdgeRecord::new(
            format!("{node_id}|{PACK_ARTIFACT_EDGE}|{art_id}"),
            node_id.clone(),
            PACK_ARTIFACT_EDGE,
            art_id,
            json!({}),
        ))?;
    }

    Ok(pack)
}

/// Fetch a registered pack by tenant + content hash.
pub fn get_pack<S: EnsembleGraphStore>(
    store: &S,
    tenant: &str,
    pack_content_hash: &str,
) -> EnsembleResult<Option<CapabilityPack>> {
    let node_id = pack_node_id(tenant, pack_content_hash);
    match store.pack_get_node(&node_id)? {
        Some(node) => {
            let pack: CapabilityPack = serde_json::from_value(node.properties)
                .map_err(|e| EnsembleError::InvalidPack(e.to_string()))?;
            Ok(Some(pack))
        }
        None => Ok(None),
    }
}

/// List registered packs for a tenant, optionally filtered to a single [`PackKind`]. Queries by the
/// `CapabilityPack` label, deserializes each node, keeps the requested tenant (and kind), and orders
/// the result by `pack_content_hash` ascending so callers (notably the selector) see a deterministic
/// candidate order. Nodes that fail to deserialize as a `CapabilityPack` are skipped rather than
/// failing the whole listing.
pub fn list_packs<S: EnsembleGraphStore>(
    store: &S,
    tenant: &str,
    kind: Option<PackKind>,
) -> EnsembleResult<Vec<CapabilityPack>> {
    let want_tenant = normalize_tenant(tenant);
    let nodes =
        store.pack_query_nodes(NodeQuery::label(PACK_LABEL).with_limit(PACK_QUERY_LIMIT))?;
    let mut packs: Vec<CapabilityPack> = nodes
        .into_iter()
        .filter_map(|node| serde_json::from_value::<CapabilityPack>(node.properties).ok())
        .filter(|pack| normalize_tenant(&pack.tenant_slug) == want_tenant)
        .filter(|pack| match kind {
            Some(want) => PackKind::parse(&pack.kind) == Some(want),
            None => true,
        })
        .collect();
    packs.sort_by(|a, b| a.pack_content_hash.cmp(&b.pack_content_hash));
    Ok(packs)
}

fn text_at(spec: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(s) = spec.get(*key).and_then(Value::as_str) {
            if !s.trim().is_empty() {
                return s.trim().to_string();
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::InMemoryGraphStore;

    fn skill_spec() -> Value {
        json!({
            "kind": "skill",
            "title": "Rust Engineering",
            "description": "Write and review Rust",
            "capabilities": ["rust", "cargo"]
        })
    }

    fn pack_from(spec: Value) -> CapabilityPack {
        CapabilityPack {
            tenant_slug: "default".to_string(),
            pack_content_hash: String::new(),
            kind: String::new(),
            title: String::new(),
            description: String::new(),
            spec,
            trust: TrustTier::default(),
            exposure: PackExposure::default(),
            source_content_hash: String::new(),
            artifact_hashes: vec![],
        }
    }

    #[test]
    fn register_then_get_round_trip() {
        let mut store = InMemoryGraphStore::new();
        let mut pack = pack_from(skill_spec());
        pack.source_content_hash = "src123".to_string();
        pack.artifact_hashes = vec!["art1".to_string()];

        let registered = register_pack(&mut store, pack).expect("register");
        assert_eq!(registered.kind, "skill");
        assert!(!registered.pack_content_hash.is_empty());
        assert_eq!(registered.title, "Rust Engineering");

        let fetched = get_pack(&store, "default", &registered.pack_content_hash)
            .expect("get")
            .expect("present");
        assert_eq!(fetched.pack_content_hash, registered.pack_content_hash);
        assert_eq!(fetched.kind, "skill");
        assert_eq!(fetched.source_content_hash, "src123");
        assert_eq!(fetched.artifact_hashes, vec!["art1".to_string()]);
    }

    #[test]
    fn register_normalizes_skill_pack_alias() {
        let mut store = InMemoryGraphStore::new();
        let registered = register_pack(
            &mut store,
            pack_from(json!({
                "kind": "skill_pack",
                "title": "Skill alias",
            })),
        )
        .expect("register");

        assert_eq!(registered.kind, "skill");
    }

    #[test]
    fn content_hash_is_deterministic() {
        let mut a = InMemoryGraphStore::new();
        let mut b = InMemoryGraphStore::new();
        let ra = register_pack(&mut a, pack_from(skill_spec())).unwrap();
        let rb = register_pack(&mut b, pack_from(skill_spec())).unwrap();
        assert_eq!(ra.pack_content_hash, rb.pack_content_hash);
    }

    #[test]
    fn rejects_unknown_kind() {
        let mut store = InMemoryGraphStore::new();
        let pack = pack_from(json!({ "kind": "nonsense" }));
        assert!(register_pack(&mut store, pack).is_err());
    }

    #[test]
    fn list_packs_reads_beyond_default_graph_query_limit() {
        let mut store = InMemoryGraphStore::new();
        for index in 0..125 {
            let pack = pack_from(json!({
                "kind": "skill",
                "title": format!("Pack {index}"),
            }));
            register_pack(&mut store, pack).expect("register");
        }

        let packs = list_packs(&store, "default", Some(PackKind::Skill)).expect("list");
        assert_eq!(packs.len(), 125);
    }
}
