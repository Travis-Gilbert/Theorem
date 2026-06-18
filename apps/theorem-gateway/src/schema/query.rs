//! Query resolvers. Each is a thin call into one theorem-grpc method, mapped to
//! a browser-facing type. Read-only and directly demoable. The three
//! recomputable reads (search, gapWalk, searchCode) are cache-aside when Valkey
//! is configured. `askAgent` lives here too (it is read-only) and delegates the
//! orchestration to the `agent` module.

use async_graphql::{Context, Object, Result};
use tonic::Request;

use crate::pb::{code, search};
use crate::schema::scene::{self, OriginInput, SceneRef};
use crate::schema::types::{
    AgentAnswer, CodeContextBlock, CodeSymbol, GraphNode, KnowledgeGraph, SearchHit,
};
use crate::schema::{agent, cached, gateway_ctx, map_status, SearchMode};

/// Default fan-out for list reads before the browser narrows further.
const DEFAULT_LIMIT: u64 = 20;

pub struct Query;

#[Object]
impl Query {
    /// Theseus search over the live substrate. Returns admitted hits
    /// (prior knowledge + new evidence) or an honest-empty list — never
    /// fabricated. -> SearchService.Search
    async fn search(
        &self,
        ctx: &Context<'_>,
        query: String,
        mode: Option<SearchMode>,
    ) -> Result<Vec<SearchHit>> {
        let gw = gateway_ctx(ctx)?;
        let mode = mode.unwrap_or_default();
        let key = format!("search:{}:{}", mode.as_proto(), query);
        let mut client = gw.search.clone();
        cached(&gw.cache, &key, async move {
            let req = search::SearchRequest {
                query,
                mode: mode.as_proto(),
                top_k: DEFAULT_LIMIT as u32,
                ..Default::default()
            };
            let resp = client
                .search(Request::new(req))
                .await
                .map_err(map_status)?
                .into_inner();
            let mut hits: Vec<SearchHit> = Vec::with_capacity(
                resp.prior_knowledge.len() + resp.new_evidence.len(),
            );
            hits.extend(resp.prior_knowledge.into_iter().map(SearchHit::from));
            hits.extend(resp.new_evidence.into_iter().map(SearchHit::from));
            Ok(hits)
        })
        .await
    }

    /// Single-round PPR gap-walk from a seed, returned as a knowledge graph
    /// (nodes are admitted evidence; edges are empty — gap-walk surfaces ranked
    /// evidence, not topology). -> SearchService.GapWalk
    async fn gap_walk(&self, ctx: &Context<'_>, seed: String) -> Result<KnowledgeGraph> {
        let gw = gateway_ctx(ctx)?;
        let key = format!("gapwalk:{seed}");
        let mut client = gw.search.clone();
        cached(&gw.cache, &key, async move {
            let req = search::GapWalkRequest {
                query: seed,
                mode: SearchMode::Deep.as_proto(),
                max_rounds: 1,
                ppr_alpha: 0.15,
                ppr_top_k: DEFAULT_LIMIT as u32,
                ..Default::default()
            };
            let resp = client
                .gap_walk(Request::new(req))
                .await
                .map_err(map_status)?
                .into_inner();
            Ok(KnowledgeGraph::from_gap_walk(resp))
        })
        .await
    }

    /// Provenance for a graph node: the root node of its evidence-and-gap-closure
    /// history, or null if none. -> SearchService.Provenance
    async fn provenance(&self, ctx: &Context<'_>, node_id: String) -> Result<Option<GraphNode>> {
        let gw = gateway_ctx(ctx)?;
        let mut client = gw.search.clone();
        let req = search::ProvenanceRequest {
            result_id: node_id,
            max_depth: 0,
        };
        let resp = client
            .provenance(Request::new(req))
            .await
            .map_err(map_status)?
            .into_inner();
        // Prefer the node the graph says is its root; fall back to the first.
        let root_id = resp.root_result_id;
        let nodes = resp.nodes;
        let root = nodes
            .iter()
            .find(|n| n.node_id == root_id)
            .cloned()
            .or_else(|| nodes.into_iter().next());
        Ok(root.map(GraphNode::from))
    }

    /// Search indexed code symbols in a repo. -> CodeCrawlerService.SearchCode
    async fn search_code(
        &self,
        ctx: &Context<'_>,
        query: String,
        repo_id: String,
    ) -> Result<Vec<CodeSymbol>> {
        let gw = gateway_ctx(ctx)?;
        let tenant_id = gw.config.tenant_id.clone();
        let key = format!("searchcode:{tenant_id}:{repo_id}:{query}");
        let mut client = gw.code.clone();
        cached(&gw.cache, &key, async move {
            let req = code::SearchCodeRequest {
                tenant_id,
                query,
                repo_id,
                limit: DEFAULT_LIMIT,
                ..Default::default()
            };
            let resp = client
                .search_code(Request::new(req))
                .await
                .map_err(map_status)?
                .into_inner();
            Ok(resp.hits.into_iter().map(CodeSymbol::from).collect())
        })
        .await
    }

    /// Expand a code symbol into its AST call/dependency subgraph.
    /// -> CodeCrawlerService.ExploreCode
    async fn explore_code(&self, ctx: &Context<'_>, symbol_id: String) -> Result<KnowledgeGraph> {
        let gw = gateway_ctx(ctx)?;
        let mut client = gw.code.clone();
        let req = code::ExploreCodeRequest {
            tenant_id: gw.config.tenant_id.clone(),
            node_id: symbol_id,
            max_depth: 2,
            limit: 50,
            ..Default::default()
        };
        let resp = client
            .explore_code(Request::new(req))
            .await
            .map_err(map_status)?
            .into_inner();
        Ok(KnowledgeGraph::from_explore(resp))
    }

    /// Expand a file or symbol hit into surrounding source.
    /// `target` is treated as a file path when it contains a path separator,
    /// otherwise as a symbol node id. -> CodeCrawlerService.CodeContext
    async fn code_context(
        &self,
        ctx: &Context<'_>,
        repo_id: String,
        target: String,
    ) -> Result<CodeContextBlock> {
        let gw = gateway_ctx(ctx)?;
        let mut client = gw.code.clone();
        let (node_id, file_path) = if target.contains('/') {
            (String::new(), target)
        } else {
            (target, String::new())
        };
        let req = code::CodeContextRequest {
            tenant_id: gw.config.tenant_id.clone(),
            node_id,
            repo_id,
            file_path,
            before_lines: 5,
            after_lines: 5,
            max_chars: 4000,
        };
        let resp = client
            .code_context(Request::new(req))
            .await
            .map_err(map_status)?
            .into_inner();
        Ok(CodeContextBlock::from(resp))
    }

    /// Compact symbol summary with trust tier and graph evidence.
    /// -> CodeCrawlerService.ExplainCode
    async fn explain_code(&self, ctx: &Context<'_>, symbol_id: String) -> Result<CodeSymbol> {
        let gw = gateway_ctx(ctx)?;
        let mut client = gw.code.clone();
        let req = code::ExplainCodeRequest {
            tenant_id: gw.config.tenant_id.clone(),
            node_id: symbol_id,
            max_chars: 4000,
            ..Default::default()
        };
        let resp = client
            .explain_code(Request::new(req))
            .await
            .map_err(map_status)?
            .into_inner();
        Ok(CodeSymbol::from_explain(resp))
    }

    /// The showpiece: answer a question grounded in a graph the visitor just
    /// built or selected, returning the graph context that fed the model.
    /// Rate-limited per IP. -> assembles graph context, then POSTs GL-Fusion.
    async fn ask_agent(
        &self,
        ctx: &Context<'_>,
        question: String,
        scope: agent::AgentScope,
    ) -> Result<AgentAnswer> {
        agent::resolve_ask_agent(ctx, question, scope).await
    }

    /// SceneOS add-on: build the KG for the input, get the model's explanation,
    /// compile a force-graph `ScenePackageV2`, store it, and return a `SceneRef`
    /// the browser embeds via `GET /scene/{sceneId}`. Rate-limited per IP.
    async fn scene_for_input(
        &self,
        ctx: &Context<'_>,
        input: String,
        scope: agent::AgentScope,
        origin: Option<OriginInput>,
    ) -> Result<SceneRef> {
        scene::resolve_scene_for_input(ctx, input, scope, origin).await
    }
}
