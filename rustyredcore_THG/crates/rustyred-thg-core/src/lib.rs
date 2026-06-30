//! THG-Core: Theorem HotGraph command runtime.
//!
//! This crate has no Django, Python, or network-server dependencies. Both
//! PyO3 in-process bindings and the standalone HTTP server call this same
//! executor.

pub mod access_method;
pub mod adaptive_index;
pub mod cold_fragments;
pub mod cold_index;
pub mod commands;
pub mod context_view;
pub mod crdt;
pub mod doc_tree;
pub mod epistemic;
pub mod errors;
pub mod executor;
pub mod feature_dsl;
pub mod fulltext;
#[cfg(feature = "tantivy")]
pub mod fulltext_tantivy;
pub mod graph;
pub mod graph_csr;
pub mod graph_store;
/// D2-D6: GraphBLAS sparse-matrix compute (typed adjacency, semiring traversal,
/// LAGraph algorithms, CFL-reachability). Requires the `graphblas` feature.
#[cfg(feature = "graphblas")]
pub mod graphblas_adjacency;
/// D5: CFL-reachability dataflow (points-to / taint) over the typed adjacency.
#[cfg(feature = "graphblas")]
pub mod graphblas_cfl;
/// D6: graph-analytic + reachability operators as first-class hybrid plan nodes.
#[cfg(feature = "graphblas")]
pub mod graphblas_plan;
pub mod hooks;
pub mod identity_index;
pub mod index_advisor;
pub mod index_manifest;
pub mod index_proposal;
pub mod index_registry;
pub mod instant_kg;
pub mod labeled_training_run;
pub mod map_artifact;
pub mod merge_registry;
pub mod object_store;
pub mod ordered;
pub mod planner;
pub mod plugin;
pub mod ppr_cache;
pub mod query_receipt;
pub mod ranking;
pub mod read_model_index;
pub mod relational;
pub mod saturation;
pub mod spatial;
#[cfg(feature = "s2")]
pub mod spatial_s2;
pub mod state;
pub mod statement;
pub mod store;
pub mod stream;
pub mod symbolic;
pub mod training_export;
pub mod vector_eval;
pub mod versioned_graph;
pub mod working_log;
pub mod zerocopy;

pub use access_method::{
    AccessMethod, AccessMethodRegistry, AccessMethodStats, AmResult, ColumnId, Cost,
    ModalityResolver, NoModalityResolver, OrderedAccessMethod, Predicate, PredicateMode,
    RankOutcome, RankedRow, RankingAccessMethod, RankingRegistry, RegionRef, RelationId, RowChange,
    RowChangeKind, RowId, RowIdStream, ScalarBound, ScalarValue, TimeSeriesAccessMethod,
};
pub use adaptive_index::{
    reconstruct_temporal_context, scalar_i64, ColdFragmentSkipMetadata, ColdSkipIndexDefinition,
    GraphStructuralIndexDefinition, GraphStructuralIndexKind, GraphStructuralIndexReceipt,
    SpatialIndexBackend, SpatialIndexDefinition, TemporalContextSlice, TemporalIndexDefinition,
};
pub use cold_fragments::{
    ColdFragment, ColdFragmentStore, CompressionFilter, FragmentColumn, FragmentRangeResult,
    FragmentRangeStats, PromotionOutcome, PromotionPolicy, ZoneMap,
};
pub use cold_index::{
    ColdIndex, ColdIndexEntry, ColdScopeEntry, ColdTierKind, DiskColdIndex, InMemoryColdIndex,
    OrderedColdIndex,
};
pub use commands::{ThgCommand, ThgRequest, ThgResponse};
pub use context_view::{ContextView, ContextViewType, FreshnessStatus, HydrationHandle};
pub use crdt::{
    diff_since, join_delta, ActorId, Hlc, HlcClock, JoinReport, StampedBatch, StampedMutation,
    VersionVector,
};
pub use doc_tree::{
    DocEntry, DocTree, PathKey, DEFAULT_INLINE_THRESHOLD, DOC_TREE_CONTENT_HASH_PROPERTY,
    DOC_TREE_PATH_PROPERTY,
};
pub use epistemic::{
    compile_user_subgraph, epistemic_egraph_dedup, epistemic_shadow_edge_id,
    epistemic_shadow_node_id, epistemic_shadow_ppr, has_epistemic_shadow_edge_id,
    read_epistemic_shadow, read_same_eclass, run_epistemic_cron_pass, same_eclass_edge_id,
    select_nli_pairs, structural_epistemic_pass, ConnectionFeatures, ConnectionScore,
    ConnectionScorer, EpistemicAnnotation, EpistemicAnnotations, EpistemicCandidatePair,
    EpistemicChokepoint, EpistemicCongruence, EpistemicCronInput, EpistemicCronReport,
    EpistemicDedupConfig, EpistemicDedupReport, EpistemicEnricher, EpistemicEnrichmentError,
    EpistemicEnrichmentMode, EpistemicEquivalenceClass, EpistemicFieldProvenance, EpistemicReadout,
    EpistemicRelationInput, EpistemicRelationKind, EpistemicRelationReadout,
    EpistemicShadowReadout, EpistemicSourceKind, GroundedExtensionStatus, LearnedConnectionScorer,
    LearnedConnectionScorerConfig, LearnedConnectionScorerPair, LearnedConnectionScorerRequest,
    LearnedConnectionScorerResponse, NliClassifier, NliEpistemicEnricher, NliPairInput,
    NliPairSelectionConfig, NliVerdict, PredictedEdgePointer, SameEClassRef, SourceReliability,
    StructuralEpistemicConfig, StructuralEpistemicInput, UserSubgraph,
    DEFAULT_CONNECTION_CALIBRATION_VERSION, DEFAULT_CONNECTION_FEATURE_VERSION,
    DEFAULT_CONNECTION_SCORER_MODEL_ID, DEFAULT_EPISTEMIC_ENGINE_VERSION, DEFAULT_NLI_MODEL_ID,
    EGRAPH_EPISTEMIC_ENGINE, EPISTEMIC_DETERMINISTIC_FALLBACK_ENV,
    EPISTEMIC_SCORER_CALIBRATION_ENV, EPISTEMIC_SCORER_ENDPOINT_ENV, EPISTEMIC_SCORER_MODEL_ENV,
    EPISTEMIC_SHADOW_LABEL, EPISTEMIC_SUPPORTS, HAS_EPISTEMIC_SHADOW, LEARNED_EPISTEMIC_ENGINE,
    NLI_EPISTEMIC_ENGINE, SAME_ECLASS, STRUCTURAL_EPISTEMIC_ENGINE, UNDERCUTS,
};
pub use errors::{ThgError, ThgResult};
pub use executor::{execute_request_json, InMemoryThgExecutor, ThgExecutor};
pub use feature_dsl::{
    eval_feature, feature_score, malicious_probe_expressions, ArithOp, CompareOp, DynamicFeature,
    DynamicFeatureStatus, EvalResult, EvalSentinel, EvalValue, FeatureEvalBudget,
    FeatureEvalContext, FeatureExpr, TraversalTarget, TraverseKind, DEFAULT_MAX_AST_DEPTH,
    DEFAULT_MAX_STEPS, DEFAULT_MAX_TRAVERSAL_DEPTH, DEFAULT_MAX_TRAVERSAL_NODES,
};
pub use fulltext::{
    make_fulltext_backend, make_fulltext_backend_from_value, FieldedFullTextDefinition,
    FieldedFullTextDocument, FieldedFullTextHit, FieldedFullTextIndex, FullTextBackend,
    FullTextBackendError, FullTextDesignation, FullTextIndex, FullTextSearchBackend,
    FullTextSnippet, RUSTY_RED_FULLTEXT_BACKEND_ENV,
};
#[allow(deprecated)]
pub use graph::louvain_communities;
pub use graph::{
    connected_components, expand_bounded, expand_bounded_weighted, label_propagation_communities,
    pagerank, paths_shortest, paths_shortest_weighted, personalized_pagerank, EdgeTuple,
};
pub use graph_csr::CsrGraph;
pub use graph_store::{
    default_hybrid_edge_type_weights, default_vector_index_bit_width, edge_time_interval,
    manifest_version_compatible, node_is_expired, node_ttl_expires_at_ms, now_ms, read_manifest,
    sanitize_tenant_segment, unix_ms, Direction, EdgeRecord, EpistemicType, GraphMutation,
    GraphMutationBatch, GraphRebuildReport, GraphSnapshot, GraphStats, GraphStore, GraphStoreError,
    GraphStoreResult, GraphTransaction, GraphWriteResult, HybridScoringConfig, InMemoryGraphStore,
    MemoryDocumentQuery, NeighborHit, NeighborQuery, NodeQuery, NodeRecord, Provenance,
    RedCoreDurability, RedCoreGraphStore, RedCoreManifest, RedCoreOptions, RedCoreStatus,
    TimeInterval, VectorDesignation, VectorIndex, VectorIndexManifest, VectorPoint, VerifyProblem,
    VerifyReport, CURRENT_FORMAT_VERSION, DEFAULT_VECTOR_INDEX_BIT_WIDTH, TTL_PROPERTY,
};
#[cfg(feature = "redis-store")]
pub use graph_store::{RedisGraphKeyspace, RedisGraphStore};
pub use hooks::{
    coalesce_per_id, substrate_sync_hook, CoalesceKeyFn, HookContext, HookDispatcher,
    HookDispatcherConfig, HookDispatcherStats, HookEmitter, HookError, HookHandler, HookOutcome,
    HookRegistration, HookStoreAccess, MutationEvent, MutationKind, MutationMatcher,
    SubstrateSyncEvent, SubstrateSyncOutbox,
};
pub use identity_index::{
    IdentityIndex, IdentityIndexDefinition, IdentityIndexKey, IdentityInsertOutcome,
    IdentityProblemRecord, IdentityTarget,
};
pub use index_advisor::{
    IndexAdvisor, IndexAdvisorConfig, IndexPainKind, IndexPainSignal, ReceiptCluster,
    ShadowValidationReport,
};
pub use index_manifest::{
    IndexBackend, IndexBuildStatus, IndexCreatedBy, IndexKind, IndexManifest, IndexScope,
};
pub use index_proposal::{IndexProposal, IndexProposalStatus, IndexRiskLevel, PromotionThreshold};
pub use index_registry::IndexRegistry;
pub use instant_kg::{
    instant_kg_payload_delta, instant_kg_payload_manifest, instant_kg_status_payload,
    CodeKgEncodedFile, CodeKgManifest, EdgeExplanation, HarnessInstantKg, ImpactResult,
    InstantKgStatus, PprResult, SearchResult, SessionDelta, INSTANT_KG_DEFAULT_ENCODER_VERSION,
    INSTANT_KG_DEFAULT_INGEST_VERSION, INSTANT_KG_PROTOCOL_VERSION,
};
pub use labeled_training_run::{
    LabeledTrainingRun, TrainingExportStatus, TrainingLabel, TrainingLabelFamily, TrainingOutcome,
    TrainingTaskType, ValidatorResult,
};
pub use map_artifact::{MapArtifact, MapArtifactDiff, MapArtifactType, MapSection};
pub use merge_registry::{
    MergeRegistry, MergeRegistryEntry, MergeRegistryResolution, MergeRegistryStrategy,
};
pub use object_store::{ColdObjectStore, DiskObjectStore, InMemoryObjectStore};
pub use ordered::{
    EvictionFrontier, OrderedDesignation, OrderedIndex, OrderedIndexRegistry, OrderedMember,
    OrderedMode, OrderedScore, ScopedOrderedEntry, ScopedOrderedIndex, ScopedOrderedIndexManifest,
    ScopedOrderedIndexRegistry,
};
pub use planner::{
    compile_graphql_selection, execute_query, execute_query_with_resolver, AccessPathTrace,
    FusionPolicy, GraphqlJoinSelection, GraphqlSelection, JoinPredicate, PlanTrace, Projection,
    QueryIr, QueryOutputRow, QueryRelation, QueryResult, RankerTrace,
};
pub use plugin::{
    normalize_plugin_command, PluginCapability, PluginCapabilityKind, PluginExecutionOutput,
    PluginOperationContext, PluginOperationHandler, PluginOperationRegistration, PluginRegistry,
    RustyRedPlugin,
};
pub use ppr_cache::{
    cached_personalized_pagerank, cached_single_seed_personalized_pagerank, clear_scoped_ppr_cache,
    merge_ppr_scores, scoped_ppr_cache_len,
};
pub use query_receipt::{
    AccessPathReceipt, QueryExplain, QueryKind, QueryOutcomeLabel, QueryReceipt, ReceiptScope,
};
pub use ranking::{
    apply_cascade, apply_feature_scores, compute_term_match, CascadeOutcome, EpistemicGate,
    ExpandRankingMethod, FeatureRankingConfig, QueryContext, RankCandidate, RankedCandidate,
    RankingRule, TermMatch, TextRankingMethod, TypoConfig, VectorRankingMethod,
};
pub use read_model_index::{
    CompositeIndex, CompositeIndexDefinition, CompositeIndexEntry, CompositeIndexKey,
    CoveringIndex, CoveringIndexDefinition, CoveringRow, PartialIndex, PartialIndexDefinition,
    PartialIndexEntry, PartialPredicateClause, PartialPredicateOp,
};
pub use relational::{
    ColumnSchema, NativeAuthPrincipalRecord, NativeBillingAccountRecord, NativeCatalog,
    NativeProjectRecord, NativeTenantRecord, Relation, RelationSchema, RelationalRow,
    RelationalStore,
};
pub use saturation::{
    coalesce_by_subgraph, differential_check, facts_from_payload, facts_from_subgraph,
    materialize_closure, run_saturation, saturation_handler, saturation_hook_registration,
    validate_egglog_program, DifferentialReport, SaturationBackend, SaturationClosure,
    SaturationConfig, SaturationContributor, SaturationDerivedStatement,
    SaturationEquivalenceClass, SaturationFacts, SaturationPlugin, SaturationProgram,
    SaturationReport, SaturationRevisionReport, SATURATION_DERIVED_STATEMENT_LABEL,
    SATURATION_DERIVES_EDGE, SATURATION_ENGINE, SATURATION_ENGINE_VERSION,
    SATURATION_SHARED_RULE_IDS,
};
pub use spatial::{
    make_spatial_backend, make_spatial_backend_from_value, SpatialBackend, SpatialDesignation,
    SpatialError, SpatialIndex, RUSTY_RED_SPATIAL_BACKEND_ENV,
};
pub use state::{stable_hash, ThgEdge, ThgNode, ThgState};
pub use statement::{
    canonical_entity_id, collapse_if_corroborated, flatten_statements, literal_ref,
    migrate_epistemic_shadows_to_statements, predicate_id, predicate_incidence_edge,
    predicate_node, promote_statement_predicate, propose_same_as, statement_id,
    statement_incidence_edge_id, statement_incidence_edges, write_statement, Confidence,
    EpistemicShadowStatementMigrationReport, FlatObject, FlatTriple, StatementFieldProvenance,
    StatementProvenance, StatementQuery, StatementRecord, StatementSemiring, StatementWriteReceipt,
    CANONICAL_ENTITY_LABEL, HAS_OBJECT, HAS_PREDICATE, HAS_SUBJECT, PREDICATE_LABEL, SAME_AS,
    STATEMENT_LABEL,
};
pub use store::{InMemoryThgStore, ThgStore};
pub use stream::{
    StreamDelta, StreamError, StreamEvent, StreamKey, StreamLog, StreamRegistry, StreamStore,
    StreamUrgency, DEFAULT_READ_LIMIT as STREAM_READ_LIMIT,
};
pub use symbolic::{
    derive_datalog_receipt, derive_datalog_receipt_from_json, evolution_archive,
    evolution_archive_from_json, probabilistic_expected_value,
    probabilistic_expected_value_from_json, probabilistic_source_reliability,
    probabilistic_source_reliability_from_json, stable_hash_json, stable_hash_value,
    DATALOG_RULE_IDS,
};
pub use training_export::{
    export_records_jsonl, RedactionStatus, TrainingExportKind, TrainingExportRecord,
};
pub use vector_eval::{
    filter_vector_candidates, vector_recall_against_exact, VectorFilterPolicy,
    VectorIndexDefinition, VectorRecallReport, VectorSearchBackend, VectorSearchCandidate,
};
pub use versioned_graph::{
    apply_graph_mutation_batch, build_prolly_tree, build_prolly_tree_from_entries,
    build_prolly_tree_incremental, checkout_graph_version, compile_graph_pack,
    compile_graph_pack_incremental, diff_graph_snapshots, diff_graph_trees,
    edge_from_content_object, edge_to_content_object, graph_version_log, merge_graph_snapshots,
    merge_graph_snapshots_with_registry, node_from_content_object, node_to_content_object,
    object_bytes, prolly_validation_enabled, resolve_auto_confidence_edge,
    snapshot_content_objects, update_graph_ref, update_graph_ref_cas, CommitCost,
    CompiledGraphPack, GraphCheckoutResult, GraphCommit, GraphCompileOptions,
    GraphCompilerCapability, GraphContentObject, GraphDiffEntry, GraphMergeConflict,
    GraphMergeOptions, GraphMergeResolution, GraphMergeResult, GraphMergeSide, GraphMergeStrategy,
    GraphObjectKind, GraphPackManifest, GraphProllyTree, GraphRefConflict, GraphRefUpdate,
    GraphTreeChild, GraphTreeEntry, GraphTreeNode, GraphVersionDiff, GraphVersionLog,
    GraphVersionRef, GraphVersionRepository, IncrementalGraphPack, IncrementalTreeBuild,
    DEFAULT_GRAPH_BRANCH, GRAPH_CHUNK_FORMAT_VERSION, GRAPH_PACK_COMPILER_VERSION,
    VERSIONED_GRAPH_PROTOCOL_VERSION,
};
pub use working_log::{
    RecencyCounter, RecencyState, TemporalFact, WorkingLog, WorkingLogEvent, WorkingLogEventKind,
};
pub use zerocopy::{
    access_archive, access_graph_archive, archive_content_object, archive_content_objects,
    archive_event_log, archive_hash, content_object_archive_bytes, edge_to_archive,
    mutation_to_archive, node_to_archive, replay_event_log, to_archive, tree_node_archive_bytes,
    ArchiveEdge, ArchiveGraphMutation, ArchiveGraphMutationLog, ArchiveNode, GraphArchiveBody,
    GraphArchiveContentObject, GraphArchiveEnvelope, GraphArchiveObjectBytes,
    GraphArchiveTreeChild, GraphArchiveTreeEntry, GraphArchiveTreeNode, MappedArchive,
    ZeroCopyArchiveError, GRAPH_ARCHIVE_FORMAT_VERSION, GRAPH_ARCHIVE_MIME_TYPE,
};
