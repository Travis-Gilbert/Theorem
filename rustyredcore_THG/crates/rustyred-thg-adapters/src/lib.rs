//! LoRA adapter catalog over RustyRedCore-THG graph records.
//!
//! The crate stays above `rustyred-thg-core`: it reuses core graph records,
//! stores, and PPR, while keeping adapter-specific routing and fitness logic
//! out of the core executor.

#[cfg(feature = "pairformer-burn-cubecl")]
pub mod burn_mpnn;
#[cfg(feature = "pairformer-burn-cubecl")]
pub mod burn_pairformer;
pub mod commands;
pub mod edge_mpnn;
pub mod fitness;
pub mod grounded_skill;
pub mod hot;
#[cfg(feature = "pairformer-burn-cubecl")]
pub mod hot_burn;
#[cfg(feature = "pairformer-burn-cubecl")]
pub mod hot_cubecl;
pub mod pairformer;
#[cfg(feature = "pairformer-burn-cubecl")]
pub mod pairformer_cubecl;
pub mod reflexive;
pub mod reflexive_executor;
pub mod routing;
pub mod situation_search;
pub mod standing_pass;
pub mod training_runner;
pub mod training_substrate;
pub mod types;
pub mod upsert;

#[cfg(feature = "pairformer-burn-cubecl")]
pub use burn_mpnn::{aggregate_messages_burn, BurnAggregator, BurnEdgeMpnnLayer};
#[cfg(feature = "pairformer-burn-cubecl")]
pub use burn_pairformer::{
    featurize_pairformer_input, load_pairformer_file,
    rank_trained_pairformer_densification_candidates, register_trained_pairformer_artifact,
    save_pairformer_file, score_links_with_trained, train_pairformer, BurnPairformer,
    BurnPairformerConfig, PairformerTrainingConfig, PairformerTrainingReport,
};
pub use commands::{execute_adapter_command, AdapterCommandResponse};
pub use edge_mpnn::{
    rank_global_completion_candidates, rank_global_completion_candidates_default,
    FixedPointAggregator, GlobalCompletionConfig, GlobalCompletionRequest, GlobalCompletionResult,
    MessageAggregator, DEFAULT_COMPLETION_HIDDEN_DIM, DEFAULT_COMPLETION_LAYERS,
    DEFAULT_COMPLETION_MAX_FRONTIER_NODES, DEFAULT_COMPLETION_MAX_SEEDS,
};
pub use fitness::{
    effective_fitness, find_adapter_by_id, list_adapters, record_fitness, supersede_adapter,
};
pub use grounded_skill::{
    build_grounded_skill_folder, GroundedSkillBuildInput, GroundedSkillFile, GroundedSkillFolder,
    GroundedSkillProvenance, GroundedSkillScript, GroundedSkillScriptLanguage,
    GroundedSkillSourceRef, AGENT_SKILL_STANDARD, DEFAULT_GROUNDED_SKILL_EMBEDDER_MODEL,
};
pub use hot::{
    build_hot_feature_matrices, evaluate_hot_predictions, extract_higher_order_temporal_neighbors,
    hot_input_from_snapshot, hot_temporal_edge_timestamp_for_record,
    hot_temporal_link_dataset_from_snapshot, hot_training_examples_from_input,
    hot_training_examples_from_timed_labels, patch_align_and_concatenate, run_hot,
    score_hot_timed_labels, tgat_time_encoding, train_hot_link_model, HotConfig,
    HotFeatureMatrices, HotInput, HotLearnedModel, HotLinkScore, HotLinkTrainingExample,
    HotNegativeSamplingScheme, HotNode, HotNodeRepresentation, HotOutput, HotPairLabel,
    HotPairPrediction, HotPairRepresentation, HotPairSequence, HotTemporalEdge,
    HotTemporalLinkDataset, HotTemporalSplitConfig, HotTimeEncoder, HotTimedPairLabel,
    HotTrainingConfig, HotTrainingReport,
    DEFAULT_HOT_ALIGNED_DIM, DEFAULT_HOT_BLOCK_SIZE, DEFAULT_HOT_BRT_CELLS,
    DEFAULT_HOT_COOCCURRENCE_DIM, DEFAULT_HOT_DECODER_HIDDEN_DIM, DEFAULT_HOT_HEADS,
    DEFAULT_HOT_HORIZONTAL_CELL_INDEX, DEFAULT_HOT_MAX_NODES, DEFAULT_HOT_MAX_TEMPORAL_EDGES,
    DEFAULT_HOT_OUTPUT_DIM, DEFAULT_HOT_PATCH_SIZE, DEFAULT_HOT_S1, DEFAULT_HOT_S2,
    DEFAULT_HOT_SEGMENT_SIZE, DEFAULT_HOT_STATE_VECTORS, DEFAULT_HOT_TIME_ENCODING_DIM,
};
#[cfg(feature = "pairformer-burn-cubecl")]
pub use hot_burn::{featurize_hot_training_examples, BurnHot, BurnHotConfig};
pub use pairformer::{
    run_pairformer, PairformerConfig, PairformerEdgeInput, PairformerInput, PairformerLinkScore,
    PairformerNodeInput, PairformerOutput, PairformerPairRepresentation, PairformerSupportPath,
    DEFAULT_PAIRFORMER_BLOCKS, DEFAULT_PAIRFORMER_MAX_NODES, DEFAULT_PAIRFORMER_PAIR_DIM,
    DEFAULT_PAIRFORMER_SINGLE_DIM, DEFAULT_PAIRFORMER_TRANSITION_HIDDEN_DIM,
};
pub use reflexive::{
    aggregate_messages_fixed_point, choose_scatter_aggregation_path,
    densification_candidate_node_id, densification_run_node_id, property_candidate_node_id,
    property_candidate_run_node_id, quarantine_densification_candidates,
    quarantine_property_candidates, quarantine_property_candidates_with_options,
    rank_classification_property_candidates, rank_densification_candidates,
    rank_hot_temporal_densification_candidates, rank_missing_property_candidates,
    rank_pairformer_densification_candidates,
    rank_property_candidates, rank_reflexive_organizing_candidates, rank_spatial_candidates,
    rank_temporal_candidates, representation_sidecar_node_id, upsert_representation_sidecar,
    DensificationQuarantineResult, DensificationRequest, DensificationResult,
    InferredEdgeCandidate, InferredPropertyCandidate, PropertyCandidateQuarantineOptions,
    PropertyCandidateQuarantineResult, PropertyCandidateResult, RepresentationSidecarInput,
    RepresentationSidecarWriteback, RepresentationTargetKind, ScatterAggregationPath,
    ScatterAggregationRequest, DEFAULT_DENSIFICATION_CONFIDENCE_CEILING,
    DEFAULT_DENSIFICATION_MAX_CANDIDATES, DEFAULT_DENSIFICATION_MAX_DEPTH,
    DEFAULT_DENSIFICATION_MAX_NODES, DEFAULT_FIXED_POINT_SCALE,
    DEFAULT_SCATTER_BURN_NATIVE_MAX_ELEMENTS, DEFAULT_SPATIAL_RADIUS_KM,
    DEFAULT_SPATIAL_RESOLUTION, DEFAULT_TEMPORAL_WINDOW_MS, REFLEXIVE_CANDIDATE_OF,
    REFLEXIVE_CANDIDATE_SOURCE, REFLEXIVE_CANDIDATE_TARGET, REFLEXIVE_DENSIFICATION_RUN_LABEL,
    REFLEXIVE_EDGE_CANDIDATE_LABEL, REFLEXIVE_PROPERTY_CANDIDATE_LABEL,
    REPRESENTATION_SIDECAR_LABEL, REPRESENTS_NODE,
};
pub use reflexive_executor::{
    adapter_factors_node_id, apply_low_rank_adapter, load_adapter_factors,
    load_node_representation, reflexive_match_inference, score_match_neighborhood,
    upsert_adapter_factors_sidecar, LowRankAdapterFactors, MatchInferenceResult,
    MatchInferenceScorer, MatchNeighborhoodInput, NodeRepresentation, ReflexiveReadStore,
    ADAPTER_FACTORS_LABEL, FACTORS_FOR_ADAPTER,
};
pub use routing::{
    adapter_training_centroid, find_adapters_by_query_embedding, find_adapters_for,
    recompute_embedding,
};
pub use situation_search::{
    context_candidates_from_similar_situation, default_situation_target_labels,
    enrich_context_candidates_from_store, record_context_scoring_result,
    record_context_use_outcome, record_similar_situation_search,
    register_semantic_vector_designations, score_context_atoms, semantic_vector_designations,
    similar_situation_search, ContextAtomCandidate, ContextScoringPolicy, ContextScoringReceipt,
    ContextScoringResult, RankedContextAtom, SimilarSituationDecision, SimilarSituationHit,
    SimilarSituationSearchMode, SimilarSituationSearchPolicy, SimilarSituationSearchReceipt,
    SimilarSituationSearchRequest, SimilarSituationSearchResult, SituationSearchGraphStore,
    CODE_FILE_LABEL, CODE_OBJECT_LABEL, CODE_SYMBOL_LABEL, CONTEXT_ATOM_LABEL,
    CONTEXT_ATOM_SELECTED, CONTEXT_PACK_LABEL, CONTEXT_PACK_OUTCOME, CONTEXT_USE_RECEIPT_LABEL,
    DEFAULT_CONTEXT_MAX_ATOMS, DEFAULT_CONTEXT_TOKEN_BUDGET, DEFAULT_CONTEXT_TOKEN_COST,
    EMBEDDING_CODEGRAPHBERT_768, EMBEDDING_CODE_UNIXCODER_768, EMBEDDING_SITUATION_SBERT_384,
    EMBEDDING_TRAINING_SBERT_384, EMBEDDING_USER_SBERT_384, ESCALATED_TO_SEARCH,
    HARNESS_EVENT_LABEL, HARNESS_RUN_LABEL, MATCHED_SIMILAR_SITUATION,
    SEARCH_ESCALATION_PLAN_LABEL, SIMILAR_SITUATION_SEARCH_LABEL, USER_MODEL_LABEL,
    USER_PREFERENCE_LABEL,
};
pub use standing_pass::{
    admitted_edge_id, default_standing_generators, standing_pass_hook,
    standing_pass_hook_with_engine, standing_pass_run_id, AdvisoryCandidate, AdvisoryPayload,
    CandidateKind, CandidatePair, CandidateRef, DatalogStandingGenerator,
    EgglogEquivalenceStandingGenerator, GeneratorInput, GeneratorQuery, GeneratorQueryKind,
    HotTemporalStandingGenerator, PairformerStandingGenerator, SourceReliabilityStandingGenerator,
    SpatialStandingGenerator, Spec1PropertyStandingGenerator, StandingGenerator,
    StandingPassConfig, StandingPassEngine, StandingPassResult, DATALOG_STANDING_GENERATOR_ID,
    DEFAULT_STANDING_PASS_CONFIDENCE_CEILING, DEFAULT_STANDING_PASS_CONFIDENCE_THRESHOLD,
    DEFAULT_STANDING_PASS_MAX_CANDIDATES, DEFAULT_STANDING_PASS_MAX_DEPTH,
    DEFAULT_STANDING_PASS_MAX_NODES, EGGLOG_EQUIVALENCE_STANDING_GENERATOR_ID,
    SOURCE_RELIABILITY_STANDING_GENERATOR_ID, SPEC1_PROPERTY_STANDING_GENERATOR_ID,
    STANDING_PASS_ADMITTED_BY,
};
pub use training_runner::{
    export_training_snapshot_files, import_gnn_export_dir, open_training_store,
    redcore_training_options, run_hot_temporal_training, run_local_training_smoke,
    runpod_input_for_manifest, seed_hot_temporal_fixture, seed_training_fixture,
    writeback_model_artifact_file, HotBaselineReport, HotTemporalFixtureResult,
    HotTrainingRunMetrics, HotTrainingRunOptions, HotTrainingRunResult, RunPodTrainingInput,
    TrainingExportFiles, TrainingSmokeResult, TrainingSnapshotBundle, TrainingSnapshotLocalFiles,
    GRAPH_SNAPSHOT_FILE, HOT_MODEL_ARTIFACT_FILE, HOT_MODEL_FILE, MANIFEST_FILE, RUNPOD_INPUT_FILE,
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
    tenant_node_id, AdapterFindRequest, AdapterFitnessRecordRequest, AdapterFitnessRecordResult,
    AdapterGraphStore, AdapterListRequest, AdapterRef, AdapterSupersedeResult, AdapterUpsertResult,
    LoraAdapter, DEFAULT_FITNESS_EPSILON, DEFAULT_MIN_FITNESS, DEFAULT_PPR_DAMPING,
    DEFAULT_PPR_MAX_PUSHES, DEFAULT_SHARED_WEIGHT, DEFAULT_THESEUS_HALF_LIFE_DAYS, DERIVED_FROM,
    FITNESS_SIGNAL, LORA_ADAPTER_LABEL, SHARED_WITH, SUPERSEDES, TENANT_LABEL, THG_ADAPTER_SOURCE,
    TRAINED_ON,
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
#[path = "tests/reflexive_test.rs"]
mod reflexive_test;

#[cfg(test)]
#[path = "tests/edge_mpnn_test.rs"]
mod edge_mpnn_test;

#[cfg(test)]
#[path = "tests/reflexive_executor_test.rs"]
mod reflexive_executor_test;

#[cfg(all(test, feature = "pairformer-burn-cubecl"))]
#[path = "tests/burn_mpnn_test.rs"]
mod burn_mpnn_test;

#[cfg(all(test, feature = "pairformer-burn-cubecl"))]
#[path = "tests/burn_pairformer_test.rs"]
mod burn_pairformer_test;

#[cfg(test)]
#[path = "tests/pairformer_test.rs"]
mod pairformer_test;

#[cfg(test)]
#[path = "tests/hot_test.rs"]
mod hot_test;

#[cfg(test)]
#[path = "tests/situation_search_test.rs"]
mod situation_search_test;

#[cfg(test)]
#[path = "tests/standing_pass_test.rs"]
mod standing_pass_test;

#[cfg(test)]
#[path = "tests/fitness_test.rs"]
mod fitness_test;

#[cfg(test)]
#[path = "tests/training_substrate_test.rs"]
mod training_substrate_test;

#[cfg(test)]
#[path = "tests/training_runner_test.rs"]
mod training_runner_test;
