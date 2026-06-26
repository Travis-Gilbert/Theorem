//! The capability-pack registry: content-addressed storage of `CapabilityPack`s of all
//! kinds, generalizing the `theorem-harness-runtime::skill_pack` storage pattern beyond the
//! single `skill_pack` kind.

use rustyred_thg_affordances::affordance_node_id;
use rustyred_thg_core::{
    EdgeRecord, GraphSnapshot, GraphStore, GraphStoreError, NodeQuery, NodeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use theorem_harness_core::{stable_map_id, stable_value_hash};

pub const PACK_LABEL: &str = "CapabilityPack";
pub const PACK_SOURCE_EDGE: &str = "PACK_SOURCE";
pub const PACK_ARTIFACT_EDGE: &str = "PACK_ARTIFACT";
pub const PACK_EXPOSES_AFFORDANCE: &str = "PACK_EXPOSES_AFFORDANCE";
pub const PACK_IN_DOMAIN: &str = "PACK_IN_DOMAIN";
pub const DOMAIN_MAP_LABEL: &str = "DomainMap";
const DEFAULT_TENANT: &str = "default";
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
    pub origin_tenant_slug: String,
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
    fn pack_graph_snapshot(&self) -> EnsembleResult<Option<GraphSnapshot>> {
        Ok(None)
    }
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

    fn pack_graph_snapshot(&self) -> EnsembleResult<Option<GraphSnapshot>> {
        Ok(Some(self.graph_snapshot()?))
    }
}

fn normalize_tenant(tenant: &str) -> String {
    let trimmed = tenant.trim();
    if trimmed.is_empty() {
        DEFAULT_TENANT.to_string()
    } else {
        trimmed.to_string()
    }
}

fn read_tenant_order(tenant: &str) -> Vec<String> {
    let tenant = normalize_tenant(tenant);
    if tenant == DEFAULT_TENANT {
        vec![tenant]
    } else {
        vec![tenant, DEFAULT_TENANT.to_string()]
    }
}

fn tenant_priority(tenants: &[String], tenant: &str) -> usize {
    let tenant = normalize_tenant(tenant);
    tenants
        .iter()
        .position(|candidate| candidate == &tenant)
        .unwrap_or(usize::MAX)
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
    if pack.origin_tenant_slug.trim().is_empty() {
        pack.origin_tenant_slug = tenant.clone();
    }
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

    for affordance_id in pack_affordance_refs(&pack) {
        let affordance_node = affordance_node_id(&tenant, &affordance_id);
        if store.pack_get_node(&affordance_node)?.is_some() {
            store.pack_upsert_edge(EdgeRecord::new(
                format!("{node_id}|{PACK_EXPOSES_AFFORDANCE}|{affordance_node}"),
                node_id.clone(),
                PACK_EXPOSES_AFFORDANCE,
                affordance_node,
                json!({
                    "tenant_slug": tenant,
                    "affordance_id": affordance_id,
                    "source": "ensemble_registry",
                }),
            ))?;
        }
    }

    for domain_ref in pack_domain_refs(&pack) {
        let domain_node = domain_node_id(&tenant, &domain_ref);
        if store.pack_get_node(&domain_node)?.is_none() {
            store.pack_upsert_node(NodeRecord::new(
                domain_node.clone(),
                [DOMAIN_MAP_LABEL],
                json!({
                    "tenant_slug": tenant,
                    "scope_kind": "domain",
                    "scope_ref": domain_ref,
                    "map_kind": "DomainMap",
                    "source": "ensemble_registry",
                }),
            ))?;
        }
        store.pack_upsert_edge(EdgeRecord::new(
            format!("{node_id}|{PACK_IN_DOMAIN}|{domain_node}"),
            node_id.clone(),
            PACK_IN_DOMAIN,
            domain_node,
            json!({
                "tenant_slug": tenant,
                "domain_ref": domain_ref,
                "source": "ensemble_registry",
            }),
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
    for candidate_tenant in read_tenant_order(tenant) {
        let node_id = pack_node_id(&candidate_tenant, pack_content_hash);
        if let Some(node) = store.pack_get_node(&node_id)? {
            return Ok(Some(pack_from_node(node)?));
        }
    }
    Ok(None)
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
    let tenants = read_tenant_order(tenant);
    let nodes =
        store.pack_query_nodes(NodeQuery::label(PACK_LABEL).with_limit(PACK_QUERY_LIMIT))?;
    let mut packs: Vec<CapabilityPack> = nodes
        .into_iter()
        .filter_map(|node| pack_from_node(node).ok())
        .filter(|pack| tenants.contains(&normalize_tenant(&pack.tenant_slug)))
        .filter(|pack| match kind {
            Some(want) => PackKind::parse(&pack.kind) == Some(want),
            None => true,
        })
        .collect();
    packs.sort_by(|a, b| {
        tenant_priority(&tenants, &a.tenant_slug)
            .cmp(&tenant_priority(&tenants, &b.tenant_slug))
            .then_with(|| a.pack_content_hash.cmp(&b.pack_content_hash))
    });
    let mut seen_hashes = std::collections::BTreeSet::new();
    packs.retain(|pack| seen_hashes.insert(pack.pack_content_hash.clone()));
    Ok(packs)
}

fn pack_from_node(node: NodeRecord) -> EnsembleResult<CapabilityPack> {
    let mut pack: CapabilityPack = serde_json::from_value(node.properties)
        .map_err(|e| EnsembleError::InvalidPack(e.to_string()))?;
    if pack.origin_tenant_slug.trim().is_empty() {
        pack.origin_tenant_slug = pack.tenant_slug.clone();
    }
    Ok(pack)
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

pub fn domain_node_id(tenant: &str, domain_ref: &str) -> String {
    let trimmed = domain_ref.trim();
    if trimmed.starts_with("capability_pack:") || trimmed.starts_with("map:") {
        trimmed.to_string()
    } else {
        stable_map_id(
            "DomainMap",
            "domain",
            &format!("{}:{trimmed}", normalize_tenant(tenant)),
        )
    }
}

fn pack_affordance_refs(pack: &CapabilityPack) -> Vec<String> {
    string_refs_at_any(
        &pack.spec,
        &[
            &["affordance_ids"][..],
            &["exposes_affordances"][..],
            &["tool_affordance_ids"][..],
            &["exposes", "affordance_ids"][..],
            &["exposes", "tools"][..],
        ],
    )
}

fn pack_domain_refs(pack: &CapabilityPack) -> Vec<String> {
    if PackKind::parse(&pack.kind) == Some(PackKind::Domain) {
        return Vec::new();
    }
    string_refs_at_any(
        &pack.spec,
        &[
            &["domain"][..],
            &["domain_ref"][..],
            &["domain_refs"][..],
            &["domains"][..],
            &["scope", "domain"][..],
            &["scope", "domain_ref"][..],
            &["scope", "domain_refs"][..],
        ],
    )
}

fn string_refs_at_any(spec: &Value, paths: &[&[&str]]) -> Vec<String> {
    let mut refs = std::collections::BTreeSet::new();
    for path in paths {
        if let Some(value) = value_at_path(spec, path) {
            collect_string_refs(value, &mut refs);
        }
    }
    refs.into_iter().collect()
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    Some(cursor)
}

fn collect_string_refs(value: &Value, refs: &mut std::collections::BTreeSet<String>) {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                refs.insert(trimmed.to_string());
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_string_refs(value, refs);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_affordances::{
        register_connector, ConnectorManifest, ToolManifest, CONNECTOR_FAMILY,
    };
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
            origin_tenant_slug: String::new(),
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

    fn connector_with_search_tool() -> ConnectorManifest {
        ConnectorManifest {
            tenant_id: "default".to_string(),
            server_id: "github".to_string(),
            label: "GitHub".to_string(),
            tools: vec![ToolManifest {
                name: "search_code".to_string(),
                label: String::new(),
                description: "search code".to_string(),
                family: CONNECTOR_FAMILY.to_string(),
                input_schema: json!({}),
                permissions: vec![],
                cost: json!({}),
                writeback_policy: "read-only".to_string(),
                tags: vec![],
                description_embedding: None,
            }],
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

    #[test]
    fn list_packs_unions_personal_tenant_with_default_commons() {
        let mut store = InMemoryGraphStore::new();
        register_pack(&mut store, pack_from(skill_spec())).unwrap();

        let mut personal = pack_from(json!({
            "kind": "skill",
            "title": "Tenant Skill",
            "capabilities": ["private"]
        }));
        personal.tenant_slug = "tenant-a".to_string();
        personal.pack_content_hash = "personal-hash".to_string();
        register_pack(&mut store, personal).unwrap();

        let packs = list_packs(&store, "tenant-a", Some(PackKind::Skill)).expect("list");
        assert_eq!(packs.len(), 2);
        assert_eq!(packs[0].tenant_slug, "tenant-a");
        assert_eq!(packs[1].tenant_slug, "default");
        assert_eq!(packs[1].origin_tenant_slug, "default");

        let default_only = list_packs(&store, "default", Some(PackKind::Skill)).expect("list");
        assert_eq!(default_only.len(), 1);
        assert_eq!(default_only[0].tenant_slug, "default");
    }

    #[test]
    fn get_pack_falls_back_to_default_commons() {
        let mut store = InMemoryGraphStore::new();
        let registered = register_pack(&mut store, pack_from(skill_spec())).unwrap();

        let fetched = get_pack(&store, "tenant-a", &registered.pack_content_hash)
            .expect("get")
            .expect("present");
        assert_eq!(fetched.tenant_slug, "default");
        assert_eq!(fetched.origin_tenant_slug, "default");
    }

    #[test]
    fn register_pack_links_existing_affordances_and_domains() {
        let mut store = InMemoryGraphStore::new();
        register_connector(&mut store, connector_with_search_tool(), Some("test")).unwrap();

        let registered = register_pack(
            &mut store,
            pack_from(json!({
                "kind": "skill",
                "title": "Code search pack",
                "affordance_ids": ["github.search_code"],
                "domain_refs": ["code"]
            })),
        )
        .expect("register");

        let pack_node = pack_node_id("default", &registered.pack_content_hash);
        let affordance_node = affordance_node_id("default", "github.search_code");
        let expose_edge = format!("{pack_node}|{PACK_EXPOSES_AFFORDANCE}|{affordance_node}");
        assert!(store.get_edge(&expose_edge).is_some());

        let domain_node = domain_node_id("default", "code");
        let domain_edge = format!("{pack_node}|{PACK_IN_DOMAIN}|{domain_node}");
        assert!(store.get_node(&domain_node).is_some());
        assert!(store.get_edge(&domain_edge).is_some());
    }
}
