//! Upstream clients, assembled once and held in the GraphQL context.
//!
//! `GatewayContext` is registered with async-graphql's `Data` and carries:
//!   - `search`: generated SearchServiceClient (theorem-grpc)
//!   - `code`:   generated CodeCrawlerServiceClient (theorem-grpc)
//!   - `model`:  GlFusionClient (reqwest, the GL-Fusion 31B endpoint)
//!   - `config`, `limiter`, `cache`: gateway state
//!
//! tonic channels are cheap to clone (they share the connection pool), so the
//! context is `Clone` and resolvers clone the sub-client they need. Channels
//! are built with `connect_lazy`, so the gateway boots even when theorem-grpc
//! is momentarily unreachable; the connection is established on first call.

use std::sync::Arc;

use tonic::transport::{Channel, Endpoint};

use crate::cache::{RateLimiter, ResponseCache};
use crate::config::GatewayConfig;
use crate::pb::{CodeCrawlerServiceClient, SearchServiceClient};
use crate::schema::scene::SceneStore;

/// The graph context shared by every resolver.
#[derive(Clone)]
pub struct GatewayContext {
    pub search: SearchServiceClient<Channel>,
    pub code: CodeCrawlerServiceClient<Channel>,
    pub model: GlFusionClient,
    pub config: Arc<GatewayConfig>,
    pub limiter: Arc<RateLimiter>,
    pub cache: ResponseCache,
    /// Bounded in-memory store of compiled scenes (SceneOS add-on). Shared with
    /// the `GET /scene/{id}` axum handler via the same `Arc`.
    pub scenes: Arc<SceneStore>,
}

impl GatewayContext {
    /// Build the context from resolved config plus the prepared limiter/cache.
    /// The gRPC channel is lazy: a bad/down upstream does not block startup.
    pub fn new(
        config: Arc<GatewayConfig>,
        limiter: Arc<RateLimiter>,
        cache: ResponseCache,
        scenes: Arc<SceneStore>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // from_shared rejects a malformed URI (InvalidUri); surface it so main()
        // can fail fast with a clear message.
        let endpoint: Endpoint = Channel::from_shared(config.grpc_url.clone())
            .inspect_err(|_| {
                tracing::error!("GATEWAY_GRPC_URL_INVALID {}", config.grpc_url);
            })?
            .connect_timeout(std::time::Duration::from_secs(5))
            .tcp_keepalive(Some(std::time::Duration::from_secs(30)));
        let channel = endpoint.connect_lazy();

        let search = SearchServiceClient::new(channel.clone());
        let code = CodeCrawlerServiceClient::new(channel);
        let model = GlFusionClient::new(
            config.glfusion_url.clone(),
            config.glfusion_token.clone(),
            config.glfusion_model.clone(),
        );

        Ok(Self {
            search,
            code,
            model,
            config,
            limiter,
            cache,
            scenes,
        })
    }
}

/// The client IP, injected into each request's GraphQL context by the axum
/// handler (from `X-Forwarded-For` / `X-Real-IP` behind Railway's proxy, else
/// the socket peer). Resolvers read it to key the rate limiter.
#[derive(Clone, Debug)]
pub struct ClientIp(pub String);

// ============================================================================
// GL-Fusion model client
// ============================================================================
//
// THE ONE OPEN ITEM (per the spec): the exact HTTP request/response schema of
// `ghcr.io/travis-gilbert/theseus-gemma-31b-glfusion` as deployed. The model
// reads graph context as a first-class structured input, not only as prompt
// text. Until the served contract is confirmed, this client sends BOTH a
// structured `graph_context` field AND a flattened `prompt` (so either ingestion
// style works), and parses the answer defensively across the few likely
// response shapes. Confirming the real contract is a one-method edit here; no
// resolver changes are required.

/// Structured graph context handed to the model alongside the question.
#[derive(serde::Serialize)]
pub struct ModelGraphContext {
    pub nodes: Vec<ModelGraphNode>,
    pub edges: Vec<ModelGraphEdge>,
    pub sources: Vec<ModelSource>,
}

#[derive(serde::Serialize)]
pub struct ModelGraphNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub score: f64,
}

#[derive(serde::Serialize)]
pub struct ModelGraphEdge {
    pub src: String,
    pub dst: String,
    pub kind: String,
    pub weight: f64,
}

#[derive(serde::Serialize)]
pub struct ModelSource {
    pub id: String,
    pub title: String,
    pub snippet: String,
}

/// The answer text plus the model name the gateway returns to the browser.
pub struct ModelAnswer {
    pub answer: String,
    pub model: String,
}

#[derive(Clone)]
pub struct GlFusionClient {
    http: reqwest::Client,
    base_url: Option<String>,
    token: Option<String>,
    /// Reported model name when the endpoint does not echo one.
    fallback_model: String,
}

impl GlFusionClient {
    pub fn new(base_url: Option<String>, token: Option<String>, fallback_model: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
            token,
            fallback_model,
        }
    }

    pub fn is_configured(&self) -> bool {
        self.base_url.is_some()
    }

    /// POST the question + assembled graph context to GL-Fusion and return the
    /// answer. When `GLFUSION_URL` is unset, returns an honest "not configured"
    /// answer so askAgent still demonstrates the graph-context half.
    pub async fn ask(
        &self,
        question: &str,
        context: &ModelGraphContext,
    ) -> Result<ModelAnswer, String> {
        let Some(base_url) = self.base_url.as_deref() else {
            return Ok(ModelAnswer {
                answer: "GL-Fusion endpoint is not configured (set GLFUSION_URL). \
                         The graph context that would have fed the model is returned below."
                    .to_string(),
                model: "unconfigured".to_string(),
            });
        };

        // PROVISIONAL request body (see the open-item note above). `prompt` is a
        // text rendering of the same context so a prompt-only serving path also
        // works; `graph_context` is the first-class structured input.
        let body = serde_json::json!({
            "question": question,
            "prompt": render_prompt(question, context),
            "graph_context": {
                "nodes": context.nodes,
                "edges": context.edges,
                "sources": context.sources,
            },
            "max_tokens": 1024,
            "stream": false,
        });

        let mut request = self.http.post(base_url).json(&body);
        if let Some(token) = self.token.as_deref() {
            request = request.bearer_auth(token);
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("GL-Fusion request failed: {e}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            return Err(format!("GL-Fusion returned {status}: {detail}"));
        }
        let payload: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("GL-Fusion response was not JSON: {e}"))?;

        let answer = extract_answer(&payload)
            .ok_or_else(|| "GL-Fusion response had no recognizable answer field".to_string())?;
        let model = payload
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.fallback_model.clone());

        Ok(ModelAnswer { answer, model })
    }
}

/// Flatten the graph context into a text prompt as a serving-path fallback.
fn render_prompt(question: &str, context: &ModelGraphContext) -> String {
    let mut out = String::new();
    out.push_str("You are a graph-aware assistant. Answer the question grounded ONLY in the\n");
    out.push_str("graph context below. If the context is insufficient, say so.\n\n");
    out.push_str("## Graph nodes\n");
    for node in &context.nodes {
        out.push_str(&format!("- [{}] {} ({})\n", node.kind, node.label, node.id));
    }
    if !context.edges.is_empty() {
        out.push_str("\n## Graph edges\n");
        for edge in &context.edges {
            out.push_str(&format!("- {} --{}--> {}\n", edge.src, edge.kind, edge.dst));
        }
    }
    if !context.sources.is_empty() {
        out.push_str("\n## Sources\n");
        for source in &context.sources {
            out.push_str(&format!("- {}: {}\n", source.title, source.snippet));
        }
    }
    out.push_str(&format!("\n## Question\n{question}\n"));
    out
}

/// Parse the answer across the few likely serving shapes (the contract is the
/// open item). Tries `answer`, `text`, `output`, `response`, then an
/// OpenAI-style `choices[0].message.content` / `choices[0].text`.
fn extract_answer(payload: &serde_json::Value) -> Option<String> {
    for key in ["answer", "text", "output", "response", "generated_text"] {
        if let Some(value) = payload.get(key).and_then(|v| v.as_str()) {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    if let Some(choice) = payload.get("choices").and_then(|c| c.get(0)) {
        if let Some(content) = choice
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
        {
            return Some(content.to_string());
        }
        if let Some(text) = choice.get("text").and_then(|t| t.as_str()) {
            return Some(text.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ModelGraphContext {
        ModelGraphContext {
            nodes: vec![ModelGraphNode {
                id: "n1".into(),
                label: "rank".into(),
                kind: "function".into(),
                score: 0.9,
            }],
            edges: vec![ModelGraphEdge {
                src: "n1".into(),
                dst: "n2".into(),
                kind: "CALLS".into(),
                weight: 1.0,
            }],
            sources: vec![ModelSource {
                id: "s1".into(),
                title: "rank.rs".into(),
                snippet: "fn rank()".into(),
            }],
        }
    }

    #[tokio::test]
    async fn unconfigured_endpoint_returns_honest_answer() {
        let client = GlFusionClient::new(None, None, "m".into());
        assert!(!client.is_configured());
        let answer = client.ask("what does rank do?", &ctx()).await.unwrap();
        assert_eq!(answer.model, "unconfigured");
        assert!(answer.answer.contains("not configured"));
    }

    #[test]
    fn extract_answer_handles_common_shapes() {
        assert_eq!(
            extract_answer(&serde_json::json!({"answer": "a"})),
            Some("a".to_string())
        );
        assert_eq!(
            extract_answer(&serde_json::json!({"text": "b"})),
            Some("b".to_string())
        );
        assert_eq!(
            extract_answer(
                &serde_json::json!({"choices":[{"message":{"content":"c"}}]})
            ),
            Some("c".to_string())
        );
        assert_eq!(extract_answer(&serde_json::json!({"nope": 1})), None);
    }

    #[test]
    fn render_prompt_includes_nodes_edges_sources() {
        let prompt = render_prompt("q?", &ctx());
        assert!(prompt.contains("rank"));
        assert!(prompt.contains("CALLS"));
        assert!(prompt.contains("rank.rs"));
        assert!(prompt.contains("q?"));
    }
}
