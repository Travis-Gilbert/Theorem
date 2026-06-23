//! GraphQL surface for the Theorem Harness MCP server.
//!
//! A single typed schema collapses flat tool sprawl: the Memory domain (recall +
//! relate + links in one nested query) and the Graph domain (the `graphAlgorithm`
//! field folding eight algorithm tools into one enum field, plus `graphNode`,
//! `neighbors`, `graphSchema`, `vectorSearch`, `vectorHybrid`, `fulltextSearch`,
//! `spatialRadius`, `spatialBbox`, the symbolic `deriveFacts` / `sourceReliability`
//! / `expectedValue` reads, and the `designate*` / `bulk*` mutations), plus the
//! Coordination domain (room context, streams, and native work-graph wrappers).
//! Three transport tools carry GraphQL: `graphql_query` (read), `graphql_mutate`
//! (write), `graphql_introspect` (SDL).
//!
//! Resolvers WRAP the existing handlers (they call the same crate-private
//! `*_payload` fns the flat tools call) -- no memory logic is reimplemented.
//!
//! Integration shape: the MCP dispatch is synchronous and the backend
//! (`Rc<RefCell<..>>` in the fixture) is neither `Send` nor `Sync`, so it cannot
//! live in async-graphql's `'static` `Data`. Instead the GraphQL document is
//! executed synchronously (`futures_executor::block_on`) on the dispatch thread,
//! and resolvers reach the backend through a scoped, thread-local invoker that is
//! only installed for the duration of that synchronous execution.
//!
//! Tenant scoping: the tenant is resolved ONCE (the connection tenant, already
//! resolved by `call_tool` before these arms) and carried on the invoker; no
//! GraphQL field accepts a tenant argument, so a field cannot mis-scope it. An
//! empty tenant is rejected rather than defaulted.
//!
//! ## Coverage map (the boundary of the typed surface)
//!
//! "Finished" has an edge: the typed surface covers the domains agents reach for
//! most, and everything else is flat-only *by decision*, not by omission. Every
//! flat-tool cluster is accounted for below. The authoritative covered-tool list
//! (the names hidden from `tools/list` in GraphQL-default mode) is
//! `crate::GRAPHQL_COVERED_FLAT_TOOLS`; this note is its human-readable rationale.
//!
//! Covered -> typed GraphQL field(s):
//! - memory: `memory` / `memoryDoc` / `memoryArchive` (+ nested `links` /
//!   `related`); mutations `rememberMemory` / `reviseMemory` / `forgetMemory` /
//!   `createHandoff`. Wraps recall/relate/remember/encode/self_revise/forget/
//!   handoff/self_recall_archive.
//! - graph: `graphAlgorithm` (folds the eight pagerank/ppr/components/communities
//!   flat tools, inline and stored), plus `graphNode` / `neighbors` /
//!   `graphSchema` / `vectorSearch` / `vectorHybrid` / `fulltextSearch` /
//!   `spatialRadius` / `spatialBbox` / the symbolic reads; mutations
//!   `designate*` / `bulk*`.
//! - coordination (core): `coordinationRoom` / `coordinationStream` / `workGraph`
//!   / `nextTaskNode`; mutations `writeCoordinationIntent` /
//!   `writeCoordinationRecord` / `publishCoordinationEvent` /
//!   `advanceCoordinationStream` / `createTaskNode` / `claimTaskNode`. This is the
//!   room-context, record, stream, and work-graph-status + task-claim path.
//! - epistemic: `epistemicNeighbors` / `epistemicFrontier` / `compileSubgraph` /
//!   `shadowPpr`; mutation `enrichApply`.
//! - code (CodeCrawler): `codeSearch` / `codeContext` / `codeExplore` /
//!   `codeExplain` / `codeRecognize` / `listRepos`; mutations `ingestCodebase` /
//!   `reindexCodebase`.
//! - instant-KG: `harnessKgStatus` / `harnessKgSearch` / `harnessKgPpr` /
//!   `harnessKgImpact` / `harnessKgRelatedObjects` / `harnessKgExplainEdge`.
//! - skills: `skillList` / `skillGet`; mutations `skillPublish` / `skillApply`.
//! - ensemble: `ensembleSelect`; mutation `ensembleRegister`.
//! - jobs: `jobList`; mutations `jobSubmit` / `jobNote` / `jobArchive`.
//! - harness-run: `harnessRun` (run + event ledger).
//!
//! Flat-only by decision (NOT wrapped in this phase):
//! - multihead lifecycle: `multihead_refine` / `_patch` / `_proof` / `_review` /
//!   `_spawn_verify` / `_submit_verify`. The work-graph *status* and the
//!   *task-claim* path are covered above; the lifecycle-heavy verbs stay flat
//!   until a later phase.
//! - short-TTL presence + attention: `presence` / `subscribe` / `mentions` /
//!   `read_*_for_room`. Their freshness semantics fit the flat transport better
//!   than a typed read.
//! - server-only (no in-crate payload; the flat arm punts to the async product
//!   server): `web_consume` / `web_search_graph` / `browse_with_me` /
//!   `browse_for_me` / `fractal_expansion` / `hippo_retrieve` /
//!   `rustyweb_search_acquisition` / `composed_agent_run` / `harness_prepare`.
//!   Wrapping these belongs to a future server-domain slice.
//! - low-level / utility transport: `invoke` / `tool_search` / `tool_result_fetch`
//!   / `upsert_note` / `describe` / `observe` / `spawn_session` / `self_note` /
//!   `self_archive` / `harness_append_transition`; the version-graph
//!   (`rustyred_thg_graph_version_*`), relational (`rustyred_thg_relational_query`),
//!   and raw graph-query/explain/index-status tools. These are either escape
//!   hatches or are already low-level enough that a typed wrapper adds no value.

mod clusters;
mod code;
mod coordination;
mod coordination_v2;
mod epistemic;
mod graph;
mod items;
mod kg;
mod memory;
pub mod projection;
mod scalars;

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;

use async_graphql::{EmptyMutation, EmptySubscription, MergedObject, Request, Schema, Variables};
use rustyred_thg_core::{NodeQuery, NodeRecord};
use serde_json::{json, Value};

use crate::{McpError, McpGraphBackend};

// ---------------------------------------------------------------------------
// The scoped invoker: how resolvers reach the live backend.
// ---------------------------------------------------------------------------

/// The capabilities the GraphQL resolvers need, each wrapping an existing
/// `*_payload` handler. The tenant is fixed by the invoker, never a field arg.
pub(crate) trait GraphqlInvoker {
    fn recall(&self, args: Value) -> Result<Value, McpError>;
    fn relate(&self, args: Value) -> Result<Value, McpError>;
    fn get_doc(&self, id: &str) -> Result<Option<Value>, McpError>;
    fn archive_recall(&self, args: Value) -> Result<Value, McpError>;
    fn remember(&self, args: Value) -> Result<Value, McpError>;
    fn encode(&self, args: Value) -> Result<Value, McpError>;
    fn revise(&self, args: Value) -> Result<Value, McpError>;
    fn forget(&self, args: Value) -> Result<Value, McpError>;
    fn handoff(&self, args: Value) -> Result<Value, McpError>;
    fn algorithm(&self, kind: &str, inline: bool, args: Value) -> Result<Value, McpError>;
    // Graph domain (A3): each wraps the matching flat-tool payload handler.
    fn neighbors(&self, args: Value) -> Result<Value, McpError>;
    fn graph_schema(&self) -> Result<Value, McpError>;
    fn vector_search(&self, args: Value) -> Result<Value, McpError>;
    fn vector_hybrid(&self, args: Value) -> Result<Value, McpError>;
    fn fulltext_search(&self, args: Value) -> Result<Value, McpError>;
    fn spatial_radius(&self, args: Value) -> Result<Value, McpError>;
    fn spatial_bbox(&self, args: Value) -> Result<Value, McpError>;
    fn derive_facts(&self, args: Value) -> Result<Value, McpError>;
    fn source_reliability(&self, args: Value) -> Result<Value, McpError>;
    fn expected_value(&self, args: Value) -> Result<Value, McpError>;
    fn designate_vector(&self, args: Value) -> Result<Value, McpError>;
    fn designate_spatial(&self, args: Value) -> Result<Value, McpError>;
    fn designate_fulltext(&self, args: Value) -> Result<Value, McpError>;
    fn bulk_nodes(&self, args: Value) -> Result<Value, McpError>;
    fn bulk_edges(&self, args: Value) -> Result<Value, McpError>;
    // Coordination domain (A2): wraps room, stream, and work-graph payloads.
    fn coordination_context(&self, args: Value) -> Result<Value, McpError>;
    fn coordination_intent(&self, args: Value) -> Result<Value, McpError>;
    fn coordination_record(&self, args: Value) -> Result<Value, McpError>;
    fn stream_publish(&self, args: Value) -> Result<Value, McpError>;
    fn stream_read(&self, args: Value, allow_advance: bool) -> Result<Value, McpError>;
    fn work_graph(&self, args: Value) -> Result<Value, McpError>;
    fn task_node(&self, args: Value) -> Result<Value, McpError>;
    fn claim_task_node(&self, args: Value) -> Result<Value, McpError>;
    fn next_task_node(&self, args: Value) -> Result<Value, McpError>;
    // Coordination v2 (Task-Reference Rooms): one dispatch over the runtime
    // engine via the McpCoordinationStore adapter, parameterized by operation.
    fn coordination_v2(&self, operation: &str, args: Value) -> Result<Value, McpError>;
    // Epistemic domain (A4): wraps the shadow-graph payloads.
    fn epistemic_neighbors(&self, args: Value) -> Result<Value, McpError>;
    fn epistemic_dirty_frontier(&self, args: Value) -> Result<Value, McpError>;
    fn epistemic_compile_subgraph(&self, args: Value) -> Result<Value, McpError>;
    fn epistemic_shadow_ppr(&self, args: Value) -> Result<Value, McpError>;
    fn epistemic_enrich_apply(&self, args: Value) -> Result<Value, McpError>;
    // Code domain (A5): wraps the CodeCrawler compute_code payload, parameterized
    // by operation (search / context / explore / explain / ingest / reindex / ...).
    fn code(&self, operation: &str, args: Value) -> Result<Value, McpError>;
    // Harness instant-KG domain (A5, KG half): wraps the consolidated
    // instant_kg_payload, parameterized by operation.
    fn instant_kg(&self, operation: &str, args: Value) -> Result<Value, McpError>;
    // Remaining-cluster domains (A6): harness-run / skills / ensemble / jobs.
    fn harness_run(&self, args: Value) -> Result<Value, McpError>;
    fn skill(&self, operation: &str, args: Value) -> Result<Value, McpError>;
    fn ensemble(&self, operation: &str, args: Value) -> Result<Value, McpError>;
    fn job(&self, operation: &str, args: Value) -> Result<Value, McpError>;
    // Item domain (SPEC-2): enumerate nodes by the projected labels, and fetch a
    // single node by id, for the projection. Wraps query_nodes / get_node.
    fn items_nodes(&self, labels: &[&str], limit: usize) -> Result<Vec<NodeRecord>, McpError>;
    fn item_node(&self, id: &str) -> Result<Option<NodeRecord>, McpError>;
    /// Item domain (SPEC-2 putItem): create-or-update a real Item node by id and
    /// return the written record (for projection). Wraps get_node + upsert_node.
    fn put_item(&self, args: Value) -> Result<NodeRecord, McpError>;
}

thread_local! {
    static INVOKER: Cell<Option<NonNull<dyn GraphqlInvoker>>> = const { Cell::new(None) };
}

/// RAII guard installing a scoped invoker; restores the previous on drop.
struct InvokerGuard(Option<NonNull<dyn GraphqlInvoker>>);

impl Drop for InvokerGuard {
    fn drop(&mut self) {
        INVOKER.with(|slot| slot.set(self.0));
    }
}

fn set_invoker<'a>(inv: &'a dyn GraphqlInvoker) -> InvokerGuard {
    let ptr: NonNull<dyn GraphqlInvoker + 'a> = NonNull::from(inv);
    // SAFETY: erase the borrow lifetime only for thread-local storage. The guard
    // restores the previous pointer on drop, and `with_invoker` dereferences it
    // only while this guard is alive. The GraphQL execution that reads it runs
    // synchronously on THIS thread via `futures_executor::block_on`, so the
    // borrow strictly outlives every dereference.
    let erased: NonNull<dyn GraphqlInvoker + 'static> = unsafe { std::mem::transmute(ptr) };
    let previous = INVOKER.with(|slot| slot.replace(Some(erased)));
    InvokerGuard(previous)
}

pub(crate) fn with_invoker<R>(
    f: impl FnOnce(&dyn GraphqlInvoker) -> async_graphql::Result<R>,
) -> async_graphql::Result<R> {
    INVOKER.with(|slot| match slot.get() {
        // SAFETY: see `set_invoker`; valid for the synchronous execution scope.
        Some(ptr) => f(unsafe { ptr.as_ref() }),
        None => Err(async_graphql::Error::new("graphql invoker is not in scope")),
    })
}

pub(crate) fn map_err(err: McpError) -> async_graphql::Error {
    async_graphql::Error::new(format!("{err:?}"))
}

/// Concrete invoker: owns the connection-tenant backend, dispatches each op to
/// the matching crate-private payload handler.
struct DispatchInvoker<B: McpGraphBackend> {
    backend: RefCell<B>,
    tenant: String,
}

impl<B: McpGraphBackend> DispatchInvoker<B> {
    fn new(backend: B, tenant: String) -> Self {
        Self {
            backend: RefCell::new(backend),
            tenant,
        }
    }
}

impl<B: McpGraphBackend> GraphqlInvoker for DispatchInvoker<B> {
    fn recall(&self, args: Value) -> Result<Value, McpError> {
        crate::recall_memory_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args, false)
    }
    fn relate(&self, args: Value) -> Result<Value, McpError> {
        crate::relate_memory_payload(&self.tenant, &*self.backend.borrow(), &args)
    }
    fn get_doc(&self, id: &str) -> Result<Option<Value>, McpError> {
        let node = self.backend.borrow().get_node(id)?;
        Ok(node.map(|node| serde_json::to_value(node).unwrap_or(Value::Null)))
    }
    fn archive_recall(&self, args: Value) -> Result<Value, McpError> {
        crate::recall_archived_memory_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn remember(&self, args: Value) -> Result<Value, McpError> {
        crate::remember_memory_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn encode(&self, args: Value) -> Result<Value, McpError> {
        crate::encode_memory_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn revise(&self, args: Value) -> Result<Value, McpError> {
        crate::revise_memory_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn forget(&self, args: Value) -> Result<Value, McpError> {
        crate::forget_memory_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn handoff(&self, args: Value) -> Result<Value, McpError> {
        crate::handoff_memory_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn algorithm(&self, kind: &str, inline: bool, args: Value) -> Result<Value, McpError> {
        let tenant = &self.tenant;
        match (kind, inline) {
            ("PPR", false) => crate::algorithm_ppr_payload(tenant, &*self.backend.borrow(), &args),
            ("PPR", true) => crate::algorithm_ppr_inline_payload(&args),
            ("PAGERANK", false) => {
                crate::algorithm_pagerank_payload(tenant, &*self.backend.borrow(), &args)
            }
            ("PAGERANK", true) => crate::algorithm_pagerank_inline_payload(&args),
            ("COMPONENTS", false) => {
                crate::algorithm_components_payload(tenant, &*self.backend.borrow(), &args)
            }
            ("COMPONENTS", true) => crate::algorithm_components_inline_payload(&args),
            ("COMMUNITIES", false) => {
                crate::algorithm_communities_payload(tenant, &*self.backend.borrow())
            }
            ("COMMUNITIES", true) => crate::algorithm_communities_inline_payload(&args),
            _ => Err(McpError::invalid_params("unknown graph algorithm kind")),
        }
    }
    fn neighbors(&self, args: Value) -> Result<Value, McpError> {
        crate::graph_neighbors_payload(&self.tenant, &*self.backend.borrow(), &args)
    }
    fn graph_schema(&self) -> Result<Value, McpError> {
        crate::schema_payload(&self.tenant, &*self.backend.borrow())
    }
    fn vector_search(&self, args: Value) -> Result<Value, McpError> {
        crate::vector_search_payload(&self.tenant, &*self.backend.borrow(), &args)
    }
    fn vector_hybrid(&self, args: Value) -> Result<Value, McpError> {
        crate::vector_hybrid_payload(&self.tenant, &*self.backend.borrow(), &args)
    }
    fn fulltext_search(&self, args: Value) -> Result<Value, McpError> {
        crate::fulltext_search_payload(
            &self.tenant,
            &*self.backend.borrow(),
            &args,
            "rustyred_thg_fulltext_search",
        )
    }
    fn spatial_radius(&self, args: Value) -> Result<Value, McpError> {
        crate::spatial_radius_payload(
            &self.tenant,
            &*self.backend.borrow(),
            &args,
            "rustyred_thg_spatial_radius",
        )
    }
    fn spatial_bbox(&self, args: Value) -> Result<Value, McpError> {
        crate::spatial_bbox_payload(
            &self.tenant,
            &*self.backend.borrow(),
            &args,
            "rustyred_thg_spatial_bbox",
        )
    }
    fn derive_facts(&self, args: Value) -> Result<Value, McpError> {
        crate::symbolic_datalog_derive_payload(&args)
    }
    fn source_reliability(&self, args: Value) -> Result<Value, McpError> {
        crate::symbolic_probabilistic_source_reliability_payload(&args)
    }
    fn expected_value(&self, args: Value) -> Result<Value, McpError> {
        crate::symbolic_probabilistic_expected_value_payload(&args)
    }
    fn designate_vector(&self, args: Value) -> Result<Value, McpError> {
        crate::vector_designate_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn designate_spatial(&self, args: Value) -> Result<Value, McpError> {
        crate::spatial_designate_payload(
            &self.tenant,
            &mut *self.backend.borrow_mut(),
            &args,
            "rustyred_thg_spatial_designate",
        )
    }
    fn designate_fulltext(&self, args: Value) -> Result<Value, McpError> {
        crate::fulltext_designate_payload(
            &self.tenant,
            &mut *self.backend.borrow_mut(),
            &args,
            "rustyred_thg_fulltext_designate",
        )
    }
    fn bulk_nodes(&self, args: Value) -> Result<Value, McpError> {
        crate::bulk_nodes_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn bulk_edges(&self, args: Value) -> Result<Value, McpError> {
        crate::bulk_edges_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn coordination_context(&self, args: Value) -> Result<Value, McpError> {
        crate::coordination_context_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn coordination_intent(&self, args: Value) -> Result<Value, McpError> {
        crate::write_intent_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn coordination_record(&self, args: Value) -> Result<Value, McpError> {
        crate::write_record_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args, None)
    }
    fn stream_publish(&self, args: Value) -> Result<Value, McpError> {
        crate::stream_publish_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn stream_read(&self, args: Value, allow_advance: bool) -> Result<Value, McpError> {
        crate::stream_read_payload(
            &self.tenant,
            &mut *self.backend.borrow_mut(),
            &args,
            allow_advance,
        )
    }
    fn work_graph(&self, mut args: Value) -> Result<Value, McpError> {
        args["action"] = json!("status");
        crate::multihead_run_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn task_node(&self, args: Value) -> Result<Value, McpError> {
        crate::multihead_task_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn claim_task_node(&self, args: Value) -> Result<Value, McpError> {
        crate::multihead_claim_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn next_task_node(&self, args: Value) -> Result<Value, McpError> {
        crate::multihead_next_payload(&self.tenant, &*self.backend.borrow(), &args)
    }
    fn coordination_v2(&self, operation: &str, args: Value) -> Result<Value, McpError> {
        crate::coordination_v2_payload(
            &self.tenant,
            &mut *self.backend.borrow_mut(),
            operation,
            &args,
        )
    }
    fn epistemic_neighbors(&self, args: Value) -> Result<Value, McpError> {
        crate::epistemic_neighbors_payload(&self.tenant, &*self.backend.borrow(), &args)
    }
    fn epistemic_dirty_frontier(&self, args: Value) -> Result<Value, McpError> {
        crate::epistemic_dirty_frontier_payload(&self.tenant, &*self.backend.borrow(), &args)
    }
    fn epistemic_compile_subgraph(&self, args: Value) -> Result<Value, McpError> {
        crate::epistemic_compile_subgraph_payload(&self.tenant, &*self.backend.borrow(), &args)
    }
    fn epistemic_shadow_ppr(&self, args: Value) -> Result<Value, McpError> {
        crate::epistemic_shadow_ppr_payload(&self.tenant, &*self.backend.borrow(), &args)
    }
    fn epistemic_enrich_apply(&self, args: Value) -> Result<Value, McpError> {
        crate::epistemic_enrich_apply_payload(&self.tenant, &mut *self.backend.borrow_mut(), &args)
    }
    fn code(&self, operation: &str, args: Value) -> Result<Value, McpError> {
        crate::code_search_payload(
            &self.tenant,
            &mut *self.backend.borrow_mut(),
            &args,
            operation,
        )
    }
    fn instant_kg(&self, operation: &str, args: Value) -> Result<Value, McpError> {
        crate::instant_kg_payload(
            &self.tenant,
            &*self.backend.borrow(),
            &args,
            operation,
            "harness_kg",
        )
    }
    fn harness_run(&self, args: Value) -> Result<Value, McpError> {
        crate::harness_run_payload(&self.tenant, &*self.backend.borrow(), &args)
    }
    fn skill(&self, operation: &str, args: Value) -> Result<Value, McpError> {
        let mut backend = self.backend.borrow_mut();
        match operation {
            "list" => crate::skill_list_payload(&self.tenant, &*backend, &args),
            "get" => crate::skill_get_payload(&self.tenant, &*backend, &args),
            "publish" => crate::skill_publish_payload(&self.tenant, &mut *backend, &args),
            "apply" => crate::skill_apply_payload(&self.tenant, &mut *backend, &args),
            _ => Err(McpError::invalid_params("unknown skill operation")),
        }
    }
    fn ensemble(&self, operation: &str, args: Value) -> Result<Value, McpError> {
        let mut backend = self.backend.borrow_mut();
        match operation {
            "register" => crate::ensemble_register_payload(&self.tenant, &mut *backend, &args),
            "select" => crate::ensemble_select_payload(&self.tenant, &*backend, &args),
            _ => Err(McpError::invalid_params("unknown ensemble operation")),
        }
    }
    fn job(&self, operation: &str, args: Value) -> Result<Value, McpError> {
        let mut backend = self.backend.borrow_mut();
        match operation {
            "submit" => crate::job_submit_payload(&self.tenant, &mut *backend, &args),
            "list" => crate::job_list_payload(&self.tenant, &*backend, &args),
            "note" => crate::job_note_payload(&self.tenant, &mut *backend, &args),
            "archive" => crate::job_archive_payload(&self.tenant, &mut *backend, &args),
            _ => Err(McpError::invalid_params("unknown job operation")),
        }
    }
    fn items_nodes(&self, labels: &[&str], limit: usize) -> Result<Vec<NodeRecord>, McpError> {
        let backend = self.backend.borrow();
        let mut out = Vec::new();
        for label in labels {
            let mut nodes = backend.query_nodes(NodeQuery::label(*label).with_limit(limit))?;
            out.append(&mut nodes);
        }
        Ok(out)
    }
    fn item_node(&self, id: &str) -> Result<Option<NodeRecord>, McpError> {
        Ok(self.backend.borrow().get_node(id)?)
    }
    fn put_item(&self, args: Value) -> Result<NodeRecord, McpError> {
        let mut backend = self.backend.borrow_mut();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("item:{now}"));
        // Preserve the original created_at_ms across updates (the substrate upsert
        // replaces the node, so a blind write would reset it).
        let existing = backend.get_node(&id)?;
        let created_at_ms = args
            .get("created_at_ms")
            .and_then(Value::as_i64)
            .or_else(|| {
                existing.as_ref().and_then(|node| {
                    node.properties
                        .get("created_at_ms")
                        .and_then(Value::as_i64)
                })
            })
            .unwrap_or(now);
        let updated_at_ms = args
            .get("updated_at_ms")
            .and_then(Value::as_i64)
            .unwrap_or(now);
        let mut props = serde_json::Map::new();
        props.insert(
            "kind".to_string(),
            args.get("kind").cloned().unwrap_or_else(|| json!("note")),
        );
        props.insert(
            "title".to_string(),
            args.get("title").cloned().unwrap_or_else(|| json!("")),
        );
        if let Some(source) = args.get("source") {
            props.insert("source".to_string(), source.clone());
        }
        props.insert("created_at_ms".to_string(), json!(created_at_ms));
        props.insert("updated_at_ms".to_string(), json!(updated_at_ms));
        if let Some(extra) = args.get("extra") {
            props.insert("extra".to_string(), extra.clone());
        }
        let record = NodeRecord::new(id, [projection::ITEM_LABEL], Value::Object(props));
        backend.upsert_node(record.clone())?;
        Ok(record)
    }
}

// ---------------------------------------------------------------------------
// Schema + transport.
// ---------------------------------------------------------------------------

#[derive(MergedObject, Default)]
pub(crate) struct QueryRoot(
    memory::MemoryQuery,
    graph::GraphQuery,
    coordination::CoordinationQuery,
    coordination_v2::CoordinationV2Query,
    epistemic::EpistemicQuery,
    code::CodeQuery,
    kg::HarnessKgQuery,
    clusters::ClustersQuery,
    items::ItemQuery,
);

#[derive(MergedObject, Default)]
pub(crate) struct MutationRoot(
    memory::MemoryMutation,
    graph::GraphMutation,
    coordination::CoordinationMutation,
    coordination_v2::CoordinationV2Mutation,
    epistemic::EpistemicMutation,
    code::CodeMutation,
    clusters::ClustersMutation,
    items::ItemMutation,
);

fn full_schema() -> Schema<QueryRoot, MutationRoot, EmptySubscription> {
    Schema::build(
        QueryRoot::default(),
        MutationRoot::default(),
        EmptySubscription,
    )
    .finish()
}

/// A query-only schema (EmptyMutation): running a mutation operation against it
/// is rejected by async-graphql, which is how `graphql_query` refuses mutations.
fn query_only_schema() -> Schema<QueryRoot, EmptyMutation, EmptySubscription> {
    Schema::build(QueryRoot::default(), EmptyMutation, EmptySubscription).finish()
}

#[derive(Clone, Copy)]
pub(crate) enum OpKind {
    Query,
    Mutate,
}

/// Execute a GraphQL document carried by `graphql_query` / `graphql_mutate`.
/// `tenant` is the connection tenant (already resolved by `call_tool`); empty is
/// rejected. `backend` is the connection-tenant backend, owned for this call.
pub(crate) fn execute_graphql<B: McpGraphBackend>(
    tenant: &str,
    backend: B,
    arguments: &Value,
    op: OpKind,
) -> Result<Value, McpError> {
    let tenant = tenant.trim();
    if tenant.is_empty() {
        return Err(McpError::invalid_params(
            "graphql tools require a non-empty connection tenant; refusing to default",
        ));
    }
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            McpError::invalid_params("graphql tools require arguments.query (a GraphQL string)")
        })?;
    let variables = match arguments.get("variables") {
        Some(value) if !value.is_null() => Variables::from_json(value.clone()),
        _ => Variables::default(),
    };

    let invoker = DispatchInvoker::new(backend, tenant.to_string());
    let _guard = set_invoker(&invoker);
    let request = Request::new(query).variables(variables);
    let response = match op {
        OpKind::Query => futures_executor::block_on(query_only_schema().execute(request)),
        OpKind::Mutate => futures_executor::block_on(full_schema().execute(request)),
    };
    serde_json::to_value(&response).map_err(|err| {
        McpError::invalid_params(format!("graphql response serialization failed: {err}"))
    })
}

/// The SDL for the full schema (for `graphql_introspect`).
pub(crate) fn introspect_sdl() -> Value {
    Value::String(full_schema().sdl())
}

/// Tool definitions for the GraphQL transport tools, in the house style. The
/// write tool (`graphql_mutate`) is listed only when writes are enabled, matching
/// how the flat write tools are hidden in read-only mode.
pub(crate) fn graphql_tool_definitions(include_mutations: bool) -> Vec<Value> {
    let mut tools = vec![
        crate::tool(
            "graphql_query",
            "Run a GraphQL QUERY (read) over the typed Harness schema: Items (items, itemsByKind, item), Memory domain, Graph domain (graphAlgorithm, graphNode, neighbors, graphSchema, vectorSearch, vectorHybrid, fulltextSearch, spatialRadius, spatialBbox, and symbolic fields), and Coordination domain (coordinationRoom, coordinationStream, workGraph, nextTaskNode). Read-only: mutation operations are refused (use graphql_mutate). Tenant is the connection tenant, not a field argument.",
            graphql_input_schema(),
        ),
        crate::tool(
            "graphql_introspect",
            "Return the GraphQL SDL for the typed Harness schema.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
    ];
    if include_mutations {
        let mut mutate = crate::tool_write(
            "graphql_mutate",
            "Run a GraphQL MUTATION (write) over the typed Harness schema: rememberMemory / reviseMemory / forgetMemory / createHandoff, Graph designate/bulk writes, and Coordination writes (writeCoordinationIntent, writeCoordinationRecord, publishCoordinationEvent, advanceCoordinationStream, createTaskNode, claimTaskNode).",
            graphql_input_schema(),
        );
        // graphql_mutate is the single door for destructive writes (forgetMemory,
        // jobArchive, reviseMemory): flag it so MCP clients can gate it. The shared
        // `tool_write` default is non-destructive, which is right for additive writes.
        mutate["annotations"]["destructiveHint"] = json!(true);
        tools.push(mutate);
    }
    tools
}

fn graphql_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": "A GraphQL document." },
            "variables": { "type": "object", "description": "Optional GraphQL variables." }
        },
        "required": ["query"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_named<'a>(tools: &'a [Value], name: &str) -> &'a Value {
        tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("tool {name} not in graphql definitions"))
    }

    // The GraphQL mutate entry point is the single door for destructive writes
    // (forgetMemory, jobArchive, reviseMemory, ...), so it must hint that it can
    // perform destructive updates -- a Connectors-Directory-style annotation.
    #[test]
    fn graphql_mutate_is_annotated_destructive() {
        let tools = graphql_tool_definitions(true);
        let mutate = tool_named(&tools, "graphql_mutate");
        // A destructive hint is only meaningful on a non-read-only tool, so the
        // mutate door must be both: not read-only, and destructive.
        assert_eq!(
            mutate["annotations"]["readOnlyHint"],
            json!(false),
            "graphql_mutate is a write door and must not be read-only"
        );
        assert_eq!(
            mutate["annotations"]["destructiveHint"],
            json!(true),
            "graphql_mutate must hint that it can perform destructive updates"
        );
    }

    // Guard: the two read entry points (query + the schema/introspect door) are
    // annotated read-only, so a Connectors-Directory client can surface them
    // without a write-gate.
    #[test]
    fn graphql_read_tools_are_annotated_read_only() {
        let tools = graphql_tool_definitions(true);
        for name in ["graphql_query", "graphql_introspect"] {
            let tool = tool_named(&tools, name);
            assert_eq!(
                tool["annotations"]["readOnlyHint"],
                json!(true),
                "{name} must be annotated read-only"
            );
            assert_eq!(
                tool["annotations"]["destructiveHint"],
                json!(false),
                "{name} is a read door and must not be flagged destructive"
            );
        }
    }
}
