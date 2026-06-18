//! GraphQL output types and their `From<ProtoMsg>` mappings.
//!
//! Each type is a browser-facing projection of a theorem-grpc proto message.
//! async-graphql renames snake_case fields to camelCase, so `node_count`
//! surfaces as `nodeCount`, `trust_tier` as `trustTier`, etc. — matching the
//! spec's schema. serde derives let the read resolvers cache these values.

use async_graphql::SimpleObject;
use serde::{Deserialize, Serialize};

use crate::pb::{code, search};

// ============================================================================
// Knowledge graph (shared by gapWalk, exploreCode, askAgent contextNodes)
// ============================================================================

#[derive(SimpleObject, Serialize, Deserialize, Clone, Debug)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub score: f64,
}

#[derive(SimpleObject, Serialize, Deserialize, Clone, Debug)]
pub struct GraphEdge {
    pub src: String,
    pub dst: String,
    pub kind: String,
    pub weight: f64,
}

#[derive(SimpleObject, Serialize, Deserialize, Clone, Debug)]
pub struct GraphStats {
    pub node_count: i64,
    pub edge_count: i64,
    /// Gap-walk / explore rounds executed, when the source reports it (0 else).
    pub rounds_executed: i64,
}

#[derive(SimpleObject, Serialize, Deserialize, Clone, Debug)]
pub struct KnowledgeGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub stats: GraphStats,
}

impl KnowledgeGraph {
    fn finalize(nodes: Vec<GraphNode>, edges: Vec<GraphEdge>, rounds: i64) -> Self {
        let stats = GraphStats {
            node_count: nodes.len() as i64,
            edge_count: edges.len() as i64,
            rounds_executed: rounds,
        };
        Self {
            nodes,
            edges,
            stats,
        }
    }

    /// GapWalk returns ranked PPR evidence, not topology, so edges are empty;
    /// nodes are the admitted evidence. This is honest: the gap-walk primitive
    /// surfaces what it admitted, not the edges between admissions.
    pub fn from_gap_walk(resp: search::GapWalkResponse) -> Self {
        let nodes = resp.admitted_evidence.into_iter().map(GraphNode::from).collect();
        Self::finalize(nodes, Vec::new(), resp.rounds_executed as i64)
    }

    /// ExploreCode returns a real subgraph: focus + related symbols as nodes,
    /// AST call/dependency edges as edges.
    pub fn from_explore(resp: code::ExploreCodeResponse) -> Self {
        let mut nodes: Vec<GraphNode> = Vec::new();
        if let Some(focus) = resp.focus {
            nodes.push(GraphNode::from(focus));
        }
        nodes.extend(resp.related_symbols.into_iter().map(GraphNode::from));
        let edges = resp.edges.into_iter().map(GraphEdge::from).collect();
        Self::finalize(nodes, edges, 0)
    }
}

impl From<search::SearchResult> for GraphNode {
    fn from(r: search::SearchResult) -> Self {
        Self {
            id: r.result_id,
            label: r.label,
            kind: r.kind,
            score: r.relevance_score,
        }
    }
}

impl From<code::CodeSymbol> for GraphNode {
    fn from(s: code::CodeSymbol) -> Self {
        Self {
            id: s.node_id,
            label: s.name,
            kind: s.kind,
            score: 0.0,
        }
    }
}

impl From<search::ProvenanceNode> for GraphNode {
    fn from(n: search::ProvenanceNode) -> Self {
        Self {
            id: n.node_id,
            label: n.label,
            kind: n.kind,
            score: 0.0,
        }
    }
}

impl From<code::CodeGraphEdge> for GraphEdge {
    fn from(e: code::CodeGraphEdge) -> Self {
        Self {
            src: e.from_node_id,
            dst: e.to_node_id,
            kind: e.edge_type,
            weight: 1.0,
        }
    }
}

// ============================================================================
// Search hits
// ============================================================================

#[derive(SimpleObject, Serialize, Deserialize, Clone, Debug)]
pub struct SearchHit {
    pub id: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
    /// Where the evidence came from: the canonical URL when present, else the
    /// source marker ("graph", "web", "source_pair", "fusion").
    pub provenance: String,
}

impl From<search::SearchResult> for SearchHit {
    fn from(r: search::SearchResult) -> Self {
        let provenance = if r.url.trim().is_empty() {
            r.source
        } else {
            r.url
        };
        Self {
            id: r.result_id,
            title: r.label,
            snippet: r.snippet,
            score: r.relevance_score,
            provenance,
        }
    }
}

/// A code search hit, projected into the SearchHit shape for askAgent `sources`.
impl From<code::CodeHit> for SearchHit {
    fn from(h: code::CodeHit) -> Self {
        Self {
            id: h.node_id,
            title: h.name,
            snippet: h.snippet,
            score: h.score,
            provenance: h.file_path,
        }
    }
}

// ============================================================================
// Code symbols + context
// ============================================================================

#[derive(SimpleObject, Serialize, Deserialize, Clone, Debug)]
pub struct CodeSymbol {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub path: String,
    pub trust_tier: String,
    /// Supporting evidence: a snippet/signature for a search or context hit;
    /// the summary plus rendered graph edges for an explain hit.
    pub evidence: Vec<String>,
}

impl From<code::CodeHit> for CodeSymbol {
    fn from(h: code::CodeHit) -> Self {
        let mut evidence = Vec::new();
        if !h.snippet.trim().is_empty() {
            evidence.push(h.snippet);
        }
        Self {
            id: h.node_id,
            name: h.name,
            kind: h.kind,
            path: h.file_path,
            trust_tier: h.trust_tier,
            evidence,
        }
    }
}

impl From<code::CodeSymbol> for CodeSymbol {
    fn from(s: code::CodeSymbol) -> Self {
        let mut evidence = Vec::new();
        if !s.signature.trim().is_empty() {
            evidence.push(s.signature);
        }
        if !s.snippet.trim().is_empty() {
            evidence.push(s.snippet);
        }
        Self {
            id: s.node_id,
            name: s.name,
            kind: s.kind,
            path: s.file_path,
            trust_tier: s.trust_tier,
            evidence,
        }
    }
}

impl CodeSymbol {
    /// ExplainCode folds the symbol summary and graph evidence into `evidence`.
    pub fn from_explain(resp: code::ExplainCodeResponse) -> Self {
        let symbol = resp.symbol.unwrap_or_default();
        let mut evidence = Vec::new();
        if !resp.summary.trim().is_empty() {
            evidence.push(resp.summary);
        }
        if !symbol.signature.trim().is_empty() {
            evidence.push(symbol.signature.clone());
        }
        for edge in resp.edges {
            let line = if edge.evidence.trim().is_empty() {
                format!("{} --{}--> {}", edge.from_name, edge.edge_type, edge.to_name)
            } else {
                format!(
                    "{} --{}--> {} ({})",
                    edge.from_name, edge.edge_type, edge.to_name, edge.evidence
                )
            };
            evidence.push(line);
        }
        Self {
            id: symbol.node_id,
            name: symbol.name,
            kind: symbol.kind,
            path: symbol.file_path,
            trust_tier: symbol.trust_tier,
            evidence,
        }
    }
}

#[derive(SimpleObject, Serialize, Deserialize, Clone, Debug)]
pub struct CodeContextBlock {
    pub path: String,
    pub snippet: String,
    pub symbols: Vec<CodeSymbol>,
}

impl From<code::CodeContextResponse> for CodeContextBlock {
    fn from(resp: code::CodeContextResponse) -> Self {
        Self {
            path: resp.file_path,
            snippet: resp.context,
            symbols: resp.symbols.into_iter().map(CodeSymbol::from).collect(),
        }
    }
}

// ============================================================================
// Ingest receipt
// ============================================================================

#[derive(SimpleObject, Serialize, Deserialize, Clone, Debug)]
pub struct IngestReceipt {
    pub repo_id: String,
    pub node_count: i64,
    pub edge_count: i64,
    /// Content-addressed receipt hash from the code service.
    pub receipt: String,
    /// Async job id (ingest runs on a server-side worker). Poll-free for the
    /// caller when the gateway waited for completion; otherwise use it to poll.
    pub job_id: String,
    /// "submitted" | "running" | "ok" | "budget_exceeded" | "failed".
    pub status: String,
    pub message: String,
    /// JSON-encoded EpistemicRAG instant structural readout from CodeCrawler.
    pub epistemic_readout_json: String,
}

impl From<code::IngestCodebaseResponse> for IngestReceipt {
    fn from(r: code::IngestCodebaseResponse) -> Self {
        Self {
            // Symbols are the graph nodes; the ingest response does not surface
            // an edge count (per-symbol edges are exposed via exploreCode), so
            // edge_count is honestly 0 here.
            repo_id: r.repo_id,
            node_count: r.symbols_indexed as i64,
            edge_count: 0,
            receipt: r.receipt_hash,
            job_id: r.job_id,
            status: r.status,
            message: r.message,
            epistemic_readout_json: r.epistemic_readout_json,
        }
    }
}

// ============================================================================
// Agent answer
// ============================================================================

#[derive(SimpleObject, Serialize, Deserialize, Clone, Debug)]
pub struct AgentAnswer {
    pub answer: String,
    /// The graph context that fed the model — the visible proof that this is a
    /// graph-aware answer and not a generic chatbot.
    pub context_nodes: Vec<GraphNode>,
    pub sources: Vec<SearchHit>,
    pub model: String,
}
