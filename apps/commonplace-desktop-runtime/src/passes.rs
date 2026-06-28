//! Ambient passes that run AFTER ingest on a settled change set (ambient-layer
//! handoff Part A deliverable 3).
//!
//! Slice 2 ([`CommonplaceIngestSink`]) lands a change set as commonplace items
//! plus a `CommonplaceChangeSet` lineage node. This slice adds the *trigger
//! architecture*: an [`AmbientPass`] seam, a provenance-bearing [`PassReceipt`]
//! written into the same sidecar graph and linked to that lineage node, and an
//! [`AmbientRuntime`] composite [`ChangeSink`] that ingests, then runs the
//! registered passes, persisting one receipt per pass.
//!
//! The canonical-git boundary from slices 1-2 holds unchanged: passes read the
//! tree (and the just-ingested graph) but write ONLY into the sidecar.
//!
//! Three passes are wired, honestly reflecting what the substrate actually
//! exposes today (the reverse-engineer rule: report unavailable tooling as a
//! degraded state, never guess a source-level API that is not there):
//!
//! * [`ReconstructionPass`] -- REAL-WIRED. Detects changed files that are binary
//!   artifacts (by extension / magic bytes) and, for each, runs the real
//!   reconstruction harness
//!   ([`run_reconstruction_pipeline`](rustyred_thg_reconstruct_harness::run_reconstruction_pipeline))
//!   and commits its facts via
//!   [`write_pipeline_output_in_store`](rustyred_thg_reconstruct_harness::write_pipeline_output_in_store).
//!   No changed artifact -> a `NotApplicable` receipt (not an error). An
//!   artifact-extension file whose bytes do not parse as a known object format
//!   -> a `Degraded` receipt naming the parse failure (applicable but
//!   unreconstructable), never a crate error.
//! * [`OffloadPass`] -- REAL-WIRED. The ingest of a settled change set triggers a
//!   graph-algorithm derivation: centrality over the just-ingested items'
//!   `SIMILAR_TO` subgraph (F2 writes those edges). That derivation is exactly
//!   the compute-offload thesis's "what are the most central entities in this
//!   knowledge subgraph?" operation -- offload-eligible (cheaper + exact as CPU
//!   substrate compute, not a GPU forward-pass approximation). The pass reads the
//!   ingested items' `SIMILAR_TO` edges out of the sidecar, classifies the
//!   derivation through the [`OffloadEngine`](rustyred_thg_offload::OffloadEngine),
//!   executes the wired exact-PageRank substrate affordance over that real graph,
//!   and records a `Produced` receipt carrying the offload decision + the
//!   `gpu_seconds_saved` ledger total. A change set whose ingested items have no
//!   `SIMILAR_TO` edges among them (e.g. a single new item) yields no subgraph to
//!   compute over -> a `NotApplicable` receipt (honest, not a degenerate result).
//! * [`StandingSeedPass`] -- DEGRADED. There is no standing-query / standing-seed
//!   evaluation mechanism in the core (all `seed` surfaces are PPR seed-node
//!   queries; all `standing` surfaces are epistemic claim standing). Registered;
//!   records an `Unavailable` receipt naming the gap.

use std::path::Path;

use rustyred_thg_core::{
    now_ms, DiskObjectStore, EdgeRecord, GraphSnapshot, GraphStore, GraphStoreResult, NeighborQuery,
    NodeRecord, RedCoreGraphStore,
};
use rustyred_thg_offload::{
    ExecutorKind, GraphAffordanceEngine, GraphAffordanceRequest, Operation, OperationKind,
    OperationPlanner, PlannerConfig, COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID,
};
use rustyred_thg_reconstruct_harness::{
    run_reconstruction_pipeline, write_pipeline_output_in_store,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use commonplace::{Commonplace, SIMILAR_TO_EDGE};

use crate::sink::IngestOutcome;
use crate::{ChangeKind, ChangeSet, FileChange};

/// Node label for a pass receipt (deliverable 3 provenance node).
pub const PASS_RECEIPT_LABEL: &str = "AmbientPassReceipt";
/// Edge from a [`PassReceipt`] node to the `CommonplaceChangeSet` lineage node it
/// was produced for, so receipts hang off the change-set history.
pub const PRODUCED_FOR_EDGE: &str = "PRODUCED_FOR";

/// The durable commonplace handle the passes write into. The sidecar graph +
/// blob store the ingest sink already owns; passes never open their own.
pub type SidecarCommonplace = Commonplace<RedCoreGraphStore, DiskObjectStore>;

/// Outcome status of a single pass run. The `Degraded`/`Unavailable` variants
/// are first-class: a pass that has no callable substrate API to wire records
/// `Unavailable` (naming the gap) instead of silently doing nothing or faking a
/// result, and a pass that is applicable but could not complete records
/// `Degraded` (naming why).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "detail")]
pub enum PassStatus {
    /// The pass ran and produced graph facts / derivations.
    Produced,
    /// The pass ran but nothing in this change set was in scope for it (e.g. no
    /// binary artifact changed). Not an error.
    NotApplicable,
    /// The pass was applicable but could not complete; the string names why
    /// (e.g. a binary that did not parse as a known object format).
    Degraded(String),
    /// The pass has no callable substrate API to wire yet; the string names the
    /// missing capability. The pass is still registered so the seam is real and
    /// the gap is auditable in the graph.
    Unavailable(String),
}

impl PassStatus {
    /// Stable lower-case tag for the receipt node's queryable `status` property.
    pub fn tag(&self) -> &'static str {
        match self {
            PassStatus::Produced => "produced",
            PassStatus::NotApplicable => "not_applicable",
            PassStatus::Degraded(_) => "degraded",
            PassStatus::Unavailable(_) => "unavailable",
        }
    }
}

/// What one pass produced for one change set. Persisted as a provenance-bearing
/// graph node (deliverable 3) linked to the change-set lineage node, and also
/// returned in-process so the runtime and tests can inspect a run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PassReceipt {
    /// The pass that produced this receipt ([`AmbientPass::name`]).
    pub pass: String,
    /// Outcome status (carries the degraded / unavailable reason).
    pub status: PassStatus,
    /// The inputs from the change set this pass considered applicable (relative
    /// paths). Empty for `NotApplicable` / `Unavailable`.
    pub applicable_inputs: Vec<String>,
    /// Ids of graph evidence this pass produced or pointed at (e.g. binary
    /// artifact ids written by the reconstruction harness). Empty when nothing
    /// was produced.
    pub evidence_ids: Vec<String>,
    /// Free-form, machine-readable detail about what was produced/derived, for
    /// surfaces that want more than the status line.
    pub produced: Value,
}

impl PassReceipt {
    /// A receipt for a pass that ran but found nothing in scope.
    pub fn not_applicable(pass: impl Into<String>) -> Self {
        Self {
            pass: pass.into(),
            status: PassStatus::NotApplicable,
            applicable_inputs: Vec::new(),
            evidence_ids: Vec::new(),
            produced: json!({}),
        }
    }

    /// A receipt for a pass with no callable substrate API yet. `missing` names
    /// the capability that is not present so the gap is auditable.
    pub fn unavailable(pass: impl Into<String>, missing: impl Into<String>) -> Self {
        Self {
            pass: pass.into(),
            status: PassStatus::Unavailable(missing.into()),
            applicable_inputs: Vec::new(),
            evidence_ids: Vec::new(),
            produced: json!({}),
        }
    }
}

/// An ambient pass: a unit of after-ingest work that derives provenance into the
/// sidecar graph. The runtime runs each registered pass once per settled change
/// set, after ingest, handing it the change set, the ingest outcome, and a
/// mutable handle to the durable commonplace store.
///
/// A pass returns a [`PassReceipt`]; the runtime writes it as a graph node and
/// links it to the change-set lineage node. Returning `Err` is reserved for a
/// genuine store failure -- "nothing applicable" and "capability missing" are
/// receipt statuses, not errors, so the pass loop never aborts on them.
pub trait AmbientPass: Send {
    /// Stable identifier for this pass (also the receipt's `pass` field).
    fn name(&self) -> &str;

    /// Run the pass for one settled change set, after ingest has committed.
    fn run(
        &self,
        change_set: &ChangeSet,
        outcome: &IngestOutcome,
        commonplace: &mut SidecarCommonplace,
    ) -> GraphStoreResult<PassReceipt>;
}

/// Binary-artifact file extensions the reconstruction pass treats as applicable.
/// Lower-cased, no leading dot.
const ARTIFACT_EXTENSIONS: &[&str] = &[
    "o", "obj", "bin", "elf", "so", "dylib", "a", "exe", "dll", "wasm", "ko", "out",
];

/// Magic-byte prefixes that mark a file as a binary artifact even without a
/// telltale extension (ELF, Mach-O 32/64 + fat, PE/`MZ`, Wasm).
const ARTIFACT_MAGICS: &[&[u8]] = &[
    b"\x7fELF",         // ELF
    b"\xfe\xed\xfa\xce", // Mach-O 32 BE
    b"\xfe\xed\xfa\xcf", // Mach-O 64 BE
    b"\xce\xfa\xed\xfe", // Mach-O 32 LE
    b"\xcf\xfa\xed\xfe", // Mach-O 64 LE
    b"\xca\xfe\xba\xbe", // Mach-O fat / Java class (object parses Mach-O fat)
    b"MZ",              // PE / DOS
    b"\x00asm",         // WebAssembly
];

/// Whether a path's extension marks it as a binary artifact.
fn has_artifact_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| ARTIFACT_EXTENSIONS.contains(&ext.as_str()))
}

/// Whether the leading bytes of a file match a known binary-artifact magic.
fn has_artifact_magic(bytes: &[u8]) -> bool {
    ARTIFACT_MAGICS
        .iter()
        .any(|magic| bytes.len() >= magic.len() && &bytes[..magic.len()] == *magic)
}

/// REAL-WIRED reconstruction pass. For each `Created`/`Modified` change whose
/// path is a binary artifact (by extension or magic bytes), it runs the real
/// reconstruction harness and commits the resulting facts into the sidecar.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReconstructionPass;

impl ReconstructionPass {
    pub const NAME: &'static str = "reconstruction";

    /// The artifact changes in a settled set: created/modified files that look
    /// like binary artifacts. Removed paths are out of scope (no bytes to read).
    fn artifact_changes<'a>(&self, change_set: &'a ChangeSet) -> Vec<&'a FileChange> {
        change_set
            .changes
            .iter()
            .filter(|change| matches!(change.kind, ChangeKind::Created | ChangeKind::Modified))
            .filter(|change| {
                if has_artifact_extension(&change.path) {
                    return true;
                }
                // Fall back to a magic sniff for extensionless artifacts.
                std::fs::read(&change.path)
                    .map(|bytes| has_artifact_magic(&bytes))
                    .unwrap_or(false)
            })
            .collect()
    }
}

impl AmbientPass for ReconstructionPass {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn run(
        &self,
        change_set: &ChangeSet,
        _outcome: &IngestOutcome,
        commonplace: &mut SidecarCommonplace,
    ) -> GraphStoreResult<PassReceipt> {
        let artifacts = self.artifact_changes(change_set);
        if artifacts.is_empty() {
            return Ok(PassReceipt::not_applicable(Self::NAME));
        }

        let mut applicable_inputs = Vec::new();
        let mut evidence_ids = Vec::new();
        let mut reconstructed = Vec::new();
        let mut failures = Vec::new();

        for change in artifacts {
            let display_path = change.path.to_string_lossy().to_string();
            applicable_inputs.push(display_path.clone());

            let bytes = match std::fs::read(&change.path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    // Vanished/unreadable between debounce and now: applicable
                    // but unreconstructable, not a store error.
                    failures.push(json!({ "path": display_path, "error": error.to_string() }));
                    continue;
                }
            };

            match run_reconstruction_pipeline(&display_path, &bytes) {
                Ok(output) => {
                    // Commit the reconstruction facts into the sidecar graph.
                    write_pipeline_output_in_store(commonplace.store_mut(), &output)?;
                    evidence_ids.push(output.load.artifact.artifact_id.clone());
                    reconstructed.push(json!({
                        "path": display_path,
                        "artifact_id": output.load.artifact.artifact_id,
                        "format": output.load.artifact.format,
                        "arch": output.load.artifact.arch,
                        "section_count": output.report.section_count,
                        "symbol_count": output.report.symbol_count,
                        "instruction_count": output.report.instruction_count,
                        "function_count": output.report.function_count,
                        "component_count": output.report.component_count,
                    }));
                }
                Err(error) => {
                    // The file looked like an artifact but did not parse as a
                    // known object format. Degraded, named -- not an error.
                    failures.push(json!({
                        "path": display_path,
                        "error": format!("{error:?}"),
                    }));
                }
            }
        }

        let status = if !reconstructed.is_empty() {
            PassStatus::Produced
        } else {
            // Applicable artifacts existed but none reconstructed.
            PassStatus::Degraded(
                "no changed artifact parsed as a known object format".to_string(),
            )
        };

        Ok(PassReceipt {
            pass: Self::NAME.to_string(),
            status,
            applicable_inputs,
            evidence_ids,
            produced: json!({
                "reconstructed": reconstructed,
                "failures": failures,
            }),
        })
    }
}

/// REAL-WIRED offload pass. On a settled change set, the ingest triggers a
/// graph-algorithm derivation -- centrality over the just-ingested items'
/// `SIMILAR_TO` subgraph (F2 writes those edges) -- which is offload-eligible
/// (cheaper + exact as CPU substrate compute, not a GPU forward-pass
/// approximation). The pass routes that derivation through the compute-offload
/// planner ([`OperationPlanner`]) as a [`OperationKind::GraphPageRank`]
/// operation: the planner picks the cheapest executor satisfying the quality
/// floor, which for an exact graph op is the CPU `graph.pagerank` affordance over
/// the [`ExecutorKind::ExpensiveModel`] baseline, and its
/// [`CostSummary`](rustyred_thg_offload::CostSummary) banks the `gpu_seconds_saved`
/// / `tokens_saved`. The pass then EXECUTES that exact PageRank over the real
/// subgraph through the wired [`GraphAffordanceEngine`] and records a `Produced`
/// receipt carrying both the routing decision and the real centrality result
/// (the most central item).
///
/// The planner is constructed per-run with the planner's default cost weights;
/// each receipt carries the per-change-set offload saving. A longer-lived
/// cumulative ledger across change sets is a later-slice concern.
#[derive(Clone, Copy, Debug, Default)]
pub struct OffloadPass;

impl OffloadPass {
    pub const NAME: &'static str = "offload_eligible_derivation";

    /// Build the `SIMILAR_TO` subgraph induced by the just-ingested items: for
    /// each ingested item, its outgoing `SIMILAR_TO` edges whose target is ALSO
    /// an ingested item this change set (so the centrality op is scoped to the
    /// change, not the whole graph). Returns the edges as substrate
    /// [`EdgeRecord`]s ready for the graph affordance.
    ///
    /// Read via the `GraphStore` trait method (UFCS): the inherent
    /// `RedCoreGraphStore::neighbors` shadows the trait one with a `Result`
    /// signature, so the trait call keeps the plain `Vec<NeighborHit>` surface.
    fn similar_subgraph(
        &self,
        outcome: &IngestOutcome,
        commonplace: &SidecarCommonplace,
    ) -> Vec<EdgeRecord> {
        let ingested: std::collections::HashSet<&str> = outcome
            .ingested
            .iter()
            .map(|entry| entry.item_id.as_str())
            .collect();

        let mut edges = Vec::new();
        for item_id in &ingested {
            let hits = GraphStore::neighbors(
                commonplace.store(),
                NeighborQuery::out(*item_id).with_edge_type(SIMILAR_TO_EDGE),
            );
            for hit in hits {
                // Scope to edges between items ingested in THIS change set.
                if !ingested.contains(hit.node_id.as_str()) {
                    continue;
                }
                edges.push(
                    EdgeRecord::new(
                        hit.edge_id,
                        (*item_id).to_string(),
                        SIMILAR_TO_EDGE,
                        hit.node_id,
                        json!({}),
                    )
                    // The similarity score rides the edge confidence (F2 writes
                    // `SIMILAR_TO` with `.with_confidence(score)`); carry it so a
                    // weight-aware algorithm could use it.
                    .with_confidence(hit.confidence.unwrap_or(1.0)),
                );
            }
        }
        edges
    }
}

impl AmbientPass for OffloadPass {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn run(
        &self,
        _change_set: &ChangeSet,
        outcome: &IngestOutcome,
        commonplace: &mut SidecarCommonplace,
    ) -> GraphStoreResult<PassReceipt> {
        // The derivation the ingest triggers: centrality over the just-ingested
        // SIMILAR_TO subgraph.
        let edges = self.similar_subgraph(outcome, commonplace);
        let item_count = outcome.ingested.len();

        // No subgraph to compute over (single item, or nothing similar among the
        // ingested items): there is no graph-algorithm derivation to offload for
        // this change set. Honest NotApplicable, not a degenerate result.
        if edges.is_empty() {
            return Ok(PassReceipt {
                pass: Self::NAME.to_string(),
                status: PassStatus::NotApplicable,
                applicable_inputs: Vec::new(),
                evidence_ids: Vec::new(),
                produced: json!({
                    "reason": "no SIMILAR_TO edges among the ingested items; no graph-algorithm derivation to offload",
                    "ingested_item_count": item_count,
                }),
            });
        }

        // The induced subgraph nodes (the centrality cost grows with the graph,
        // so the planner's row estimate is scaled by the node count).
        let node_ids: std::collections::BTreeSet<String> = edges
            .iter()
            .flat_map(|edge| [edge.from_id.clone(), edge.to_id.clone()])
            .collect();
        let node_count = node_ids.len();

        // Route the centrality derivation through the compute-offload planner as
        // a GraphPageRank operation. A graph op is CPU-symbolic, so the planner
        // picks the exact `graph.pagerank` CPU affordance over the expensive-model
        // baseline; the plan's totals bank the GPU-seconds the offload saved.
        let operation = Operation {
            operation_id: "ingested_item_similarity_centrality".to_string(),
            kind: OperationKind::GraphPageRank,
            description: "centrality over the just-ingested SIMILAR_TO item subgraph".to_string(),
            estimated_rows: node_count.max(1) as u64,
            quality_floor: 1.0,
            ..Operation::new("", OperationKind::GraphPageRank)
        };
        let plan = OperationPlanner::new(PlannerConfig::default()).plan(vec![operation]);
        let step = plan
            .steps
            .first()
            .expect("planner returns one step for one operation");
        let selected_executor = step.selected.executor.clone();
        let selected_affordance = step.selected.affordance_id.clone();

        // Execute the real exact PageRank over the induced subgraph through the
        // wired CPU graph affordance, then read the most central item off its
        // payload. The snapshot is the scoped subgraph: the ingested items that
        // participate in a SIMILAR_TO edge among the change set, plus those edges.
        let snapshot = GraphSnapshot {
            version: plan.graph_version,
            nodes: node_ids
                .iter()
                .map(|id| NodeRecord::new(id, ["Item"], json!({ "id": id })))
                .collect(),
            edges: edges.clone(),
        };
        let pagerank = GraphAffordanceEngine::run(
            &snapshot,
            GraphAffordanceRequest::PageRank {
                damping: 0.85,
                max_iter: 100,
                tolerance: 1e-9,
            },
        );
        let top_node = pagerank.payload["scores"]
            .as_object()
            .and_then(|scores| {
                scores
                    .iter()
                    .filter_map(|(id, score)| score.as_f64().map(|s| (id.clone(), s)))
                    // Highest score wins; ties break on id for determinism.
                    .max_by(|left, right| {
                        left.1
                            .partial_cmp(&right.1)
                            .unwrap_or(std::cmp::Ordering::Equal)
                            .then_with(|| right.0.cmp(&left.0))
                    })
                    .map(|(id, _)| id)
            });

        // The planner routed the op to the CPU affordance (the offload), and the
        // affordance executed: a Produced receipt carrying the decision, the real
        // centrality result, and the gpu_seconds_saved the offload banked. If the
        // planner unexpectedly kept the op on a model executor, that is a genuine
        // routing regression -- surface it as Degraded, named, not faked.
        let routed_to_cpu = matches!(
            selected_executor,
            ExecutorKind::CpuAffordance | ExecutorKind::Cache
        );
        let status = if routed_to_cpu && top_node.is_some() {
            PassStatus::Produced
        } else if top_node.is_none() {
            PassStatus::Degraded(
                "exact PageRank affordance returned no scores for the subgraph".to_string(),
            )
        } else {
            PassStatus::Degraded(format!(
                "graph-algorithm op unexpectedly routed to {selected_executor:?} instead of a CPU affordance"
            ))
        };

        Ok(PassReceipt {
            pass: Self::NAME.to_string(),
            status,
            applicable_inputs: outcome
                .ingested
                .iter()
                .map(|entry| entry.relative_path.clone())
                .collect(),
            evidence_ids: top_node.iter().cloned().collect(),
            produced: json!({
                "operation": operation_id_of(&plan),
                "kind": "graph_page_rank",
                "route_affordance": COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID,
                "selected_executor": selected_executor,
                "affordance": selected_affordance,
                "gpu_seconds_saved": plan.totals.gpu_seconds_saved,
                "tokens_saved": plan.totals.tokens_saved,
                "node_count": node_count,
                "edge_count": edges.len(),
                "result_summary": json!({
                    "top_node": top_node,
                    "affordance_id": pagerank.affordance_id,
                    "scores": pagerank.payload["scores"].clone(),
                }),
            }),
        })
    }
}

/// The operation id of the single planned step (kept stable for the receipt's
/// `operation` field after the planner normalizes the operation).
fn operation_id_of(plan: &rustyred_thg_offload::OffloadPlan) -> String {
    plan.steps
        .first()
        .map(|step| step.operation.operation_id.clone())
        .unwrap_or_default()
}

/// DEGRADED standing-seed pass. No standing-query / standing-seed evaluation
/// mechanism exists in the core, so this records an `Unavailable` receipt naming
/// the gap instead of guessing.
#[derive(Clone, Copy, Debug, Default)]
pub struct StandingSeedPass;

impl StandingSeedPass {
    pub const NAME: &'static str = "standing_seed_evaluation";
    /// The capability this pass would call if it existed.
    pub const MISSING: &'static str = "rustyred-thg-core exposes no standing-query / \
standing-seed evaluation mechanism (all `seed` surfaces are PPR seed-node \
queries; all `standing` surfaces are epistemic claim standing); no API to \
evaluate standing seeds against a change set";
}

impl AmbientPass for StandingSeedPass {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn run(
        &self,
        _change_set: &ChangeSet,
        _outcome: &IngestOutcome,
        _commonplace: &mut SidecarCommonplace,
    ) -> GraphStoreResult<PassReceipt> {
        Ok(PassReceipt::unavailable(Self::NAME, Self::MISSING))
    }
}

/// Write a [`PassReceipt`] as a provenance node (deliverable 3) and link it to
/// the change-set lineage node it was produced for. Returns the receipt node id.
///
/// `change_set_node_id` is the `CommonplaceChangeSet` node id from the ingest
/// outcome (slice 2); when present, a `PRODUCED_FOR` edge connects the receipt
/// to it so receipts are reachable from the change-set history.
pub fn write_pass_receipt(
    commonplace: &mut SidecarCommonplace,
    receipt: &PassReceipt,
    change_set_node_id: Option<&str>,
    sequence: usize,
) -> GraphStoreResult<String> {
    let now = now_ms();
    // Deterministic-per-(time,pass,seq) id; sequence disambiguates two passes
    // that land in the same millisecond for one change set.
    let node_id = format!("commonplace:pass:{now:x}-{}-{sequence}", receipt.pass);

    let record = NodeRecord::new(
        node_id.clone(),
        [PASS_RECEIPT_LABEL],
        json!({
            "recorded_at_ms": now,
            "pass": receipt.pass,
            "status": receipt.status.tag(),
            "status_detail": receipt.status,
            "applicable_inputs": receipt.applicable_inputs,
            "evidence_ids": receipt.evidence_ids,
            "produced": receipt.produced,
            "change_set": change_set_node_id,
        }),
    );
    commonplace.store_mut().upsert_node(record)?;

    if let Some(change_set_id) = change_set_node_id {
        let edge = EdgeRecord::new(
            format!("produced_for:{node_id}:{change_set_id}"),
            &node_id,
            PRODUCED_FOR_EDGE,
            change_set_id,
            json!({ "pass": receipt.pass }),
        );
        commonplace.store_mut().upsert_edge(edge)?;
    }

    Ok(node_id)
}

/// What one full ambient cycle produced: the ingest outcome plus, for each
/// registered pass, its receipt and the id of the receipt node written. Returned
/// by [`AmbientRuntime::run_cycle`] so the runtime and tests can inspect a run;
/// [`ChangeSink::apply`] drops it.
#[derive(Clone, Debug, Default)]
pub struct AmbientCycleReport {
    /// The ingest outcome from slice 2 (items ingested + change-set node id).
    pub ingest: IngestOutcome,
    /// One `(receipt, receipt_node_id)` per registered pass, in registration
    /// order.
    pub passes: Vec<(PassReceipt, String)>,
}

/// The composite [`ChangeSink`] the watcher drives: on each settled change set it
/// ingests through the wrapped [`CommonplaceIngestSink`] (slice 2), then runs
/// every registered [`AmbientPass`] (this slice) against the same durable
/// sidecar store, persisting one provenance receipt per pass linked to the
/// change-set lineage node.
///
/// This is the thing slice 4 (`run()` on a thread in the desktop runtime) will
/// construct and hand to [`crate::run`]. Construct it with [`AmbientRuntime::new`]
/// and register passes with [`AmbientRuntime::with_pass`] / the
/// [`AmbientRuntime::default_passes`] convenience.
pub struct AmbientRuntime {
    sink: crate::sink::SharedSink,
    passes: Vec<Box<dyn AmbientPass>>,
}

impl AmbientRuntime {
    /// Build a runtime over an already-open ingest sink with no passes yet. The
    /// sink is shared so the control endpoint's read routes can query the SAME
    /// durable graph the watcher writes to (one process, one graph); use
    /// [`shared_sink`](Self::shared_sink) to get a read handle.
    pub fn new(sink: crate::sink::CommonplaceIngestSink) -> Self {
        Self::from_shared(crate::sink::SharedSink::new(sink))
    }

    /// Build a runtime over an already-shared sink (so a caller can keep a clone
    /// of the handle for the read routes before handing the runtime to the
    /// watcher).
    pub fn from_shared(sink: crate::sink::SharedSink) -> Self {
        Self {
            sink,
            passes: Vec::new(),
        }
    }

    /// The shared sink handle backing this runtime: clone it into
    /// [`ControlState`](crate::ControlState) so the data routes read the live
    /// durable graph the watcher writes to.
    pub fn shared_sink(&self) -> crate::sink::SharedSink {
        self.sink.clone()
    }

    /// Register a pass (builder style). Passes run in registration order.
    pub fn with_pass(mut self, pass: impl AmbientPass + 'static) -> Self {
        self.passes.push(Box::new(pass));
        self
    }

    /// Register the three standard ambient passes: reconstruction (real-wired),
    /// offload-eligible derivation (real-wired: exact-PageRank centrality over
    /// the ingested SIMILAR_TO subgraph), and standing-seed evaluation
    /// (degraded). This is the default trigger set slice 4 wires.
    pub fn default_passes(self) -> Self {
        self.with_pass(ReconstructionPass)
            .with_pass(OffloadPass)
            .with_pass(StandingSeedPass)
    }

    /// Lock and borrow the wrapped ingest sink (for queries / verification). The
    /// returned guard derefs to the inner [`CommonplaceIngestSink`]; do not hold it
    /// across an `await`.
    pub fn sink(&self) -> std::sync::MutexGuard<'_, crate::sink::CommonplaceIngestSink> {
        self.sink.lock()
    }

    /// Run one full cycle: ingest the change set, then run every registered pass
    /// and persist its receipt. The testable core of [`ChangeSink::apply`].
    ///
    /// The whole cycle holds the shared-sink lock once, so an ingest plus its
    /// passes commit atomically with respect to a concurrent read route, and the
    /// read routes see a fully-settled cycle rather than a half-ingested one.
    ///
    /// A pass that returns `Err` (a genuine store failure) aborts the cycle and
    /// surfaces the error; `NotApplicable` / `Unavailable` / `Degraded` are
    /// receipt statuses and never abort the loop.
    pub fn run_cycle(&mut self, change_set: &ChangeSet) -> GraphStoreResult<AmbientCycleReport> {
        let mut sink = self.sink.lock();
        let ingest = sink.ingest_change_set(change_set)?;
        let change_set_node_id = ingest.change_set_node_id.clone();

        let mut pass_reports = Vec::with_capacity(self.passes.len());
        for (sequence, pass) in self.passes.iter().enumerate() {
            let receipt = pass.run(change_set, &ingest, sink.commonplace_mut())?;
            let node_id = write_pass_receipt(
                sink.commonplace_mut(),
                &receipt,
                change_set_node_id.as_deref(),
                sequence,
            )?;
            pass_reports.push((receipt, node_id));
        }

        Ok(AmbientCycleReport {
            ingest,
            passes: pass_reports,
        })
    }
}

impl crate::ChangeSink for AmbientRuntime {
    fn apply(&mut self, change_set: ChangeSet) {
        // Same swallow-after-stderr contract as the ingest sink: a store error in
        // the watcher thread must not poison the watch loop. Richer error routing
        // (a degraded-state receipt back to the runtime) is a later-slice concern.
        if let Err(error) = self.run_cycle(&change_set) {
            eprintln!("commonplace-desktop-runtime: ambient cycle failed: {error:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_extension_detection() {
        assert!(has_artifact_extension(Path::new("/repo/target/app.o")));
        assert!(has_artifact_extension(Path::new("/repo/lib.so")));
        assert!(has_artifact_extension(Path::new("/repo/a.OUT"))); // case-insensitive
        assert!(!has_artifact_extension(Path::new("/repo/src/main.rs")));
        assert!(!has_artifact_extension(Path::new("/repo/README.md")));
        assert!(!has_artifact_extension(Path::new("/repo/noext")));
    }

    #[test]
    fn artifact_magic_detection() {
        assert!(has_artifact_magic(b"\x7fELF\x02\x01\x01"));
        assert!(has_artifact_magic(b"\x00asm\x01\x00\x00\x00"));
        assert!(has_artifact_magic(b"MZ\x90\x00"));
        assert!(!has_artifact_magic(b"fn main() {}"));
        assert!(!has_artifact_magic(b"MX")); // near-miss
        assert!(!has_artifact_magic(b"")); // empty
    }

    /// Open an in-memory-graph sidecar commonplace for an in-process pass test.
    fn test_commonplace() -> (tempfile::TempDir, SidecarCommonplace) {
        let dir = tempfile::tempdir().unwrap();
        let blobs = DiskObjectStore::open(dir.path()).unwrap();
        let commonplace = SidecarCommonplace::new(RedCoreGraphStore::memory(), blobs);
        (dir, commonplace)
    }

    /// Seed an `Item` node into the sidecar (mirrors how the commonplace store
    /// writes items: an `Item`-labelled node keyed by id).
    fn seed_item(commonplace: &mut SidecarCommonplace, id: &str) {
        commonplace
            .store_mut()
            .upsert_node(NodeRecord::new(id, ["Item"], json!({ "id": id })))
            .unwrap();
    }

    /// Seed a `SIMILAR_TO` edge `from -> to` with a similarity score on the edge
    /// confidence, exactly as F2 writes it.
    fn seed_similar(commonplace: &mut SidecarCommonplace, from: &str, to: &str, score: f64) {
        let edge = EdgeRecord::new(
            format!("similar:{from}:{to}"),
            from,
            SIMILAR_TO_EDGE,
            to,
            json!({ "score": score }),
        )
        .with_confidence(score);
        commonplace.store_mut().upsert_edge(edge).unwrap();
    }

    /// Build an [`IngestOutcome`] whose `ingested` items have the given ids (the
    /// other fields are not read by the offload pass).
    fn outcome_for(item_ids: &[&str]) -> IngestOutcome {
        IngestOutcome {
            ingested: item_ids
                .iter()
                .enumerate()
                .map(|(i, id)| crate::sink::IngestedPath {
                    relative_path: format!("note_{i}.md"),
                    content_hash: format!("sha256:{id}"),
                    item_id: (*id).to_string(),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn artifact_extension_and_magic_unchanged() {
        // (sanity: the reconstruction detectors are untouched by the offload wiring)
        assert!(has_artifact_extension(Path::new("/repo/lib.so")));
        assert!(has_artifact_magic(b"\x7fELF\x02"));
    }

    #[test]
    fn standing_seed_pass_names_its_gap() {
        // StandingSeedPass remains the honest degraded pass: no standing-seed
        // evaluator exists in the core, so it records an Unavailable receipt.
        let (_dir, mut commonplace) = test_commonplace();
        let receipt = StandingSeedPass
            .run(&ChangeSet::default(), &IngestOutcome::default(), &mut commonplace)
            .unwrap();
        assert_eq!(receipt.pass, StandingSeedPass::NAME);
        assert!(matches!(receipt.status, PassStatus::Unavailable(_)));
        assert_eq!(receipt.status.tag(), "unavailable");
    }

    #[test]
    fn offload_pass_produces_real_centrality_receipt_with_gpu_seconds_saved() {
        // Seed a star of similar items in the sidecar: three leaves all
        // SIMILAR_TO one hub. The offload pass must read this subgraph, route
        // the centrality derivation through the offload engine, run the wired
        // exact-PageRank affordance, and Produce a receipt naming the hub as the
        // most central item with a positive gpu_seconds_saved.
        let (_dir, mut commonplace) = test_commonplace();
        for id in ["hub", "leaf_a", "leaf_b", "leaf_c"] {
            seed_item(&mut commonplace, id);
        }
        seed_similar(&mut commonplace, "leaf_a", "hub", 0.9);
        seed_similar(&mut commonplace, "leaf_b", "hub", 0.8);
        seed_similar(&mut commonplace, "leaf_c", "hub", 0.7);

        let outcome = outcome_for(&["hub", "leaf_a", "leaf_b", "leaf_c"]);
        let receipt = OffloadPass
            .run(&ChangeSet::default(), &outcome, &mut commonplace)
            .unwrap();

        assert_eq!(receipt.pass, OffloadPass::NAME);
        assert_eq!(
            receipt.status,
            PassStatus::Produced,
            "the planner routed the centrality op to the CPU affordance and it ran"
        );

        // The planner routed the graph op to the exact CPU PageRank affordance
        // (the offload), not a model executor.
        assert_eq!(
            receipt.produced["selected_executor"],
            json!("cpu_affordance"),
            "an exact graph op must route to the CPU affordance, not a model"
        );
        assert_eq!(receipt.produced["affordance"], json!("graph.pagerank"));
        assert_eq!(receipt.produced["kind"], json!("graph_page_rank"));
        assert_eq!(
            receipt.produced["route_affordance"],
            json!(COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID)
        );

        // A REAL PageRank result: the hub is the most central item.
        assert_eq!(
            receipt.produced["result_summary"]["top_node"],
            json!("hub"),
            "PageRank must rank the star hub highest"
        );
        assert_eq!(receipt.evidence_ids, vec!["hub".to_string()]);

        // The monetization metric is recorded and positive: routing the exact
        // graph op to CPU banks the expensive-model baseline's GPU-seconds.
        let saved = receipt.produced["gpu_seconds_saved"].as_f64().unwrap();
        assert!(saved > 0.0, "offload must record a positive gpu_seconds_saved");
        let tokens_saved = receipt.produced["tokens_saved"].as_f64().unwrap();
        assert!(
            tokens_saved > 0.0,
            "offloading off the model baseline must bank tokens too"
        );
    }

    #[test]
    fn offload_pass_not_applicable_without_similar_subgraph() {
        // A single ingested item with no SIMILAR_TO edges among the ingested set:
        // there is no graph-algorithm derivation to offload. Honest
        // NotApplicable, not a degenerate Produced.
        let (_dir, mut commonplace) = test_commonplace();
        seed_item(&mut commonplace, "lonely");
        let outcome = outcome_for(&["lonely"]);

        let receipt = OffloadPass
            .run(&ChangeSet::default(), &outcome, &mut commonplace)
            .unwrap();
        assert_eq!(receipt.status, PassStatus::NotApplicable);
        assert_eq!(receipt.produced["ingested_item_count"], json!(1));
    }

    #[test]
    fn offload_pass_scopes_to_ingested_items() {
        // SIMILAR_TO edges to items NOT ingested this change set are excluded, so
        // the centrality op is scoped to the change. Seed an edge from an
        // ingested item to a non-ingested one -> no in-scope subgraph ->
        // NotApplicable.
        let (_dir, mut commonplace) = test_commonplace();
        seed_item(&mut commonplace, "ingested");
        seed_item(&mut commonplace, "older_item");
        seed_similar(&mut commonplace, "ingested", "older_item", 0.95);

        let outcome = outcome_for(&["ingested"]);
        let receipt = OffloadPass
            .run(&ChangeSet::default(), &outcome, &mut commonplace)
            .unwrap();
        assert_eq!(
            receipt.status,
            PassStatus::NotApplicable,
            "an edge to a non-ingested item is out of scope for this change set"
        );
    }
}
