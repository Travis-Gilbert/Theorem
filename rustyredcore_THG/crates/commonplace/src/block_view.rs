//! Stable block/view contract over the CommonPlace object model.
//!
//! Blocks depend on this seam instead of depending on RustyRed, the harness, the
//! router, or theme internals. The host owns query compilation, action execution,
//! view matching, provenance, and live binding strategy.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use rustyred_thg_core::{EdgeRecord, GraphStore, GraphStoreError, GraphStoreResult, NeighborQuery};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::blob::BlobStore;
use crate::item::{Item, ItemBody, ItemKind};
use crate::renderable::{item_object_type_slug, renderable_from_item};
use crate::store::Commonplace;

pub const BLOCK_VIEW_CONTRACT_VERSION: &str = "block-view-contract/v1";
pub const DEFAULT_RECORD_POLL_INTERVAL_MS: u64 = 2_500;

pub type TypeRef = String;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropType {
    String,
    Text,
    Number,
    Integer,
    Boolean,
    Json,
    Id,
    TimestampMs,
    Vector,
    StringList,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Constraint {
    Required,
    Enum { values: Vec<String> },
    Min { value: f64 },
    Max { value: f64 },
    Pattern { regex: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PropertyDef {
    pub name: String,
    #[serde(rename = "type")]
    pub prop_type: PropType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<Constraint>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeDirection {
    In,
    Out,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationDef {
    pub edge: String,
    pub dir: EdgeDirection,
    pub target: TypeRef,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypeAxes {
    #[serde(default)]
    pub spatial: bool,
    #[serde(default)]
    pub temporal: bool,
    #[serde(default)]
    pub embeddable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypeDef {
    pub name: TypeRef,
    #[serde(default)]
    pub properties: Vec<PropertyDef>,
    #[serde(default)]
    pub relations: Vec<RelationDef>,
    #[serde(default)]
    pub axes: TypeAxes,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_ms: Option<i64>,
}

impl TimeRange {
    pub fn instant(ms: i64) -> Self {
        Self {
            from_ms: Some(ms),
            to_ms: Some(ms),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct H3Window {
    pub cells: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectAxes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h3: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid: Option<TimeRange>,
    #[serde(default)]
    pub embeddable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObjectRef {
    pub id: String,
    #[serde(rename = "type")]
    pub type_ref: TypeRef,
    #[serde(default)]
    pub properties: Map<String, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub relations: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub axes: ObjectAxes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectCardinality {
    Empty,
    One,
    Many,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapeRelation {
    pub edge: String,
    pub dir: EdgeDirection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<TypeRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectShape {
    #[serde(default)]
    pub types: Vec<TypeRef>,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub relations: Vec<ShapeRelation>,
    #[serde(default)]
    pub axes: TypeAxes,
    pub cardinality: ObjectCardinality,
}

impl ObjectShape {
    pub fn from_objects(objects: &[ObjectRef]) -> Self {
        let mut types = BTreeSet::new();
        let mut fields = BTreeSet::new();
        let mut relations =
            BTreeMap::<(String, EdgeDirection, Option<String>), ShapeRelation>::new();
        let mut axes = TypeAxes::default();

        for object in objects {
            types.insert(normalize_type_ref(&object.type_ref));
            for field in object.properties.keys() {
                fields.insert(field.clone());
            }
            for edge in object.relations.keys() {
                let relation = ShapeRelation {
                    edge: edge.clone(),
                    dir: EdgeDirection::Out,
                    target: None,
                };
                relations.insert((edge.clone(), EdgeDirection::Out, None), relation);
            }
            axes.spatial |= object.axes.h3.is_some();
            axes.temporal |= object.axes.valid.is_some();
            axes.embeddable |= object.axes.embeddable;
        }

        Self {
            types: types.into_iter().collect(),
            fields: fields.into_iter().collect(),
            relations: relations.into_values().collect(),
            axes,
            cardinality: match objects.len() {
                0 => ObjectCardinality::Empty,
                1 => ObjectCardinality::One,
                _ => ObjectCardinality::Many,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Predicate {
    Eq {
        field: String,
        value: Value,
    },
    NotEq {
        field: String,
        value: Value,
    },
    Contains {
        field: String,
        value: Value,
    },
    Exists {
        field: String,
    },
    RelationExists {
        edge: String,
        dir: EdgeDirection,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
    },
    And {
        all: Vec<Predicate>,
    },
    Or {
        any: Vec<Predicate>,
    },
    Not {
        predicate: Box<Predicate>,
    },
}

impl Predicate {
    pub fn not_eq(field: impl Into<String>, value: Value) -> Self {
        Self::NotEq {
            field: field.into(),
            value,
        }
    }

    pub fn relation_exists(edge: impl Into<String>, dir: EdgeDirection) -> Self {
        Self::RelationExists {
            edge: edge.into(),
            dir,
            target: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeWalk {
    pub edge: String,
    pub dir: EdgeDirection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<TypeRef>,
}

impl EdgeWalk {
    pub fn out(edge: impl Into<String>) -> Self {
        Self {
            edge: edge.into(),
            dir: EdgeDirection::Out,
            target: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankDirection {
    Asc,
    Desc,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ranker {
    Field {
        field: String,
        direction: RankDirection,
    },
    VectorKnn {
        field: String,
        vector: Vec<f32>,
        k: usize,
    },
    Fulltext {
        query: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<String>,
    },
    Graph {
        seeds: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        edge: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        direction: Option<EdgeDirection>,
    },
}

impl Ranker {
    pub fn field(field: impl Into<String>, direction: RankDirection) -> Self {
        Self::Field {
            field: field.into(),
            direction,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObjectFusionPolicy {
    Rrf { k: u32 },
    Weighted { weights: BTreeMap<String, f32> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectQuerySlice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid: Option<TimeRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx: Option<TimeRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<H3Window>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedRelation {
    pub edge: String,
    pub dir: EdgeDirection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<TypeRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Projection {
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub relations: Vec<ProjectedRelation>,
    #[serde(default = "default_true")]
    pub include_body_preview: bool,
    #[serde(default = "default_true")]
    pub include_metadata: bool,
}

impl Default for Projection {
    fn default() -> Self {
        Self {
            fields: Vec::new(),
            relations: Vec::new(),
            include_body_preview: true,
            include_metadata: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRequest {
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObjectQuery {
    pub types: Vec<TypeRef>,
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub where_clause: Option<Predicate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traverse: Vec<EdgeWalk>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rank: Vec<Ranker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuse: Option<ObjectFusionPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice: Option<ObjectQuerySlice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<Projection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<PageRequest>,
    #[serde(default = "default_true")]
    pub live: bool,
}

impl ObjectQuery {
    pub fn new(types: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            types: types.into_iter().map(Into::into).collect(),
            where_clause: None,
            traverse: Vec::new(),
            rank: Vec::new(),
            fuse: None,
            slice: None,
            project: None,
            page: None,
            live: true,
        }
    }

    pub fn with_where(mut self, predicate: Predicate) -> Self {
        self.where_clause = Some(predicate);
        self
    }

    pub fn with_traverse(mut self, walk: EdgeWalk) -> Self {
        self.traverse.push(walk);
        self
    }

    pub fn with_rank(mut self, ranker: Ranker) -> Self {
        self.rank.push(ranker);
        self
    }

    pub fn with_page(mut self, limit: usize, cursor: Option<String>) -> Self {
        self.page = Some(PageRequest { limit, cursor });
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LiveBinding {
    ChangeFeed { stream: String },
    Poll { interval_ms: u64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObjectSet {
    pub objects: Vec<ObjectRef>,
    pub shape: ObjectShape,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live: Option<LiveBinding>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTier {
    Simple,
    Difficult,
    Max,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectPointer {
    pub id: String,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_ref: Option<TypeRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ObjectActionTarget {
    Object(ObjectPointer),
    Query(ObjectQuery),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct JobSpec {
    pub name: String,
    #[serde(default)]
    pub args: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObjectAction {
    Create {
        #[serde(rename = "type")]
        type_ref: TypeRef,
        props: Map<String, Value>,
    },
    Update {
        id: String,
        patch: Map<String, Value>,
    },
    Delete {
        id: String,
    },
    Link {
        from: String,
        edge: String,
        to: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<f64>,
    },
    Unlink {
        from: String,
        edge: String,
        to: String,
    },
    RunAgent {
        target: ObjectActionTarget,
        tier: AgentTier,
    },
    InvokeTool {
        tool: String,
        #[serde(default)]
        args: Map<String, Value>,
    },
    Dispatch {
        job: JobSpec,
    },
    Open {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        view: Option<String>,
    },
    Select {
        ids: Vec<String>,
    },
}

impl ObjectAction {
    pub fn action_kind(&self) -> ActionKind {
        match self {
            Self::Create { .. } => ActionKind::Create,
            Self::Update { .. } => ActionKind::Update,
            Self::Delete { .. } => ActionKind::Delete,
            Self::Link { .. } => ActionKind::Link,
            Self::Unlink { .. } => ActionKind::Unlink,
            Self::RunAgent { .. } => ActionKind::RunAgent,
            Self::InvokeTool { .. } => ActionKind::InvokeTool,
            Self::Dispatch { .. } => ActionKind::Dispatch,
            Self::Open { .. } => ActionKind::Open,
            Self::Select { .. } => ActionKind::Select,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Create,
    Update,
    Delete,
    Link,
    Unlink,
    RunAgent,
    InvokeTool,
    Dispatch,
    Open,
    Select,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectActionStatus {
    Accepted,
    Applied,
    Deferred,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObjectActionReceipt {
    pub action_kind: ActionKind,
    pub status: ObjectActionStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_transform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ThemeTokens {
    #[serde(default)]
    pub color: Map<String, Value>,
    #[serde(default)]
    pub space: Map<String, Value>,
    #[serde(default)]
    pub typography: Map<String, Value>,
    #[serde(default)]
    pub radius: Map<String, Value>,
    #[serde(default)]
    pub raw: Map<String, Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardinalityRequirement {
    Any,
    One,
    Many,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapeRelationMatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<EdgeDirection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<TypeRef>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectShapeMatch {
    #[serde(default)]
    pub required_types: Vec<TypeRef>,
    #[serde(default)]
    pub required_fields: Vec<String>,
    #[serde(default)]
    pub required_axes: TypeAxes,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<CardinalityRequirement>,
    #[serde(default)]
    pub requires_relation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_edge: Option<ShapeRelationMatch>,
}

impl ObjectShapeMatch {
    pub fn matches(&self, shape: &ObjectShape) -> bool {
        shape_matches(self, shape)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewDescriptor {
    pub id: String,
    pub name: String,
    pub accepts: ObjectShapeMatch,
    #[serde(default)]
    pub emits: Vec<ActionKind>,
    pub renderer: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewRegistry {
    descriptors: Vec<ViewDescriptor>,
}

impl ViewRegistry {
    pub fn new(descriptors: impl IntoIterator<Item = ViewDescriptor>) -> Self {
        Self {
            descriptors: descriptors.into_iter().collect(),
        }
    }

    pub fn default_commonplace() -> Self {
        Self::new([
            ViewDescriptor {
                id: "table".to_string(),
                name: "Table".to_string(),
                accepts: ObjectShapeMatch {
                    required_fields: vec!["title".to_string()],
                    cardinality: Some(CardinalityRequirement::Many),
                    ..ObjectShapeMatch::default()
                },
                emits: vec![ActionKind::Open, ActionKind::Select],
                renderer: "table".to_string(),
            },
            ViewDescriptor {
                id: "board".to_string(),
                name: "Board".to_string(),
                accepts: ObjectShapeMatch {
                    required_fields: vec!["status".to_string()],
                    required_axes: TypeAxes {
                        temporal: true,
                        ..TypeAxes::default()
                    },
                    cardinality: Some(CardinalityRequirement::Many),
                    ..ObjectShapeMatch::default()
                },
                emits: vec![ActionKind::Update, ActionKind::Open, ActionKind::Select],
                renderer: "board".to_string(),
            },
            ViewDescriptor {
                id: "card".to_string(),
                name: "Card".to_string(),
                accepts: ObjectShapeMatch {
                    required_fields: vec!["title".to_string()],
                    cardinality: Some(CardinalityRequirement::Any),
                    ..ObjectShapeMatch::default()
                },
                emits: vec![ActionKind::Open, ActionKind::Select],
                renderer: "card".to_string(),
            },
            ViewDescriptor {
                id: "timeline".to_string(),
                name: "Timeline".to_string(),
                accepts: ObjectShapeMatch {
                    required_axes: TypeAxes {
                        temporal: true,
                        ..TypeAxes::default()
                    },
                    cardinality: Some(CardinalityRequirement::Many),
                    ..ObjectShapeMatch::default()
                },
                emits: vec![ActionKind::Open, ActionKind::Select],
                renderer: "timeline".to_string(),
            },
            ViewDescriptor {
                id: "graph".to_string(),
                name: "Graph".to_string(),
                accepts: ObjectShapeMatch {
                    requires_relation: true,
                    cardinality: Some(CardinalityRequirement::Many),
                    ..ObjectShapeMatch::default()
                },
                emits: vec![ActionKind::Link, ActionKind::Unlink, ActionKind::Open],
                renderer: "graph".to_string(),
            },
            ViewDescriptor {
                id: "patch-review".to_string(),
                name: "PatchReviewPanel".to_string(),
                accepts: ObjectShapeMatch {
                    required_types: vec!["patch".to_string()],
                    cardinality: Some(CardinalityRequirement::One),
                    ..ObjectShapeMatch::default()
                },
                emits: vec![ActionKind::Dispatch, ActionKind::RunAgent, ActionKind::Open],
                renderer: "patch-review".to_string(),
            },
            ViewDescriptor {
                id: "file-tree".to_string(),
                name: "FileTreePanel".to_string(),
                accepts: ObjectShapeMatch {
                    required_types: vec!["file".to_string()],
                    required_edge: Some(ShapeRelationMatch {
                        edge: Some("CONTAINS".to_string()),
                        dir: Some(EdgeDirection::Out),
                        target: None,
                    }),
                    ..ObjectShapeMatch::default()
                },
                emits: vec![ActionKind::Open, ActionKind::Select],
                renderer: "file-tree".to_string(),
            },
        ])
    }

    pub fn register(&mut self, descriptor: ViewDescriptor) {
        self.descriptors.retain(|entry| entry.id != descriptor.id);
        self.descriptors.push(descriptor);
    }

    pub fn views_for(&self, shape: &ObjectShape) -> Vec<ViewDescriptor> {
        self.descriptors
            .iter()
            .filter(|descriptor| descriptor.accepts.matches(shape))
            .cloned()
            .collect()
    }

    pub fn descriptors(&self) -> &[ViewDescriptor] {
        &self.descriptors
    }
}

pub trait BlockHost {
    fn query(&self, query: ObjectQuery) -> GraphStoreResult<ObjectSet>;
    fn emit(&mut self, action: ObjectAction) -> GraphStoreResult<ObjectActionReceipt>;
    fn views_for(&self, shape: &ObjectShape) -> Vec<ViewDescriptor>;
    fn tokens(&self) -> &ThemeTokens;
}

pub struct CommonplaceBlockHost<'a, S, B>
where
    S: GraphStore,
    B: BlobStore,
{
    commonplace: &'a mut Commonplace<S, B>,
    registry: ViewRegistry,
    tokens: ThemeTokens,
    actor_id: Option<String>,
}

impl<'a, S, B> CommonplaceBlockHost<'a, S, B>
where
    S: GraphStore,
    B: BlobStore,
{
    pub fn new(commonplace: &'a mut Commonplace<S, B>) -> Self {
        Self {
            commonplace,
            registry: ViewRegistry::default_commonplace(),
            tokens: ThemeTokens::default(),
            actor_id: None,
        }
    }

    pub fn with_actor(mut self, actor_id: impl Into<String>) -> Self {
        self.actor_id = Some(actor_id.into());
        self
    }

    pub fn with_tokens(mut self, tokens: ThemeTokens) -> Self {
        self.tokens = tokens;
        self
    }

    pub fn registry_mut(&mut self) -> &mut ViewRegistry {
        &mut self.registry
    }
}

impl<S, B> BlockHost for CommonplaceBlockHost<'_, S, B>
where
    S: GraphStore,
    B: BlobStore,
{
    fn query(&self, query: ObjectQuery) -> GraphStoreResult<ObjectSet> {
        self.commonplace.query_object_set(query)
    }

    fn emit(&mut self, action: ObjectAction) -> GraphStoreResult<ObjectActionReceipt> {
        self.commonplace
            .emit_object_action(action, self.actor_id.clone())
    }

    fn views_for(&self, shape: &ObjectShape) -> Vec<ViewDescriptor> {
        self.registry.views_for(shape)
    }

    fn tokens(&self) -> &ThemeTokens {
        &self.tokens
    }
}

impl<S, B> Commonplace<S, B>
where
    S: GraphStore,
    B: BlobStore,
{
    pub fn query_object_set(&self, query: ObjectQuery) -> GraphStoreResult<ObjectSet> {
        let mut items = self.items_for_query_types(&query.types)?;
        if let Some(predicate) = &query.where_clause {
            items.retain(|item| predicate_matches_item(self, item, predicate));
        }
        sort_items(&mut items, &query.rank);

        let total = items.len();
        let start = query
            .page
            .as_ref()
            .and_then(|page| page.cursor.as_ref())
            .and_then(|cursor| cursor.parse::<usize>().ok())
            .unwrap_or(0)
            .min(total);
        let limit = query
            .page
            .as_ref()
            .map(|page| page.limit)
            .filter(|limit| *limit > 0)
            .unwrap_or(total);
        let end = start.saturating_add(limit).min(total);
        let next_cursor = (end < total).then(|| end.to_string());
        let requested_relations = requested_relations(&query);
        let objects: Vec<ObjectRef> = items[start..end]
            .iter()
            .map(|item| object_ref_from_item(self, item, &query.project, &requested_relations))
            .collect::<GraphStoreResult<Vec<_>>>()?;
        let mut shape = ObjectShape::from_objects(&objects);
        merge_query_shape_relations(&mut shape, &query);

        Ok(ObjectSet {
            objects,
            shape,
            next_cursor,
            live: query.live.then_some(LiveBinding::Poll {
                interval_ms: DEFAULT_RECORD_POLL_INTERVAL_MS,
            }),
        })
    }

    pub fn emit_object_action(
        &mut self,
        action: ObjectAction,
        actor_id: Option<String>,
    ) -> GraphStoreResult<ObjectActionReceipt> {
        let action_kind = action.action_kind();
        match action {
            ObjectAction::Create { type_ref, props } => {
                let item = item_from_props(&type_ref, props)?;
                let item = self.put_item(item)?;
                Ok(ObjectActionReceipt {
                    action_kind,
                    status: ObjectActionStatus::Applied,
                    target_ids: vec![item.id],
                    graph_transform: Some("Item".to_string()),
                    actor_id,
                    note: None,
                })
            }
            ObjectAction::Update { id, patch } => {
                let mut item = self.get_item(&id)?.ok_or_else(|| {
                    GraphStoreError::new(
                        "commonplace_object_missing",
                        format!("object not found: {id}"),
                    )
                })?;
                apply_item_patch(&mut item, patch)?;
                let item = self.put_item(item)?;
                Ok(ObjectActionReceipt {
                    action_kind,
                    status: ObjectActionStatus::Applied,
                    target_ids: vec![item.id],
                    graph_transform: Some("Item.patch".to_string()),
                    actor_id,
                    note: None,
                })
            }
            ObjectAction::Link {
                from,
                edge,
                to,
                confidence,
            } => {
                let mut edge_record = EdgeRecord::new(
                    object_edge_id(&edge, &from, &to),
                    from.clone(),
                    edge.clone(),
                    to.clone(),
                    json!({}),
                );
                if let Some(confidence) = confidence {
                    edge_record = edge_record.with_confidence(confidence);
                }
                self.store_mut().upsert_edge(edge_record)?;
                Ok(ObjectActionReceipt {
                    action_kind,
                    status: ObjectActionStatus::Applied,
                    target_ids: vec![from, to],
                    graph_transform: Some(edge),
                    actor_id,
                    note: None,
                })
            }
            ObjectAction::Delete { id } => Ok(ObjectActionReceipt {
                action_kind,
                status: ObjectActionStatus::Deferred,
                target_ids: vec![id],
                graph_transform: None,
                actor_id,
                note: Some("delete is deferred to the host undo/tombstone layer".to_string()),
            }),
            ObjectAction::Unlink { from, edge, to } => Ok(ObjectActionReceipt {
                action_kind,
                status: ObjectActionStatus::Deferred,
                target_ids: vec![from, to],
                graph_transform: Some(edge),
                actor_id,
                note: Some("unlink is deferred to the host undo/tombstone layer".to_string()),
            }),
            ObjectAction::RunAgent { .. }
            | ObjectAction::InvokeTool { .. }
            | ObjectAction::Dispatch { .. }
            | ObjectAction::Open { .. }
            | ObjectAction::Select { .. } => Ok(ObjectActionReceipt {
                action_kind,
                status: ObjectActionStatus::Accepted,
                target_ids: Vec::new(),
                graph_transform: None,
                actor_id,
                note: Some("intent accepted for the shell or harness resolver".to_string()),
            }),
        }
    }

    fn items_for_query_types(&self, types: &[TypeRef]) -> GraphStoreResult<Vec<Item>> {
        if types.is_empty() {
            return self.all_items();
        }

        let mut seen = BTreeSet::new();
        let mut items = Vec::new();
        for type_ref in types {
            let type_slug = normalize_type_ref(type_ref);
            let candidates = if type_slug == "item" || type_slug == "*" {
                self.all_items()?
            } else {
                self.items_by_kind(&ItemKind::from(type_slug))?
            };
            for item in candidates {
                if seen.insert(item.id.clone()) {
                    items.push(item);
                }
            }
        }
        Ok(items)
    }
}

fn object_ref_from_item<S, B>(
    commonplace: &Commonplace<S, B>,
    item: &Item,
    projection: &Option<Projection>,
    requested_relations: &[ProjectedRelation],
) -> GraphStoreResult<ObjectRef>
where
    S: GraphStore,
    B: BlobStore,
{
    let renderable = renderable_from_item(item);
    let mut properties = renderable.metadata.clone();
    properties.insert("title".to_string(), json!(renderable.title));
    properties.insert("summary".to_string(), json!(renderable.summary));
    properties.insert("body_preview".to_string(), json!(renderable.body_preview));
    properties.insert("source".to_string(), json!(renderable.source));
    properties.insert("tags".to_string(), json!(renderable.tags));
    properties.insert("collections".to_string(), json!(renderable.collections));
    properties.insert("created_at_ms".to_string(), json!(renderable.created_at_ms));
    properties.insert("updated_at_ms".to_string(), json!(renderable.updated_at_ms));
    properties.insert(
        "object_type_slug".to_string(),
        json!(renderable.object_type_slug),
    );

    if let Some(projection) = projection {
        if !projection.include_body_preview {
            properties.remove("body_preview");
        }
        if !projection.include_metadata {
            let keep: BTreeSet<String> = projection.fields.iter().cloned().collect();
            properties.retain(|field, _| keep.contains(field));
        } else if !projection.fields.is_empty() {
            let keep: BTreeSet<String> = projection.fields.iter().cloned().collect();
            properties.retain(|field, _| keep.contains(field));
        }
    }

    let mut relations = BTreeMap::new();
    for relation in requested_relations {
        let query =
            neighbor_query_for(&item.id, relation.dir).with_edge_type(relation.edge.clone());
        let ids = commonplace
            .store()
            .neighbors(query)
            .into_iter()
            .map(|hit| hit.node_id)
            .collect::<Vec<_>>();
        if !ids.is_empty() {
            relations.insert(relation.edge.clone(), ids);
        }
    }

    let axes = ObjectAxes {
        h3: item
            .extra
            .get("h3")
            .or_else(|| item.extra.get("h3_cell"))
            .and_then(Value::as_str)
            .map(str::to_string),
        valid: item.due_at_ms.map(TimeRange::instant),
        embeddable: item.embedding.is_some() || item.embedding_ref.is_some(),
    };

    Ok(ObjectRef {
        id: item.id.clone(),
        type_ref: item_object_type_slug(item),
        properties,
        relations,
        axes,
    })
}

fn requested_relations(query: &ObjectQuery) -> Vec<ProjectedRelation> {
    let mut seen = BTreeSet::new();
    let mut relations = Vec::new();

    for walk in &query.traverse {
        let key = (walk.edge.clone(), walk.dir, walk.target.clone());
        if seen.insert(key) {
            relations.push(ProjectedRelation {
                edge: walk.edge.clone(),
                dir: walk.dir,
                target: walk.target.clone(),
            });
        }
    }
    if let Some(projection) = &query.project {
        for relation in &projection.relations {
            let key = (relation.edge.clone(), relation.dir, relation.target.clone());
            if seen.insert(key) {
                relations.push(relation.clone());
            }
        }
    }
    relations
}

fn merge_query_shape_relations(shape: &mut ObjectShape, query: &ObjectQuery) {
    let mut existing = shape
        .relations
        .iter()
        .map(|relation| (relation.edge.clone(), relation.dir, relation.target.clone()))
        .collect::<BTreeSet<_>>();

    for relation in requested_relations(query) {
        let key = (relation.edge.clone(), relation.dir, relation.target.clone());
        if existing.insert(key) {
            shape.relations.push(ShapeRelation {
                edge: relation.edge,
                dir: relation.dir,
                target: relation.target,
            });
        }
    }
    shape.relations.sort_by(|left, right| {
        left.edge
            .cmp(&right.edge)
            .then_with(|| format!("{:?}", left.dir).cmp(&format!("{:?}", right.dir)))
            .then_with(|| left.target.cmp(&right.target))
    });
}

fn predicate_matches_item<S, B>(
    commonplace: &Commonplace<S, B>,
    item: &Item,
    predicate: &Predicate,
) -> bool
where
    S: GraphStore,
    B: BlobStore,
{
    match predicate {
        Predicate::Eq { field, value } => item_field_value(item, field).as_ref() == Some(value),
        Predicate::NotEq { field, value } => item_field_value(item, field).as_ref() != Some(value),
        Predicate::Contains { field, value } => {
            contains_value(item_field_value(item, field), value)
        }
        Predicate::Exists { field } => item_field_value(item, field).is_some(),
        Predicate::RelationExists { edge, dir, target } => {
            let query = neighbor_query_for(&item.id, *dir).with_edge_type(edge.clone());
            commonplace
                .store()
                .neighbors(query)
                .into_iter()
                .any(|hit| target.as_ref().map(|id| id == &hit.node_id).unwrap_or(true))
        }
        Predicate::And { all } => all
            .iter()
            .all(|predicate| predicate_matches_item(commonplace, item, predicate)),
        Predicate::Or { any } => any
            .iter()
            .any(|predicate| predicate_matches_item(commonplace, item, predicate)),
        Predicate::Not { predicate } => !predicate_matches_item(commonplace, item, predicate),
    }
}

fn item_field_value(item: &Item, field: &str) -> Option<Value> {
    let value = serde_json::to_value(item).ok()?;
    value.get(field).cloned()
}

fn contains_value(haystack: Option<Value>, needle: &Value) -> bool {
    match haystack {
        Some(Value::String(text)) => needle
            .as_str()
            .map(|needle| text.contains(needle))
            .unwrap_or(false),
        Some(Value::Array(values)) => values.iter().any(|value| value == needle),
        Some(value) => value == *needle,
        None => false,
    }
}

fn sort_items(items: &mut [Item], rankers: &[Ranker]) {
    for ranker in rankers.iter().rev() {
        if let Ranker::Field { field, direction } = ranker {
            items.sort_by(|left, right| {
                let ordering = compare_optional_values(
                    item_field_value(left, field).as_ref(),
                    item_field_value(right, field).as_ref(),
                );
                match direction {
                    RankDirection::Asc => ordering,
                    RankDirection::Desc => ordering.reverse(),
                }
            });
        }
    }
}

fn compare_optional_values(left: Option<&Value>, right: Option<&Value>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => compare_values(left, right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_values(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => left
            .as_f64()
            .partial_cmp(&right.as_f64())
            .unwrap_or(Ordering::Equal),
        (Value::String(left), Value::String(right)) => left.cmp(right),
        (Value::Bool(left), Value::Bool(right)) => left.cmp(right),
        _ => left.to_string().cmp(&right.to_string()),
    }
}

fn item_from_props(type_ref: &str, mut props: Map<String, Value>) -> GraphStoreResult<Item> {
    let title = take_string(&mut props, "title").unwrap_or_else(|| "Untitled".to_string());
    let mut item = Item::new(ItemKind::from(normalize_type_ref(type_ref)), title);

    if let Some(text) = take_string(&mut props, "body").or_else(|| take_string(&mut props, "text"))
    {
        item = item.with_text(text);
    }
    if let Some(source) = take_string(&mut props, "source") {
        item = item.with_source(source);
    }
    if let Some(status) = take_string(&mut props, "status") {
        item = item.with_status(status);
    }
    if let Some(priority) = take_string(&mut props, "priority") {
        item = item.with_priority(priority);
    }
    if let Some(due_at_ms) = take_i64(&mut props, "due_at_ms") {
        item = item.with_due_at(due_at_ms);
    }
    if let Some(tags) = take_string_vec(&mut props, "tags")? {
        item = item.with_tags(tags);
    }
    for (key, value) in props {
        item = item.with_extra(key, value);
    }
    Ok(item)
}

fn apply_item_patch(item: &mut Item, mut patch: Map<String, Value>) -> GraphStoreResult<()> {
    if let Some(title) = take_string(&mut patch, "title") {
        item.title = title;
    }
    if let Some(text) = take_string(&mut patch, "body").or_else(|| take_string(&mut patch, "text"))
    {
        item.body = ItemBody::Inline { text };
    }
    if let Some(source) = take_string(&mut patch, "source") {
        item.source = Some(source);
    }
    if patch.contains_key("source") && patch.get("source").and_then(Value::as_str).is_none() {
        item.source = None;
        patch.remove("source");
    }
    if let Some(status) = take_optional_string(&mut patch, "status") {
        item.status = status;
    }
    if let Some(priority) = take_optional_string(&mut patch, "priority") {
        item.priority = priority;
    }
    if let Some(due_at_ms) = take_optional_i64(&mut patch, "due_at_ms") {
        item.due_at_ms = due_at_ms;
    }
    if let Some(tags) = take_string_vec(&mut patch, "tags")? {
        item.tags = tags;
    }
    for (key, value) in patch {
        item.extra.insert(key, value);
    }
    Ok(())
}

fn take_string(props: &mut Map<String, Value>, key: &str) -> Option<String> {
    props
        .remove(key)
        .and_then(|value| value.as_str().map(str::to_string))
}

fn take_optional_string(props: &mut Map<String, Value>, key: &str) -> Option<Option<String>> {
    props.remove(key).map(|value| match value {
        Value::Null => None,
        value => value.as_str().map(str::to_string),
    })
}

fn take_i64(props: &mut Map<String, Value>, key: &str) -> Option<i64> {
    props.remove(key).and_then(|value| value.as_i64())
}

fn take_optional_i64(props: &mut Map<String, Value>, key: &str) -> Option<Option<i64>> {
    props.remove(key).map(|value| match value {
        Value::Null => None,
        value => value.as_i64(),
    })
}

fn take_string_vec(
    props: &mut Map<String, Value>,
    key: &str,
) -> GraphStoreResult<Option<Vec<String>>> {
    match props.remove(key) {
        Some(value) => serde_json::from_value(value)
            .map(Some)
            .map_err(|error| GraphStoreError::new("commonplace_object_patch", error.to_string())),
        None => Ok(None),
    }
}

fn neighbor_query_for(id: &str, dir: EdgeDirection) -> NeighborQuery {
    match dir {
        EdgeDirection::Out => NeighborQuery::out(id),
        EdgeDirection::In => NeighborQuery::in_(id),
    }
}

fn shape_matches(accepts: &ObjectShapeMatch, shape: &ObjectShape) -> bool {
    let shape_types = shape
        .types
        .iter()
        .map(|type_ref| normalize_type_ref(type_ref))
        .collect::<BTreeSet<_>>();
    if accepts
        .required_types
        .iter()
        .map(|type_ref| normalize_type_ref(type_ref))
        .any(|type_ref| !shape_types.contains(&type_ref))
    {
        return false;
    }

    let shape_fields = shape.fields.iter().cloned().collect::<BTreeSet<_>>();
    if accepts
        .required_fields
        .iter()
        .any(|field| !shape_fields.contains(field))
    {
        return false;
    }

    if accepts.required_axes.spatial && !shape.axes.spatial {
        return false;
    }
    if accepts.required_axes.temporal && !shape.axes.temporal {
        return false;
    }
    if accepts.required_axes.embeddable && !shape.axes.embeddable {
        return false;
    }

    if let Some(cardinality) = accepts.cardinality {
        let matches = match cardinality {
            CardinalityRequirement::Any => true,
            CardinalityRequirement::One => shape.cardinality == ObjectCardinality::One,
            CardinalityRequirement::Many => shape.cardinality == ObjectCardinality::Many,
        };
        if !matches {
            return false;
        }
    }

    if accepts.requires_relation && shape.relations.is_empty() {
        return false;
    }

    if let Some(required_edge) = &accepts.required_edge {
        return shape
            .relations
            .iter()
            .any(|relation| relation_matches(required_edge, relation));
    }

    true
}

fn relation_matches(required: &ShapeRelationMatch, relation: &ShapeRelation) -> bool {
    if required
        .edge
        .as_ref()
        .map(|edge| edge != &relation.edge)
        .unwrap_or(false)
    {
        return false;
    }
    if required.dir.map(|dir| dir != relation.dir).unwrap_or(false) {
        return false;
    }
    if required
        .target
        .as_ref()
        .map(|target| relation.target.as_ref() != Some(target))
        .unwrap_or(false)
    {
        return false;
    }
    true
}

fn object_edge_id(edge: &str, from: &str, to: &str) -> String {
    format!("object:{}:{from}:{to}", normalize_type_ref(edge))
}

fn normalize_type_ref(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .replace('_', "-")
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ABOUT_EDGE;
    use crate::InMemoryBlobStore;
    use rustyred_thg_core::InMemoryGraphStore;
    use serde_json::json;

    fn fresh() -> Commonplace<InMemoryGraphStore, InMemoryBlobStore> {
        Commonplace::new(InMemoryGraphStore::new(), InMemoryBlobStore::new())
    }

    #[test]
    fn task_board_query_infers_shape_and_views() {
        let mut cp = fresh();
        let note = cp.put_item(Item::note("Knowledge", "background")).unwrap();
        let first = cp
            .put_item(
                Item::task("First task", "ship it")
                    .with_status("todo")
                    .with_due_at(10),
            )
            .unwrap();
        let second = cp
            .put_item(
                Item::task("Second task", "ship it too")
                    .with_status("doing")
                    .with_due_at(20),
            )
            .unwrap();
        cp.put_item(
            Item::task("Done task", "ignore")
                .with_status("done")
                .with_due_at(5),
        )
        .unwrap();
        cp.link_about(&first.id, &note.id).unwrap();
        cp.link_about(&second.id, &note.id).unwrap();

        let query = ObjectQuery::new(["task"])
            .with_where(Predicate::not_eq("status", json!("done")))
            .with_traverse(EdgeWalk::out(ABOUT_EDGE))
            .with_rank(Ranker::field("due_at_ms", RankDirection::Asc));
        let set = cp.query_object_set(query).unwrap();

        assert_eq!(set.objects.len(), 2);
        assert_eq!(set.objects[0].id, first.id);
        assert_eq!(
            set.live,
            Some(LiveBinding::Poll {
                interval_ms: DEFAULT_RECORD_POLL_INTERVAL_MS
            })
        );
        assert_eq!(set.shape.types, vec!["task".to_string()]);
        assert!(set.shape.fields.contains(&"status".to_string()));
        assert!(set.shape.axes.temporal);
        assert!(set
            .shape
            .relations
            .iter()
            .any(|relation| relation.edge == ABOUT_EDGE));

        let views = ViewRegistry::default_commonplace()
            .views_for(&set.shape)
            .into_iter()
            .map(|view| view.id)
            .collect::<BTreeSet<_>>();
        assert!(views.contains("table"));
        assert!(views.contains("board"));
        assert!(views.contains("card"));
        assert!(views.contains("timeline"));
        assert!(views.contains("graph"));
    }

    #[test]
    fn object_actions_apply_mutations_and_keep_invocations_declarative() {
        let mut cp = fresh();
        let mut props = Map::new();
        props.insert("title".to_string(), json!("Investigate"));
        props.insert("status".to_string(), json!("todo"));
        let create = cp
            .emit_object_action(
                ObjectAction::Create {
                    type_ref: "task".to_string(),
                    props,
                },
                Some("codex".to_string()),
            )
            .unwrap();
        assert_eq!(create.status, ObjectActionStatus::Applied);
        let task_id = create.target_ids[0].clone();

        let mut patch = Map::new();
        patch.insert("status".to_string(), json!("doing"));
        let update = cp
            .emit_object_action(
                ObjectAction::Update {
                    id: task_id.clone(),
                    patch,
                },
                Some("codex".to_string()),
            )
            .unwrap();
        assert_eq!(update.graph_transform.as_deref(), Some("Item.patch"));
        assert_eq!(
            cp.get_item(&task_id).unwrap().unwrap().status.as_deref(),
            Some("doing")
        );

        let note = cp.put_item(Item::note("Context", "body")).unwrap();
        let link = cp
            .emit_object_action(
                ObjectAction::Link {
                    from: task_id.clone(),
                    edge: ABOUT_EDGE.to_string(),
                    to: note.id.clone(),
                    confidence: Some(0.8),
                },
                Some("codex".to_string()),
            )
            .unwrap();
        assert_eq!(link.status, ObjectActionStatus::Applied);
        assert_eq!(cp.task_about(&task_id).unwrap(), vec![note.id]);

        let dispatch = cp
            .emit_object_action(
                ObjectAction::Dispatch {
                    job: JobSpec {
                        name: "applyPatch".to_string(),
                        ..JobSpec::default()
                    },
                },
                Some("codex".to_string()),
            )
            .unwrap();
        assert_eq!(dispatch.status, ObjectActionStatus::Accepted);
    }

    #[test]
    fn view_matching_is_shape_based_not_task_specific() {
        let shape = ObjectShape {
            types: vec!["issue".to_string()],
            fields: vec![
                "title".to_string(),
                "status".to_string(),
                "due_at_ms".to_string(),
            ],
            relations: vec![ShapeRelation {
                edge: ABOUT_EDGE.to_string(),
                dir: EdgeDirection::Out,
                target: None,
            }],
            axes: TypeAxes {
                temporal: true,
                ..TypeAxes::default()
            },
            cardinality: ObjectCardinality::Many,
        };

        let views = ViewRegistry::default_commonplace()
            .views_for(&shape)
            .into_iter()
            .map(|view| view.id)
            .collect::<BTreeSet<_>>();

        assert!(views.contains("board"));
        assert!(views.contains("timeline"));
        assert!(views.contains("graph"));
    }

    #[test]
    fn action_protocol_serializes_stable_kind_tags() {
        let action = ObjectAction::RunAgent {
            target: ObjectActionTarget::Query(ObjectQuery::new(["patch"])),
            tier: AgentTier::Difficult,
        };
        let value = serde_json::to_value(action).unwrap();

        assert_eq!(value.get("kind"), Some(&json!("run_agent")));
        assert_eq!(value.get("tier"), Some(&json!("difficult")));
    }
}
