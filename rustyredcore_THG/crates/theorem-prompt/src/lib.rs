//! Cache-aware prompt rendering for resolved Theorem head invocations.
//!
//! This crate deliberately stays outside `theorem-harness-core`: it knows how to
//! assemble provider-facing messages, while the core crate owns the invocation
//! contract and the fake-first `HeadInvoker` seam.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use theorem_harness_core::{
    ContextMembranePrime, GroundedClaim, HeadInvocationRequest, RevisionContext,
    ScratchpadCrdtBacking,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExemplarPlacement {
    DynamicSuffix,
    StaticPrefix,
}

impl Default for ExemplarPlacement {
    fn default() -> Self {
        Self::DynamicSuffix
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PromptExemplar {
    pub task: String,
    pub output: String,
    pub outcome: Value,
    #[serde(default)]
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PromptSpec {
    pub instruction_key: String,
    pub instruction_text: String,
    pub task: String,
    pub field_schema: Value,
    #[serde(default)]
    pub tool_refs: Vec<String>,
    #[serde(default)]
    pub exemplars: Vec<PromptExemplar>,
    #[serde(default)]
    pub exemplar_placement: ExemplarPlacement,
    #[serde(default)]
    pub persona: Option<String>,
    #[serde(default)]
    pub scratchpad_crdt: ScratchpadCrdtBacking,
    #[serde(default)]
    pub context_membrane: Vec<ContextMembranePrime>,
    #[serde(default)]
    pub prior_context: Vec<RevisionContext>,
    #[serde(default)]
    pub claims: Vec<GroundedClaim>,
}

impl PromptSpec {
    pub fn from_request(
        request: &HeadInvocationRequest,
        instruction_key: impl Into<String>,
        instruction_text: impl Into<String>,
    ) -> Self {
        Self {
            instruction_key: instruction_key.into(),
            instruction_text: instruction_text.into(),
            task: request.task.clone(),
            field_schema: default_claims_schema(),
            tool_refs: request.head.allowed_tools.clone(),
            exemplars: Vec::new(),
            exemplar_placement: ExemplarPlacement::default(),
            persona: request.constitution.clone(),
            scratchpad_crdt: request.scratchpad_crdt.clone(),
            context_membrane: request.context_membrane.clone(),
            prior_context: request.prior_context.clone(),
            claims: request.claims.clone(),
        }
    }

    pub fn with_exemplars(mut self, exemplars: Vec<PromptExemplar>) -> Self {
        self.exemplars = exemplars;
        self
    }

    pub fn with_exemplar_placement(mut self, placement: ExemplarPlacement) -> Self {
        self.exemplar_placement = placement;
        self
    }

    pub fn assemble(&self) -> PromptAssembly {
        let mut blocks = Vec::new();
        push_block(
            &mut blocks,
            PromptBlockKind::System,
            CacheRole::Static,
            self.instruction_text.clone(),
        );
        if !self.tool_refs.is_empty() {
            push_block(
                &mut blocks,
                PromptBlockKind::Tools,
                CacheRole::Static,
                render_tools(&self.tool_refs),
            );
        }
        if let Some(persona) = self
            .persona
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            push_block(
                &mut blocks,
                PromptBlockKind::Persona,
                CacheRole::Static,
                persona.to_string(),
            );
        }
        push_block(
            &mut blocks,
            PromptBlockKind::FieldSchema,
            CacheRole::Static,
            self.field_schema.to_string(),
        );
        if self.exemplar_placement == ExemplarPlacement::StaticPrefix {
            push_exemplars(&mut blocks, CacheRole::Static, &self.exemplars);
        }

        let cache_breakpoint = blocks.len();

        push_block(
            &mut blocks,
            PromptBlockKind::Task,
            CacheRole::Dynamic,
            self.task.clone(),
        );
        push_block(
            &mut blocks,
            PromptBlockKind::Scratchpad,
            CacheRole::Dynamic,
            render_scratchpad(&self.scratchpad_crdt),
        );
        if !self.context_membrane.is_empty() {
            push_block(
                &mut blocks,
                PromptBlockKind::Memory,
                CacheRole::Dynamic,
                render_context_membrane(&self.context_membrane),
            );
        }
        if self.exemplar_placement == ExemplarPlacement::DynamicSuffix {
            push_exemplars(&mut blocks, CacheRole::Dynamic, &self.exemplars);
        }
        if !self.prior_context.is_empty() {
            push_block(
                &mut blocks,
                PromptBlockKind::History,
                CacheRole::Dynamic,
                render_prior_context(&self.prior_context),
            );
        }
        if !self.claims.is_empty() {
            push_block(
                &mut blocks,
                PromptBlockKind::Claims,
                CacheRole::Dynamic,
                render_claims(&self.claims),
            );
        }

        PromptAssembly {
            blocks,
            cache_breakpoint,
            dynamic_budget_owner: "context_web/context_manager".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheRole {
    Static,
    Dynamic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptBlockKind {
    System,
    Tools,
    Persona,
    FieldSchema,
    Task,
    Scratchpad,
    Memory,
    Exemplars,
    History,
    Claims,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PromptBlock {
    pub kind: PromptBlockKind,
    pub cache_role: CacheRole,
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PromptAssembly {
    pub blocks: Vec<PromptBlock>,
    pub cache_breakpoint: usize,
    pub dynamic_budget_owner: String,
}

impl PromptAssembly {
    pub fn static_prefix(&self) -> &[PromptBlock] {
        &self.blocks[..self.cache_breakpoint]
    }

    pub fn dynamic_suffix(&self) -> &[PromptBlock] {
        &self.blocks[self.cache_breakpoint..]
    }

    pub fn dynamic_content_before_breakpoint(&self) -> bool {
        self.static_prefix()
            .iter()
            .any(|block| block.cache_role == CacheRole::Dynamic)
    }
}

pub trait Renderer {
    fn render(&self, spec: &PromptSpec) -> RenderedPrompt;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProviderMessage {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RenderedPrompt {
    pub messages: Vec<ProviderMessage>,
    pub blocks: Vec<PromptBlock>,
    pub cache_breakpoint: usize,
}

#[derive(Clone, Debug, Default)]
pub struct MarkerRenderer;

impl Renderer for MarkerRenderer {
    fn render(&self, spec: &PromptSpec) -> RenderedPrompt {
        let assembly = spec.assemble();
        let system = render_marked_blocks(assembly.static_prefix());
        let user = render_marked_blocks(assembly.dynamic_suffix());
        RenderedPrompt {
            messages: vec![
                ProviderMessage {
                    role: "system".to_string(),
                    content: system,
                },
                ProviderMessage {
                    role: "user".to_string(),
                    content: user,
                },
            ],
            blocks: assembly.blocks,
            cache_breakpoint: assembly.cache_breakpoint,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct StructuredOutputRenderer;

impl Renderer for StructuredOutputRenderer {
    fn render(&self, spec: &PromptSpec) -> RenderedPrompt {
        let assembly = spec.assemble();
        let system = json!({
            "static_prefix": assembly.static_prefix(),
            "instruction_key": spec.instruction_key,
            "field_schema": spec.field_schema,
        })
        .to_string();
        let user = json!({
            "dynamic_suffix": assembly.dynamic_suffix(),
            "response_contract": "return content plus Claims JSON when claims are asserted",
        })
        .to_string();
        RenderedPrompt {
            messages: vec![
                ProviderMessage {
                    role: "system".to_string(),
                    content: system,
                },
                ProviderMessage {
                    role: "user".to_string(),
                    content: user,
                },
            ],
            blocks: assembly.blocks,
            cache_breakpoint: assembly.cache_breakpoint,
        }
    }
}

pub fn render_prompt<R: Renderer>(renderer: &R, spec: &PromptSpec) -> RenderedPrompt {
    renderer.render(spec)
}

fn default_claims_schema() -> Value {
    json!({
        "claims_json": [
            { "text": "string", "provenance": "string" }
        ]
    })
}

fn push_block(
    blocks: &mut Vec<PromptBlock>,
    kind: PromptBlockKind,
    cache_role: CacheRole,
    content: String,
) {
    let content = content.trim().to_string();
    if !content.is_empty() {
        blocks.push(PromptBlock {
            kind,
            cache_role,
            content,
        });
    }
}

fn push_exemplars(
    blocks: &mut Vec<PromptBlock>,
    cache_role: CacheRole,
    exemplars: &[PromptExemplar],
) {
    if !exemplars.is_empty() {
        push_block(
            blocks,
            PromptBlockKind::Exemplars,
            cache_role,
            render_exemplars(exemplars),
        );
    }
}

fn render_marked_blocks(blocks: &[PromptBlock]) -> String {
    blocks
        .iter()
        .map(|block| format!("[{}]\n{}", block_label(block.kind), block.content))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn block_label(kind: PromptBlockKind) -> &'static str {
    match kind {
        PromptBlockKind::System => "System instruction",
        PromptBlockKind::Tools => "Tools",
        PromptBlockKind::Persona => "Persona",
        PromptBlockKind::FieldSchema => "Field schema",
        PromptBlockKind::Task => "Task",
        PromptBlockKind::Scratchpad => "Shared CRDT scratchpad",
        PromptBlockKind::Memory => "Context membrane primes",
        PromptBlockKind::Exemplars => "Exemplars",
        PromptBlockKind::History => "Prior revisions",
        PromptBlockKind::Claims => "Seed grounding claims",
    }
}

fn render_tools(tool_refs: &[String]) -> String {
    tool_refs
        .iter()
        .map(|tool| format!("- {tool}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_scratchpad(scratchpad: &ScratchpadCrdtBacking) -> String {
    let mut text = format!(
        "- kind: {:?}\n- graph_root_id: {}\n- yrs_doc_id: {}\n- stream_topic: {}\n- awareness_log_id: {}",
        scratchpad.kind,
        scratchpad.graph_root_id,
        scratchpad.yrs_doc_id,
        scratchpad.stream_topic,
        scratchpad.awareness_log_id
    );
    if !scratchpad.text_regions.is_empty() {
        text.push_str("\n- text_regions:");
        for region in &scratchpad.text_regions {
            text.push_str(&format!(
                "\n  - {}: {}",
                region.region_id, region.description
            ));
        }
    }
    text
}

fn render_context_membrane(context: &[ContextMembranePrime]) -> String {
    context
        .iter()
        .map(|prime| {
            let mut text = format!(
                "- {} ({}, confidence {:.2}): {}",
                prime.artifact_id, prime.label, prime.confidence, prime.summary
            );
            if !prime.source.trim().is_empty() {
                text.push_str(&format!(" [{}]", prime.source));
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_exemplars(exemplars: &[PromptExemplar]) -> String {
    exemplars
        .iter()
        .map(|example| {
            format!(
                "- task: {}\n  output: {}\n  outcome: {}\n  source: {}",
                example.task, example.output, example.outcome, example.source
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_prior_context(context: &[RevisionContext]) -> String {
    context
        .iter()
        .map(|revision| {
            let mut text = format!(
                "- {} ({}) {}",
                revision.revision_id, revision.kind, revision.output_summary
            );
            if let Some(body) = revision.payload.get("text").and_then(Value::as_str) {
                text.push('\n');
                text.push_str(body);
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_claims(claims: &[GroundedClaim]) -> String {
    claims
        .iter()
        .map(|claim| format!("- {} [{}]", claim.text, claim.provenance))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use theorem_harness_core::{
        AgentHeadEndpoint, HeadCostProfile, HeadInvocationKind, HeadKind, HeadReliabilityProfile,
        HeadTransport, ResolvedAgentHead, TraceTier,
    };

    #[test]
    fn prompt_spec_carries_request_fields_and_empty_exemplars() {
        let request = request_fixture();

        let spec = PromptSpec::from_request(&request, "instruction.head.test", "Be precise.");

        assert_eq!(spec.instruction_key, "instruction.head.test");
        assert_eq!(spec.instruction_text, "Be precise.");
        assert_eq!(spec.task, "answer the question");
        assert_eq!(spec.tool_refs, vec!["test.run"]);
        assert_eq!(spec.persona.as_deref(), Some("Theorem voice"));
        assert_eq!(spec.context_membrane.len(), 1);
        assert_eq!(spec.prior_context.len(), 1);
        assert_eq!(spec.claims.len(), 1);
        assert!(spec.exemplars.is_empty());
    }

    #[test]
    fn renderers_keep_prompt_spec_provider_neutral() {
        let spec =
            PromptSpec::from_request(&request_fixture(), "instruction.head.test", "Be precise.");

        let marker = MarkerRenderer.render(&spec);
        let structured = StructuredOutputRenderer.render(&spec);

        assert_eq!(marker.cache_breakpoint, structured.cache_breakpoint);
        assert_ne!(marker.messages[0].content, structured.messages[0].content);
        assert!(marker.messages[1].content.contains("answer the question"));
        assert!(structured.messages[1].content.contains("dynamic_suffix"));
    }

    #[test]
    fn dynamic_blocks_never_precede_cache_breakpoint() {
        let spec =
            PromptSpec::from_request(&request_fixture(), "instruction.head.test", "Be precise.");

        let assembly = spec.assemble();

        assert!(!assembly.dynamic_content_before_breakpoint());
        assert!(assembly
            .static_prefix()
            .iter()
            .all(|block| block.cache_role == CacheRole::Static));
        assert!(assembly
            .dynamic_suffix()
            .iter()
            .all(|block| block.cache_role == CacheRole::Dynamic));
    }

    #[test]
    fn exemplar_placement_controls_cache_side() {
        let exemplar = PromptExemplar {
            task: "prior task".to_string(),
            output: "prior answer".to_string(),
            outcome: json!({"accepted": true}),
            source: "test".to_string(),
        };

        let dynamic =
            PromptSpec::from_request(&request_fixture(), "instruction.head.test", "Be precise.")
                .with_exemplars(vec![exemplar.clone()])
                .assemble();
        let static_prefix =
            PromptSpec::from_request(&request_fixture(), "instruction.head.test", "Be precise.")
                .with_exemplars(vec![exemplar])
                .with_exemplar_placement(ExemplarPlacement::StaticPrefix)
                .assemble();

        assert!(dynamic
            .dynamic_suffix()
            .iter()
            .any(|block| block.kind == PromptBlockKind::Exemplars));
        assert!(static_prefix
            .static_prefix()
            .iter()
            .any(|block| block.kind == PromptBlockKind::Exemplars));
    }

    fn request_fixture() -> HeadInvocationRequest {
        HeadInvocationRequest::new_with_context(
            head(),
            HeadInvocationKind::Synthesis,
            "answer the question",
            3,
            vec!["scratchrev:1".to_string()],
            vec![RevisionContext {
                revision_id: "scratchrev:1".to_string(),
                kind: "proposal".to_string(),
                output_summary: "proposal summary".to_string(),
                payload: serde_json::Map::from_iter([(
                    "text".to_string(),
                    Value::String("proposal body".to_string()),
                )]),
            }],
            vec![GroundedClaim::new("claim", "source:a")],
            "2026-06-30T00:00:00Z",
        )
        .with_context_membrane(vec![ContextMembranePrime::new(
            "context:1",
            "memory",
            "important memory",
            "test",
            0.9,
        )])
        .with_constitution(Some("Theorem voice".to_string()))
    }

    fn head() -> ResolvedAgentHead {
        ResolvedAgentHead {
            head_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            provider: "openai".to_string(),
            model: "model".to_string(),
            kind: HeadKind::ReasoningCore,
            endpoint: AgentHeadEndpoint {
                transport: HeadTransport::Api,
                target: "fake://target".to_string(),
                fake: true,
            },
            credential_ref: "env:TEST".to_string(),
            capabilities: Vec::new(),
            cost_profile: HeadCostProfile::default(),
            reliability_profile: HeadReliabilityProfile::default(),
            allowed_tools: vec!["test.run".to_string()],
            trace_tier: TraceTier::Receipt,
        }
    }
}
