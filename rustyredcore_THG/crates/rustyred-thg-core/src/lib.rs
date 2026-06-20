//! THG-Core: Theorem HotGraph command runtime.
//!
//! This crate has no Django, Python, or network-server dependencies. Both
//! PyO3 in-process bindings and the standalone HTTP server call this same
//! executor.

pub mod access_method;
pub mod cold_fragments;
pub mod cold_index;
pub mod commands;
pub mod crdt;
pub mod doc_tree;
pub mod epistemic;
pub mod errors;
pub mod executor;
pub mod fulltext;
#[cfg(feature = "tantivy")]
pub mod fulltext_tantivy;
pub mod graph;
pub mod graph_csr;
pub mod graph_store;
pub mod hooks;
pub mod instant_kg;
pub mod object_store;
pub mod ordered;
pub mod planner;
pub mod plugin;
pub mod ppr_cache;
pub mod ranking;
pub mod relational;
pub mod spatial;
#[cfg(feature = "s2")]
pub mod spatial_s2;
pub mod state;
pub mod store;
pub mod stream;
pub mod symbolic;
pub mod versioned_graph;
pub mod working_log;

pub use access_method::{
    AccessMethod, AccessMethodRegistry, AccessMethodStats, AmResult, ColumnId, Cost,
    ModalityResolver, NoModalityResolver, OrderedAccessMethod, Predicate, PredicateMode,
    RankOutcome, RankedRow, RankingAccessMethod, RankingRegistry, RegionRef, RelationId, RowChange,
    RowChangeKind, RowId, RowIdStream, ScalarBound, ScalarValue, TimeSeriesAccessMethod,
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
    structural_epistemic_pass, EpistemicAnnotation, EpistemicAnnotations, EpistemicCandidatePair,
    EpistemicChokepoint, EpistemicCongruence, EpistemicCronInput, EpistemicCronReport,
    EpistemicDedupConfig, EpistemicDedupReport, EpistemicEnricher, EpistemicEnrichmentError,
    EpistemicEnrichmentMode, EpistemicEquivalenceClass, EpistemicFieldProvenance, EpistemicReadout,
    EpistemicRelationInput, EpistemicRelationKind, EpistemicRelationReadout,
    EpistemicShadowReadout, EpistemicSourceKind, GroundedExtensionStatus, PredictedEdgePointer,
    SameEClassRef, SourceReliability, StructuralEpistemicConfig, StructuralEpistemicInput,
    UserSubgraph, DEFAULT_EPISTEMIC_ENGINE_VERSION, EGRAPH_EPISTEMIC_ENGINE,
    EPISTEMIC_SHADOW_LABEL, EPISTEMIC_SUPPORTS, HAS_EPISTEMIC_SHADOW, LEARNED_EPISTEMIC_ENGINE,
    SAME_ECLASS, STRUCTURAL_EPISTEMIC_ENGINE, UNDERCUTS,
};
pub use errors::{ThgError, ThgResult};
pub use executor::{execute_request_json, InMemoryThgExecutor, ThgExecutor};
pub use fulltext::{
    make_fulltext_backend, make_fulltext_backend_from_value, FullTextBackend, FullTextBackendError,
    FullTextDesignation, FullTextIndex, RUSTY_RED_FULLTEXT_BACKEND_ENV,
};
#[allow(deprecated)]
pub use graph::louvain_communities;
pub use graph::{
    connected_components, expand_bounded, expand_bounded_weighted, label_propagation_communities,
    pagerank, paths_shortest, paths_shortest_weighted, personalized_pagerank, EdgeTuple,
};
pub use graph_csr::CsrGraph;
pub use graph_store::{
    default_hybrid_edge_type_weights, edge_time_interval, manifest_version_compatible,
    node_is_expired, node_ttl_expires_at_ms, now_ms, read_manifest, sanitize_tenant_segment,
    unix_ms, Direction, EdgeRecord, EpistemicType, GraphMutation, GraphMutationBatch,
    GraphRebuildReport, GraphSnapshot, GraphStats, GraphStore, GraphStoreError, GraphStoreResult,
    GraphTransaction, GraphWriteResult, HybridScoringConfig, InMemoryGraphStore, NeighborHit,
    NeighborQuery, NodeQuery, NodeRecord, Provenance, RedCoreDurability, RedCoreGraphStore,
    RedCoreManifest, RedCoreOptions, RedCoreStatus, TimeInterval, VectorDesignation, VectorIndex,
    VectorPoint, VerifyProblem, VerifyReport, CURRENT_FORMAT_VERSION, TTL_PROPERTY,
};
#[cfg(feature = "redis-store")]
pub use graph_store::{RedisGraphKeyspace, RedisGraphStore};
pub use hooks::{
    coalesce_per_id, CoalesceKeyFn, HookContext, HookDispatcher, HookDispatcherConfig,
    HookDispatcherStats, HookEmitter, HookError, HookHandler, HookOutcome, HookRegistration,
    HookStoreAccess, MutationEvent, MutationKind, MutationMatcher,
};
pub use instant_kg::{
    instant_kg_payload_delta, instant_kg_payload_manifest, instant_kg_status_payload,
    CodeKgEncodedFile, CodeKgManifest, EdgeExplanation, HarnessInstantKg, ImpactResult,
    InstantKgStatus, PprResult, SearchResult, SessionDelta, INSTANT_KG_DEFAULT_ENCODER_VERSION,
    INSTANT_KG_DEFAULT_INGEST_VERSION, INSTANT_KG_PROTOCOL_VERSION,
};
pub use object_store::{ColdObjectStore, DiskObjectStore, InMemoryObjectStore};
pub use ordered::{
    EvictionFrontier, OrderedDesignation, OrderedIndex, OrderedIndexRegistry, OrderedMember,
    OrderedMode, OrderedScore,
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
pub use ranking::{ExpandRankingMethod, TextRankingMethod, VectorRankingMethod};
pub use relational::{
    ColumnSchema, NativeAuthPrincipalRecord, NativeBillingAccountRecord, NativeCatalog,
    NativeProjectRecord, NativeTenantRecord, Relation, RelationSchema, RelationalRow,
    RelationalStore,
};
pub use spatial::{
    make_spatial_backend, make_spatial_backend_from_value, SpatialBackend, SpatialDesignation,
    SpatialError, SpatialIndex, RUSTY_RED_SPATIAL_BACKEND_ENV,
};
pub use state::{stable_hash, ThgEdge, ThgNode, ThgState};
pub use store::{InMemoryThgStore, ThgStore};
pub use stream::{StreamDelta, StreamEvent, StreamKey, StreamLog, StreamRegistry, StreamUrgency};
pub use symbolic::{
    derive_datalog_receipt, derive_datalog_receipt_from_json, evolution_archive,
    evolution_archive_from_json, probabilistic_expected_value,
    probabilistic_expected_value_from_json, probabilistic_source_reliability,
    probabilistic_source_reliability_from_json, stable_hash_json, stable_hash_value,
    DATALOG_RULE_IDS,
};
pub use versioned_graph::{
    apply_graph_mutation_batch, build_prolly_tree, build_prolly_tree_from_entries,
    build_prolly_tree_incremental, checkout_graph_version, compile_graph_pack,
    compile_graph_pack_incremental, diff_graph_snapshots, diff_graph_trees,
    edge_from_content_object, edge_to_content_object, graph_version_log, merge_graph_snapshots,
    node_from_content_object, node_to_content_object, prolly_validation_enabled,
    resolve_auto_confidence_edge, snapshot_content_objects, update_graph_ref, update_graph_ref_cas,
    CommitCost, CompiledGraphPack, GraphCheckoutResult, GraphCommit, GraphCompileOptions,
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
