//! Runtime persistence bridge for request-time prompt adaptation.
//!
//! The core prompt runtime owns pure selection, filtering, and refinement
//! logic. This module only translates accepted runs into durable memory
//! documents and hydrates those documents back into core evidence records.

use crate::memory::{
    recall_memory, remember_memory, MemoryGraphStore, MemoryRecallItem, MemoryResult,
    MemoryWriteInput, RecallMemoryInput, RememberMemoryReceipt,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use theorem_harness_core::{
    exemplar_evidence_from_accepted_run, exemplars_from_evidence, ExemplarRecord,
    PrepareMemoryEvidence, EXEMPLAR_EVIDENCE_KIND,
};

pub const PROMPT_RUNTIME_SURFACE: &str = "prompt-runtime";
pub const PROMPT_EXEMPLAR_TAG: &str = "prompt_exemplar";
pub const DEFAULT_PROMPT_EXEMPLAR_LIMIT: usize = 4;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct CapturePromptExemplarInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub project_slug: String,
    #[serde(default)]
    pub intent_id: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub outcome: Value,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct RecallPromptExemplarsInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub intent_id: String,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub project_slug: String,
    #[serde(default)]
    pub limit: usize,
}

pub fn capture_prompt_exemplar<S: MemoryGraphStore>(
    store: &mut S,
    input: CapturePromptExemplarInput,
) -> MemoryResult<RememberMemoryReceipt> {
    let evidence = exemplar_evidence_from_accepted_run(
        &input.intent_id,
        &input.task,
        &input.output,
        input.outcome.clone(),
    );
    remember_memory(store, prompt_exemplar_memory_input(&input, &evidence))
}

pub fn recall_prompt_exemplars<S: MemoryGraphStore>(
    store: &mut S,
    input: RecallPromptExemplarsInput,
) -> MemoryResult<Vec<ExemplarRecord>> {
    let limit = effective_prompt_exemplar_limit(input.limit);
    let recalled = recall_memory(
        store,
        RecallMemoryInput {
            tenant_slug: input.tenant_slug,
            query: input.query,
            kind: EXEMPLAR_EVIDENCE_KIND.to_string(),
            project_slug: input.project_slug,
            limit: recall_scan_limit(limit),
            hydrate: true,
            suppress_recall_metadata_updates: true,
            ..RecallMemoryInput::default()
        },
    )?;
    let evidence = recalled
        .iter()
        .filter_map(prepare_evidence_from_recall_item)
        .collect::<Vec<_>>();
    Ok(exemplars_from_evidence(&evidence, &input.intent_id, limit))
}

fn prompt_exemplar_memory_input(
    input: &CapturePromptExemplarInput,
    evidence: &PrepareMemoryEvidence,
) -> MemoryWriteInput {
    MemoryWriteInput {
        tenant_slug: input.tenant_slug.clone(),
        actor_id: input.actor_id.clone(),
        session_id: input.session_id.clone(),
        origin_surface: PROMPT_RUNTIME_SURFACE.to_string(),
        project_slug: input.project_slug.clone(),
        kind: EXEMPLAR_EVIDENCE_KIND.to_string(),
        title: prompt_exemplar_title(&input.intent_id),
        content: prompt_exemplar_content(&input.task, &input.output),
        summary: format!("Accepted prompt exemplar for intent {}.", input.intent_id),
        tags: vec![
            PROMPT_EXEMPLAR_TAG.to_string(),
            format!("intent:{}", input.intent_id),
        ],
        status: "active".to_string(),
        metadata: prompt_exemplar_metadata(evidence),
        created_at: input.created_at.clone(),
        ..MemoryWriteInput::default()
    }
}

fn prompt_exemplar_title(intent_id: &str) -> String {
    if intent_id.trim().is_empty() {
        "Prompt exemplar".to_string()
    } else {
        format!("Prompt exemplar: {intent_id}")
    }
}

fn prompt_exemplar_content(task: &str, output: &str) -> String {
    format!("Task:\n{}\n\nOutput:\n{}", task.trim(), output.trim())
}

fn prompt_exemplar_metadata(evidence: &PrepareMemoryEvidence) -> Map<String, Value> {
    let mut metadata = Map::new();
    metadata.insert(
        "evidence_id".to_string(),
        Value::String(evidence.evidence_id.clone()),
    );
    metadata.insert("kind".to_string(), Value::String(evidence.kind.clone()));
    metadata.insert(
        "evidence_kind".to_string(),
        Value::String(evidence.kind.clone()),
    );
    metadata.insert("source".to_string(), Value::String(evidence.source.clone()));
    metadata.insert("immutable".to_string(), Value::Bool(evidence.immutable));
    metadata.insert(
        "payload".to_string(),
        Value::Object(evidence.payload.clone()),
    );
    metadata.insert(
        "rationale".to_string(),
        Value::String(evidence.rationale.clone()),
    );
    for (key, value) in &evidence.payload {
        metadata.insert(key.clone(), value.clone());
    }
    metadata
}

fn prepare_evidence_from_recall_item(item: &MemoryRecallItem) -> Option<PrepareMemoryEvidence> {
    let metadata = item
        .document
        .as_ref()
        .map(|document| document.metadata.clone())
        .or_else(|| item.node.as_ref().map(|node| node.metadata.clone()))?;
    if metadata.is_empty() {
        return None;
    }

    let mut evidence = PrepareMemoryEvidence::from_value(&Value::Object(metadata.clone()));
    if evidence.kind.trim().is_empty() {
        evidence.kind = item.kind.clone();
    }
    if evidence.evidence_id.trim().is_empty() {
        evidence.evidence_id = format!("evidence:prompt-exemplar:{}", item.id);
    }
    if evidence.source.trim().is_empty() {
        evidence.source = item.origin_surface.clone();
    }
    if evidence.payload.is_empty() {
        evidence.payload = prompt_exemplar_payload_from_metadata(&metadata);
    }

    (evidence.kind == EXEMPLAR_EVIDENCE_KIND).then_some(evidence)
}

fn prompt_exemplar_payload_from_metadata(metadata: &Map<String, Value>) -> Map<String, Value> {
    ["intent_id", "task", "output", "outcome"]
        .into_iter()
        .filter_map(|key| {
            metadata
                .get(key)
                .map(|value| (key.to_string(), value.clone()))
        })
        .collect()
}

fn effective_prompt_exemplar_limit(limit: usize) -> usize {
    if limit == 0 {
        DEFAULT_PROMPT_EXEMPLAR_LIMIT
    } else {
        limit
    }
}

fn recall_scan_limit(limit: usize) -> usize {
    (limit * 4).max(16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::InMemoryGraphStore;
    use serde_json::json;

    #[test]
    fn captures_and_recalls_prompt_exemplars_by_intent() {
        let mut store = InMemoryGraphStore::new();
        capture_prompt_exemplar(
            &mut store,
            CapturePromptExemplarInput {
                tenant_slug: "Travis-Gilbert".to_string(),
                actor_id: "codex".to_string(),
                session_id: "session-1".to_string(),
                project_slug: "theorem".to_string(),
                intent_id: "intent-alpha".to_string(),
                task: "Assemble a prompt with cache-stable instructions.".to_string(),
                output: "Use static role instructions and dynamic exemplar suffixes.".to_string(),
                outcome: json!({ "accepted": true }),
                created_at: "2026-06-30T12:00:00Z".to_string(),
            },
        )
        .unwrap();
        capture_prompt_exemplar(
            &mut store,
            CapturePromptExemplarInput {
                tenant_slug: "Travis-Gilbert".to_string(),
                actor_id: "codex".to_string(),
                session_id: "session-1".to_string(),
                project_slug: "theorem".to_string(),
                intent_id: "intent-beta".to_string(),
                task: "Route executable outputs to checks.".to_string(),
                output: "Use executable critic routing for code.".to_string(),
                outcome: json!({ "accepted": true }),
                created_at: "2026-06-30T12:01:00Z".to_string(),
            },
        )
        .unwrap();

        let examples = recall_prompt_exemplars(
            &mut store,
            RecallPromptExemplarsInput {
                tenant_slug: "Travis-Gilbert".to_string(),
                intent_id: "intent-alpha".to_string(),
                query: String::new(),
                project_slug: "theorem".to_string(),
                limit: 8,
            },
        )
        .unwrap();

        assert_eq!(examples.len(), 1);
        assert_eq!(examples[0].intent_id, "intent-alpha");
        assert_eq!(
            examples[0].output,
            "Use static role instructions and dynamic exemplar suffixes."
        );
    }

    #[test]
    fn recall_filters_non_positive_exemplar_outcomes() {
        let mut store = InMemoryGraphStore::new();
        let input = CapturePromptExemplarInput {
            tenant_slug: "Travis-Gilbert".to_string(),
            actor_id: "codex".to_string(),
            session_id: "session-1".to_string(),
            project_slug: "theorem".to_string(),
            intent_id: "intent-alpha".to_string(),
            task: "Bad exemplar".to_string(),
            output: "This failed the acceptance oracle.".to_string(),
            outcome: json!({ "accepted": false }),
            created_at: "2026-06-30T12:02:00Z".to_string(),
        };
        let evidence = exemplar_evidence_from_accepted_run(
            &input.intent_id,
            &input.task,
            &input.output,
            input.outcome.clone(),
        );
        remember_memory(&mut store, prompt_exemplar_memory_input(&input, &evidence)).unwrap();

        let examples = recall_prompt_exemplars(
            &mut store,
            RecallPromptExemplarsInput {
                tenant_slug: "Travis-Gilbert".to_string(),
                intent_id: "intent-alpha".to_string(),
                query: String::new(),
                project_slug: "theorem".to_string(),
                limit: 8,
            },
        )
        .unwrap();

        assert!(examples.is_empty());
    }
}
