//! Proactive harness governor policy.
//!
//! The governor is control-plane logic for the harness UI runtime: it decides
//! which substrate tool should run before an agent answer is generated. The
//! actual tool engines live elsewhere; this module produces auditable dispatch
//! receipts and memory records.

use crate::memory::{
    encode_memory, EncodeMemoryInput, MemoryDocumentState, MemoryError, MemoryGraphStore,
    MemoryResult, MemoryWriteInput,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

pub const GOVERNOR_DECISION_TAG: &str = "governor-decision";
pub const REASONING_STRATEGY_TAG: &str = "reasoning-strategy";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GovernorConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_reranker_threshold")]
    pub reranker_threshold: f64,
    #[serde(default = "default_max_dispatches")]
    pub max_dispatches: usize,
}

impl Default for GovernorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            reranker_threshold: default_reranker_threshold(),
            max_dispatches: default_max_dispatches(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CostlyCheck {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub current_uncertainty: f64,
    #[serde(default)]
    pub expected_uncertainty_after: f64,
    #[serde(default = "default_decision_value")]
    pub decision_value: f64,
    #[serde(default)]
    pub validator_cost: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GovernorTurnInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub pending_action: String,
    #[serde(default)]
    pub working_set_node_types: Vec<String>,
    #[serde(default)]
    pub source_ids: Vec<String>,
    #[serde(default)]
    pub typed_facts: Vec<Value>,
    #[serde(default)]
    pub recent_events: Vec<String>,
    #[serde(default)]
    pub assembling_cited_evidence: bool,
    #[serde(default)]
    pub costly_check: Option<CostlyCheck>,
    #[serde(default)]
    pub code_symbol: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernorTool {
    SourceReliability,
    DatalogDerive,
    ExpectedValue,
    ComputeCode,
    HarnessKgImpact,
}

impl GovernorTool {
    pub fn name(&self) -> &'static str {
        match self {
            Self::SourceReliability => "rustyred_thg_symbolic_probabilistic_source_reliability",
            Self::DatalogDerive => "rustyred_thg_symbolic_datalog_derive",
            Self::ExpectedValue => "rustyred_thg_symbolic_probabilistic_expected_value",
            Self::ComputeCode => "compute_code",
            Self::HarnessKgImpact => "harness_kg_impact",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::SourceReliability => {
                "score source or claim reliability while assembling cited evidence"
            }
            Self::DatalogDerive => "derive facts when new typed facts match rule bodies",
            Self::ExpectedValue => "gate costly verification, deep retrieval, or large recall by expected value of information",
            Self::ComputeCode => "search the code graph when a coding task or code symbol is in the working set",
            Self::HarnessKgImpact => "compute the code knowledge-graph blast radius for a symbol or change",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GovernorCandidate {
    pub tool_name: String,
    pub description: String,
    pub eligible_reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GovernorScoredCandidate {
    pub tool_name: String,
    pub description: String,
    pub eligible_reason: String,
    pub relevance_score: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GovernorDispatchDecision {
    pub tool_name: String,
    pub arguments: Value,
    pub rationale: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GovernorReceipt {
    pub enabled: bool,
    pub tier3_fired: bool,
    pub eligible: Vec<GovernorCandidate>,
    pub scored: Vec<GovernorScoredCandidate>,
    pub decisions: Vec<GovernorDispatchDecision>,
    pub injected_context: String,
}

pub fn govern_turn(input: &GovernorTurnInput, config: &GovernorConfig) -> GovernorReceipt {
    if !config.enabled {
        return GovernorReceipt {
            enabled: false,
            tier3_fired: false,
            eligible: Vec::new(),
            scored: Vec::new(),
            decisions: Vec::new(),
            injected_context: String::new(),
        };
    }

    let eligible = eligible_tools(input);
    let state_text = state_vector_text(input);
    let mut scored = eligible
        .iter()
        .map(|candidate| GovernorScoredCandidate {
            tool_name: candidate.tool_name.clone(),
            description: candidate.description.clone(),
            eligible_reason: candidate.eligible_reason.clone(),
            relevance_score: score_candidate(candidate, &state_text),
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .relevance_score
            .partial_cmp(&left.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.tool_name.cmp(&right.tool_name))
    });

    let max_dispatches = config.max_dispatches.max(1);
    let decisions = scored
        .iter()
        .filter(|candidate| candidate.relevance_score >= config.reranker_threshold)
        .take(max_dispatches)
        .map(|candidate| decision_for_candidate(candidate, input))
        .collect::<Vec<_>>();
    let injected_context = governor_context_message(&decisions);

    GovernorReceipt {
        enabled: true,
        tier3_fired: !decisions.is_empty(),
        eligible,
        scored,
        decisions,
        injected_context,
    }
}

pub fn encode_governor_receipt<S: MemoryGraphStore>(
    store: &mut S,
    input: &GovernorTurnInput,
    receipt: &GovernorReceipt,
    outcome: impl Into<String>,
    reason: impl Into<String>,
) -> MemoryResult<MemoryDocumentState> {
    let outcome = outcome.into();
    let reason = reason.into();
    let content = serde_json::to_string_pretty(receipt)
        .map_err(|error| MemoryError::Serialization(error.to_string()))?;
    let mut metadata = Map::new();
    metadata.insert("source".to_string(), json!("harness_governor"));
    metadata.insert("task".to_string(), json!(input.task));
    metadata.insert("tier3_fired".to_string(), json!(receipt.tier3_fired));
    metadata.insert("decision_count".to_string(), json!(receipt.decisions.len()));

    let mut tags = vec![GOVERNOR_DECISION_TAG.to_string()];
    for decision in &receipt.decisions {
        tags.push(decision.tool_name.clone());
    }
    tags.sort();
    tags.dedup();

    encode_memory(
        store,
        MemoryWriteInput {
            tenant_slug: input.tenant_slug.clone(),
            actor_id: input.actor_id.clone(),
            origin_surface: "harness-governor".to_string(),
            kind: "feedback".to_string(),
            title: governor_memory_title(receipt),
            summary: governor_memory_summary(receipt),
            content,
            tags,
            metadata,
            ..MemoryWriteInput::default()
        },
        EncodeMemoryInput {
            outcome,
            signal: "governor_dispatch".to_string(),
            reason,
            auto_triggered: true,
            ..EncodeMemoryInput::default()
        },
    )
}

fn eligible_tools(input: &GovernorTurnInput) -> Vec<GovernorCandidate> {
    let mut out = Vec::new();
    let node_types = normalized_set(&input.working_set_node_types);
    let action = input.pending_action.to_ascii_lowercase();
    let task = input.task.to_ascii_lowercase();
    let event_text = input.recent_events.join(" ").to_ascii_lowercase();

    if input.assembling_cited_evidence
        || node_types.contains("source")
        || node_types.contains("claim")
        || action.contains("cited")
        || action.contains("evidence")
        || action.contains("assertion")
    {
        out.push(candidate(
            GovernorTool::SourceReliability,
            "source or claim context entered the answer path",
        ));
    }

    if !input.typed_facts.is_empty()
        || event_text.contains("typed fact")
        || event_text.contains("rule body")
        || action.contains("derive")
    {
        out.push(candidate(
            GovernorTool::DatalogDerive,
            "new typed facts may satisfy a symbolic rule body",
        ));
    }

    if input.costly_check.is_some()
        || action.contains("verify")
        || action.contains("deep retrieval")
        || action.contains("large recall")
        || action.contains("adversarial")
    {
        out.push(candidate(
            GovernorTool::ExpectedValue,
            "a costly check is about to run",
        ));
    }

    if node_types.contains("code-symbol")
        || node_types.contains("code_symbol")
        || task.contains("code")
        || task.contains("implement")
        || action.contains("code")
    {
        out.push(candidate(
            GovernorTool::ComputeCode,
            "code context is part of the working set",
        ));
    }

    if !input.code_symbol.trim().is_empty()
        || action.contains("blast radius")
        || action.contains("impact")
        || task.contains("impact")
    {
        out.push(candidate(
            GovernorTool::HarnessKgImpact,
            "a code-object impact check is relevant",
        ));
    }

    out
}

fn candidate(tool: GovernorTool, reason: &str) -> GovernorCandidate {
    GovernorCandidate {
        tool_name: tool.name().to_string(),
        description: tool.description().to_string(),
        eligible_reason: reason.to_string(),
    }
}

fn score_candidate(candidate: &GovernorCandidate, state_text: &str) -> f64 {
    let state_tokens = tokenize(state_text);
    let mut score: f64 = 0.35;
    for token in tokenize(&candidate.description) {
        if state_tokens.contains(&token) {
            score += 0.08;
        }
    }
    for token in tokenize(&candidate.eligible_reason) {
        if state_tokens.contains(&token) {
            score += 0.05;
        }
    }
    if candidate.eligible_reason.contains("costly check") {
        score += 0.12;
    }
    score.min(1.0)
}

fn decision_for_candidate(
    candidate: &GovernorScoredCandidate,
    input: &GovernorTurnInput,
) -> GovernorDispatchDecision {
    match candidate.tool_name.as_str() {
        "rustyred_thg_symbolic_probabilistic_source_reliability" => {
            let source_id = input
                .source_ids
                .iter()
                .find(|value| !value.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| "working-set-source".to_string());
            GovernorDispatchDecision {
                tool_name: candidate.tool_name.clone(),
                arguments: json!({
                    "source_id": source_id,
                    "prior_alpha": 1.0,
                    "prior_beta": 1.0,
                    "corroborated": 0,
                    "contradicted": 0
                }),
                rationale: candidate.eligible_reason.clone(),
            }
        }
        "rustyred_thg_symbolic_probabilistic_expected_value" => {
            let check = input.costly_check.clone().unwrap_or(CostlyCheck {
                name: "costly_check".to_string(),
                current_uncertainty: 0.6,
                expected_uncertainty_after: 0.3,
                decision_value: 1.0,
                validator_cost: 0.1,
            });
            GovernorDispatchDecision {
                tool_name: candidate.tool_name.clone(),
                arguments: json!({
                    "current_uncertainty": check.current_uncertainty,
                    "expected_uncertainty_after": check.expected_uncertainty_after,
                    "decision_value": check.decision_value,
                    "validator_cost": check.validator_cost,
                    "check": check.name
                }),
                rationale: candidate.eligible_reason.clone(),
            }
        }
        "rustyred_thg_symbolic_datalog_derive" => GovernorDispatchDecision {
            tool_name: candidate.tool_name.clone(),
            arguments: json!({ "facts": input.typed_facts }),
            rationale: candidate.eligible_reason.clone(),
        },
        "harness_kg_impact" => GovernorDispatchDecision {
            tool_name: candidate.tool_name.clone(),
            arguments: json!({
                "symbol_name": if input.code_symbol.trim().is_empty() {
                    input.task.trim()
                } else {
                    input.code_symbol.trim()
                },
                "direction": "out",
                "max_depth": 2
            }),
            rationale: candidate.eligible_reason.clone(),
        },
        _ => GovernorDispatchDecision {
            tool_name: candidate.tool_name.clone(),
            arguments: json!({
                "operation": "search",
                "query": input.task,
                "limit": 10
            }),
            rationale: candidate.eligible_reason.clone(),
        },
    }
}

fn governor_context_message(decisions: &[GovernorDispatchDecision]) -> String {
    if decisions.is_empty() {
        return String::new();
    }
    let mut lines = vec!["Governor proactive dispatch:".to_string()];
    for decision in decisions {
        lines.push(format!("- {}: {}", decision.tool_name, decision.rationale));
    }
    lines.join("\n")
}

fn governor_memory_title(receipt: &GovernorReceipt) -> String {
    if let Some(first) = receipt.decisions.first() {
        format!("Governor dispatch: {}", first.tool_name)
    } else {
        "Governor dispatch: no tool fired".to_string()
    }
}

fn governor_memory_summary(receipt: &GovernorReceipt) -> String {
    if receipt.decisions.is_empty() {
        "Governor evaluated the turn and did not dispatch a tool.".to_string()
    } else {
        format!(
            "Governor dispatched {} tool(s); tier3_fired={}.",
            receipt.decisions.len(),
            receipt.tier3_fired
        )
    }
}

fn state_vector_text(input: &GovernorTurnInput) -> String {
    [
        input.task.as_str(),
        input.pending_action.as_str(),
        &input.working_set_node_types.join(" "),
        &input.recent_events.join(" "),
    ]
    .join(" ")
}

fn tokenize(value: &str) -> BTreeSet<String> {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .map(str::trim)
        .filter(|part| part.len() >= 3)
        .map(|part| part.to_ascii_lowercase())
        .collect()
}

fn normalized_set(values: &[String]) -> BTreeSet<String> {
    values
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn default_true() -> bool {
    true
}

fn default_reranker_threshold() -> f64 {
    0.45
}

fn default_max_dispatches() -> usize {
    3
}

fn default_decision_value() -> f64 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::recall_memory;
    use rustyred_thg_core::InMemoryGraphStore;

    #[test]
    fn governor_dispatches_source_reliability_and_evoi_for_cited_verify() {
        let input = GovernorTurnInput {
            tenant_slug: "Travis-Gilbert".to_string(),
            actor_id: "codex".to_string(),
            task: "Assemble cited evidence for a claim".to_string(),
            pending_action: "run adversarial verify".to_string(),
            working_set_node_types: vec!["source".to_string(), "claim".to_string()],
            source_ids: vec!["source:paper".to_string()],
            assembling_cited_evidence: true,
            costly_check: Some(CostlyCheck {
                name: "verify".to_string(),
                current_uncertainty: 0.7,
                expected_uncertainty_after: 0.2,
                decision_value: 2.0,
                validator_cost: 0.4,
            }),
            ..GovernorTurnInput::default()
        };

        let receipt = govern_turn(&input, &GovernorConfig::default());

        let names = receipt
            .decisions
            .iter()
            .map(|decision| decision.tool_name.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"rustyred_thg_symbolic_probabilistic_source_reliability"));
        assert!(names.contains(&"rustyred_thg_symbolic_probabilistic_expected_value"));
        assert!(receipt.tier3_fired);
        assert!(receipt
            .injected_context
            .contains("Governor proactive dispatch"));
    }

    #[test]
    fn governor_can_be_disabled_without_error() {
        let receipt = govern_turn(
            &GovernorTurnInput {
                task: "implement code".to_string(),
                pending_action: "verify".to_string(),
                ..GovernorTurnInput::default()
            },
            &GovernorConfig {
                enabled: false,
                ..GovernorConfig::default()
            },
        );

        assert!(!receipt.enabled);
        assert!(receipt.decisions.is_empty());
    }

    #[test]
    fn governor_receipt_encodes_as_recallable_feedback() {
        let mut store = InMemoryGraphStore::new();
        let input = GovernorTurnInput {
            tenant_slug: "Travis-Gilbert".to_string(),
            actor_id: "codex".to_string(),
            task: "implement code impact check".to_string(),
            code_symbol: "renderable_from_item".to_string(),
            ..GovernorTurnInput::default()
        };
        let receipt = govern_turn(&input, &GovernorConfig::default());

        let memory =
            encode_governor_receipt(&mut store, &input, &receipt, "positive", "paid off").unwrap();

        assert_eq!(memory.kind, "feedback");
        assert!(memory.tags.contains(&GOVERNOR_DECISION_TAG.to_string()));
        let recalled = recall_memory(
            &mut store,
            crate::memory::RecallMemoryInput {
                tenant_slug: "Travis-Gilbert".to_string(),
                query: "governor dispatch".to_string(),
                limit: 10,
                ..crate::memory::RecallMemoryInput::default()
            },
        )
        .unwrap();
        assert!(recalled.iter().any(|item| item.id == memory.doc_id));
    }
}
