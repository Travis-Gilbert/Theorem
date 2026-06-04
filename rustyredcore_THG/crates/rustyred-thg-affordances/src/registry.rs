//! Connector registration: turn an MCP server's tool catalog into Affordance
//! graph nodes. This is the inverse of the MCP adapter: the adapter exposes the
//! core as tools; the registry ingests connectors' tools as affordance nodes
//! through the same store. Registration is idempotent on re-registration:
//! re-registering the same connector preserves accumulated fitness, embeddings,
//! and outcome history.

use serde_json::{json, Value};

use rustyred_thg_core::{GraphMutation, GraphMutationBatch, NodeRecord, ThgResult};
use theorem_harness_core::default_affordance_registry;

use crate::types::{
    connector_node_id, edge_with_affordance_provenance, normalize_tenant_id, tenant_node_id,
    thg_error_from_store, Affordance, AffordanceGraphStore, AffordanceUpsertResult,
    ConnectorManifest, ConnectorRegisterResult, ToolManifest, CONNECTOR_LABEL,
    DEFAULT_HALF_LIFE_DAYS, OFFERS, TENANT_LABEL, THG_AFFORDANCE_SOURCE,
};

pub const THEOREM_GRPC_SERVER_ID: &str = "theorem_grpc";
pub const THEOREM_GRPC_TIMEOUT_MS: u64 = 30_000;

/// Register an entire connector: one `Connector` node + one `Affordance` node
/// per tool + `OFFERS` edges, in a single transaction. Idempotent.
pub fn register_connector<S: AffordanceGraphStore>(
    store: &mut S,
    manifest: ConnectorManifest,
    actor: Option<&str>,
) -> ThgResult<ConnectorRegisterResult> {
    register_connector_with_target(store, manifest, None, actor)
}

/// Like `register_connector`, plus persist an opaque transport descriptor (how to
/// reach the owning server again) on the `Connector` node, so a later selection
/// can re-invoke a registered tool. The descriptor is stored verbatim and never
/// interpreted here: the connectors crate owns its shape (it depends on this
/// crate, not the reverse). Idempotent: a `None` target preserves any
/// previously-persisted target rather than wiping the server's learned reach.
pub fn register_connector_with_target<S: AffordanceGraphStore>(
    store: &mut S,
    manifest: ConnectorManifest,
    connection_target: Option<Value>,
    actor: Option<&str>,
) -> ThgResult<ConnectorRegisterResult> {
    let tenant_id = normalize_tenant_id(&manifest.tenant_id);
    let server_id = manifest.server_id.trim().to_string();
    if server_id.is_empty() {
        return Err(rustyred_thg_core::ThgError::new(
            "invalid_connector",
            "server_id is required",
        ));
    }

    let connector_node = connector_node_id(&tenant_id, &server_id);

    // Persist the connection target; when none is supplied, preserve the existing
    // one so idempotent re-registration does not wipe the server's reach.
    let persisted_target = connection_target.or_else(|| {
        store
            .get_node(&connector_node)
            .ok()
            .flatten()
            .and_then(|node| node.properties.get("connection_target").cloned())
    });
    let mut connector_props = json!({
        "tenant_id": tenant_id,
        "server_id": server_id,
        "label": manifest.label,
        "tool_count": manifest.tools.len(),
        "source": THG_AFFORDANCE_SOURCE,
    });
    if let Some(target) = persisted_target {
        connector_props["connection_target"] = target;
    }

    let mut mutations = vec![
        GraphMutation::NodeUpsert(NodeRecord::new(
            tenant_node_id(&tenant_id),
            [TENANT_LABEL],
            json!({ "tenant_id": tenant_id, "source": THG_AFFORDANCE_SOURCE }),
        )),
        GraphMutation::NodeUpsert(NodeRecord::new(
            &connector_node,
            [CONNECTOR_LABEL],
            connector_props,
        )),
    ];

    let mut affordance_node_ids = Vec::with_capacity(manifest.tools.len());
    for tool in &manifest.tools {
        let affordance = affordance_from_tool(&tenant_id, &server_id, tool);
        affordance.validate()?;
        let node_id = affordance.node_id();
        let extra = preserved_affordance_properties(
            store
                .get_node(&node_id)
                .map_err(thg_error_from_store)?
                .as_ref(),
            affordance.embedding.is_some(),
        );
        mutations.push(GraphMutation::NodeUpsert(
            affordance.to_node_record(actor, extra),
        ));
        mutations.push(GraphMutation::EdgeUpsert(edge_with_affordance_provenance(
            offers_edge_id(&connector_node, &node_id),
            &connector_node,
            OFFERS,
            &node_id,
            json!({ "tenant_id": tenant_id }),
            actor,
        )));
        affordance_node_ids.push(node_id);
    }

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;

    Ok(ConnectorRegisterResult {
        tenant_id,
        server_id,
        connector_node_id: connector_node,
        affordance_node_ids,
        transaction,
    })
}

/// Upsert a single affordance (its node + owning connector + `OFFERS` edge),
/// preserving accumulated fitness/embedding on re-registration.
pub fn upsert_affordance<S: AffordanceGraphStore>(
    store: &mut S,
    affordance: Affordance,
    actor: Option<&str>,
) -> ThgResult<AffordanceUpsertResult> {
    let affordance = affordance.normalized();
    affordance.validate()?;
    let node_id = affordance.node_id();
    let connector_node = connector_node_id(&affordance.tenant_id, &affordance.server_id);

    let extra = preserved_affordance_properties(
        store
            .get_node(&node_id)
            .map_err(thg_error_from_store)?
            .as_ref(),
        affordance.embedding.is_some(),
    );

    let mutations = vec![
        GraphMutation::NodeUpsert(NodeRecord::new(
            &connector_node,
            [CONNECTOR_LABEL],
            json!({
                "tenant_id": affordance.tenant_id,
                "server_id": affordance.server_id,
                "source": THG_AFFORDANCE_SOURCE,
            }),
        )),
        GraphMutation::NodeUpsert(affordance.to_node_record(actor, extra)),
        GraphMutation::EdgeUpsert(edge_with_affordance_provenance(
            offers_edge_id(&connector_node, &node_id),
            &connector_node,
            OFFERS,
            &node_id,
            json!({ "tenant_id": affordance.tenant_id }),
            actor,
        )),
    ];

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;
    Ok(AffordanceUpsertResult {
        node_id: affordance.node_id(),
        affordance,
        edge_count: 1,
        transaction,
    })
}

/// Register the remaining Theseus app surface as explicit `theorem_grpc`
/// affordances. This is metadata only: live gRPC invocation belongs to the
/// runtime adapter, while the registry/selection/receipt layer can see the
/// app capabilities now.
pub fn register_theseus_app_affordances<S: AffordanceGraphStore>(
    store: &mut S,
    tenant_id: &str,
    actor: Option<&str>,
) -> ThgResult<ConnectorRegisterResult> {
    let tenant_id = normalize_tenant_id(tenant_id);
    let connector_node = connector_node_id(&tenant_id, THEOREM_GRPC_SERVER_ID);
    let affordances = theseus_app_affordances(&tenant_id);

    let mut mutations = vec![
        GraphMutation::NodeUpsert(NodeRecord::new(
            tenant_node_id(&tenant_id),
            [TENANT_LABEL],
            json!({ "tenant_id": tenant_id, "source": THG_AFFORDANCE_SOURCE }),
        )),
        GraphMutation::NodeUpsert(NodeRecord::new(
            &connector_node,
            [CONNECTOR_LABEL],
            json!({
                "tenant_id": tenant_id,
                "server_id": THEOREM_GRPC_SERVER_ID,
                "label": "Theorem gRPC app surface",
                "connection_target": {
                    "transport": "theorem_grpc",
                    "service": "theorem_grpc.AppAffordanceService",
                    "method": "InvokeAffordance",
                    "timeout_ms": THEOREM_GRPC_TIMEOUT_MS,
                    "failure_receipt": theorem_grpc_failure_receipt_shape(),
                },
                "source": THG_AFFORDANCE_SOURCE,
            }),
        )),
    ];

    let mut affordance_node_ids = Vec::with_capacity(affordances.len());
    for affordance in affordances {
        affordance.validate()?;
        let node_id = affordance.node_id();
        let extra = preserved_affordance_properties(
            store
                .get_node(&node_id)
                .map_err(thg_error_from_store)?
                .as_ref(),
            false,
        );
        mutations.push(GraphMutation::NodeUpsert(
            affordance.to_node_record(actor, extra),
        ));
        mutations.push(GraphMutation::EdgeUpsert(edge_with_affordance_provenance(
            offers_edge_id(&connector_node, &node_id),
            &connector_node,
            OFFERS,
            &node_id,
            json!({ "tenant_id": tenant_id }),
            actor,
        )));
        affordance_node_ids.push(node_id);
    }

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;

    Ok(ConnectorRegisterResult {
        tenant_id,
        server_id: THEOREM_GRPC_SERVER_ID.to_string(),
        connector_node_id: connector_node,
        affordance_node_ids,
        transaction,
    })
}

pub fn theseus_app_affordances(tenant_id: &str) -> Vec<Affordance> {
    let tenant_id = normalize_tenant_id(tenant_id);
    theseus_app_specs()
        .iter()
        .map(|spec| spec.to_affordance(&tenant_id))
        .collect()
}

/// Read the persisted opaque connection target off a `Connector` node, if any.
/// The connectors crate's invoke bridge uses this to re-reach a selected tool's
/// server. Returns `None` when the connector is unknown or was registered without
/// a target.
pub fn connector_connection_target<S: AffordanceGraphStore>(
    store: &S,
    tenant_id: &str,
    server_id: &str,
) -> ThgResult<Option<Value>> {
    let connector_node = connector_node_id(&normalize_tenant_id(tenant_id), server_id.trim());
    Ok(store
        .get_node(&connector_node)
        .map_err(thg_error_from_store)?
        .and_then(|node| node.properties.get("connection_target").cloned()))
}

/// Project the built-in `theorem-harness-core` affordance registry (the 11
/// symbolic engines) into graph nodes, so the existing affordances are
/// first-class learning nodes too, not only newly connected MCP tools.
pub fn register_builtin_affordances<S: AffordanceGraphStore>(
    store: &mut S,
    tenant_id: &str,
    actor: Option<&str>,
) -> ThgResult<ConnectorRegisterResult> {
    let tenant_id = normalize_tenant_id(tenant_id);
    let mut mutations = vec![GraphMutation::NodeUpsert(NodeRecord::new(
        tenant_node_id(&tenant_id),
        [TENANT_LABEL],
        json!({ "tenant_id": tenant_id, "source": THG_AFFORDANCE_SOURCE }),
    ))];

    let mut affordance_node_ids = Vec::new();
    let mut connectors_seen = std::collections::BTreeSet::new();
    for contract in default_affordance_registry() {
        let affordance = Affordance::from_contract(&contract, &tenant_id);
        affordance.validate()?;
        let node_id = affordance.node_id();
        let connector_node = connector_node_id(&tenant_id, &affordance.server_id);
        if connectors_seen.insert(connector_node.clone()) {
            mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
                &connector_node,
                [CONNECTOR_LABEL],
                json!({
                    "tenant_id": tenant_id,
                    "server_id": affordance.server_id,
                    "label": "theorem-native",
                    "source": THG_AFFORDANCE_SOURCE,
                }),
            )));
        }
        let extra = preserved_affordance_properties(
            store
                .get_node(&node_id)
                .map_err(thg_error_from_store)?
                .as_ref(),
            false,
        );
        mutations.push(GraphMutation::NodeUpsert(
            affordance.to_node_record(actor, extra),
        ));
        mutations.push(GraphMutation::EdgeUpsert(edge_with_affordance_provenance(
            offers_edge_id(&connector_node, &node_id),
            &connector_node,
            OFFERS,
            &node_id,
            json!({ "tenant_id": tenant_id }),
            actor,
        )));
        affordance_node_ids.push(node_id);
    }

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;
    Ok(ConnectorRegisterResult {
        tenant_id,
        server_id: "theorem-native".to_string(),
        connector_node_id: String::new(),
        affordance_node_ids,
        transaction,
    })
}

fn affordance_from_tool(tenant_id: &str, server_id: &str, tool: &ToolManifest) -> Affordance {
    let tool_name = tool.name.trim().to_string();
    let affordance_id = format!("{server_id}.{tool_name}");
    Affordance {
        affordance_id,
        tenant_id: tenant_id.to_string(),
        server_id: server_id.to_string(),
        tool_name,
        family: "connector".to_string(),
        label: if tool.label.trim().is_empty() {
            tool.name.trim().to_string()
        } else {
            tool.label.trim().to_string()
        },
        description: tool.description.clone(),
        input_schema: if tool.input_schema.is_null() {
            json!({})
        } else {
            tool.input_schema.clone()
        },
        permissions: tool.permissions.clone(),
        cost: if tool.cost.is_null() {
            json!({})
        } else {
            tool.cost.clone()
        },
        writeback_policy: tool.writeback_policy.clone(),
        tags: tool.tags.clone(),
        embedding: tool.description_embedding.clone(),
        fitness: 0.0,
        version: 1,
        created_at_ms: 0,
        manifest_version: 1,
    }
    .normalized()
}

#[derive(Clone, Copy)]
struct TheseusAppAffordanceSpec {
    tool_name: &'static str,
    family: &'static str,
    label: &'static str,
    description: &'static str,
    permissions: &'static [&'static str],
    writeback_policy: &'static str,
    latency_class: &'static str,
    cost_class: &'static str,
    write_class: &'static str,
    tags: &'static [&'static str],
}

impl TheseusAppAffordanceSpec {
    fn to_affordance(self, tenant_id: &str) -> Affordance {
        let mut tags = vec![
            "theseus_app".to_string(),
            "theorem_grpc".to_string(),
            self.family.to_string(),
        ];
        tags.extend(self.tags.iter().map(|tag| (*tag).to_string()));
        tags.sort();
        tags.dedup();

        Affordance {
            affordance_id: format!("{THEOREM_GRPC_SERVER_ID}.{}", self.tool_name),
            tenant_id: tenant_id.to_string(),
            server_id: THEOREM_GRPC_SERVER_ID.to_string(),
            tool_name: self.tool_name.to_string(),
            family: self.family.to_string(),
            label: self.label.to_string(),
            description: self.description.to_string(),
            input_schema: json!({
                "type": "object",
                "transport": "theorem_grpc",
                "timeout_ms": THEOREM_GRPC_TIMEOUT_MS,
                "request": {
                    "type": "object",
                    "additionalProperties": true
                },
                "failure_receipt": theorem_grpc_failure_receipt_shape(),
            }),
            permissions: self
                .permissions
                .iter()
                .map(|permission| (*permission).to_string())
                .collect(),
            cost: json!({
                "execution_surface": "theorem_grpc",
                "transport": "theorem_grpc",
                "timeout_ms": THEOREM_GRPC_TIMEOUT_MS,
                "latency_class": self.latency_class,
                "cost_class": self.cost_class,
                "write_class": self.write_class,
                "failure_receipt": theorem_grpc_failure_receipt_shape(),
                "source_module": "theseus_apps",
                "parity_status": "app-wrapper-metadata",
            }),
            writeback_policy: self.writeback_policy.to_string(),
            tags,
            embedding: None,
            fitness: 0.0,
            version: 1,
            created_at_ms: 0,
            manifest_version: 1,
        }
        .normalized()
    }
}

fn theseus_app_specs() -> &'static [TheseusAppAffordanceSpec] {
    &[
        TheseusAppAffordanceSpec {
            tool_name: "code_search.ingest",
            family: "code_search",
            label: "Ingest Codebase",
            description: "Index a codebase into the native Theorem RedCore code graph.",
            permissions: &["code_read", "graph_write", "receipt_write"],
            writeback_policy: "write-graph",
            latency_class: "background",
            cost_class: "low",
            write_class: "graph",
            tags: &["code", "ingest", "writeback"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "code_search.reindex",
            family: "code_search",
            label: "Reindex Codebase",
            description: "Refresh a previously indexed codebase in the native Theorem code graph.",
            permissions: &["code_read", "graph_write", "receipt_write"],
            writeback_policy: "write-graph",
            latency_class: "background",
            cost_class: "low",
            write_class: "graph",
            tags: &["code", "ingest", "reindex", "writeback"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "code_search.search",
            family: "code_search",
            label: "Search Code",
            description:
                "Search indexed code symbols and files from the native Theorem code graph.",
            permissions: &["code_read", "graph_read", "receipt_write"],
            writeback_policy: "receipt-only",
            latency_class: "interactive",
            cost_class: "low",
            write_class: "receipt",
            tags: &["code", "read", "search"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "code_search.context",
            family: "code_search",
            label: "Read Code Context",
            description: "Expand an indexed code hit into surrounding file and symbol context.",
            permissions: &["code_read", "graph_read", "receipt_write"],
            writeback_policy: "receipt-only",
            latency_class: "interactive",
            cost_class: "low",
            write_class: "receipt",
            tags: &["code", "context", "read"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "code_search.recognize",
            family: "code_search",
            label: "Recognize Code",
            description: "Recognize code symbols from indexed files or inline source text.",
            permissions: &["code_read", "graph_read", "receipt_write"],
            writeback_policy: "receipt-only",
            latency_class: "interactive",
            cost_class: "low",
            write_class: "receipt",
            tags: &["code", "recognize", "read"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "code_search.explore",
            family: "code_search",
            label: "Explore Code Graph",
            description: "Expand related code symbols through dependency and call graph edges.",
            permissions: &["code_read", "graph_read", "receipt_write"],
            writeback_policy: "receipt-only",
            latency_class: "interactive",
            cost_class: "low",
            write_class: "receipt",
            tags: &["code", "explore", "graph", "read"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "code_search.explain",
            family: "code_search",
            label: "Explain Code",
            description:
                "Summarize an indexed code symbol with context, trust tier, and graph evidence.",
            permissions: &["code_read", "graph_read", "receipt_write"],
            writeback_policy: "receipt-only",
            latency_class: "interactive",
            cost_class: "low",
            write_class: "receipt",
            tags: &["code", "explain", "graph", "read"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "anti_misinfo_algo.inspect_claim",
            family: "anti_misinfo_algo",
            label: "Inspect Claim",
            description: "Run the Theseus anti-misinformation claim inspection pathway.",
            permissions: &["graph_read", "receipt_write"],
            writeback_policy: "receipt-only",
            latency_class: "interactive",
            cost_class: "standard",
            write_class: "receipt",
            tags: &["claim_check", "read", "receipt"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "corpus_surface.retrieve",
            family: "corpus_surface",
            label: "Retrieve Corpus Surface",
            description: "Read candidate corpus surfaces and source packets from Theseus.",
            permissions: &["graph_read"],
            writeback_policy: "read-only",
            latency_class: "interactive",
            cost_class: "low",
            write_class: "none",
            tags: &["corpus", "read"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "federation.sync",
            family: "federation",
            label: "Sync Federation State",
            description: "Exchange room or substrate state through the Theseus federation layer.",
            permissions: &["graph_read", "graph_write", "receipt_write"],
            writeback_policy: "write-graph",
            latency_class: "background",
            cost_class: "standard",
            write_class: "graph",
            tags: &["coordination", "write", "writeback"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "epistemic_federation.merge",
            family: "epistemic_federation",
            label: "Merge Epistemic Federation",
            description: "Merge cross-agent epistemic records with provenance and receipts.",
            permissions: &["graph_read", "graph_write", "receipt_write"],
            writeback_policy: "write-graph",
            latency_class: "background",
            cost_class: "standard",
            write_class: "graph",
            tags: &["coordination", "epistemic", "write", "writeback"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "paper_trail.trace",
            family: "paper_trail",
            label: "Trace Paper Trail",
            description: "Create or extend a provenance trail for a claim, artifact, or run.",
            permissions: &["graph_read", "graph_write", "receipt_write"],
            writeback_policy: "write-graph",
            latency_class: "interactive",
            cost_class: "low",
            write_class: "graph",
            tags: &["provenance", "receipt", "writeback"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "public_verbs.execute",
            family: "public_verbs",
            label: "Execute Public Verb",
            description: "Invoke an audited public verb exposed by the Theseus app boundary.",
            permissions: &["graph_read", "external_action", "receipt_write"],
            writeback_policy: "confirm-before-write",
            latency_class: "interactive",
            cost_class: "standard",
            write_class: "external",
            tags: &["external_action", "public", "write"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "publisher.publish",
            family: "publisher",
            label: "Publish Artifact",
            description: "Publish a Theseus artifact only through the confirmation-gated boundary.",
            permissions: &["graph_read", "external_action", "receipt_write"],
            writeback_policy: "confirm-before-external",
            latency_class: "interactive",
            cost_class: "standard",
            write_class: "external",
            tags: &["external_action", "publish", "write"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "research.expand",
            family: "research",
            label: "Expand Research Frontier",
            description: "Run the Theseus research expansion surface and write evidence receipts.",
            permissions: &["graph_read", "graph_write", "receipt_write"],
            writeback_policy: "write-graph",
            latency_class: "background",
            cost_class: "standard",
            write_class: "graph",
            tags: &["research", "writeback"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "user_model.update",
            family: "user_model",
            label: "Update User Model",
            description: "Update private user-model facts through a binding-private receipt.",
            permissions: &["private_read", "private_write", "receipt_write"],
            writeback_policy: "binding-private",
            latency_class: "interactive",
            cost_class: "low",
            write_class: "private",
            tags: &["binding_private", "private", "user_model", "writeback"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "memory_tensions.detect",
            family: "memory_tensions",
            label: "Detect Memory Tensions",
            description: "Detect contradictions or tensions among active memory atoms.",
            permissions: &["graph_read", "graph_write", "receipt_write"],
            writeback_policy: "write-graph",
            latency_class: "interactive",
            cost_class: "low",
            write_class: "graph",
            tags: &["memory", "tension", "writeback"],
        },
        TheseusAppAffordanceSpec {
            tool_name: "observability.read_trace",
            family: "observability",
            label: "Read Observability Trace",
            description: "Read run, action, and provenance traces for inspection and debugging.",
            permissions: &["trace_read", "graph_read"],
            writeback_policy: "read-only",
            latency_class: "interactive",
            cost_class: "low",
            write_class: "none",
            tags: &["observability", "read", "trace"],
        },
    ]
}

fn theorem_grpc_failure_receipt_shape() -> Value {
    json!({
        "receipt_type": "THEOREM_GRPC.AFFORDANCE_FAILED",
        "status": "failed",
        "fields": [
            "tenant_id",
            "affordance_id",
            "server_id",
            "tool_name",
            "transport",
            "timeout_ms",
            "error_code",
            "message",
            "elapsed_ms"
        ],
    })
}

/// Preserve learned state across re-registration: fitness, fitness decay
/// metadata, original creation time, and the existing embedding unless the new
/// manifest supplies one. Defaults the fitness half-life if absent.
fn preserved_affordance_properties(
    existing: Option<&NodeRecord>,
    has_new_embedding: bool,
) -> Value {
    let mut preserved = json!({});
    if let Some(node) = existing {
        for key in [
            "fitness",
            "fitness_updated_at_ms",
            "fitness_half_life_days",
            "created_at_ms",
        ] {
            if let Some(value) = node.properties.get(key) {
                preserved[key] = value.clone();
            }
        }
        if !has_new_embedding {
            if let Some(embedding) = node.properties.get("embedding") {
                preserved["embedding"] = embedding.clone();
            }
        }
    }
    if preserved.get("fitness_half_life_days").is_none() {
        preserved["fitness_half_life_days"] = json!(DEFAULT_HALF_LIFE_DAYS);
    }
    preserved
}

fn offers_edge_id(connector_node_id: &str, affordance_node_id: &str) -> String {
    format!("edge:{connector_node_id}:offers:{affordance_node_id}")
}
