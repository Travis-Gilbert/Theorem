use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use theorem_harness_core::stable_value_hash;

use crate::memory::{
    create_memory_document, list_memory_documents_since, MemoryDocumentState, MemoryGraphStore,
    MemoryResult, MemoryWriteInput,
};
use crate::tenant::normalize_tenant_slug;

const DEFAULT_MAX_DOCUMENTS: usize = 12;
const SLEEP_CANDIDATE_KIND: &str = "sleep_candidate";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SleepWorkerConfig {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub origin_surface: String,
    #[serde(default)]
    pub since: String,
    #[serde(default)]
    pub max_documents: usize,
    #[serde(default)]
    pub model_id: String,
}

impl Default for SleepWorkerConfig {
    fn default() -> Self {
        Self {
            tenant_slug: String::new(),
            actor_id: "sleep-worker".to_string(),
            session_id: String::new(),
            origin_surface: "sleep_worker".to_string(),
            since: String::new(),
            max_documents: DEFAULT_MAX_DOCUMENTS,
            model_id: "deterministic-sleep-worker-v1".to_string(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SleepWorkerCandidate {
    pub doc_id: String,
    pub title: String,
    pub source_doc_ids: Vec<String>,
    pub confidence_basis: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SleepWorkerReceipt {
    pub tenant_slug: String,
    pub inspected_documents: usize,
    pub candidate_count: usize,
    pub candidates: Vec<SleepWorkerCandidate>,
}

pub fn run_sleep_worker_once<S: MemoryGraphStore>(
    store: &mut S,
    config: SleepWorkerConfig,
) -> MemoryResult<SleepWorkerReceipt> {
    let tenant_slug = normalize_tenant_slug(&config.tenant_slug);
    let max_documents = config.max_documents.clamp(1, 100);
    let documents = list_memory_documents_since(store, &tenant_slug, &config.since, false)?;
    let sources = documents
        .iter()
        .filter(|document| document.kind != SLEEP_CANDIDATE_KIND)
        .take(max_documents)
        .cloned()
        .collect::<Vec<_>>();

    if sources.is_empty() {
        return Ok(SleepWorkerReceipt {
            tenant_slug,
            inspected_documents: documents.len(),
            candidate_count: 0,
            candidates: Vec::new(),
        });
    }

    let source_doc_ids = sources
        .iter()
        .map(|document| document.doc_id.clone())
        .collect::<Vec<_>>();
    let title = format!("Sleep candidate: {}", compact_title(&sources[0]));
    let content = candidate_content(&sources);
    let hash = stable_value_hash(&json!({
        "tenant_slug": tenant_slug,
        "source_doc_ids": source_doc_ids,
        "content": content,
        "model_id": normalized_model_id(&config),
    }));
    let doc_id = format!("sleep-candidate:{hash}");
    let mut metadata = Map::new();
    metadata.insert("candidate_only".to_string(), Value::Bool(true));
    metadata.insert("promoted".to_string(), Value::Bool(false));
    metadata.insert("source_doc_ids".to_string(), json!(source_doc_ids));
    metadata.insert(
        "model_id".to_string(),
        Value::String(normalized_model_id(&config)),
    );
    metadata.insert(
        "confidence_basis".to_string(),
        Value::String("deterministic consolidation candidate; not promoted".to_string()),
    );

    let document = create_memory_document(
        store,
        MemoryWriteInput {
            tenant_slug: tenant_slug.clone(),
            actor_id: normalized_actor_id(&config),
            session_id: config.session_id.trim().to_string(),
            origin_surface: normalized_surface(&config),
            doc_id: doc_id.clone(),
            kind: SLEEP_CANDIDATE_KIND.to_string(),
            title: title.clone(),
            content,
            summary: "Candidate-only idle consolidation; requires explicit promotion.".to_string(),
            metadata,
            ..MemoryWriteInput::default()
        },
    )?;

    Ok(SleepWorkerReceipt {
        tenant_slug,
        inspected_documents: documents.len(),
        candidate_count: 1,
        candidates: vec![SleepWorkerCandidate {
            doc_id: document.doc_id,
            title,
            source_doc_ids,
            confidence_basis: "deterministic consolidation candidate; not promoted".to_string(),
        }],
    })
}

fn candidate_content(documents: &[MemoryDocumentState]) -> String {
    let mut lines = vec![
        "Sleep-time consolidation candidate.".to_string(),
        "This record is advisory and must be explicitly promoted before it changes standing."
            .to_string(),
        String::new(),
    ];
    for document in documents {
        let body = if document.summary.trim().is_empty() {
            document.content.trim()
        } else {
            document.summary.trim()
        };
        lines.push(format!("- {}: {}", compact_title(document), body));
    }
    lines.join("\n")
}

fn compact_title(document: &MemoryDocumentState) -> String {
    if document.title.trim().is_empty() {
        document.doc_id.clone()
    } else {
        document.title.trim().to_string()
    }
}

fn normalized_actor_id(config: &SleepWorkerConfig) -> String {
    if config.actor_id.trim().is_empty() {
        "sleep-worker".to_string()
    } else {
        config.actor_id.trim().to_string()
    }
}

fn normalized_surface(config: &SleepWorkerConfig) -> String {
    if config.origin_surface.trim().is_empty() {
        "sleep_worker".to_string()
    } else {
        config.origin_surface.trim().to_string()
    }
}

fn normalized_model_id(config: &SleepWorkerConfig) -> String {
    if config.model_id.trim().is_empty() {
        "deterministic-sleep-worker-v1".to_string()
    } else {
        config.model_id.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{load_memory_document, recall_memory, RecallMemoryInput};
    use rustyred_thg_core::InMemoryGraphStore;

    #[test]
    fn sleep_worker_writes_candidate_only_memory_without_revising_sources() {
        let mut store = InMemoryGraphStore::new();
        create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: "Travis-Gilbert".to_string(),
                kind: "decision".to_string(),
                title: "Sync boundary".to_string(),
                content: "Durable state and ephemeral awareness are separate.".to_string(),
                summary: "Split durable state from awareness.".to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();

        let receipt = run_sleep_worker_once(
            &mut store,
            SleepWorkerConfig {
                tenant_slug: "Travis-Gilbert".to_string(),
                ..SleepWorkerConfig::default()
            },
        )
        .unwrap();

        assert_eq!(receipt.candidate_count, 1);
        let candidate =
            load_memory_document(&store, "Travis-Gilbert", &receipt.candidates[0].doc_id)
                .unwrap()
                .expect("candidate document");
        assert_eq!(candidate.kind, SLEEP_CANDIDATE_KIND);
        assert_eq!(
            candidate.metadata.get("candidate_only"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            candidate.metadata.get("promoted"),
            Some(&Value::Bool(false))
        );

        let recalled = recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: "Travis-Gilbert".to_string(),
                query: "Sync boundary".to_string(),
                limit: 10,
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        assert!(recalled
            .iter()
            .any(|item| item.title == "Sync boundary" && item.kind == "decision"));
    }

    #[test]
    fn sleep_worker_noops_without_source_documents() {
        let mut store = InMemoryGraphStore::new();
        let receipt = run_sleep_worker_once(
            &mut store,
            SleepWorkerConfig {
                tenant_slug: "Travis-Gilbert".to_string(),
                ..SleepWorkerConfig::default()
            },
        )
        .unwrap();

        assert_eq!(receipt.candidate_count, 0);
        assert!(receipt.candidates.is_empty());
    }
}
