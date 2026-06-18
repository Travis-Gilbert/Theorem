//! The showpiece resolver: `askAgent`.
//!
//! The point of the demo is that the model answers grounded in a graph the
//! visitor just built or selected, and the response carries the graph context
//! the model used. `contextNodes` is the differentiator: the UI renders the
//! answer next to the graph the model read.
//!
//! Two scopes:
//!   - code-KG (`repoId`): searchCode -> exploreCode (+ codeContext) around the
//!     symbols matched from the question.
//!   - instant-KG (`seed`): gapWalk (single-round PPR) from the seed.
//!
//! The assembled context is sent to GL-Fusion as a first-class structured graph
//! input plus a flattened prompt (the serving contract is the spec's open item;
//! see clients.rs). Rate-limited per IP.

use async_graphql::{Context, InputObject, Result};
use tonic::Request;

use crate::clients::{GatewayContext, ModelGraphContext, ModelGraphEdge, ModelGraphNode, ModelSource};
use crate::pb::{code, search};
use crate::schema::types::{AgentAnswer, GraphEdge, GraphNode, SearchHit};
use crate::schema::{enforce_rate_limit, gateway_ctx, map_status, SearchMode};

/// How many code symbols to pull as the matched set for a code-scope question.
const CODE_MATCH_LIMIT: u64 = 6;
/// Truncation cap for the inlined source snippet handed to the model.
const SOURCE_SNIPPET_CHARS: usize = 2000;

/// The scope of an `askAgent` call: exactly one of `repoId` (code-KG) or `seed`
/// (instant-KG over the substrate) must be set.
#[derive(InputObject, Default)]
pub struct AgentScope {
    /// Code-KG scope: ground the answer in this repo's code graph.
    pub repo_id: Option<String>,
    /// Instant-KG scope: ground the answer in a PPR gap-walk from this seed.
    pub seed: Option<String>,
}

enum ResolvedScope {
    Code(String),
    Seed(String),
}

impl AgentScope {
    fn resolve(self) -> Result<ResolvedScope> {
        match (self.repo_id, self.seed) {
            (Some(repo_id), None) if !repo_id.trim().is_empty() => {
                Ok(ResolvedScope::Code(repo_id))
            }
            (None, Some(seed)) if !seed.trim().is_empty() => Ok(ResolvedScope::Seed(seed)),
            (Some(_), Some(_)) => Err(async_graphql::Error::new(
                "askAgent scope must set exactly one of { repoId } or { seed }, not both",
            )),
            _ => Err(async_graphql::Error::new(
                "askAgent scope must set exactly one of { repoId } or { seed }",
            )),
        }
    }
}

/// Assembled context shared between scope branches before the model call.
/// Crate-visible so the SceneOS resolver (`scene.rs`) reuses the exact same
/// graph-context assembly that `askAgent` uses.
pub(crate) struct AssembledContext {
    /// Graph nodes returned to the browser as `contextNodes` / scene atoms.
    pub nodes: Vec<GraphNode>,
    /// Graph edges (code scope only; instant-KG gap-walk has none).
    pub edges: Vec<GraphEdge>,
    /// Hits returned to the browser as `sources`.
    pub sources: Vec<SearchHit>,
    /// Source snippets handed to the model (may include inlined code context).
    pub model_sources: Vec<ModelSource>,
}

/// Assemble the graph context for a scope. Shared by `askAgent` and
/// `sceneForInput`. Consumes `scope` (validated to exactly one variant).
pub(crate) async fn assemble_for_scope(
    gw: &GatewayContext,
    question: &str,
    scope: AgentScope,
) -> Result<AssembledContext> {
    match scope.resolve()? {
        ResolvedScope::Code(repo_id) => assemble_code_scope(gw, question, &repo_id).await,
        ResolvedScope::Seed(seed) => assemble_seed_scope(gw, &seed).await,
    }
}

/// Project the assembled context into the model's graph-context input.
pub(crate) fn model_context_from(assembled: &AssembledContext) -> ModelGraphContext {
    ModelGraphContext {
        nodes: assembled.nodes.iter().map(to_model_node).collect(),
        edges: assembled.edges.iter().map(to_model_edge).collect(),
        sources: assembled
            .model_sources
            .iter()
            .map(|s| ModelSource {
                id: s.id.clone(),
                title: s.title.clone(),
                snippet: s.snippet.clone(),
            })
            .collect(),
    }
}

pub async fn resolve_ask_agent(
    ctx: &Context<'_>,
    question: String,
    scope: AgentScope,
) -> Result<AgentAnswer> {
    enforce_rate_limit(ctx).await?;
    let gw = gateway_ctx(ctx)?;

    let assembled = assemble_for_scope(gw, &question, scope).await?;
    let model_ctx = model_context_from(&assembled);

    let model_answer = gw
        .model
        .ask(&question, &model_ctx)
        .await
        .map_err(async_graphql::Error::new)?;

    Ok(AgentAnswer {
        answer: model_answer.answer,
        context_nodes: assembled.nodes,
        sources: assembled.sources,
        model: model_answer.model,
    })
}

/// Code-KG scope: match symbols, expand the top match into its call/dependency
/// subgraph, and inline its surrounding source for the model.
async fn assemble_code_scope(
    gw: &GatewayContext,
    question: &str,
    repo_id: &str,
) -> Result<AssembledContext> {
    let tenant_id = gw.config.tenant_id.clone();
    let mut code = gw.code.clone();

    let search_resp = code
        .search_code(Request::new(code::SearchCodeRequest {
            tenant_id: tenant_id.clone(),
            query: question.to_string(),
            repo_id: repo_id.to_string(),
            limit: CODE_MATCH_LIMIT,
            ..Default::default()
        }))
        .await
        .map_err(map_status)?
        .into_inner();
    let hits = search_resp.hits;

    let sources: Vec<SearchHit> = hits.iter().cloned().map(SearchHit::from).collect();
    let mut model_sources: Vec<ModelSource> = hits
        .iter()
        .map(|h| ModelSource {
            id: h.node_id.clone(),
            title: h.name.clone(),
            snippet: h.snippet.clone(),
        })
        .collect();

    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut edges: Vec<GraphEdge> = Vec::new();

    if let Some(top) = hits.first() {
        let explore = code
            .explore_code(Request::new(code::ExploreCodeRequest {
                tenant_id: tenant_id.clone(),
                node_id: top.node_id.clone(),
                max_depth: 2,
                limit: 50,
                ..Default::default()
            }))
            .await
            .map_err(map_status)?
            .into_inner();
        if let Some(focus) = explore.focus {
            nodes.push(GraphNode::from(focus));
        }
        nodes.extend(explore.related_symbols.into_iter().map(GraphNode::from));
        edges.extend(explore.edges.into_iter().map(GraphEdge::from));

        let context = code
            .code_context(Request::new(code::CodeContextRequest {
                tenant_id: tenant_id.clone(),
                node_id: top.node_id.clone(),
                repo_id: repo_id.to_string(),
                before_lines: 5,
                after_lines: 5,
                max_chars: 4000,
                ..Default::default()
            }))
            .await
            .map_err(map_status)?
            .into_inner();
        if !context.context.trim().is_empty() {
            model_sources.push(ModelSource {
                id: context.symbol_id,
                title: context.file_path,
                snippet: truncate_chars(&context.context, SOURCE_SNIPPET_CHARS),
            });
        }
    }

    // If the explore graph was empty, fall back to the matched hits as nodes so
    // the model still receives the context it has.
    if nodes.is_empty() {
        nodes = hits
            .into_iter()
            .map(|h| GraphNode {
                id: h.node_id,
                label: h.name,
                kind: h.kind,
                score: h.score,
            })
            .collect();
    }

    Ok(AssembledContext {
        nodes,
        edges,
        sources,
        model_sources,
    })
}

/// Instant-KG scope: a single-round PPR gap-walk from the seed over the
/// substrate. The admitted evidence becomes both contextNodes and sources.
async fn assemble_seed_scope(gw: &GatewayContext, seed: &str) -> Result<AssembledContext> {
    let mut search = gw.search.clone();
    let resp = search
        .gap_walk(Request::new(search::GapWalkRequest {
            query: seed.to_string(),
            mode: SearchMode::Deep.as_proto(),
            max_rounds: 1,
            ppr_alpha: 0.15,
            ppr_top_k: 20,
            ..Default::default()
        }))
        .await
        .map_err(map_status)?
        .into_inner();

    let evidence = resp.admitted_evidence;
    let nodes: Vec<GraphNode> = evidence.iter().cloned().map(GraphNode::from).collect();
    let sources: Vec<SearchHit> = evidence.iter().cloned().map(SearchHit::from).collect();
    let model_sources: Vec<ModelSource> = evidence
        .into_iter()
        .map(|r| ModelSource {
            id: r.result_id,
            title: r.label,
            snippet: r.snippet,
        })
        .collect();

    Ok(AssembledContext {
        nodes,
        edges: Vec::new(),
        sources,
        model_sources,
    })
}

fn to_model_node(n: &GraphNode) -> ModelGraphNode {
    ModelGraphNode {
        id: n.id.clone(),
        label: n.label.clone(),
        kind: n.kind.clone(),
        score: n.score,
    }
}

fn to_model_edge(e: &GraphEdge) -> ModelGraphEdge {
    ModelGraphEdge {
        src: e.src.clone(),
        dst: e.dst.clone(),
        kind: e.kind.clone(),
        weight: e.weight,
    }
}

/// Truncate on a char boundary so a multibyte snippet never panics.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_accepts_exactly_one() {
        let code = AgentScope {
            repo_id: Some("owner/repo".into()),
            seed: None,
        };
        assert!(matches!(code.resolve(), Ok(ResolvedScope::Code(_))));

        let seed = AgentScope {
            repo_id: None,
            seed: Some("graph databases".into()),
        };
        assert!(matches!(seed.resolve(), Ok(ResolvedScope::Seed(_))));
    }

    #[test]
    fn scope_rejects_both_or_neither() {
        let both = AgentScope {
            repo_id: Some("r".into()),
            seed: Some("s".into()),
        };
        assert!(both.resolve().is_err());

        let neither = AgentScope {
            repo_id: None,
            seed: None,
        };
        assert!(neither.resolve().is_err());

        let blank = AgentScope {
            repo_id: Some("   ".into()),
            seed: None,
        };
        assert!(blank.resolve().is_err());
    }

    #[test]
    fn truncate_respects_char_boundary() {
        assert_eq!(truncate_chars("hello", 10), "hello");
        assert_eq!(truncate_chars("hello", 3), "hel");
        // multibyte must not panic
        let s = "héllo wörld";
        assert_eq!(truncate_chars(s, 4).chars().count(), 4);
    }
}
