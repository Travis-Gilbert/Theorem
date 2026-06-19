use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use crate::tenant::normalize_tenant_slug;
use rustyred_thg_core::{EdgeRecord, GraphStore, GraphStoreError, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use theorem_harness_core::stable_value_hash;

pub type CanonicalWriteResult<T> = Result<T, CanonicalWriteError>;

pub const CANONICAL_FACT_LABEL: &str = "CanonicalFact";
pub const EMBEDDING_NOMINATION_LABEL: &str = "EmbeddingNomination";
pub const ALIAS_WITNESS_LABEL: &str = "AliasWitness";
pub const EDGE_EMBEDDING_NOMINATED: &str = "EMBEDDING_NOMINATED";
pub const EDGE_ALIAS_WITNESS_FOR: &str = "ALIAS_WITNESS_FOR";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct TypedFact {
    pub fact_type: String,
    pub canonical_key: String,
    pub statement: String,
    #[serde(default)]
    pub properties: Map<String, Value>,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct EmbeddingNomination {
    pub candidate_id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub query_ref: String,
    #[serde(default)]
    pub evidence_ref: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct AliasWitness {
    pub alias: String,
    pub source_ref: String,
    #[serde(default)]
    pub observed_as: String,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct CanonicalizeOnWriteInput {
    #[serde(default)]
    pub tenant_slug: String,
    pub fact: TypedFact,
    #[serde(default)]
    pub nominations: Vec<EmbeddingNomination>,
    #[serde(default)]
    pub aliases: Vec<AliasWitness>,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct CanonicalWriteReceipt {
    pub tenant_slug: String,
    pub canonical_node_id: String,
    pub canonical_hash: String,
    pub typed_fact_hash: String,
    pub nomination_node_ids: Vec<String>,
    pub alias_witness_node_ids: Vec<String>,
    pub created: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CanonicalWriteError {
    InvalidInput { field: String, message: String },
    Store(String),
    Serialization(String),
}

impl fmt::Display for CanonicalWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput { field, message } => {
                write!(f, "invalid canonical write input {field}: {message}")
            }
            Self::Store(message) => write!(f, "store error: {message}"),
            Self::Serialization(message) => write!(f, "serialization error: {message}"),
        }
    }
}

impl Error for CanonicalWriteError {}

impl From<GraphStoreError> for CanonicalWriteError {
    fn from(value: GraphStoreError) -> Self {
        Self::Store(format!("{}: {}", value.code, value.message))
    }
}

pub fn canonical_fact_node_id(tenant: &str, fact_type: &str, canonical_key: &str) -> String {
    let key_hash = stable_value_hash(&json!({
        "fact_type": fact_type.trim(),
        "canonical_key": canonical_key.trim(),
    }));
    format!(
        "canonical_fact:{}:{}:{}",
        normalize_tenant(tenant),
        slugify(fact_type),
        &key_hash[..16]
    )
}

pub fn embedding_nomination_node_id(
    tenant: &str,
    canonical_node_id: &str,
    nomination: &EmbeddingNomination,
) -> String {
    let hash = stable_value_hash(&json!({
        "tenant_slug": normalize_tenant(tenant),
        "canonical_node_id": canonical_node_id,
        "candidate_id": nomination.candidate_id,
        "model": nomination.model,
        "query_ref": nomination.query_ref,
        "evidence_ref": nomination.evidence_ref,
    }));
    format!(
        "embedding_nomination:{}:{}",
        normalize_tenant(tenant),
        &hash[..16]
    )
}

pub fn alias_witness_node_id(
    tenant: &str,
    canonical_node_id: &str,
    alias: &AliasWitness,
) -> String {
    let hash = stable_value_hash(&json!({
        "tenant_slug": normalize_tenant(tenant),
        "canonical_node_id": canonical_node_id,
        "alias": alias.alias,
        "source_ref": alias.source_ref,
        "observed_as": alias.observed_as,
    }));
    format!("alias_witness:{}:{}", normalize_tenant(tenant), &hash[..16])
}

pub fn canonicalize_on_write<S: GraphStore>(
    store: &mut S,
    input: CanonicalizeOnWriteInput,
) -> CanonicalWriteResult<CanonicalWriteReceipt> {
    validate_fact(&input.fact)?;
    validate_nominations(&input.nominations)?;
    validate_aliases(&input.aliases)?;
    let tenant = require_tenant(&input.tenant_slug)?;

    let canonical_node_id =
        canonical_fact_node_id(&tenant, &input.fact.fact_type, &input.fact.canonical_key);
    let created = store.get_node(&canonical_node_id).is_none();
    let typed_fact_hash = stable_value_hash(&fact_value(&input.fact));
    let canonical_hash = stable_value_hash(&json!({
        "tenant_slug": tenant,
        "canonical_node_id": canonical_node_id,
        "typed_fact_hash": typed_fact_hash,
    }));

    store.upsert_node(canonical_fact_node(
        &tenant,
        &canonical_node_id,
        &canonical_hash,
        &typed_fact_hash,
        &input,
    )?)?;

    let mut nomination_node_ids = Vec::new();
    for nomination in dedupe_nominations(&input.nominations) {
        let node_id = embedding_nomination_node_id(&tenant, &canonical_node_id, nomination);
        store.upsert_node(embedding_nomination_node(
            &tenant,
            &canonical_node_id,
            &typed_fact_hash,
            &node_id,
            nomination,
        )?)?;
        store.upsert_edge(embedding_nomination_edge(
            &tenant,
            &canonical_node_id,
            &node_id,
            nomination,
        ))?;
        nomination_node_ids.push(node_id);
    }

    let mut alias_witness_node_ids = Vec::new();
    for alias in dedupe_aliases(&input.aliases) {
        let node_id = alias_witness_node_id(&tenant, &canonical_node_id, alias);
        store.upsert_node(alias_witness_node(
            &tenant,
            &canonical_node_id,
            &typed_fact_hash,
            &node_id,
            alias,
        )?)?;
        store.upsert_edge(alias_witness_edge(
            &tenant,
            &canonical_node_id,
            &node_id,
            alias,
        ))?;
        alias_witness_node_ids.push(node_id);
    }

    Ok(CanonicalWriteReceipt {
        tenant_slug: tenant,
        canonical_node_id,
        canonical_hash,
        typed_fact_hash,
        nomination_node_ids,
        alias_witness_node_ids,
        created,
    })
}

fn canonical_fact_node(
    tenant: &str,
    canonical_node_id: &str,
    canonical_hash: &str,
    typed_fact_hash: &str,
    input: &CanonicalizeOnWriteInput,
) -> CanonicalWriteResult<NodeRecord> {
    Ok(NodeRecord::new(
        canonical_node_id,
        [CANONICAL_FACT_LABEL],
        json!({
            "tenant_slug": tenant,
            "canonical_hash": canonical_hash,
            "typed_fact_hash": typed_fact_hash,
            "fact": input.fact,
            "status": "canonical",
            "ratified_by": "typed_fact",
            "embedding_role": "nomination_only",
            "alias_role": "witness_only",
            "metadata": input.metadata,
            "updated_by": input.actor_id,
            "updated_at": timestamp_or_now(&input.updated_at),
        }),
    ))
}

fn embedding_nomination_node(
    tenant: &str,
    canonical_node_id: &str,
    typed_fact_hash: &str,
    node_id: &str,
    nomination: &EmbeddingNomination,
) -> CanonicalWriteResult<NodeRecord> {
    Ok(NodeRecord::new(
        node_id,
        [EMBEDDING_NOMINATION_LABEL],
        json!({
            "tenant_slug": tenant,
            "canonical_node_id": canonical_node_id,
            "typed_fact_hash": typed_fact_hash,
            "candidate_id": nomination.candidate_id,
            "model": nomination.model,
            "score": nomination.score,
            "query_ref": nomination.query_ref,
            "evidence_ref": nomination.evidence_ref,
            "authority": "nomination_not_truth",
        }),
    ))
}

fn embedding_nomination_edge(
    tenant: &str,
    canonical_node_id: &str,
    nomination_node_id: &str,
    nomination: &EmbeddingNomination,
) -> EdgeRecord {
    EdgeRecord::new(
        format!("{nomination_node_id}:nominated:{canonical_node_id}"),
        nomination_node_id,
        EDGE_EMBEDDING_NOMINATED,
        canonical_node_id,
        json!({
            "tenant_slug": tenant,
            "candidate_id": nomination.candidate_id,
            "score": nomination.score,
            "model": nomination.model,
            "authority": "nomination_only",
        }),
    )
}

fn alias_witness_node(
    tenant: &str,
    canonical_node_id: &str,
    typed_fact_hash: &str,
    node_id: &str,
    alias: &AliasWitness,
) -> CanonicalWriteResult<NodeRecord> {
    Ok(NodeRecord::new(
        node_id,
        [ALIAS_WITNESS_LABEL],
        json!({
            "tenant_slug": tenant,
            "canonical_node_id": canonical_node_id,
            "typed_fact_hash": typed_fact_hash,
            "alias": alias.alias,
            "source_ref": alias.source_ref,
            "observed_as": alias.observed_as,
            "confidence": alias.confidence,
            "metadata": alias.metadata,
            "authority": "witness_not_truth",
        }),
    ))
}

fn alias_witness_edge(
    tenant: &str,
    canonical_node_id: &str,
    alias_node_id: &str,
    alias: &AliasWitness,
) -> EdgeRecord {
    EdgeRecord::new(
        format!("{alias_node_id}:witness-for:{canonical_node_id}"),
        alias_node_id,
        EDGE_ALIAS_WITNESS_FOR,
        canonical_node_id,
        json!({
            "tenant_slug": tenant,
            "alias": alias.alias,
            "source_ref": alias.source_ref,
            "confidence": alias.confidence,
            "authority": "witness_only",
        }),
    )
}

fn fact_value(fact: &TypedFact) -> Value {
    json!({
        "fact_type": fact.fact_type.trim(),
        "canonical_key": fact.canonical_key.trim(),
        "statement": fact.statement.trim(),
        "properties": fact.properties,
        "source_refs": fact.source_refs,
        "confidence": fact.confidence,
    })
}

fn validate_fact(fact: &TypedFact) -> CanonicalWriteResult<()> {
    require_nonempty("fact.fact_type", &fact.fact_type)?;
    require_nonempty("fact.canonical_key", &fact.canonical_key)?;
    require_nonempty("fact.statement", &fact.statement)?;
    if fact
        .source_refs
        .iter()
        .all(|source| source.trim().is_empty())
    {
        return Err(CanonicalWriteError::InvalidInput {
            field: "fact.source_refs".to_string(),
            message: "at least one typed-fact source is required".to_string(),
        });
    }
    validate_confidence("fact.confidence", fact.confidence)?;
    Ok(())
}

fn validate_nominations(nominations: &[EmbeddingNomination]) -> CanonicalWriteResult<()> {
    for nomination in nominations {
        require_nonempty("nomination.candidate_id", &nomination.candidate_id)?;
        validate_confidence("nomination.score", nomination.score)?;
    }
    Ok(())
}

fn validate_aliases(aliases: &[AliasWitness]) -> CanonicalWriteResult<()> {
    for alias in aliases {
        require_nonempty("alias.alias", &alias.alias)?;
        require_nonempty("alias.source_ref", &alias.source_ref)?;
        validate_confidence("alias.confidence", alias.confidence)?;
    }
    Ok(())
}

fn validate_confidence(field: &str, value: Option<f64>) -> CanonicalWriteResult<()> {
    if let Some(value) = value {
        if !(0.0..=1.0).contains(&value) || !value.is_finite() {
            return Err(CanonicalWriteError::InvalidInput {
                field: field.to_string(),
                message: "confidence/score must be a finite value between 0 and 1".to_string(),
            });
        }
    }
    Ok(())
}

fn require_nonempty(field: &str, value: &str) -> CanonicalWriteResult<()> {
    if value.trim().is_empty() {
        return Err(CanonicalWriteError::InvalidInput {
            field: field.to_string(),
            message: "value is required".to_string(),
        });
    }
    Ok(())
}

fn dedupe_nominations(nominations: &[EmbeddingNomination]) -> Vec<&EmbeddingNomination> {
    let mut seen = BTreeSet::new();
    nominations
        .iter()
        .filter(|nomination| {
            seen.insert((
                nomination.candidate_id.trim().to_string(),
                nomination.model.trim().to_string(),
                nomination.query_ref.trim().to_string(),
                nomination.evidence_ref.trim().to_string(),
            ))
        })
        .collect()
}

fn dedupe_aliases(aliases: &[AliasWitness]) -> Vec<&AliasWitness> {
    let mut seen = BTreeSet::new();
    aliases
        .iter()
        .filter(|alias| {
            seen.insert((
                alias.alias.trim().to_string(),
                alias.source_ref.trim().to_string(),
                alias.observed_as.trim().to_string(),
            ))
        })
        .collect()
}

fn normalize_tenant(tenant: &str) -> String {
    normalize_tenant_slug(tenant)
}

fn require_tenant(tenant: &str) -> CanonicalWriteResult<String> {
    if tenant.trim().is_empty() {
        return Err(CanonicalWriteError::InvalidInput {
            field: "tenant_slug".to_string(),
            message: "is required".to_string(),
        });
    }
    Ok(normalize_tenant_slug(tenant))
}

fn timestamp_or_now(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "unix_ms:0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn slugify(value: &str) -> String {
    let mut out = String::new();
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    if out.trim_matches('-').is_empty() {
        "unknown".to_string()
    } else {
        out.trim_matches('-').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::InMemoryGraphStore;

    fn fact(key: &str) -> TypedFact {
        TypedFact {
            fact_type: "project_decision".to_string(),
            canonical_key: key.to_string(),
            statement: "Patch sequencing is optimistic and git-backed.".to_string(),
            properties: Map::new(),
            source_refs: vec!["handoff:multi-head-run-execution".to_string()],
            confidence: Some(0.94),
        }
    }

    #[test]
    fn typed_fact_not_embedding_candidate_chooses_canonical_id() {
        let mut store = InMemoryGraphStore::new();
        let first = canonicalize_on_write(
            &mut store,
            CanonicalizeOnWriteInput {
                tenant_slug: "tenant-a".to_string(),
                fact: fact("multihead/write-arbitration"),
                nominations: vec![EmbeddingNomination {
                    candidate_id: "nearest-vector-a".to_string(),
                    model: "embedding-small".to_string(),
                    score: Some(0.91),
                    ..EmbeddingNomination::default()
                }],
                ..CanonicalizeOnWriteInput::default()
            },
        )
        .unwrap();
        let second = canonicalize_on_write(
            &mut store,
            CanonicalizeOnWriteInput {
                tenant_slug: "tenant-a".to_string(),
                fact: fact("multihead/write-arbitration"),
                nominations: vec![EmbeddingNomination {
                    candidate_id: "nearest-vector-b".to_string(),
                    model: "embedding-small".to_string(),
                    score: Some(0.99),
                    ..EmbeddingNomination::default()
                }],
                ..CanonicalizeOnWriteInput::default()
            },
        )
        .unwrap();

        assert_eq!(first.canonical_node_id, second.canonical_node_id);
        assert!(first.created);
        assert!(!second.created);
        let node = store.get_node(&first.canonical_node_id).unwrap();
        assert_eq!(node.properties["ratified_by"], "typed_fact");
        assert_eq!(node.properties["embedding_role"], "nomination_only");
    }

    #[test]
    fn aliases_persist_as_witnesses_to_the_canonical_fact() {
        let mut store = InMemoryGraphStore::new();
        let receipt = canonicalize_on_write(
            &mut store,
            CanonicalizeOnWriteInput {
                tenant_slug: "tenant-a".to_string(),
                fact: fact("multihead/state-substrate"),
                aliases: vec![AliasWitness {
                    alias: "one agent, two heads".to_string(),
                    source_ref: "conversation:claude-codex".to_string(),
                    observed_as: "user phrase".to_string(),
                    confidence: Some(0.88),
                    ..AliasWitness::default()
                }],
                ..CanonicalizeOnWriteInput::default()
            },
        )
        .unwrap();

        assert_eq!(receipt.alias_witness_node_ids.len(), 1);
        let alias_node = store
            .get_node(&receipt.alias_witness_node_ids[0])
            .expect("alias witness node exists");
        assert_eq!(alias_node.properties["authority"], "witness_not_truth");
        let edge_id = format!(
            "{}:witness-for:{}",
            receipt.alias_witness_node_ids[0], receipt.canonical_node_id
        );
        let edge = store.get_edge(&edge_id).expect("alias witness edge exists");
        assert_eq!(edge.edge_type, EDGE_ALIAS_WITNESS_FOR);
        assert_eq!(edge.to_id, receipt.canonical_node_id);
    }

    #[test]
    fn embedding_candidates_persist_as_nominations_only() {
        let mut store = InMemoryGraphStore::new();
        let receipt = canonicalize_on_write(
            &mut store,
            CanonicalizeOnWriteInput {
                tenant_slug: "tenant-a".to_string(),
                fact: fact("multihead/canonical-write"),
                nominations: vec![EmbeddingNomination {
                    candidate_id: "vector-neighbor-42".to_string(),
                    model: "embedding-small".to_string(),
                    score: Some(0.77),
                    query_ref: "query:canonical-write".to_string(),
                    evidence_ref: "embedding-run:17".to_string(),
                }],
                ..CanonicalizeOnWriteInput::default()
            },
        )
        .unwrap();

        assert_eq!(receipt.nomination_node_ids.len(), 1);
        let nomination_node = store
            .get_node(&receipt.nomination_node_ids[0])
            .expect("nomination node exists");
        assert_eq!(
            nomination_node.properties["authority"],
            "nomination_not_truth"
        );
        let edge_id = format!(
            "{}:nominated:{}",
            receipt.nomination_node_ids[0], receipt.canonical_node_id
        );
        let edge = store
            .get_edge(&edge_id)
            .expect("embedding nomination edge exists");
        assert_eq!(edge.edge_type, EDGE_EMBEDDING_NOMINATED);
        assert_eq!(edge.to_id, receipt.canonical_node_id);
    }

    #[test]
    fn valid_canonical_write_requires_tenant() {
        let mut store = InMemoryGraphStore::new();
        let error = canonicalize_on_write(
            &mut store,
            CanonicalizeOnWriteInput {
                fact: fact("tenant-required"),
                ..CanonicalizeOnWriteInput::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            CanonicalWriteError::InvalidInput { ref field, .. } if field == "tenant_slug"
        ));
        assert!(store
            .get_node(&canonical_fact_node_id(
                "default",
                "project_decision",
                "tenant-required"
            ))
            .is_none());
    }

    #[test]
    fn typed_fact_requires_key_statement_and_source() {
        let mut store = InMemoryGraphStore::new();
        let invalid = canonicalize_on_write(
            &mut store,
            CanonicalizeOnWriteInput {
                fact: TypedFact {
                    fact_type: "decision".to_string(),
                    canonical_key: String::new(),
                    statement: "missing key".to_string(),
                    source_refs: vec!["src".to_string()],
                    ..TypedFact::default()
                },
                ..CanonicalizeOnWriteInput::default()
            },
        );
        assert!(matches!(
            invalid,
            Err(CanonicalWriteError::InvalidInput { field, .. })
                if field == "fact.canonical_key"
        ));

        let no_source = canonicalize_on_write(
            &mut store,
            CanonicalizeOnWriteInput {
                fact: TypedFact {
                    fact_type: "decision".to_string(),
                    canonical_key: "key".to_string(),
                    statement: "missing source".to_string(),
                    source_refs: vec![],
                    ..TypedFact::default()
                },
                ..CanonicalizeOnWriteInput::default()
            },
        );
        assert!(matches!(
            no_source,
            Err(CanonicalWriteError::InvalidInput { field, .. })
                if field == "fact.source_refs"
        ));
    }
}
