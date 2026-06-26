use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::graph_store::{GraphStoreError, GraphStoreResult};
use crate::labeled_training_run::LabeledTrainingRun;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingExportKind {
    ContextRank,
    ToolPolicy,
    AdapterPolicy,
    ValidatorPolicy,
    MemoryRecall,
    MapReuse,
    ArtifactOutcome,
    TrainingSft,
    TrainingPreference,
    GraphPack,
    MultiVectorRerank,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionStatus {
    Pending,
    Safe,
    Redacted,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TrainingExportRecord {
    pub export_kind: TrainingExportKind,
    pub graph_version: u64,
    pub redaction_status: RedactionStatus,
    pub source_run_id: String,
    pub source_artifact_id: String,
    pub payload: Value,
}

impl TrainingExportRecord {
    pub fn from_labeled_run(
        export_kind: TrainingExportKind,
        run: &LabeledTrainingRun,
        redaction_status: RedactionStatus,
    ) -> GraphStoreResult<Self> {
        let payload = serde_json::to_value(run).map_err(|err| {
            GraphStoreError::new(
                "training_export_serialize_error",
                format!("could not serialize labeled training run {}: {err}", run.id),
            )
        })?;
        Ok(Self {
            export_kind,
            graph_version: run.graph_version,
            redaction_status,
            source_run_id: run.run_id.clone(),
            source_artifact_id: run.artifact_id.clone(),
            payload,
        })
    }
}

pub fn export_records_jsonl(records: &[TrainingExportRecord]) -> GraphStoreResult<String> {
    let mut out = String::new();
    for record in records {
        let line = serde_json::to_string(record).map_err(|err| {
            GraphStoreError::new(
                "training_export_serialize_error",
                format!("could not serialize training export record: {err}"),
            )
        })?;
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out)
}
