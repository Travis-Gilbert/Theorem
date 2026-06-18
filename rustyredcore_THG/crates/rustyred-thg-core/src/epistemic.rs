use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::graph::personalized_pagerank;
use crate::graph_store::{
    now_ms, EdgeRecord, EpistemicType, GraphStore, GraphStoreError, GraphStoreResult,
    NeighborQuery, NodeQuery, NodeRecord, Provenance,
};
use crate::state::stable_hash;

pub const EPISTEMIC_SHADOW_LABEL: &str = "EpistemicShadow";
pub const HAS_EPISTEMIC_SHADOW: &str = "HasEpistemicShadow";
pub const UNDERCUTS: &str = "Undercuts";
pub const EPISTEMIC_SUPPORTS: &str = "Supports";
pub const SAME_ECLASS: &str = "SameEClass";
pub const DEFAULT_EPISTEMIC_ENGINE_VERSION: &str = "epistemic-v1";
pub const STRUCTURAL_EPISTEMIC_ENGINE: &str = "rustyred-thg-core.structural_epistemic";
pub const LEARNED_EPISTEMIC_ENGINE: &str = "theseus.epistemic_enrichment";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EpistemicSourceKind {
    Structural,
    Learned,
    Mixed,
}

impl EpistemicSourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Structural => "structural",
            Self::Learned => "learned",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundedExtensionStatus {
    In,
    Out,
    Undecided,
}

impl GroundedExtensionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
            Self::Undecided => "undecided",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PredictedEdgePointer {
    pub target_content_id: String,
    pub relation: String,
    pub confidence: f64,
    #[serde(default = "default_true")]
    pub quarantine: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SourceReliability {
    pub alpha: f64,
    pub beta: f64,
    pub mean: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EpistemicFieldProvenance {
    pub source_kind: EpistemicSourceKind,
    pub engine: String,
    pub engine_version: String,
    pub computed_at: i64,
}

impl EpistemicFieldProvenance {
    pub fn structural(config: &StructuralEpistemicConfig) -> Self {
        Self {
            source_kind: EpistemicSourceKind::Structural,
            engine: config.engine.clone(),
            engine_version: config.engine_version.clone(),
            computed_at: config.computed_at,
        }
    }

    pub fn learned(config: &EpistemicCronInput) -> Self {
        Self {
            source_kind: EpistemicSourceKind::Learned,
            engine: config.engine.clone(),
            engine_version: config.engine_version.clone(),
            computed_at: config.computed_at,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct EpistemicReadout {
    pub shadows: Vec<EpistemicShadowReadout>,
    pub contradictions: Vec<EpistemicRelationReadout>,
    pub unsupported: Vec<String>,
    pub orphans: Vec<String>,
    pub chokepoints: Vec<EpistemicChokepoint>,
    pub checked_pair_count: usize,
    pub candidate_pair_bound: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EpistemicShadowReadout {
    pub content_node_id: String,
    pub shadow_node_id: String,
    pub grounded_extension_status: GroundedExtensionStatus,
    pub support_in_degree: u64,
    pub attack_in_degree: u64,
    pub unsupported_leaf: bool,
    pub orphan: bool,
    pub bridge_score: f64,
    pub contradiction_cycle_id: Option<String>,
    pub predicted_edges: Vec<PredictedEdgePointer>,
    pub completion_confidence: Option<f64>,
    pub structural_role_vector: Vec<f32>,
    pub source_reliability: Option<SourceReliability>,
    pub community_id: Option<String>,
    pub source_kind: EpistemicSourceKind,
    pub engine: String,
    pub engine_version: String,
    pub computed_at: i64,
    pub quarantine: bool,
    pub field_provenance: BTreeMap<String, EpistemicFieldProvenance>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EpistemicRelationKind {
    Supports,
    Undercuts,
}

impl EpistemicRelationKind {
    pub fn edge_type(&self) -> &'static str {
        match self {
            Self::Supports => EPISTEMIC_SUPPORTS,
            Self::Undercuts => UNDERCUTS,
        }
    }

    fn epistemic_type(&self) -> EpistemicType {
        match self {
            Self::Supports => EpistemicType::Supports,
            Self::Undercuts => EpistemicType::Contradicts,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EpistemicRelationInput {
    pub from_content_id: String,
    pub to_content_id: String,
    pub kind: EpistemicRelationKind,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub evidence: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EpistemicCandidatePair {
    pub left_content_id: String,
    pub right_content_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EpistemicRelationReadout {
    pub from_content_id: String,
    pub to_content_id: String,
    pub from_shadow_id: String,
    pub to_shadow_id: String,
    pub kind: EpistemicRelationKind,
    pub confidence: f64,
    pub evidence: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EpistemicChokepoint {
    pub content_node_id: String,
    pub shadow_node_id: String,
    pub bridge_score: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StructuralEpistemicConfig {
    pub engine: String,
    pub engine_version: String,
    pub computed_at: i64,
    pub support_edge_types: Vec<String>,
    pub attack_edge_types: Vec<String>,
    pub candidate_top_k: usize,
}

impl Default for StructuralEpistemicConfig {
    fn default() -> Self {
        Self {
            engine: STRUCTURAL_EPISTEMIC_ENGINE.to_string(),
            engine_version: DEFAULT_EPISTEMIC_ENGINE_VERSION.to_string(),
            computed_at: now_ms(),
            support_edge_types: vec![
                EPISTEMIC_SUPPORTS.to_string(),
                "supports".to_string(),
                "SUPPORTS".to_string(),
                "CITES".to_string(),
                "DERIVED_FROM".to_string(),
            ],
            attack_edge_types: vec![
                UNDERCUTS.to_string(),
                "CONTRADICTS".to_string(),
                "contradicts".to_string(),
                "ATTACKS".to_string(),
                "attacks".to_string(),
            ],
            candidate_top_k: 8,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct StructuralEpistemicInput {
    pub batch_node_ids: Vec<String>,
    #[serde(default)]
    pub candidate_pairs: Vec<EpistemicCandidatePair>,
    #[serde(default)]
    pub explicit_relations: Vec<EpistemicRelationInput>,
    #[serde(default)]
    pub config: StructuralEpistemicConfig,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EpistemicEnrichmentMode {
    Delta,
    Full,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct UserSubgraph {
    pub nodes: Vec<NodeRecord>,
    pub edges: Vec<EdgeRecord>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct EpistemicAnnotations {
    pub annotations: Vec<EpistemicAnnotation>,
    #[serde(default)]
    pub support_relations: Vec<EpistemicRelationInput>,
    #[serde(default)]
    pub attack_relations: Vec<EpistemicRelationInput>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct EpistemicAnnotation {
    pub content_node_id: String,
    #[serde(default)]
    pub predicted_edges: Vec<PredictedEdgePointer>,
    #[serde(default)]
    pub completion_confidence: Option<f64>,
    #[serde(default)]
    pub structural_role_vector: Vec<f32>,
    #[serde(default)]
    pub source_reliability: Option<SourceReliability>,
    #[serde(default)]
    pub community_id: Option<String>,
    #[serde(default)]
    pub grounded_extension_status: Option<GroundedExtensionStatus>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EpistemicCronInput {
    pub content_node_ids: Vec<String>,
    pub mode: EpistemicEnrichmentMode,
    pub engine: String,
    pub engine_version: String,
    pub computed_at: i64,
    pub density_floor: f64,
}

impl Default for EpistemicCronInput {
    fn default() -> Self {
        Self {
            content_node_ids: Vec::new(),
            mode: EpistemicEnrichmentMode::Delta,
            engine: LEARNED_EPISTEMIC_ENGINE.to_string(),
            engine_version: DEFAULT_EPISTEMIC_ENGINE_VERSION.to_string(),
            computed_at: now_ms(),
            density_floor: 0.0,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct EpistemicCronReport {
    pub attempted: bool,
    pub no_op: bool,
    pub grpc_ok: bool,
    pub skipped_reason: String,
    pub annotations_received: usize,
    pub shadows_written: usize,
    pub shadow_edges_written: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EpistemicEnrichmentError {
    pub code: String,
    pub message: String,
}

impl EpistemicEnrichmentError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

pub trait EpistemicEnricher {
    fn enrich(
        &self,
        subgraph: UserSubgraph,
        mode: EpistemicEnrichmentMode,
    ) -> Result<EpistemicAnnotations, EpistemicEnrichmentError>;
}

pub fn epistemic_shadow_node_id(content_node_id: &str, engine_version: &str) -> String {
    format!(
        "epistemic:shadow:{}",
        stable_hash(json!([content_node_id, engine_version]))
    )
}

pub fn has_epistemic_shadow_edge_id(content_node_id: &str, shadow_node_id: &str) -> String {
    format!(
        "epistemic:has_shadow:{}",
        stable_hash(json!([content_node_id, shadow_node_id]))
    )
}

pub fn epistemic_shadow_edge_id(
    kind: EpistemicRelationKind,
    from_shadow_id: &str,
    to_shadow_id: &str,
    engine_version: &str,
) -> String {
    format!(
        "epistemic:edge:{}:{}",
        kind.edge_type(),
        stable_hash(json!([from_shadow_id, to_shadow_id, engine_version]))
    )
}

pub fn structural_epistemic_pass<S: GraphStore>(
    store: &mut S,
    input: StructuralEpistemicInput,
) -> GraphStoreResult<EpistemicReadout> {
    let config = input.config;
    let mut node_ids = input
        .batch_node_ids
        .into_iter()
        .filter(|id| !id.trim().is_empty())
        .collect::<BTreeSet<_>>();
    for pair in &input.candidate_pairs {
        node_ids.insert(pair.left_content_id.clone());
        node_ids.insert(pair.right_content_id.clone());
    }
    for relation in &input.explicit_relations {
        node_ids.insert(relation.from_content_id.clone());
        node_ids.insert(relation.to_content_id.clone());
    }

    let nodes = node_ids
        .iter()
        .filter_map(|id| store.get_node(id).cloned())
        .collect::<Vec<_>>();
    let node_set = nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    let existing_edges = induced_edges(store, &node_set);

    let mut relations = existing_epistemic_relations(&existing_edges, &config);
    let checked_pair_count = input.candidate_pairs.len();
    for pair in input.candidate_pairs {
        if let Some(relation) = infer_pair_relation(store, &pair) {
            relations.push(relation);
        }
    }
    relations.extend(input.explicit_relations);
    dedupe_relations(&mut relations);

    let undirected = undirected_adjacency(&node_set, &existing_edges, &relations);
    let bridge_scores = bridge_scores(&node_set, &undirected);
    let cycle_ids = contradiction_cycles(&relations);
    let mut support_in = HashMap::<String, u64>::new();
    let mut attack_in = HashMap::<String, u64>::new();
    for relation in &relations {
        match relation.kind {
            EpistemicRelationKind::Supports => {
                *support_in
                    .entry(relation.to_content_id.clone())
                    .or_insert(0) += 1;
            }
            EpistemicRelationKind::Undercuts => {
                *attack_in.entry(relation.to_content_id.clone()).or_insert(0) += 1;
            }
        }
    }

    let mut readout = EpistemicReadout {
        checked_pair_count,
        candidate_pair_bound: node_set.len().saturating_mul(config.candidate_top_k),
        ..EpistemicReadout::default()
    };
    for node in &nodes {
        let support = support_in.get(&node.id).copied().unwrap_or(0);
        let attack = attack_in.get(&node.id).copied().unwrap_or(0);
        let orphan = undirected
            .get(&node.id)
            .map(|neighbors| neighbors.is_empty())
            .unwrap_or(true);
        let unsupported_leaf = support == 0;
        let contradiction_cycle_id = cycle_ids.get(&node.id).cloned();
        let grounded_extension_status = if contradiction_cycle_id.is_some() {
            GroundedExtensionStatus::Undecided
        } else if attack > 0 {
            GroundedExtensionStatus::Out
        } else {
            GroundedExtensionStatus::In
        };
        let bridge_score = bridge_scores.get(&node.id).copied().unwrap_or(0.0);
        let shadow = write_structural_shadow(
            store,
            node,
            &config,
            grounded_extension_status,
            support,
            attack,
            unsupported_leaf,
            orphan,
            bridge_score,
            contradiction_cycle_id,
        )?;
        if unsupported_leaf {
            readout.unsupported.push(node.id.clone());
        }
        if orphan {
            readout.orphans.push(node.id.clone());
        }
        if bridge_score > 0.0 {
            readout.chokepoints.push(EpistemicChokepoint {
                content_node_id: node.id.clone(),
                shadow_node_id: shadow.shadow_node_id.clone(),
                bridge_score,
            });
        }
        readout.shadows.push(shadow);
    }

    for relation in &relations {
        let from_shadow =
            epistemic_shadow_node_id(&relation.from_content_id, &config.engine_version);
        let to_shadow = epistemic_shadow_node_id(&relation.to_content_id, &config.engine_version);
        if store.get_node(&from_shadow).is_none() || store.get_node(&to_shadow).is_none() {
            continue;
        }
        write_shadow_relation(store, relation, &from_shadow, &to_shadow, &config)?;
        if relation.kind == EpistemicRelationKind::Undercuts {
            readout.contradictions.push(EpistemicRelationReadout {
                from_content_id: relation.from_content_id.clone(),
                to_content_id: relation.to_content_id.clone(),
                from_shadow_id: from_shadow,
                to_shadow_id: to_shadow,
                kind: relation.kind.clone(),
                confidence: relation.confidence,
                evidence: relation.evidence.clone(),
            });
        }
    }

    readout
        .shadows
        .sort_by(|left, right| left.content_node_id.cmp(&right.content_node_id));
    readout.unsupported.sort();
    readout.orphans.sort();
    readout.chokepoints.sort_by(|left, right| {
        right
            .bridge_score
            .partial_cmp(&left.bridge_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    readout.contradictions.sort_by(|left, right| {
        left.from_content_id
            .cmp(&right.from_content_id)
            .then_with(|| left.to_content_id.cmp(&right.to_content_id))
    });
    Ok(readout)
}

pub fn read_epistemic_shadow<S: GraphStore>(
    store: &S,
    content_node_id: &str,
) -> Option<EpistemicShadowReadout> {
    for hit in store
        .neighbors(NeighborQuery::out(content_node_id).with_edge_type(HAS_EPISTEMIC_SHADOW))
        .into_iter()
    {
        let shadow = store.get_node(&hit.node_id)?;
        if shadow
            .labels
            .iter()
            .any(|label| label == EPISTEMIC_SHADOW_LABEL)
        {
            return shadow_readout_from_node(content_node_id, shadow);
        }
    }
    None
}

pub fn epistemic_shadow_ppr<S: GraphStore>(
    store: &S,
    seeds: &HashMap<String, f64>,
    top_k: usize,
    alpha: f64,
    epsilon: f64,
    max_pushes: usize,
) -> Vec<(String, f64)> {
    let mut shadow_seeds = HashMap::new();
    for (seed, weight) in seeds {
        if store
            .get_node(seed)
            .map(|node| {
                node.labels
                    .iter()
                    .any(|label| label == EPISTEMIC_SHADOW_LABEL)
            })
            .unwrap_or(false)
        {
            shadow_seeds.insert(seed.clone(), *weight);
        } else if let Some(shadow) = read_epistemic_shadow(store, seed) {
            shadow_seeds.insert(shadow.shadow_node_id, *weight);
        }
    }
    if shadow_seeds.is_empty() {
        return Vec::new();
    }

    let shadow_nodes = store
        .query_nodes(NodeQuery::label(EPISTEMIC_SHADOW_LABEL).with_limit(100_000))
        .into_iter()
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    let mut adjacency = HashMap::new();
    for node_id in &shadow_nodes {
        let mut outs = Vec::new();
        for edge_type in [UNDERCUTS, EPISTEMIC_SUPPORTS, SAME_ECLASS] {
            for hit in store
                .neighbors(NeighborQuery::out(node_id).with_edge_type(edge_type))
                .into_iter()
            {
                if shadow_nodes.contains(&hit.node_id) {
                    outs.push((hit.node_id, hit.confidence.unwrap_or(1.0).max(0.0)));
                }
            }
        }
        adjacency.insert(node_id.clone(), outs);
    }
    let mut ranked = personalized_pagerank(&adjacency, &shadow_seeds, alpha, epsilon, max_pushes)
        .into_iter()
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    ranked.truncate(top_k.max(1));
    ranked
}

pub fn run_epistemic_cron_pass<S: GraphStore, E: EpistemicEnricher>(
    store: &mut S,
    input: EpistemicCronInput,
    enricher: &E,
) -> GraphStoreResult<EpistemicCronReport> {
    let subgraph = compile_user_subgraph(store, &input.content_node_ids);
    if subgraph.nodes.is_empty() {
        return Ok(EpistemicCronReport {
            no_op: true,
            skipped_reason: "empty_subgraph".to_string(),
            ..EpistemicCronReport::default()
        });
    }
    if input.density_floor > 0.0 {
        let density = edge_density(&subgraph);
        if density < input.density_floor {
            return Ok(EpistemicCronReport {
                no_op: true,
                skipped_reason: format!("edge_density_below_floor:{density:.6}"),
                ..EpistemicCronReport::default()
            });
        }
    }

    let annotations = match enricher.enrich(subgraph, input.mode.clone()) {
        Ok(annotations) => annotations,
        Err(err) => {
            return Ok(EpistemicCronReport {
                attempted: true,
                no_op: true,
                grpc_ok: false,
                skipped_reason: format!("{}: {}", err.code, err.message),
                ..EpistemicCronReport::default()
            });
        }
    };

    let received = annotations.annotations.len();
    let mut report = EpistemicCronReport {
        attempted: true,
        grpc_ok: true,
        annotations_received: received,
        ..EpistemicCronReport::default()
    };
    for annotation in &annotations.annotations {
        if write_learned_shadow(store, annotation, &input)?.is_some() {
            report.shadows_written += 1;
        }
    }
    let mut relations = annotations.support_relations;
    relations.extend(annotations.attack_relations);
    dedupe_relations(&mut relations);
    for relation in &relations {
        let from_shadow =
            epistemic_shadow_node_id(&relation.from_content_id, &input.engine_version);
        let to_shadow = epistemic_shadow_node_id(&relation.to_content_id, &input.engine_version);
        if store.get_node(&from_shadow).is_none() || store.get_node(&to_shadow).is_none() {
            continue;
        }
        let config = StructuralEpistemicConfig {
            engine: input.engine.clone(),
            engine_version: input.engine_version.clone(),
            computed_at: input.computed_at,
            ..StructuralEpistemicConfig::default()
        };
        write_shadow_relation(store, relation, &from_shadow, &to_shadow, &config)?;
        report.shadow_edges_written += 1;
    }
    Ok(report)
}

fn write_structural_shadow<S: GraphStore>(
    store: &mut S,
    content_node: &NodeRecord,
    config: &StructuralEpistemicConfig,
    grounded_extension_status: GroundedExtensionStatus,
    support_in_degree: u64,
    attack_in_degree: u64,
    unsupported_leaf: bool,
    orphan: bool,
    bridge_score: f64,
    contradiction_cycle_id: Option<String>,
) -> GraphStoreResult<EpistemicShadowReadout> {
    let shadow_id = epistemic_shadow_node_id(&content_node.id, &config.engine_version);
    let existing = store.get_node(&shadow_id).cloned();
    let mut properties = existing
        .as_ref()
        .map(|node| node.properties.clone())
        .unwrap_or_else(|| json!({}));
    let provenance = EpistemicFieldProvenance::structural(config);
    set_field_provenance(
        &mut properties,
        &[
            "grounded_extension_status",
            "support_in_degree",
            "attack_in_degree",
            "unsupported_leaf",
            "orphan",
            "bridge_score",
            "contradiction_cycle_id",
        ],
        &provenance,
    );
    let source_kind = if has_learned_fields(&properties) {
        EpistemicSourceKind::Mixed
    } else {
        EpistemicSourceKind::Structural
    };
    let object = ensure_object(&mut properties);
    object.insert("content_node_id".to_string(), json!(content_node.id));
    object.insert(
        "grounded_extension_status".to_string(),
        json!(grounded_extension_status.as_str()),
    );
    object.insert("support_in_degree".to_string(), json!(support_in_degree));
    object.insert("attack_in_degree".to_string(), json!(attack_in_degree));
    object.insert("unsupported_leaf".to_string(), json!(unsupported_leaf));
    object.insert("orphan".to_string(), json!(orphan));
    object.insert("bridge_score".to_string(), json!(round6(bridge_score)));
    object.insert(
        "contradiction_cycle_id".to_string(),
        contradiction_cycle_id
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    object.insert("source_kind".to_string(), json!(source_kind.as_str()));
    object.insert("engine".to_string(), json!(config.engine));
    object.insert("engine_version".to_string(), json!(config.engine_version));
    object.insert("computed_at".to_string(), json!(config.computed_at));
    object.insert("quarantine".to_string(), json!(true));
    object
        .entry("predicted_edges".to_string())
        .or_insert_with(|| json!([]));
    object
        .entry("structural_role_vector".to_string())
        .or_insert_with(|| json!([]));
    if let Some(tenant) = prop_str(&content_node.properties, "tenant_id") {
        object
            .entry("tenant_id".to_string())
            .or_insert(json!(tenant));
    }
    if let Some(repo) = prop_str(&content_node.properties, "repo_id") {
        object.entry("repo_id".to_string()).or_insert(json!(repo));
    }

    store.upsert_node(NodeRecord::new(
        &shadow_id,
        [EPISTEMIC_SHADOW_LABEL],
        properties,
    ))?;
    store.upsert_edge(EdgeRecord::new(
        has_epistemic_shadow_edge_id(&content_node.id, &shadow_id),
        &content_node.id,
        HAS_EPISTEMIC_SHADOW,
        &shadow_id,
        json!({
            "engine_version": config.engine_version,
            "computed_at": config.computed_at,
            "source": STRUCTURAL_EPISTEMIC_ENGINE,
        }),
    ))?;
    store
        .get_node(&shadow_id)
        .and_then(|shadow| shadow_readout_from_node(&content_node.id, shadow))
        .ok_or_else(|| {
            GraphStoreError::new(
                "epistemic_shadow_write_failed",
                format!("shadow {shadow_id} was not readable after write"),
            )
        })
}

fn write_learned_shadow<S: GraphStore>(
    store: &mut S,
    annotation: &EpistemicAnnotation,
    config: &EpistemicCronInput,
) -> GraphStoreResult<Option<EpistemicShadowReadout>> {
    if annotation.content_node_id.trim().is_empty() {
        return Ok(None);
    }
    let Some(content_node) = store.get_node(&annotation.content_node_id).cloned() else {
        return Ok(None);
    };
    let shadow_id = epistemic_shadow_node_id(&annotation.content_node_id, &config.engine_version);
    let existing = store.get_node(&shadow_id).cloned();
    let mut properties = existing
        .as_ref()
        .map(|node| node.properties.clone())
        .unwrap_or_else(|| json!({}));
    let provenance = EpistemicFieldProvenance::learned(config);
    set_field_provenance(
        &mut properties,
        &[
            "predicted_edges",
            "completion_confidence",
            "structural_role_vector",
            "source_reliability",
            "community_id",
        ],
        &provenance,
    );
    let object = ensure_object(&mut properties);
    object.insert(
        "content_node_id".to_string(),
        json!(annotation.content_node_id),
    );
    object.insert(
        "predicted_edges".to_string(),
        serde_json::to_value(&annotation.predicted_edges).unwrap_or_else(|_| json!([])),
    );
    object.insert(
        "completion_confidence".to_string(),
        annotation
            .completion_confidence
            .and_then(|value| serde_json::Number::from_f64(value.clamp(0.0, 1.0)))
            .map(Value::Number)
            .unwrap_or(Value::Null),
    );
    object.insert(
        "structural_role_vector".to_string(),
        serde_json::to_value(&annotation.structural_role_vector).unwrap_or_else(|_| json!([])),
    );
    object.insert(
        "source_reliability".to_string(),
        annotation
            .source_reliability
            .as_ref()
            .map(|value| serde_json::to_value(value).unwrap_or(Value::Null))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "community_id".to_string(),
        annotation
            .community_id
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    if let Some(status) = &annotation.grounded_extension_status {
        object.insert(
            "grounded_extension_status".to_string(),
            json!(status.as_str()),
        );
    }
    object.insert(
        "source_kind".to_string(),
        json!(if existing.is_some() {
            "mixed"
        } else {
            "learned"
        }),
    );
    object.insert("engine".to_string(), json!(config.engine));
    object.insert("engine_version".to_string(), json!(config.engine_version));
    object.insert("computed_at".to_string(), json!(config.computed_at));
    object.insert("quarantine".to_string(), json!(true));
    if let Some(tenant) = prop_str(&content_node.properties, "tenant_id") {
        object
            .entry("tenant_id".to_string())
            .or_insert(json!(tenant));
    }
    if let Some(repo) = prop_str(&content_node.properties, "repo_id") {
        object.entry("repo_id".to_string()).or_insert(json!(repo));
    }

    store.upsert_node(NodeRecord::new(
        &shadow_id,
        [EPISTEMIC_SHADOW_LABEL],
        properties,
    ))?;
    store.upsert_edge(EdgeRecord::new(
        has_epistemic_shadow_edge_id(&annotation.content_node_id, &shadow_id),
        &annotation.content_node_id,
        HAS_EPISTEMIC_SHADOW,
        &shadow_id,
        json!({
            "engine_version": config.engine_version,
            "computed_at": config.computed_at,
            "source": LEARNED_EPISTEMIC_ENGINE,
        }),
    ))?;
    Ok(store
        .get_node(&shadow_id)
        .and_then(|shadow| shadow_readout_from_node(&annotation.content_node_id, shadow)))
}

fn write_shadow_relation<S: GraphStore>(
    store: &mut S,
    relation: &EpistemicRelationInput,
    from_shadow_id: &str,
    to_shadow_id: &str,
    config: &StructuralEpistemicConfig,
) -> GraphStoreResult<()> {
    let edge = EdgeRecord::new(
        epistemic_shadow_edge_id(
            relation.kind.clone(),
            from_shadow_id,
            to_shadow_id,
            &config.engine_version,
        ),
        from_shadow_id,
        relation.kind.edge_type(),
        to_shadow_id,
        json!({
            "from_content_id": relation.from_content_id,
            "to_content_id": relation.to_content_id,
            "confidence": normalized_confidence(relation.confidence),
            "evidence": relation.evidence,
            "source_kind": "structural",
            "engine": config.engine,
            "engine_version": config.engine_version,
            "computed_at": config.computed_at,
            "quarantine": true,
        }),
    )
    .with_confidence(normalized_confidence(relation.confidence))
    .with_epistemic_type(relation.kind.epistemic_type())
    .with_provenance(Provenance {
        source_id: Some(config.engine.clone()),
        timestamp: Some(config.computed_at.to_string()),
        method: Some("epistemic_shadow_relation".to_string()),
    });
    store.upsert_edge(edge)?;
    Ok(())
}

fn induced_edges<S: GraphStore>(store: &S, node_set: &BTreeSet<String>) -> Vec<EdgeRecord> {
    let mut seen = BTreeSet::new();
    let mut edges = Vec::new();
    for node_id in node_set {
        for hit in store
            .neighbors(NeighborQuery::out(node_id).with_include_expired(true))
            .into_iter()
        {
            if !node_set.contains(&hit.node_id) || !seen.insert(hit.edge_id.clone()) {
                continue;
            }
            if let Some(edge) = store.get_edge(&hit.edge_id) {
                edges.push(edge.clone());
            }
        }
    }
    edges
}

fn existing_epistemic_relations(
    edges: &[EdgeRecord],
    config: &StructuralEpistemicConfig,
) -> Vec<EpistemicRelationInput> {
    let support = config
        .support_edge_types
        .iter()
        .map(|edge| edge.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let attack = config
        .attack_edge_types
        .iter()
        .map(|edge| edge.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut relations = Vec::new();
    for edge in edges {
        let edge_type = edge.edge_type.to_ascii_lowercase();
        let kind = if support.contains(&edge_type) {
            Some(EpistemicRelationKind::Supports)
        } else if attack.contains(&edge_type) {
            Some(EpistemicRelationKind::Undercuts)
        } else {
            None
        };
        if let Some(kind) = kind {
            relations.push(EpistemicRelationInput {
                from_content_id: edge.from_id.clone(),
                to_content_id: edge.to_id.clone(),
                kind,
                confidence: edge.confidence.unwrap_or(1.0),
                evidence: prop_str(&edge.properties, "evidence").unwrap_or_default(),
            });
        }
    }
    relations
}

fn infer_pair_relation<S: GraphStore>(
    store: &S,
    pair: &EpistemicCandidatePair,
) -> Option<EpistemicRelationInput> {
    let left = store.get_node(&pair.left_content_id)?;
    let right = store.get_node(&pair.right_content_id)?;
    let left_text = claim_text(left);
    let right_text = claim_text(right);
    if left_text.trim().is_empty() || right_text.trim().is_empty() {
        return None;
    }
    let left_norm = normalize_claim(&left_text);
    let right_norm = normalize_claim(&right_text);
    if left_norm.is_empty() || right_norm.is_empty() {
        return None;
    }
    let left_neg = contains_negation(&left_norm);
    let right_neg = contains_negation(&right_norm);
    if left_neg != right_neg && without_negation(&left_norm) == without_negation(&right_norm) {
        return Some(EpistemicRelationInput {
            from_content_id: pair.left_content_id.clone(),
            to_content_id: pair.right_content_id.clone(),
            kind: EpistemicRelationKind::Undercuts,
            confidence: 0.75,
            evidence: format!("bounded_pair_contradiction: {left_text} :: {right_text}"),
        });
    }
    if left_norm == right_norm {
        return Some(EpistemicRelationInput {
            from_content_id: pair.left_content_id.clone(),
            to_content_id: pair.right_content_id.clone(),
            kind: EpistemicRelationKind::Supports,
            confidence: 0.6,
            evidence: "bounded_pair_equivalent_claim".to_string(),
        });
    }
    None
}

fn undirected_adjacency(
    node_set: &BTreeSet<String>,
    edges: &[EdgeRecord],
    relations: &[EpistemicRelationInput],
) -> HashMap<String, BTreeSet<String>> {
    let mut adjacency = node_set
        .iter()
        .map(|id| (id.clone(), BTreeSet::new()))
        .collect::<HashMap<_, _>>();
    for edge in edges {
        if node_set.contains(&edge.from_id) && node_set.contains(&edge.to_id) {
            adjacency
                .entry(edge.from_id.clone())
                .or_default()
                .insert(edge.to_id.clone());
            adjacency
                .entry(edge.to_id.clone())
                .or_default()
                .insert(edge.from_id.clone());
        }
    }
    for relation in relations {
        if node_set.contains(&relation.from_content_id)
            && node_set.contains(&relation.to_content_id)
        {
            adjacency
                .entry(relation.from_content_id.clone())
                .or_default()
                .insert(relation.to_content_id.clone());
            adjacency
                .entry(relation.to_content_id.clone())
                .or_default()
                .insert(relation.from_content_id.clone());
        }
    }
    adjacency
}

fn bridge_scores(
    node_set: &BTreeSet<String>,
    adjacency: &HashMap<String, BTreeSet<String>>,
) -> HashMap<String, f64> {
    let baseline = component_count(node_set, adjacency, None);
    node_set
        .iter()
        .map(|node_id| {
            let without = component_count(node_set, adjacency, Some(node_id));
            let score = without.saturating_sub(baseline).max(0) as f64;
            (node_id.clone(), score)
        })
        .collect()
}

fn component_count(
    node_set: &BTreeSet<String>,
    adjacency: &HashMap<String, BTreeSet<String>>,
    removed: Option<&String>,
) -> usize {
    let mut visited = HashSet::new();
    let mut count = 0usize;
    for start in node_set {
        if removed == Some(start) || !visited.insert(start.clone()) {
            continue;
        }
        count += 1;
        let mut queue = VecDeque::from([start.clone()]);
        while let Some(node) = queue.pop_front() {
            for neighbor in adjacency.get(&node).into_iter().flatten() {
                if removed == Some(neighbor) || !visited.insert(neighbor.clone()) {
                    continue;
                }
                queue.push_back(neighbor.clone());
            }
        }
    }
    count
}

fn contradiction_cycles(relations: &[EpistemicRelationInput]) -> HashMap<String, String> {
    let mut attacks = HashSet::new();
    for relation in relations {
        if relation.kind == EpistemicRelationKind::Undercuts {
            attacks.insert((
                relation.from_content_id.clone(),
                relation.to_content_id.clone(),
            ));
        }
    }
    let mut cycles = HashMap::new();
    for (left, right) in &attacks {
        if attacks.contains(&(right.clone(), left.clone())) {
            let cycle_id = format!("contradiction:{}", stable_hash(json!([left, right])));
            cycles.insert(left.clone(), cycle_id.clone());
            cycles.insert(right.clone(), cycle_id);
        }
    }
    cycles
}

fn dedupe_relations(relations: &mut Vec<EpistemicRelationInput>) {
    let mut seen = BTreeSet::new();
    relations.retain(|relation| {
        seen.insert((
            relation.from_content_id.clone(),
            relation.to_content_id.clone(),
            relation.kind.edge_type().to_string(),
        ))
    });
}

fn shadow_readout_from_node(
    content_node_id: &str,
    shadow: &NodeRecord,
) -> Option<EpistemicShadowReadout> {
    let props = &shadow.properties;
    Some(EpistemicShadowReadout {
        content_node_id: prop_str(props, "content_node_id")
            .unwrap_or_else(|| content_node_id.to_string()),
        shadow_node_id: shadow.id.clone(),
        grounded_extension_status: parse_grounded_status(
            &prop_str(props, "grounded_extension_status")
                .unwrap_or_else(|| "undecided".to_string()),
        ),
        support_in_degree: prop_u64(props, "support_in_degree").unwrap_or(0),
        attack_in_degree: prop_u64(props, "attack_in_degree").unwrap_or(0),
        unsupported_leaf: prop_bool(props, "unsupported_leaf"),
        orphan: prop_bool(props, "orphan"),
        bridge_score: prop_f64(props, "bridge_score").unwrap_or(0.0),
        contradiction_cycle_id: prop_str(props, "contradiction_cycle_id")
            .filter(|value| !value.is_empty()),
        predicted_edges: serde_json::from_value(
            props
                .get("predicted_edges")
                .cloned()
                .unwrap_or_else(|| json!([])),
        )
        .unwrap_or_default(),
        completion_confidence: prop_f64(props, "completion_confidence"),
        structural_role_vector: props
            .get("structural_role_vector")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|value| value.as_f64().map(|f| f as f32))
                    .collect()
            })
            .unwrap_or_default(),
        source_reliability: props
            .get("source_reliability")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok()),
        community_id: prop_str(props, "community_id").filter(|value| !value.is_empty()),
        source_kind: parse_source_kind(
            &prop_str(props, "source_kind").unwrap_or_else(|| "structural".to_string()),
        ),
        engine: prop_str(props, "engine").unwrap_or_default(),
        engine_version: prop_str(props, "engine_version").unwrap_or_default(),
        computed_at: prop_i64(props, "computed_at").unwrap_or(0),
        quarantine: prop_bool(props, "quarantine"),
        field_provenance: props
            .get("field_provenance")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default(),
    })
}

pub fn compile_user_subgraph<S: GraphStore>(store: &S, node_ids: &[String]) -> UserSubgraph {
    let nodes = if node_ids.is_empty() {
        store
            .query_nodes(NodeQuery::default().with_limit(100_000))
            .into_iter()
            .filter(|node| {
                !node
                    .labels
                    .iter()
                    .any(|label| label == EPISTEMIC_SHADOW_LABEL)
            })
            .collect::<Vec<_>>()
    } else {
        node_ids
            .iter()
            .filter_map(|id| store.get_node(id).cloned())
            .collect::<Vec<_>>()
    };
    let node_set = nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    let edges = induced_edges(store, &node_set)
        .into_iter()
        .filter(|edge| edge.edge_type != HAS_EPISTEMIC_SHADOW)
        .collect();
    UserSubgraph { nodes, edges }
}

fn edge_density(subgraph: &UserSubgraph) -> f64 {
    let n = subgraph.nodes.len();
    if n < 2 {
        return 0.0;
    }
    let possible = n.saturating_mul(n - 1) as f64;
    subgraph.edges.len() as f64 / possible
}

fn set_field_provenance(
    properties: &mut Value,
    fields: &[&str],
    provenance: &EpistemicFieldProvenance,
) {
    let serialized = serde_json::to_value(provenance).unwrap_or_else(|_| json!({}));
    let object = ensure_object(properties);
    let entry = object
        .entry("field_provenance".to_string())
        .or_insert_with(|| json!({}));
    let provenance_map = ensure_object(entry);
    for field in fields {
        provenance_map.insert((*field).to_string(), serialized.clone());
    }
}

fn has_learned_fields(properties: &Value) -> bool {
    properties
        .get("predicted_edges")
        .and_then(Value::as_array)
        .map(|items| !items.is_empty())
        .unwrap_or(false)
        || properties
            .get("completion_confidence")
            .is_some_and(|value| !value.is_null())
        || properties
            .get("source_reliability")
            .is_some_and(|value| !value.is_null())
}

fn claim_text(node: &NodeRecord) -> String {
    [
        "claim_text",
        "content",
        "summary",
        "doc",
        "signature",
        "snippet",
        "name",
    ]
    .into_iter()
    .filter_map(|key| prop_str(&node.properties, key))
    .filter(|part| !part.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" ")
}

fn normalize_claim(text: &str) -> String {
    text.to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn contains_negation(normalized: &str) -> bool {
    normalized
        .split_whitespace()
        .any(|token| matches!(token, "not" | "no" | "never" | "without"))
}

fn without_negation(normalized: &str) -> String {
    normalized
        .split_whitespace()
        .filter(|token| !matches!(*token, "not" | "no" | "never" | "without"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_grounded_status(raw: &str) -> GroundedExtensionStatus {
    match raw {
        "in" => GroundedExtensionStatus::In,
        "out" => GroundedExtensionStatus::Out,
        _ => GroundedExtensionStatus::Undecided,
    }
}

fn parse_source_kind(raw: &str) -> EpistemicSourceKind {
    match raw {
        "learned" => EpistemicSourceKind::Learned,
        "mixed" => EpistemicSourceKind::Mixed,
        _ => EpistemicSourceKind::Structural,
    }
}

fn normalized_confidence(confidence: f64) -> f64 {
    if confidence <= 0.0 {
        1.0
    } else {
        confidence.clamp(0.0, 1.0)
    }
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn prop_str(properties: &Value, key: &str) -> Option<String> {
    properties.get(key).and_then(|value| {
        value.as_str().map(str::to_string).or_else(|| {
            if value.is_null() {
                None
            } else {
                Some(value.to_string())
            }
        })
    })
}

fn prop_u64(properties: &Value, key: &str) -> Option<u64> {
    properties.get(key).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
    })
}

fn prop_i64(properties: &Value, key: &str) -> Option<i64> {
    properties.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
    })
}

fn prop_f64(properties: &Value, key: &str) -> Option<f64> {
    properties.get(key).and_then(Value::as_f64)
}

fn prop_bool(properties: &Value, key: &str) -> bool {
    properties
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn ensure_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut().expect("value is object")
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_store::InMemoryGraphStore;

    fn claim(id: &str, text: &str) -> NodeRecord {
        NodeRecord::new(
            id,
            ["Claim"],
            json!({ "tenant_id": "t", "claim_text": text }),
        )
    }

    #[test]
    fn structural_pass_writes_shadow_and_bounded_contradiction() {
        let mut store = InMemoryGraphStore::new();
        store.upsert_node(claim("a", "cache is enabled")).unwrap();
        store
            .upsert_node(claim("b", "cache is not enabled"))
            .unwrap();

        let readout = structural_epistemic_pass(
            &mut store,
            StructuralEpistemicInput {
                batch_node_ids: vec!["a".to_string(), "b".to_string()],
                candidate_pairs: vec![EpistemicCandidatePair {
                    left_content_id: "a".to_string(),
                    right_content_id: "b".to_string(),
                }],
                config: StructuralEpistemicConfig {
                    candidate_top_k: 1,
                    ..StructuralEpistemicConfig::default()
                },
                ..StructuralEpistemicInput::default()
            },
        )
        .unwrap();

        assert_eq!(readout.checked_pair_count, 1);
        assert_eq!(readout.candidate_pair_bound, 2);
        assert_eq!(readout.contradictions.len(), 1);
        let shadow = read_epistemic_shadow(&store, "b").expect("shadow");
        assert_eq!(shadow.attack_in_degree, 1);
        assert_eq!(
            shadow.grounded_extension_status,
            GroundedExtensionStatus::Out
        );
        assert!(shadow.field_provenance.contains_key("support_in_degree"));
    }

    #[test]
    fn shadow_ppr_ranks_over_shadow_edges() {
        let mut store = InMemoryGraphStore::new();
        store.upsert_node(claim("a", "A")).unwrap();
        store.upsert_node(claim("b", "B")).unwrap();
        structural_epistemic_pass(
            &mut store,
            StructuralEpistemicInput {
                batch_node_ids: vec!["a".to_string(), "b".to_string()],
                explicit_relations: vec![EpistemicRelationInput {
                    from_content_id: "a".to_string(),
                    to_content_id: "b".to_string(),
                    kind: EpistemicRelationKind::Supports,
                    confidence: 1.0,
                    evidence: "test".to_string(),
                }],
                ..StructuralEpistemicInput::default()
            },
        )
        .unwrap();
        let mut seeds = HashMap::new();
        seeds.insert("a".to_string(), 1.0);
        let ranked = epistemic_shadow_ppr(&store, &seeds, 4, 0.15, 1e-5, 20_000);
        assert!(ranked
            .iter()
            .any(|(id, _)| id == &epistemic_shadow_node_id("b", DEFAULT_EPISTEMIC_ENGINE_VERSION)));
    }

    struct DroppingEnricher;

    impl EpistemicEnricher for DroppingEnricher {
        fn enrich(
            &self,
            _subgraph: UserSubgraph,
            _mode: EpistemicEnrichmentMode,
        ) -> Result<EpistemicAnnotations, EpistemicEnrichmentError> {
            Err(EpistemicEnrichmentError::new("unavailable", "grpc dropped"))
        }
    }

    #[test]
    fn cron_drop_noops_without_deleting_existing_shadow() {
        let mut store = InMemoryGraphStore::new();
        store.upsert_node(claim("a", "A")).unwrap();
        structural_epistemic_pass(
            &mut store,
            StructuralEpistemicInput {
                batch_node_ids: vec!["a".to_string()],
                ..StructuralEpistemicInput::default()
            },
        )
        .unwrap();
        let before = store.stats().nodes_total;
        let report = run_epistemic_cron_pass(
            &mut store,
            EpistemicCronInput {
                content_node_ids: vec!["a".to_string()],
                ..EpistemicCronInput::default()
            },
            &DroppingEnricher,
        )
        .unwrap();
        assert!(report.no_op);
        assert!(!report.grpc_ok);
        assert_eq!(store.stats().nodes_total, before);
        assert!(read_epistemic_shadow(&store, "a").is_some());
    }

    struct LearnedEnricher;

    impl EpistemicEnricher for LearnedEnricher {
        fn enrich(
            &self,
            _subgraph: UserSubgraph,
            _mode: EpistemicEnrichmentMode,
        ) -> Result<EpistemicAnnotations, EpistemicEnrichmentError> {
            Ok(EpistemicAnnotations {
                annotations: vec![EpistemicAnnotation {
                    content_node_id: "a".to_string(),
                    predicted_edges: vec![PredictedEdgePointer {
                        target_content_id: "b".to_string(),
                        relation: "depends_on".to_string(),
                        confidence: 0.8,
                        quarantine: true,
                    }],
                    completion_confidence: Some(0.8),
                    structural_role_vector: vec![0.1, 0.2],
                    source_reliability: Some(SourceReliability {
                        alpha: 3.0,
                        beta: 1.0,
                        mean: 0.75,
                    }),
                    community_id: Some("community:test".to_string()),
                    grounded_extension_status: None,
                }],
                ..EpistemicAnnotations::default()
            })
        }
    }

    #[test]
    fn learned_cron_adds_fields_to_same_shadow_without_overwriting_structural() {
        let mut store = InMemoryGraphStore::new();
        store.upsert_node(claim("a", "A")).unwrap();
        store.upsert_node(claim("b", "B")).unwrap();
        structural_epistemic_pass(
            &mut store,
            StructuralEpistemicInput {
                batch_node_ids: vec!["a".to_string(), "b".to_string()],
                ..StructuralEpistemicInput::default()
            },
        )
        .unwrap();
        let before = read_epistemic_shadow(&store, "a").expect("structural shadow");
        assert_eq!(
            before.shadow_node_id,
            epistemic_shadow_node_id("a", DEFAULT_EPISTEMIC_ENGINE_VERSION)
        );

        let report = run_epistemic_cron_pass(
            &mut store,
            EpistemicCronInput {
                content_node_ids: vec!["a".to_string(), "b".to_string()],
                ..EpistemicCronInput::default()
            },
            &LearnedEnricher,
        )
        .unwrap();
        assert_eq!(report.shadows_written, 1);

        let after = read_epistemic_shadow(&store, "a").expect("learned shadow");
        assert_eq!(after.shadow_node_id, before.shadow_node_id);
        assert_eq!(after.support_in_degree, before.support_in_degree);
        assert_eq!(after.attack_in_degree, before.attack_in_degree);
        assert_eq!(after.predicted_edges.len(), 1);
        assert!(after.predicted_edges[0].quarantine);
        assert_eq!(after.source_kind, EpistemicSourceKind::Mixed);
    }
}
