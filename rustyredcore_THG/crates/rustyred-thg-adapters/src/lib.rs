//! LoRA adapter catalog over RustyRedCore-THG graph records.
//!
//! The crate stays above `rustyred-thg-core`: it reuses core graph records,
//! stores, and PPR, while keeping adapter-specific routing and fitness logic
//! out of the core executor.

pub mod commands;
pub mod fitness;
pub mod grounded_skill;
pub mod routing;
pub mod situation_search;
pub mod training_runner;
pub mod training_substrate;
pub mod types;
pub mod upsert;

pub use commands::{execute_adapter_command, AdapterCommandResponse};
pub use fitness::{
    effective_fitness, find_adapter_by_id, list_adapters, record_fitness, supersede_adapter,
};
pub use grounded_skill::{
    build_grounded_skill_folder, GroundedSkillBuildInput, GroundedSkillFile, GroundedSkillFolder,
    GroundedSkillProvenance, GroundedSkillScript, GroundedSkillScriptLanguage,
    GroundedSkillSourceRef, AGENT_SKILL_STANDARD, DEFAULT_GROUNDED_SKILL_EMBEDDER_MODEL,
};
pub use routing::{
    adapter_training_centroid, find_adapters_by_query_embedding, find_adapters_for,
    recompute_embedding,
};
pub use situation_search::{
    default_situation_target_labels, record_similar_situation_search,
    register_semantic_vector_designations, semantic_vector_designations, similar_situation_search,
    SimilarSituationDecision, SimilarSituationHit, SimilarSituationSearchMode,
    SimilarSituationSearchPolicy, SimilarSituationSearchReceipt, SimilarSituationSearchRequest,
    SimilarSituationSearchResult, SituationSearchGraphStore, CODE_FILE_LABEL, CODE_OBJECT_LABEL,
    CODE_SYMBOL_LABEL, EMBEDDING_CODEGRAPHBERT_768, EMBEDDING_CODE_UNIXCODER_768,
    EMBEDDING_SITUATION_SBERT_384, EMBEDDING_TRAINING_SBERT_384, EMBEDDING_USER_SBERT_384,
    ESCALATED_TO_SEARCH, HARNESS_EVENT_LABEL, HARNESS_RUN_LABEL, MATCHED_SIMILAR_SITUATION,
    SEARCH_ESCALATION_PLAN_LABEL, SIMILAR_SITUATION_SEARCH_LABEL, USER_MODEL_LABEL,
    USER_PREFERENCE_LABEL,
};
pub use training_runner::{
    export_training_snapshot_files, import_gnn_export_dir, open_training_store,
    redcore_training_options, run_local_training_smoke, runpod_input_for_manifest,
    seed_training_fixture, writeback_model_artifact_file, RunPodTrainingInput, TrainingExportFiles,
    TrainingSmokeResult, TrainingSnapshotBundle, TrainingSnapshotLocalFiles, GRAPH_SNAPSHOT_FILE,
    MANIFEST_FILE, RUNPOD_INPUT_FILE,
};
pub use training_substrate::{
    artifact_node_id, evaluation_receipt_node_id, export_training_snapshot, gnn_export_node_id,
    model_artifact_node_id, paraphrase_pair_node_id, postmortem_node_id, reasoning_trace_node_id,
    register_gnn_export_dir, register_model_artifact, register_training_fixture,
    trace_step_node_id, training_pack_node_id, GnnExportImportOptions, GnnExportImportResult,
    ModelArtifactInput, ModelWritebackResult, TrainingExportCounts, TrainingExportManifest,
    TrainingFixtureResult, ARTIFACT_LABEL, EVALUATED_BY, EVALUATION_RECEIPT_LABEL,
    GNN_ENTITY_LABEL, GNN_EXPORT_LABEL, HAS_ENTITY, HAS_GNN_EXPORT, HAS_STEP, HAS_TRAINING_PAIR,
    MODEL_ARTIFACT_LABEL, OBJECT_LABEL, PARAPHRASE_PAIR_LABEL, PART_OF_PACK, POSTMORTEM_LABEL,
    PRODUCED_ARTIFACT, PROMOTED_TO_ACTIVE, REASONING_TRACE_LABEL, TRACE_STEP_LABEL,
    TRAINING_EXPORT_LABEL, TRAINING_PACK_LABEL, USED_ARTIFACT,
};
pub use types::{
    adapter_node_id, adapter_vector_designation, normalize_tenant_id, object_node_id,
    AdapterFindRequest, AdapterFitnessRecordRequest, AdapterFitnessRecordResult, AdapterGraphStore,
    AdapterListRequest, AdapterRef, AdapterSupersedeResult, AdapterUpsertResult, LoraAdapter,
    DEFAULT_FITNESS_EPSILON, DEFAULT_MIN_FITNESS, DEFAULT_PPR_DAMPING, DEFAULT_PPR_MAX_PUSHES,
    DEFAULT_SHARED_WEIGHT, DEFAULT_THESEUS_HALF_LIFE_DAYS, DERIVED_FROM, FITNESS_SIGNAL,
    LORA_ADAPTER_LABEL, SHARED_WITH, SUPERSEDES, TENANT_LABEL, THG_ADAPTER_SOURCE, TRAINED_ON,
};
pub use upsert::upsert_adapter;

#[cfg(test)]
#[path = "tests/upsert_test.rs"]
mod upsert_test;

#[cfg(test)]
#[path = "tests/routing_test.rs"]
mod routing_test;

#[cfg(test)]
#[path = "tests/grounded_skill_test.rs"]
mod grounded_skill_test;

#[cfg(test)]
#[path = "tests/situation_search_test.rs"]
mod situation_search_test;

#[cfg(test)]
#[path = "tests/fitness_test.rs"]
mod fitness_test;

#[cfg(test)]
#[path = "tests/training_substrate_test.rs"]
mod training_substrate_test;

#[cfg(test)]
#[path = "tests/training_runner_test.rs"]
mod training_runner_test;
