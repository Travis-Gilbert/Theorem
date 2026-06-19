//! # theorem-harness-runtime
//!
//! GraphStore-backed persistence for the harness. It writes the kernel's
//! (`theorem-harness-core`) transition receipts into a graph store as run and
//! event nodes joined by an append-chain, keeping storage out of the
//! parity-tested kernel, and carries the live coordination and dispatch
//! primitives the MCP and HTTP transports expose.
//!
//! Key modules:
//! - [`event_log`]: persist a run as run/event nodes plus append-chain edges;
//!   the read side ([`load_run`], [`load_events`]) backs the run read contract.
//! - [`coordination`] and [`coordination_push`]: rooms, intents, presence,
//!   messages, durable records, and @mentions -- the shared-awareness layer.
//! - [`job_queue`]: the Dispatch v2 job board (submit/list/note/archive).
//! - [`memory`]: durable memory documents (remember/recall/encode) and the
//!   Obsidian-sync read and upsert surface.
//! - [`binding_store`], [`skill_pack`], [`work_graph_store`]: AgentBinding
//!   scratchpad persistence, skill packs, and the multi-head work graph store.
//! - [`agent_runner`]: the in-process room runner that turns wake messages into
//!   head invocations.
//!
//! Product-level picture: see `docs/site/concepts/the-harness.md`.

pub mod agent_runner;
pub mod binding_store;
pub mod canonical_write;
pub mod composed_agent;
pub mod compound_engineering;
pub mod coordination;
pub mod coordination_push;
pub mod event_log;
pub mod head_invoker;
pub mod job_queue;
pub mod library_encoding;
pub mod memory;
pub mod node_type_binding;
pub mod overlap;
pub mod patch_sequencer;
pub mod skill_pack;
pub mod tenant;
pub mod work_graph_store;
pub mod writing_style;

pub use agent_runner::{
    run_agent_room_cycle, AgentRoomRunnerConfig, AgentRoomRunnerCycle, AgentRoomRunnerError,
    AgentRoomRunnerTurn, AgentRoomRunnerTurnStatus, DEFAULT_AGENT_ACTOR, DEFAULT_AGENT_SURFACE,
    DEFAULT_HEARTBEAT_TTL_SECONDS, DEFAULT_MENTION_LIMIT,
};
pub use binding_store::{
    append_binding_transition, binding_event_node_id, binding_node_id, load_binding,
    load_binding_events, load_scratchpad_revisions, persist_binding, persist_binding_event_state,
    persist_binding_run_result, persist_binding_transition_result, scratchpad_revision_node_id,
    BindingRuntimeError, BindingRuntimeResult,
};
pub use canonical_write::{
    alias_witness_node_id, canonical_fact_node_id, canonicalize_on_write,
    embedding_nomination_node_id, AliasWitness, CanonicalWriteError, CanonicalWriteReceipt,
    CanonicalWriteResult, CanonicalizeOnWriteInput, EmbeddingNomination, TypedFact,
    ALIAS_WITNESS_LABEL, CANONICAL_FACT_LABEL, EDGE_ALIAS_WITNESS_FOR, EDGE_EMBEDDING_NOMINATED,
    EMBEDDING_NOMINATION_LABEL,
};
pub use composed_agent::{
    default_theorem_binding, run_composed_agent, run_composed_agent_with_claims,
    ComposedAgentRunResult, ComposedAgentRuntimeError, ComposedAgentRuntimeResult,
    DEFAULT_BINDING_ID,
};
pub use work_graph_store::{
    claim_task_node_durable, load_task_node, load_work_graph, persist_task_node,
    persist_work_graph, refine_task_node_durable, task_node_graph_id, WorkGraphStoreError,
    EDGE_CLAIMED_BY, EDGE_PREREQUISITE_OF, EDGE_REFINED_INTO, TASK_NODE_LABEL,
};

pub use compound_engineering::{
    apply_compound_standing, apply_run_close_hook, compound_config_hash, compound_config_node_id,
    compound_state_node_id, list_compound_captures, load_compound_config, persist_compound_config,
    CompoundConfig, CompoundHookReceipt, CompoundStandingReceipt, CompoundingArtifact,
    OutcomeSignal, RetrievalAttribution, COMPOUND_CAPTURE_TAG, COMPOUND_CONFIG_NODE_LABEL,
    COMPOUND_ROOM_ID, COMPOUND_STATE_NODE_LABEL,
};
pub use coordination::{
    canonical_stream_key, coordination_binding_id, coordination_intent_edge_id,
    coordination_intent_node_id, coordination_intent_scratchpad_edge_id,
    coordination_member_edge_id, coordination_member_node_id, coordination_mention_edge_id,
    coordination_message_edge_id, coordination_message_node_id, coordination_presence_node_id,
    coordination_record_edge_id, coordination_record_node_id, coordination_room_binding_edge_id,
    coordination_room_node_id, coordination_stream_cursor_node_id,
    coordination_stream_event_edge_id, coordination_stream_event_node_id,
    coordination_stream_node_id, coordination_stream_subscription_node_id, end_presence,
    heartbeat_presence, infer_coordination_room_id, join_room, list_presence, load_presence,
    normalize_coordination_urgency, parse_coordination_mentions, read_intents_for_room,
    read_mentions_for_actor, read_mentions_for_actor_in_room,
    read_mentions_for_actor_in_room_with_urgencies, read_mentions_for_actor_with_urgencies,
    read_messages_for_room, read_records_for_room, room_status, stable_coordination_message_id,
    stable_coordination_record_id, write_intent, write_message, write_record, CoordinationError,
    CoordinationIntentState, CoordinationMessageState, CoordinationPresenceState,
    CoordinationRecordState, CoordinationResult, CoordinationRoomMember, CoordinationRoomState,
    JoinRoomInput, PresenceInput, WriteIntentInput, WriteMessageInput, WriteRecordInput,
};
pub use coordination_push::{
    agent_space_event_kind, agent_space_event_matches, agent_space_high_water_seq,
    global_agent_space_bus, global_coordination_room_bus, publish_agent_space_event,
    publish_agent_space_room_message, publish_coordination_room_event_from_state,
    publish_crdt_delta, publish_footprint_event, publish_presence_event, publish_record_event,
    publish_work_graph_transition, stream_event_matches, subscribe_agent_space_events,
    subscribe_coordination_room_events, wake_targets, AddOrRemove, AgentSpaceEnvelope,
    AgentSpaceEvent, AgentSpaceEventBus, CausalMeta, CrdtDelta, DeltaOp, RoomEventBus,
    RoomMessageDelivery, RoomMessageEvent, DEFAULT_ROOM_BUS_CAPACITY,
};
pub use design_check::{
    contrast_ratio as design_contrast_ratio, css_static_report, design_engineering_pack_payload,
    design_rules, fixture_reports as design_fixture_reports, lower_css as lower_design_css,
    lower_tokens_json as lower_design_tokens_json, pack_hash as design_pack_hash,
    parse_hex_color as parse_design_hex_color, relative_luminance as design_relative_luminance,
    token_lint_report, CheckerFinding as DesignCheckerFinding, CssStaticInput, DesignAtom,
    DesignCheckReport, DesignRule,
};
pub use event_log::{
    append_transition, append_transition_from_store, event_node_id, load_events, load_run,
    persist_transition_result, replay_persisted_run, run_node_id, HarnessRuntimeError,
    RuntimeResult,
};
pub use job_queue::{
    job_archive, job_list, job_node_id, job_note, job_submit, load_job, JobActionResult,
    JobNoteInput, JobSubmitOutcome, EDGE_DISPATCHED_AS, EDGE_JOB_FOR_SPEC, JOB_LABEL,
};
pub use library_encoding::{
    library_encoding_pack_payload, library_encoding_plan, library_encoding_plan_hash,
    library_encoding_plan_value, library_pack_by_source, library_source_is_infrastructure,
    LibraryEncodingPlan, LibraryKeystone, LibraryPackSpec, RetiredProcessPlugin,
};
pub use memory::{
    archive_memory_document, create_memory_document, create_memory_node, encode_memory,
    forget_memory, handoff_memory, list_memory_documents_since, load_memory_document,
    load_memory_node, memory_content_hash, memory_document_node_id, memory_edge_id,
    memory_node_node_id, recall_archived_memory, recall_memory, relate_memory, remember_memory,
    revise_memory_document, self_note_memory, upsert_note, ArchiveMemoryInput,
    ArchiveMemoryReceipt, EncodeMemoryInput, ForgetMemoryInput, ForgetMemoryReceipt,
    HandoffMemoryInput, MemoryDocumentState, MemoryError, MemoryGraphStore, MemoryNodeState,
    MemoryRecallItem, MemoryRelationItem, MemoryResult, MemoryWriteInput, RecallMemoryInput,
    RelateMemoryInput, RememberMemoryReceipt, ReviseMemoryInput, ReviseMemoryReceipt,
    UpsertNoteInput, UpsertNoteReceipt,
};
pub use node_type_binding::{
    bind_node_type_skill_packs, load_node_type_skill_pack_binding, node_type_binding_node_id,
    node_type_skill_pack_edge_id, resolve_node_type_skill_packs, resolve_task_node_skill_packs,
    BindNodeTypeSkillPacksInput, NodeTypeBindingError, NodeTypeBindingResult,
    NodeTypeSkillPackBindingReceipt, NodeTypeSkillPackBindingState, NodeTypeSkillPackRef,
    NodeTypeSkillPackResolution, ResolveNodeTypeSkillPacksInput, ResolvedNodeTypeSkillPack,
    EDGE_NODE_TYPE_USES_SKILL_PACK, NODE_TYPE_BINDING_LABEL,
};
pub use overlap::{
    detect_and_emit_overlap_tensions, detect_overlaps, emit_overlap_tension, neighborhood_of,
    Footprint, Neighborhood, Overlap,
};
pub use patch_sequencer::{
    PatchApplyReceipt, PatchApplyStatus, PatchProposal, PatchSequencer, PatchSequencerError,
    PatchSequencerResult,
};
pub use tenant::{
    normalize_actor_id, normalize_tenant_slug, tenant_slug_aliases, DEFAULT_TENANT_SLUG,
};
pub mod provider_invoker;
pub use head_invoker::{
    api_provider_profile, default_api_profiles, ApiProviderProfile, ApiRequestShape,
    CredentialResolutionError, CredentialResolver, EndpointMap, ProviderHeadInvoker,
    RealHeadInvoker,
};
pub use skill_pack::{
    apply_skill_pack, get_skill_pack, list_skill_packs, publish_skill_pack,
    skill_pack_artifact_node_id, skill_pack_node_id, skill_pack_source_node_id,
    skill_pack_use_receipt_node_id, SkillPackApplyInput, SkillPackApplyReceipt, SkillPackError,
    SkillPackGetInput, SkillPackGraphStore, SkillPackListInput, SkillPackPublishInput,
    SkillPackPublishReceipt, SkillPackState, SkillPackValidatorReceipt,
};
pub use writing_style::{
    check_boundary_text, enrich_binding_transition, enrich_run_transition,
    metadata_with_style_receipt, prepare_run_transition, register_for_boundary,
    summarize_style_receipts_for_fitness, BoundaryStyleReceipt, WritingStyleFitnessSummary,
    STYLE_FITNESS_FIELD, STYLE_RECEIPTS_FIELD, WRITING_ENGINEERING_STATUS_FIELD,
};
