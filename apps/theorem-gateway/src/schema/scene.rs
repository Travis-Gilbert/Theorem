//! Deliverable A: SceneOS scene compilation on the gateway.
//!
//! After the instant-KG or code-KG context is assembled (the same path
//! `askAgent` uses), this compiles a `ScenePackageV2` in process through
//! `scene-os-core`: graph nodes -> atoms (sized by degree centrality, grouped by
//! weakly-connected-component community), edges -> relations (backbone flagged),
//! and the GL-Fusion explanation -> an annotation atom anchored to the most
//! central node. The package is force-graph projected and stashed in a bounded
//! in-memory store; `sceneForInput` returns a `SceneRef { sceneId, url }` the
//! browser embeds via `GET /scene/{sceneId}`.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Mutex;

use async_graphql::{Context, InputObject, Result, SimpleObject};
use scene_os_core::{
    compile_scene_package, AtomLifecycle, SceneAtom, SceneCompileInput, SceneRelation, SceneScene,
    ScenePackageV2,
};
use serde_json::{json, Value};

use crate::schema::agent::{self, AgentScope, AssembledContext};
use crate::schema::types::{GraphEdge, GraphNode};
use crate::schema::{enforce_rate_limit, gateway_ctx};

/// Atom kind for model-explanation atoms (rendered as callouts, not graph nodes).
pub const ANNOTATION_KIND: &str = "annotation";
/// A node needs at least this degree on both ends for its edge to be "backbone".
const BACKBONE_MIN_DEGREE: usize = 2;

/// A handle to a compiled scene: an id plus the URL the browser embeds.
#[derive(SimpleObject, Clone, Debug)]
pub struct SceneRef {
    pub scene_id: String,
    pub url: String,
}

/// The omni-bar screen coordinate the explosion spawns from (Deliverable C
/// reads it from the package provenance). Optional; defaults to viewport center.
#[derive(InputObject, Clone, Copy, Debug)]
pub struct OriginInput {
    pub x: f64,
    pub y: f64,
}

// ============================================================================
// Scene store (ephemeral, bounded, recomputable)
// ============================================================================

/// In-memory bounded store of compiled scenes. The gateway stores nothing
/// durable: scenes are recomputable accelerator state keyed by a content hash,
/// evicted oldest-first past the cap, and lost on restart.
pub struct SceneStore {
    inner: Mutex<SceneStoreInner>,
    cap: usize,
}

struct SceneStoreInner {
    map: HashMap<String, ScenePackageV2>,
    order: VecDeque<String>,
}

impl SceneStore {
    pub fn new(cap: usize) -> Self {
        Self {
            inner: Mutex::new(SceneStoreInner {
                map: HashMap::new(),
                order: VecDeque::new(),
            }),
            cap: cap.max(1),
        }
    }

    pub fn insert(&self, id: String, package: ScenePackageV2) {
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        // insert() returns the prior value: `None` means this id is new, so it
        // joins the eviction order; `Some` means an in-place refresh (same scene
        // recompiled), which neither reorders nor grows the store.
        if guard.map.insert(id.clone(), package).is_none() {
            guard.order.push_back(id);
        }
        // Evict oldest until within the cap.
        while guard.order.len() > self.cap {
            if let Some(old) = guard.order.pop_front() {
                guard.map.remove(&old);
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<ScenePackageV2> {
        let guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        guard.map.get(id).cloned()
    }
}

// ============================================================================
// Resolver
// ============================================================================

pub async fn resolve_scene_for_input(
    ctx: &Context<'_>,
    input: String,
    scope: AgentScope,
    origin: Option<OriginInput>,
) -> Result<SceneRef> {
    enforce_rate_limit(ctx).await?;
    let gw = gateway_ctx(ctx)?;

    // Same context assembly + model call askAgent uses.
    let assembled = agent::assemble_for_scope(gw, &input, scope).await?;
    let model_ctx = agent::model_context_from(&assembled);
    let answer = gw
        .model
        .ask(&input, &model_ctx)
        .await
        .map_err(async_graphql::Error::new)?;

    let package = compile_scene(&input, &assembled, &answer.answer, &answer.model, origin)?;
    let json = serde_json::to_string(&package)
        .map_err(|e| async_graphql::Error::new(format!("scene serialize failed: {e}")))?;
    let scene_id = content_scene_id(&json);
    gw.scenes.insert(scene_id.clone(), package);

    let url = gw.config.scene_url(&scene_id);
    Ok(SceneRef { scene_id, url })
}

/// Build a force-graph `ScenePackageV2` from the assembled context + the model's
/// explanation.
fn compile_scene(
    input: &str,
    assembled: &AssembledContext,
    answer: &str,
    model: &str,
    origin: Option<OriginInput>,
) -> Result<ScenePackageV2> {
    let degrees = degree_map(&assembled.nodes, &assembled.edges);
    let communities = community_map(&assembled.nodes, &assembled.edges);
    let max_degree = degrees.values().copied().max().unwrap_or(0).max(1);

    // Graph nodes -> atoms (community + centrality in metadata; size by weight).
    let mut atoms: Vec<SceneAtom> = assembled
        .nodes
        .iter()
        .map(|node| {
            let degree = degrees.get(&node.id).copied().unwrap_or(0);
            let community = communities.get(&node.id).copied().unwrap_or(0);
            let weight = if node.score > 0.0 {
                node.score
            } else {
                degree as f64 / max_degree as f64
            };
            let mut metadata = BTreeMap::new();
            metadata.insert("community".to_string(), json!(community));
            metadata.insert("centrality".to_string(), json!(degree));
            metadata.insert("score".to_string(), json!(node.score));
            make_atom(
                node.id.clone(),
                if node.kind.is_empty() {
                    "evidence".to_string()
                } else {
                    node.kind.clone()
                },
                Some(node.label.clone()),
                Some(weight),
                metadata,
            )
        })
        .collect();

    // Edges -> relations (backbone = both endpoints are hubs).
    let relations: Vec<SceneRelation> = assembled
        .edges
        .iter()
        .enumerate()
        .map(|(i, edge)| {
            let backbone = degrees.get(&edge.src).copied().unwrap_or(0) >= BACKBONE_MIN_DEGREE
                && degrees.get(&edge.dst).copied().unwrap_or(0) >= BACKBONE_MIN_DEGREE;
            let mut metadata = BTreeMap::new();
            metadata.insert("backbone".to_string(), json!(backbone));
            SceneRelation {
                id: format!("{}->{}#{i}", edge.src, edge.dst),
                source_id: edge.src.clone(),
                target_id: edge.dst.clone(),
                kind: if edge.kind.is_empty() {
                    "related".to_string()
                } else {
                    edge.kind.clone()
                },
                weight: Some(edge.weight),
                color: None,
                opacity: None,
                glyph: None,
                lifecycle: AtomLifecycle::Present,
                metadata,
                source_refs: Vec::new(),
            }
        })
        .collect();

    // The model explanation -> an annotation atom anchored to the top node.
    let anchor = top_central_node(&assembled.nodes, &degrees);
    let mut annotation_meta = BTreeMap::new();
    annotation_meta.insert("model".to_string(), json!(model));
    match &anchor {
        Some(id) => {
            annotation_meta.insert("anchorId".to_string(), json!(id));
            annotation_meta.insert("role".to_string(), json!("node"));
        }
        None => {
            annotation_meta.insert("role".to_string(), json!("global"));
        }
    }
    atoms.push(make_atom(
        "annotation-primary".to_string(),
        ANNOTATION_KIND.to_string(),
        Some(answer.to_string()),
        None,
        annotation_meta,
    ));

    // Provenance carries the omni-bar origin for the explosion enter animation.
    let mut provenance = BTreeMap::new();
    if let Some(o) = origin {
        provenance.insert("origin".to_string(), json!({ "x": o.x, "y": o.y }));
    }
    provenance.insert("sceneInput".to_string(), json!(input));

    compile_scene_package(SceneCompileInput {
        query: input.to_string(),
        answer_type: Some("force_graph".to_string()),
        title: Some(truncate_chars(input, 80)),
        scene: SceneScene { atoms, relations },
        trace_id: None,
        manifest_ref: None,
        provenance,
    })
    .map_err(|e| async_graphql::Error::new(format!("scene compile failed: {e}")))
}

fn make_atom(
    id: String,
    kind: String,
    label: Option<String>,
    weight: Option<f64>,
    metadata: BTreeMap<String, Value>,
) -> SceneAtom {
    SceneAtom {
        id,
        kind,
        label,
        position: None,
        weight,
        color: None,
        opacity: None,
        glyph: None,
        scale: None,
        lifecycle: AtomLifecycle::Present,
        metadata,
        source_refs: Vec::new(),
    }
}

/// Incident-edge count per node id (a cheap, honest centrality proxy).
fn degree_map(nodes: &[GraphNode], edges: &[GraphEdge]) -> HashMap<String, usize> {
    let mut degrees: HashMap<String, usize> =
        nodes.iter().map(|n| (n.id.clone(), 0usize)).collect();
    for edge in edges {
        if let Some(d) = degrees.get_mut(&edge.src) {
            *d += 1;
        }
        if let Some(d) = degrees.get_mut(&edge.dst) {
            *d += 1;
        }
    }
    degrees
}

/// Weakly-connected-component id per node (union-find): the "community".
fn community_map(nodes: &[GraphNode], edges: &[GraphEdge]) -> HashMap<String, usize> {
    let index: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let mut parent: Vec<usize> = (0..nodes.len()).collect();

    fn find(parent: &mut [usize], x: usize) -> usize {
        let mut root = x;
        while parent[root] != root {
            root = parent[root];
        }
        let mut cur = x;
        while parent[cur] != root {
            let next = parent[cur];
            parent[cur] = root;
            cur = next;
        }
        root
    }

    for edge in edges {
        if let (Some(&a), Some(&b)) =
            (index.get(edge.src.as_str()), index.get(edge.dst.as_str()))
        {
            let ra = find(&mut parent, a);
            let rb = find(&mut parent, b);
            if ra != rb {
                parent[ra] = rb;
            }
        }
    }

    let mut compact: HashMap<usize, usize> = HashMap::new();
    let mut next = 0usize;
    let mut out = HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        let root = find(&mut parent, i);
        let cid = *compact.entry(root).or_insert_with(|| {
            let c = next;
            next += 1;
            c
        });
        out.insert(node.id.clone(), cid);
    }
    out
}

/// The most central node (max degree, tie-broken by score) to anchor the answer.
fn top_central_node(nodes: &[GraphNode], degrees: &HashMap<String, usize>) -> Option<String> {
    nodes
        .iter()
        .max_by(|a, b| {
            let da = degrees.get(&a.id).copied().unwrap_or(0);
            let db = degrees.get(&b.id).copied().unwrap_or(0);
            da.cmp(&db)
                .then(a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal))
        })
        .map(|n| n.id.clone())
}

/// Deterministic content id for a serialized package (no RNG dependency).
fn content_scene_id(package_json: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    package_json.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, score: f64) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            label: format!("label-{id}"),
            kind: "symbol".to_string(),
            score,
        }
    }

    fn edge(src: &str, dst: &str) -> GraphEdge {
        GraphEdge {
            src: src.to_string(),
            dst: dst.to_string(),
            kind: "CALLS".to_string(),
            weight: 1.0,
        }
    }

    #[test]
    fn degree_counts_both_endpoints() {
        let nodes = vec![node("a", 0.0), node("b", 0.0), node("c", 0.0)];
        let edges = vec![edge("a", "b"), edge("b", "c")];
        let d = degree_map(&nodes, &edges);
        assert_eq!(d["a"], 1);
        assert_eq!(d["b"], 2);
        assert_eq!(d["c"], 1);
    }

    #[test]
    fn community_splits_disconnected_subgraphs() {
        // a-b connected; c-d connected; two components.
        let nodes = vec![node("a", 0.0), node("b", 0.0), node("c", 0.0), node("d", 0.0)];
        let edges = vec![edge("a", "b"), edge("c", "d")];
        let c = community_map(&nodes, &edges);
        assert_eq!(c["a"], c["b"]);
        assert_eq!(c["c"], c["d"]);
        assert_ne!(c["a"], c["c"]);
    }

    #[test]
    fn top_central_is_highest_degree() {
        let nodes = vec![node("a", 0.0), node("hub", 0.0), node("c", 0.0)];
        let edges = vec![edge("a", "hub"), edge("hub", "c")];
        let d = degree_map(&nodes, &edges);
        assert_eq!(top_central_node(&nodes, &d).as_deref(), Some("hub"));
    }

    #[test]
    fn compile_produces_force_graph_with_annotation() {
        let assembled = AssembledContext {
            nodes: vec![node("a", 0.5), node("b", 0.0)],
            edges: vec![edge("a", "b")],
            sources: Vec::new(),
            model_sources: Vec::new(),
        };
        let pkg = compile_scene(
            "what does a do?",
            &assembled,
            "a calls b.",
            "test-model",
            Some(OriginInput { x: 10.0, y: 20.0 }),
        )
        .expect("compile");
        assert_eq!(pkg.projection.id, "force_graph");
        // 2 graph atoms + 1 annotation atom
        assert_eq!(pkg.atoms.len(), 3);
        let annotation = pkg
            .atoms
            .iter()
            .find(|a| a.kind == ANNOTATION_KIND)
            .expect("annotation atom present");
        assert_eq!(annotation.label.as_deref(), Some("a calls b."));
        assert_eq!(annotation.metadata["anchorId"], json!("a"));
        assert_eq!(pkg.provenance["origin"]["x"], json!(10.0));
    }

    #[test]
    fn scene_store_evicts_oldest_past_cap() {
        let store = SceneStore::new(2);
        let assembled = AssembledContext {
            nodes: vec![node("a", 0.0)],
            edges: Vec::new(),
            sources: Vec::new(),
            model_sources: Vec::new(),
        };
        let pkg = compile_scene("q", &assembled, "ans", "m", None).unwrap();
        store.insert("one".into(), pkg.clone());
        store.insert("two".into(), pkg.clone());
        store.insert("three".into(), pkg);
        assert!(store.get("one").is_none(), "oldest evicted");
        assert!(store.get("two").is_some());
        assert!(store.get("three").is_some());
    }

    #[test]
    fn content_id_is_stable() {
        assert_eq!(content_scene_id("abc"), content_scene_id("abc"));
        assert_ne!(content_scene_id("abc"), content_scene_id("abd"));
    }

    /// Add-on acceptance: a crawled/agent-authored label containing `</script>`
    /// must not break out of the injected payload. Compiling then rendering
    /// through scene-os-web must escape it.
    #[test]
    fn malicious_label_is_escaped_in_rendered_html() {
        let mut node = node("x", 1.0);
        node.label = "</script><script>alert(1)</script>".to_string();
        let assembled = AssembledContext {
            nodes: vec![node],
            edges: Vec::new(),
            sources: Vec::new(),
            model_sources: Vec::new(),
        };
        let pkg = compile_scene("q", &assembled, "ans", "m", None).unwrap();
        let html = scene_os_web::render_scene(&pkg).expect("render");
        assert!(
            !html.contains("</script><script>alert"),
            "raw breakout sequence must not appear in served HTML"
        );
        assert!(
            html.contains("\\u003c/script") || html.contains("\\u003cscript"),
            "the angle brackets must be \\uXXXX-escaped in the payload"
        );
    }
}
