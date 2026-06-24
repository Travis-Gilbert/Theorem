//! Reasoning strategy memory.
//!
//! ReasoningBank is represented here as distilled strategy memories over the
//! existing `encode` and slim-first `recall` primitives. Raw traces stay out of
//! the strategy bank; only the reusable lesson is stored.

use crate::governor::REASONING_STRATEGY_TAG;
use crate::memory::{
    encode_memory, recall_memory, EncodeMemoryInput, MemoryDocumentState, MemoryGraphStore,
    MemoryRecallItem, MemoryResult, MemoryWriteInput, RecallMemoryInput,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrajectoryOutcome {
    Positive,
    Negative,
    Mixed,
    Neutral,
}

impl TrajectoryOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
            Self::Mixed => "mixed",
            Self::Neutral => "neutral",
        }
    }

    pub fn memory_kind(&self) -> &'static str {
        match self {
            Self::Negative => "postmortem",
            Self::Positive => "solution",
            Self::Mixed | Self::Neutral => "feedback",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReasoningStrategyInput {
    pub tenant_slug: String,
    pub actor_id: String,
    #[serde(default)]
    pub session_id: String,
    pub task: String,
    pub outcome: TrajectoryOutcome,
    #[serde(default)]
    pub trajectory_summary: String,
    #[serde(default)]
    pub approach_taken: String,
    #[serde(default)]
    pub worked: String,
    #[serde(default)]
    pub failed: String,
    pub lesson: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DistilledReasoningStrategy {
    pub title: String,
    pub summary: String,
    pub content: String,
    pub kind: String,
    pub outcome: TrajectoryOutcome,
    pub tags: Vec<String>,
}

pub fn distill_reasoning_strategy(input: &ReasoningStrategyInput) -> DistilledReasoningStrategy {
    let title = if input.task.trim().is_empty() {
        "Reasoning strategy".to_string()
    } else {
        format!("Reasoning strategy: {}", compact_line(&input.task, 80))
    };
    let summary = if input.lesson.trim().is_empty() {
        compact_line(&input.trajectory_summary, 160)
    } else {
        compact_line(&input.lesson, 160)
    };
    let mut tags = input.tags.clone();
    tags.push(REASONING_STRATEGY_TAG.to_string());
    tags.sort();
    tags.dedup();

    DistilledReasoningStrategy {
        title,
        summary,
        content: strategy_content(input),
        kind: input.outcome.memory_kind().to_string(),
        outcome: input.outcome.clone(),
        tags,
    }
}

pub fn write_reasoning_strategy<S: MemoryGraphStore>(
    store: &mut S,
    input: ReasoningStrategyInput,
) -> MemoryResult<MemoryDocumentState> {
    let strategy = distill_reasoning_strategy(&input);
    let mut metadata = Map::new();
    metadata.insert("source".to_string(), json!("reasoning_bank"));
    metadata.insert("task".to_string(), json!(input.task));
    metadata.insert("strategy_memory".to_string(), json!(true));
    metadata.insert("raw_trace_stored".to_string(), json!(false));

    encode_memory(
        store,
        MemoryWriteInput {
            tenant_slug: input.tenant_slug,
            actor_id: input.actor_id,
            session_id: input.session_id,
            origin_surface: "reasoning-bank".to_string(),
            kind: strategy.kind,
            title: strategy.title,
            summary: strategy.summary,
            content: strategy.content,
            tags: strategy.tags,
            metadata,
            created_at: input.created_at,
            ..MemoryWriteInput::default()
        },
        EncodeMemoryInput {
            outcome: strategy.outcome.as_str().to_string(),
            signal: "reasoning_strategy".to_string(),
            reason: "distilled trajectory into reusable strategy memory".to_string(),
            auto_triggered: true,
            ..EncodeMemoryInput::default()
        },
    )
}

pub fn recall_reasoning_strategies<S: MemoryGraphStore>(
    store: &mut S,
    tenant_slug: impl Into<String>,
    query: impl Into<String>,
    limit: usize,
) -> MemoryResult<Vec<MemoryRecallItem>> {
    let limit = limit.max(1);
    let results = recall_memory(
        store,
        RecallMemoryInput {
            tenant_slug: tenant_slug.into(),
            query: query.into(),
            limit: limit * 3,
            include_low_fitness: true,
            ..RecallMemoryInput::default()
        },
    )?;
    Ok(results
        .into_iter()
        .filter(|item| item.tags.iter().any(|tag| tag == REASONING_STRATEGY_TAG))
        .take(limit)
        .collect())
}

fn strategy_content(input: &ReasoningStrategyInput) -> String {
    let mut lines = vec![
        format!("Situation: {}", fallback(&input.task, "unspecified task")),
        format!(
            "Approach: {}",
            fallback(&input.approach_taken, "not specified")
        ),
        format!("Outcome: {}", input.outcome.as_str()),
        format!("Lesson: {}", fallback(&input.lesson, "not specified")),
    ];
    if !input.worked.trim().is_empty() {
        lines.push(format!("Worked: {}", input.worked.trim()));
    }
    if !input.failed.trim().is_empty() {
        lines.push(format!("Failed: {}", input.failed.trim()));
    }
    if !input.trajectory_summary.trim().is_empty() {
        lines.push(format!(
            "Evidence summary: {}",
            input.trajectory_summary.trim()
        ));
    }
    lines.join("\n")
}

fn compact_line(value: &str, max_chars: usize) -> String {
    let trimmed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.chars().count() <= max_chars {
        return trimmed;
    }
    trimmed.chars().take(max_chars).collect::<String>()
}

fn fallback<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::InMemoryGraphStore;

    #[test]
    fn writes_success_strategy_as_solution_not_transcript() {
        let mut store = InMemoryGraphStore::new();
        let document = write_reasoning_strategy(
            &mut store,
            ReasoningStrategyInput {
                tenant_slug: "Travis-Gilbert".to_string(),
                actor_id: "codex".to_string(),
                session_id: "s1".to_string(),
                task: "Fix slim recall payload".to_string(),
                outcome: TrajectoryOutcome::Positive,
                trajectory_summary: "Tests proved full content was omitted by default.".to_string(),
                approach_taken: "Start at the harness memory layer, not the lower memory crate."
                    .to_string(),
                worked: "Focused recall tests".to_string(),
                failed: String::new(),
                lesson: "When recall burns tokens, inspect the harness payload policy first."
                    .to_string(),
                tags: vec!["memory".to_string()],
                created_at: "2026-06-01T00:00:00Z".to_string(),
            },
        )
        .unwrap();

        assert_eq!(document.kind, "solution");
        assert!(document.tags.contains(&REASONING_STRATEGY_TAG.to_string()));
        assert_eq!(
            document
                .metadata
                .get("raw_trace_stored")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn recalls_strategy_slim_first() {
        let mut store = InMemoryGraphStore::new();
        write_reasoning_strategy(
            &mut store,
            ReasoningStrategyInput {
                tenant_slug: "Travis-Gilbert".to_string(),
                actor_id: "codex".to_string(),
                task: "Investigate failing provider keys".to_string(),
                outcome: TrajectoryOutcome::Negative,
                approach_taken: "Assumed keys were present before checking provider state."
                    .to_string(),
                failed: "Skipped the provider-key status surface.".to_string(),
                lesson: "Check provider key health before blaming the agent loop.".to_string(),
                tags: vec!["providers".to_string()],
                ..ReasoningStrategyInput {
                    tenant_slug: String::new(),
                    actor_id: String::new(),
                    task: String::new(),
                    outcome: TrajectoryOutcome::Neutral,
                    trajectory_summary: String::new(),
                    approach_taken: String::new(),
                    worked: String::new(),
                    failed: String::new(),
                    lesson: String::new(),
                    tags: Vec::new(),
                    session_id: String::new(),
                    created_at: String::new(),
                }
            },
        )
        .unwrap();

        let recalled =
            recall_reasoning_strategies(&mut store, "Travis-Gilbert", "provider key health", 3)
                .unwrap();

        assert_eq!(recalled.len(), 1);
        assert_eq!(recalled[0].kind, "postmortem");
        assert!(recalled[0].content.is_empty());
        assert!(recalled[0].document.is_none());
        assert!(!recalled[0].content_preview.is_empty());
    }
}
