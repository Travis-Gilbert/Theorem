use rustyred_thg_core::{
    stable_hash, EdgeRecord, GraphStore, GraphStoreResult, NodeQuery, NodeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::features::{feature_record_from_node, CodeFeatureRecord, CODE_FEATURE_LABEL};
use super::ir::{CODE_COMPILER_FEATURE_VERSION, CODE_COMPILER_VERSION};
use crate::SOURCE;

pub const CODE_ANNOTATION_LABEL: &str = "CodeCompilerAnnotation";
pub const ANNOTATES_CODE_FEATURE: &str = "ANNOTATES_CODE_FEATURE";

const EDL_MIN_EVIDENCE: f64 = 2.0;
const EDL_MAX_EVIDENCE: f64 = 50.0;
const FEATURE_ACTIVE_THRESHOLD: f64 = 0.01;
const EBL_MIN_FEATURE_IMPORTANCE: f64 = 0.03;
const EBL_MAX_FEATURES: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeAnnotationInput {
    pub tenant_id: String,
    pub repo_id: String,
    pub max_features: usize,
}

impl CodeAnnotationInput {
    pub fn new(tenant_id: impl Into<String>, repo_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            max_features: EBL_MAX_FEATURES,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CodeEblFeatureContribution {
    pub feature: String,
    pub value: f64,
    pub importance: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CodeCompilerAnnotationRecord {
    pub annotation_id: String,
    pub feature_id: String,
    pub epistemic_uncertainty: f64,
    pub aleatoric_uncertainty: f64,
    pub evidence_count: f64,
    pub active_feature_count: usize,
    pub explanation: String,
    pub top_features: Vec<CodeEblFeatureContribution>,
    pub calibration_version: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodeAnnotationOutput {
    pub tenant_id: String,
    pub repo_id: String,
    pub annotations: Vec<CodeCompilerAnnotationRecord>,
}

pub fn annotate_code_features_in_store<S: GraphStore>(
    store: &mut S,
    input: CodeAnnotationInput,
) -> GraphStoreResult<CodeAnnotationOutput> {
    let feature_nodes = store
        .query_nodes(
            NodeQuery::label(CODE_FEATURE_LABEL)
                .with_property("tenant_id", json!(&input.tenant_id))
                .with_property("repo_id", json!(&input.repo_id))
                .with_limit(100_000),
        )
        .into_iter()
        .filter(|node| !node.tombstone)
        .collect::<Vec<_>>();

    let mut annotations = Vec::new();
    for node in feature_nodes {
        let Some(feature) = feature_record_from_node(&node) else {
            continue;
        };
        let annotation = annotation_for_feature(&feature, input.max_features);
        write_code_annotation_record(store, &input.tenant_id, &input.repo_id, &annotation)?;
        annotations.push(annotation);
    }
    Ok(CodeAnnotationOutput {
        tenant_id: input.tenant_id,
        repo_id: input.repo_id,
        annotations,
    })
}

pub(super) fn write_code_annotation_record<S: GraphStore>(
    store: &mut S,
    tenant_id: &str,
    repo_id: &str,
    record: &CodeCompilerAnnotationRecord,
) -> GraphStoreResult<()> {
    store.upsert_node(annotation_node(tenant_id, repo_id, record))?;
    store.upsert_edge(EdgeRecord::new(
        annotation_edge_id(&record.annotation_id, &record.feature_id),
        &record.annotation_id,
        ANNOTATES_CODE_FEATURE,
        &record.feature_id,
        json!({
            "tenant_id": tenant_id,
            "repo_id": repo_id,
            "compiler_version": CODE_COMPILER_VERSION,
            "feature_version": CODE_COMPILER_FEATURE_VERSION,
            "source": SOURCE,
        }),
    ))?;
    Ok(())
}

pub(super) fn count_annotations<S: GraphStore>(store: &S, tenant_id: &str, repo_id: &str) -> usize {
    store
        .query_nodes(
            NodeQuery::label(CODE_ANNOTATION_LABEL)
                .with_property("tenant_id", json!(tenant_id))
                .with_property("repo_id", json!(repo_id))
                .with_limit(100_000),
        )
        .into_iter()
        .filter(|node| !node.tombstone)
        .count()
}

fn annotation_for_feature(
    feature: &CodeFeatureRecord,
    max_features: usize,
) -> CodeCompilerAnnotationRecord {
    let active_feature_count = feature.features.active_feature_count();
    let evidence_count = active_feature_count as f64;
    let epistemic_uncertainty = if evidence_count <= EDL_MIN_EVIDENCE {
        1.0
    } else {
        1.0 - ((evidence_count - EDL_MIN_EVIDENCE) / (EDL_MAX_EVIDENCE - EDL_MIN_EVIDENCE))
            .clamp(0.0, 1.0)
    };
    let contradiction = feature.features.nli_contradiction_score.clamp(0.0, 1.0);
    let novelty = feature.features.rnd_novelty_score.clamp(0.0, 1.0);
    let aleatoric_uncertainty = ((contradiction + novelty) / 2.0).clamp(0.0, 1.0);
    let mut top_features = feature
        .features
        .values()
        .into_iter()
        .filter(|(_, value)| value.abs() >= FEATURE_ACTIVE_THRESHOLD)
        .map(|(name, value)| CodeEblFeatureContribution {
            feature: name.to_string(),
            value,
            importance: value.abs(),
        })
        .filter(|contribution| contribution.importance >= EBL_MIN_FEATURE_IMPORTANCE)
        .collect::<Vec<_>>();
    top_features.sort_by(|left, right| {
        right
            .importance
            .partial_cmp(&left.importance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.feature.cmp(&right.feature))
    });
    top_features.truncate(if max_features == 0 {
        EBL_MAX_FEATURES
    } else {
        max_features.min(EBL_MAX_FEATURES)
    });
    let explanation = if top_features.is_empty() {
        "No active compiler features exceeded the EBL explanation threshold.".to_string()
    } else {
        format!(
            "Connection evidence is driven by {}.",
            top_features
                .iter()
                .map(|feature| feature.feature.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let annotation_id = format!(
        "code:annotation:{}",
        stable_hash(json!([
            &feature.feature_id,
            evidence_count,
            epistemic_uncertainty,
            aleatoric_uncertainty,
            CODE_COMPILER_FEATURE_VERSION
        ]))
    );
    CodeCompilerAnnotationRecord {
        annotation_id,
        feature_id: feature.feature_id.clone(),
        epistemic_uncertainty: round6(epistemic_uncertainty),
        aleatoric_uncertainty: round6(aleatoric_uncertainty),
        evidence_count,
        active_feature_count,
        explanation,
        top_features,
        calibration_version: "edl-ebl-local-v1".to_string(),
    }
}

fn annotation_node(
    tenant_id: &str,
    repo_id: &str,
    record: &CodeCompilerAnnotationRecord,
) -> NodeRecord {
    NodeRecord::new(
        &record.annotation_id,
        [CODE_ANNOTATION_LABEL],
        json!({
            "tenant_id": tenant_id,
            "repo_id": repo_id,
            "feature_id": &record.feature_id,
            "epistemic_uncertainty": record.epistemic_uncertainty,
            "aleatoric_uncertainty": record.aleatoric_uncertainty,
            "evidence_count": record.evidence_count,
            "active_feature_count": record.active_feature_count,
            "explanation": &record.explanation,
            "top_features": &record.top_features,
            "calibration_version": &record.calibration_version,
            "compiler_version": CODE_COMPILER_VERSION,
            "feature_version": CODE_COMPILER_FEATURE_VERSION,
            "source": SOURCE,
        }),
    )
}

fn annotation_edge_id(annotation_id: &str, feature_id: &str) -> String {
    format!(
        "code:edge:annotation:{}",
        stable_hash(json!([annotation_id, feature_id]))
    )
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}
