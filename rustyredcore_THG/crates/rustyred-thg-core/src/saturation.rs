use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use egg::{Id, RecExpr, Runner, SymbolLang};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::epistemic::{
    same_eclass_edge_id, EGRAPH_EPISTEMIC_ENGINE, EPISTEMIC_SHADOW_LABEL, HAS_EPISTEMIC_SHADOW,
    SAME_ECLASS,
};
use crate::graph_store::{
    now_ms, EdgeRecord, EpistemicType, GraphMutation, GraphMutationBatch, GraphStore,
    GraphStoreResult, NeighborQuery, NodeQuery, NodeRecord, Provenance,
};
use crate::hooks::{
    HookContext, HookError, HookOutcome, HookRegistration, MutationEvent, MutationKind,
    MutationMatcher,
};
use crate::plugin::{PluginCapability, PluginCapabilityKind, RustyRedPlugin};
use crate::state::stable_hash;
use crate::statement::{
    literal_ref, statement_id as hyper_statement_id, statement_incidence_edges,
    StatementFieldProvenance, StatementProvenance, StatementRecord, StatementSemiring, HAS_OBJECT,
    HAS_SUBJECT, STATEMENT_LABEL,
};
use crate::symbolic::{derive_datalog_receipt, stable_hash_value};
use crate::versioned_graph::{
    compile_graph_pack, compile_graph_pack_incremental, CommitCost, GraphCompileOptions,
};

pub const SATURATION_ENGINE: &str = "egglog-saturation";
pub const SATURATION_ENGINE_VERSION: &str = "egglog-saturation-v1";
pub const SATURATION_DERIVED_STATEMENT_LABEL: &str = "SaturationDerivedStatement";
pub const SATURATION_DERIVES_EDGE: &str = "SaturationDerives";
pub const SATURATION_SHARED_RULE_IDS: [&str; 3] = [
    "dependent_claim",
    "evidence_path_too_long",
    "object_in_unresolved_tension_neighborhood",
];

const SATURATION_FACT_LIMIT: usize = 100_000;
const SUPPORT_REACHABLE_RULE: &str = "support_reachability";
const CONTRADICTION_REACHES_RULE: &str = "contradiction_propagation";
const SUPPORT_ATTENUATION: f64 = 0.9;
const CONTRADICTION_ATTENUATION: f64 = 0.9;
const MAX_SATURATION_PATH_LEN: usize = 32;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SaturationBackend {
    /// Parse and run the public rule program through the real `egglog` crate.
    /// The graph bridge still reads closure rows through the deterministic
    /// Rust extractor below until the crate exposes a stable row API for this
    /// use case.
    Egglog,
    /// The spec's named fallback path: use the existing `egg` substrate plus an
    /// explicit monotone fixpoint loop if the egglog embedding becomes
    /// unavailable.
    EggFallback,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SaturationProgram {
    pub backend: SaturationBackend,
    pub engine: String,
    pub engine_version: String,
    pub program: String,
    pub shared_rule_ids: Vec<String>,
}

impl Default for SaturationProgram {
    fn default() -> Self {
        Self {
            backend: SaturationBackend::Egglog,
            engine: SATURATION_ENGINE.to_string(),
            engine_version: SATURATION_ENGINE_VERSION.to_string(),
            program: default_egglog_program(),
            shared_rule_ids: SATURATION_SHARED_RULE_IDS
                .iter()
                .map(|rule| (*rule).to_string())
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SaturationConfig {
    pub engine: String,
    pub engine_version: String,
    pub computed_at: i64,
    pub component_key: Option<String>,
    pub retracted_fact_ids: Vec<String>,
    pub prune_stale: bool,
}

impl Default for SaturationConfig {
    fn default() -> Self {
        Self {
            engine: SATURATION_ENGINE.to_string(),
            engine_version: SATURATION_ENGINE_VERSION.to_string(),
            computed_at: now_ms(),
            component_key: None,
            retracted_fact_ids: Vec::new(),
            prune_stale: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SaturationFacts {
    pub component_key: String,
    pub content_node_ids: Vec<String>,
    pub nodes: Vec<NodeRecord>,
    pub edges: Vec<EdgeRecord>,
    pub facts: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SaturationContributor {
    pub dependency_fact_ids: Vec<String>,
    pub confidence: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SaturationDerivedStatement {
    pub fact_id: String,
    pub rule_id: String,
    pub relation: String,
    pub subject_id: String,
    pub reason: String,
    pub dependency_fact_ids: Vec<String>,
    pub contributors: Vec<SaturationContributor>,
    pub attributes: Value,
    pub confidence: f64,
    pub writeback_policy: String,
    pub engine: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SaturationEquivalenceClass {
    pub class_id: String,
    pub canonical_form: String,
    pub representative_content_id: String,
    pub representative_shadow_id: String,
    pub member_content_ids: Vec<String>,
    pub member_shadow_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SaturationClosure {
    pub component_key: String,
    pub derived_statements: Vec<SaturationDerivedStatement>,
    pub equivalence_classes: Vec<SaturationEquivalenceClass>,
    pub iterations: usize,
    pub egglog_tuple_count: usize,
    pub egglog_output_count: usize,
    pub egglog_error: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SaturationRevisionReport {
    pub stale_nodes_tombstoned: usize,
    pub stale_edges_tombstoned: usize,
    pub retracted_fact_ids: Vec<String>,
    pub commit_cost: Option<CommitCost>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SaturationReport {
    pub component_key: String,
    pub derived_statement_count: usize,
    pub equivalence_class_count: usize,
    pub fixpoint_iterations: usize,
    pub nodes_written: usize,
    pub edges_written: usize,
    pub same_eclass_edges_written: usize,
    pub idempotent_skips: usize,
    pub revision: SaturationRevisionReport,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DifferentialReport {
    pub engine: String,
    pub reference_engine: String,
    pub shared_rule_ids: Vec<String>,
    pub expected_count: usize,
    pub actual_count: usize,
    pub missing_fact_ids: Vec<String>,
    pub extra_fact_ids: Vec<String>,
}

#[derive(Clone, Debug)]
struct SupportEdge {
    from_id: String,
    to_id: String,
    confidence: f64,
    dependency_fact_id: String,
}

#[derive(Clone, Debug)]
struct ContradictionEdge {
    from_id: String,
    to_id: String,
    confidence: f64,
    dependency_fact_id: String,
}

#[derive(Clone, Debug)]
struct PathState {
    from_id: String,
    to_id: String,
    path_length: usize,
    confidence: f64,
    dependency_fact_ids: Vec<String>,
}

struct StatementSeed<'a> {
    rule_id: &'a str,
    relation: &'a str,
    subject_id: &'a str,
    reason: &'a str,
    dependency_fact_ids: Vec<String>,
    attributes: Value,
    confidence: f64,
    writeback_policy: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ClaimForm {
    prop: String,
    neg_count: usize,
}

impl ClaimForm {
    fn negated(&self) -> bool {
        self.neg_count % 2 == 1
    }

    fn canonical_label(&self) -> String {
        if self.negated() {
            format!("not {}", self.prop)
        } else {
            self.prop.clone()
        }
    }

    fn claim_term(&self) -> String {
        let mut term = format!("p_{}", self.prop.replace(' ', "_"));
        for _ in 0..self.neg_count {
            term = format!("(not {term})");
        }
        term
    }
}

struct EqualityCandidate {
    shadow_id: String,
    content_id: String,
    form: ClaimForm,
    term: RecExpr<SymbolLang>,
}

pub fn facts_from_subgraph<S: GraphStore>(
    store: &S,
    content_node_ids: &[String],
) -> SaturationFacts {
    let mut content_ids: BTreeSet<String> = content_node_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    let full_store = content_ids.is_empty();

    if full_store {
        for node in store
            .query_nodes(NodeQuery::default().with_limit(SATURATION_FACT_LIMIT))
            .into_iter()
        {
            if is_saturation_artifact_node(&node) {
                continue;
            }
            if !node
                .labels
                .iter()
                .any(|label| label == EPISTEMIC_SHADOW_LABEL)
            {
                content_ids.insert(node.id);
            }
        }
    }

    let mut node_ids = content_ids.clone();
    let mut edge_ids: BTreeSet<String> = BTreeSet::new();
    for id in &content_ids {
        for hit in store
            .neighbors(NeighborQuery::out(id).with_include_expired(true))
            .into_iter()
            .chain(store.neighbors(NeighborQuery::in_(id).with_include_expired(true)))
        {
            if hit.edge_type == SATURATION_DERIVES_EDGE {
                continue;
            }
            edge_ids.insert(hit.edge_id.clone());
            node_ids.insert(hit.node_id);
        }
        for hit in store
            .neighbors(
                NeighborQuery::out(id)
                    .with_edge_type(HAS_EPISTEMIC_SHADOW)
                    .with_include_expired(true),
            )
            .into_iter()
        {
            edge_ids.insert(hit.edge_id.clone());
            node_ids.insert(hit.node_id);
        }
    }

    if full_store {
        for shadow in store
            .query_nodes(NodeQuery::label(EPISTEMIC_SHADOW_LABEL).with_limit(SATURATION_FACT_LIMIT))
            .into_iter()
        {
            node_ids.insert(shadow.id);
        }
    }

    let mut nodes = Vec::new();
    for id in node_ids {
        if let Some(node) = store.get_node(&id) {
            if !is_saturation_artifact_node(node) {
                nodes.push(node.clone());
            }
        }
    }
    nodes.sort_by(|left, right| left.id.cmp(&right.id));

    let mut edges = Vec::new();
    for id in edge_ids {
        if let Some(edge) = store.get_edge(&id) {
            if !is_saturation_artifact_edge(edge) {
                edges.push(edge.clone());
            }
        }
    }
    edges.sort_by(|left, right| left.id.cmp(&right.id));

    let facts = datalog_facts_from_graph_records(&nodes, &edges);
    let content_node_ids = content_ids.into_iter().collect::<Vec<_>>();
    let component_key = if full_store {
        "full-store".to_string()
    } else {
        format!("component:{}", stable_hash(&content_node_ids))
    };

    SaturationFacts {
        component_key,
        content_node_ids,
        nodes,
        edges,
        facts,
    }
}

pub fn facts_from_payload(payload: &Value) -> Result<SaturationFacts, String> {
    let raw_facts = payload_facts(payload)?;
    let mut facts = Vec::new();
    for raw in raw_facts {
        facts.push(normalize_datalog_fact(raw)?);
    }
    facts.sort_by(|left, right| object_field(left, "fact_id").cmp(object_field(right, "fact_id")));
    Ok(SaturationFacts {
        component_key: format!("payload:{}", stable_hash(&facts)),
        content_node_ids: Vec::new(),
        nodes: Vec::new(),
        edges: Vec::new(),
        facts,
    })
}

pub fn run_saturation(facts: SaturationFacts, program: &SaturationProgram) -> SaturationClosure {
    let egglog_report = run_egglog_program(&facts, program);
    let mut statements: BTreeMap<String, SaturationDerivedStatement> = BTreeMap::new();
    let facts_by_relation = facts_by_relation(&facts.facts);

    derive_shared_subset(&facts_by_relation, &mut statements, program);
    let support_edges = support_edges_from_facts(&facts.facts);
    derive_support_fixpoint(&support_edges, &mut statements, program);
    let contradiction_edges = contradiction_edges_from_facts(&facts.facts);
    let iterations = derive_contradiction_fixpoint(
        &support_edges,
        &contradiction_edges,
        &mut statements,
        program,
    )
    .max(derive_support_iterations(&support_edges));
    let equivalence_classes = derive_equivalence_classes(&facts);

    SaturationClosure {
        component_key: facts.component_key,
        derived_statements: statements.into_values().collect(),
        equivalence_classes,
        iterations,
        egglog_tuple_count: egglog_report
            .as_ref()
            .map(|report| report.tuple_count)
            .unwrap_or(0),
        egglog_output_count: egglog_report
            .as_ref()
            .map(|report| report.output_count)
            .unwrap_or(0),
        egglog_error: egglog_report.err(),
    }
}

pub fn validate_egglog_program(program: &SaturationProgram) -> Result<(usize, usize), String> {
    let facts = SaturationFacts {
        component_key: "egglog-validation".to_string(),
        content_node_ids: Vec::new(),
        nodes: Vec::new(),
        edges: Vec::new(),
        facts: Vec::new(),
    };
    let report = run_egglog_program(&facts, program)?;
    Ok((report.tuple_count, report.output_count))
}

pub fn materialize_closure<S: GraphStore>(
    store: &mut S,
    closure: SaturationClosure,
    mut config: SaturationConfig,
) -> GraphStoreResult<SaturationReport> {
    if config.component_key.is_none() {
        config.component_key = Some(closure.component_key.clone());
    }
    let component_key = config
        .component_key
        .clone()
        .unwrap_or_else(|| closure.component_key.clone());
    let before_pack = store
        .graph_snapshot()
        .ok()
        .map(|snapshot| compile_graph_pack(&snapshot, graph_compile_options("before saturation")));
    let mut mutations = Vec::new();

    let mut report = SaturationReport {
        component_key: component_key.clone(),
        derived_statement_count: closure.derived_statements.len(),
        equivalence_class_count: closure.equivalence_classes.len(),
        fixpoint_iterations: closure.iterations,
        ..SaturationReport::default()
    };

    let live_statement_ids = closure
        .derived_statements
        .iter()
        .map(|statement| derived_statement_hyper_id(store, statement))
        .collect::<BTreeSet<_>>();
    if config.prune_stale {
        let revision = prune_stale_derived_artifacts(
            store,
            &component_key,
            &live_statement_ids,
            &config.retracted_fact_ids,
            &mut mutations,
        )?;
        report.revision.stale_nodes_tombstoned = revision.0;
        report.revision.stale_edges_tombstoned = revision.1;
        report.revision.retracted_fact_ids = sorted_strings(config.retracted_fact_ids.clone());
    }

    for class in &closure.equivalence_classes {
        let edge_count = materialize_same_eclass(store, class, &config, &mut mutations)?;
        report.same_eclass_edges_written += edge_count;
        report.edges_written += edge_count;
        report.idempotent_skips += class.member_shadow_ids.len().saturating_sub(1) - edge_count;
    }

    for statement in &closure.derived_statements {
        let node = derived_statement_node(store, statement, &component_key, &config);
        match upsert_node_if_changed(store, node.clone())? {
            WriteDisposition::Written => {
                report.nodes_written += 1;
                mutations.push(GraphMutation::NodeUpsert(node.clone()));
            }
            WriteDisposition::Skipped => report.idempotent_skips += 1,
        }

        for edge in statement_incidence_edges(&node) {
            if store.get_node(&edge.to_id).is_none() {
                continue;
            }
            match upsert_edge_if_changed(store, edge.clone())? {
                WriteDisposition::Written => {
                    report.edges_written += 1;
                    mutations.push(GraphMutation::EdgeUpsert(edge));
                }
                WriteDisposition::Skipped => report.idempotent_skips += 1,
            }
        }

        if store.get_node(&statement.subject_id).is_some() {
            let edge = derived_statement_edge(statement, &node.id, &config);
            match upsert_edge_if_changed(store, edge.clone())? {
                WriteDisposition::Written => {
                    report.edges_written += 1;
                    mutations.push(GraphMutation::EdgeUpsert(edge));
                }
                WriteDisposition::Skipped => report.idempotent_skips += 1,
            }
        }
    }

    if let Some(before_pack) = before_pack.filter(|_| !mutations.is_empty()) {
        let incremental = compile_graph_pack_incremental(
            &before_pack,
            &GraphMutationBatch::new(mutations),
            graph_compile_options("saturation materialized view"),
        );
        report.revision.commit_cost = Some(incremental.commit_cost);
    }

    Ok(report)
}

pub fn differential_check(payload: &Value) -> Result<DifferentialReport, String> {
    let reference_payload = shared_subset_payload(payload)?;
    let receipt = derive_datalog_receipt(&reference_payload)?;
    let expected = fact_ids_from_receipt(&receipt);
    let facts = facts_from_payload(&reference_payload)?;
    let closure = run_saturation(facts, &SaturationProgram::default());
    if let Some(error) = &closure.egglog_error {
        return Err(format!(
            "egglog saturation failed differential preflight: {error}"
        ));
    }
    let actual = closure
        .derived_statements
        .iter()
        .filter(|statement| SATURATION_SHARED_RULE_IDS.contains(&statement.rule_id.as_str()))
        .map(|statement| statement.fact_id.clone())
        .collect::<BTreeSet<_>>();

    let missing_fact_ids = expected
        .difference(&actual)
        .cloned()
        .collect::<Vec<String>>();
    let extra_fact_ids = actual
        .difference(&expected)
        .cloned()
        .collect::<Vec<String>>();
    let report = DifferentialReport {
        engine: SATURATION_ENGINE.to_string(),
        reference_engine: object_field(&receipt, "engine").to_string(),
        shared_rule_ids: SATURATION_SHARED_RULE_IDS
            .iter()
            .map(|rule| (*rule).to_string())
            .collect(),
        expected_count: expected.len(),
        actual_count: actual.len(),
        missing_fact_ids,
        extra_fact_ids,
    };
    if !report.missing_fact_ids.is_empty() {
        return Err(format!(
            "egglog saturation differential check missing {} fact(s): {:?}",
            report.missing_fact_ids.len(),
            report.missing_fact_ids
        ));
    }
    Ok(report)
}

pub fn coalesce_by_subgraph(_event: &MutationEvent) -> Option<String> {
    Some("saturation-subgraph".to_string())
}

pub fn saturation_hook_registration() -> HookRegistration {
    HookRegistration::new(
        "egglog-saturation",
        MutationMatcher::any()
            .with_kinds([
                MutationKind::NodeUpserted,
                MutationKind::EdgeUpserted,
                MutationKind::NodeDeleted,
                MutationKind::EdgeDeleted,
            ])
            .with_labels([
                "Claim",
                "Evidence",
                "EvidenceLink",
                "ClaimDependency",
                "CONTRADICTS",
                "Contradicts",
                "Undercuts",
                "UNDERCUTS",
                "SUPPORTS",
                "Supports",
            ]),
        coalesce_by_subgraph,
        Arc::new(saturation_handler),
    )
}

pub fn saturation_handler(
    ctx: &mut HookContext,
    events: &[MutationEvent],
) -> Result<HookOutcome, HookError> {
    let has_delete = events.iter().any(|event| {
        matches!(
            event.kind,
            MutationKind::NodeDeleted | MutationKind::EdgeDeleted
        )
    });
    let mut ids = Vec::new();
    if !has_delete {
        let mut seen = BTreeSet::new();
        for event in events {
            match event.kind {
                MutationKind::NodeUpserted => {
                    if seen.insert(event.id.clone()) {
                        ids.push(event.id.clone());
                    }
                }
                MutationKind::EdgeUpserted => {
                    if let Some(edge) = GraphStore::get_edge(ctx.store, &event.id).cloned() {
                        for id in [edge.from_id, edge.to_id] {
                            if seen.insert(id.clone()) {
                                ids.push(id);
                            }
                        }
                    }
                }
                MutationKind::NodeDeleted | MutationKind::EdgeDeleted => {}
            }
        }
    }

    let facts = facts_from_subgraph(ctx.store, &ids);
    if !facts.facts.is_empty() {
        let payload =
            json!({ "facts": facts.facts.clone(), "rule_ids": SATURATION_SHARED_RULE_IDS });
        differential_check(&payload)?;
    }
    let closure = run_saturation(facts, &SaturationProgram::default());
    if let Some(error) = &closure.egglog_error {
        return Err(HookError::from(format!(
            "egglog saturation failed before materialization: {error}"
        )));
    }
    let report = materialize_closure(
        ctx.store,
        closure,
        SaturationConfig {
            computed_at: now_ms(),
            retracted_fact_ids: if has_delete {
                events.iter().map(|event| event.id.clone()).collect()
            } else {
                Vec::new()
            },
            ..SaturationConfig::default()
        },
    )?;
    let mutations = report.nodes_written
        + report.edges_written
        + report.revision.stale_nodes_tombstoned
        + report.revision.stale_edges_tombstoned;
    if mutations == 0 {
        Ok(HookOutcome::Done)
    } else {
        Ok(HookOutcome::Wrote { mutations })
    }
}

#[derive(Clone, Debug, Default)]
pub struct SaturationPlugin;

impl RustyRedPlugin for SaturationPlugin {
    fn name(&self) -> &'static str {
        "egglog-saturation"
    }

    fn capabilities(&self) -> Vec<PluginCapability> {
        vec![PluginCapability {
            kind: PluginCapabilityKind::Hook,
            name: "egglog-saturation".to_string(),
        }]
    }

    fn hooks(&self) -> Vec<HookRegistration> {
        vec![saturation_hook_registration()]
    }
}

fn default_egglog_program() -> String {
    r#"
; Unified egglog rule program. Runtime facts are appended by the graph bridge
; before `(run ...)`; the Rust closure reader materializes the resulting view.
(datatype Prop
  (Atom String)
  (Not Prop)
  (Shadow String Prop))

(datatype SaturationStmt (SatStmt String String String))
(function stmt-confidence (SaturationStmt) f64 :merge (max old new))

(relation support-edge (String String f64 String))
(relation contradiction-edge (String String f64 String))
(relation support-reachable (String String))
(relation contradiction-reaches (String String))

(function support-confidence (String String) f64 :merge (max old new))
(function contradiction-confidence (String String) f64 :merge (max old new))

(ruleset equivalence)
(ruleset derivation)

(rewrite (Not (Not x)) x :ruleset equivalence)

(rule ((support-edge a b confidence dependency))
      ((support-reachable a b)
       (set (support-confidence a b) confidence))
      :ruleset derivation
      :name "support-base")

(rule ((support-reachable a b)
       (= prior (support-confidence a b))
       (support-edge b c next dependency))
      ((support-reachable a c)
       (set (support-confidence a c) (* (* prior next) 0.9)))
      :ruleset derivation
      :name "support-transitive")

(rule ((contradiction-edge a b confidence dependency))
      ((contradiction-reaches a b)
       (set (contradiction-confidence a b) confidence))
      :ruleset derivation
      :name "contradiction-base")

(rule ((contradiction-reaches a b)
       (= prior (contradiction-confidence a b))
       (support-edge b c next dependency))
      ((contradiction-reaches a c)
       (set (contradiction-confidence a c) (* (* prior next) 0.9)))
      :ruleset derivation
      :name "contradiction-propagates")
"#
    .trim()
    .to_string()
}

#[derive(Clone, Debug)]
struct EgglogRunReport {
    tuple_count: usize,
    output_count: usize,
}

fn run_egglog_program(
    facts: &SaturationFacts,
    program: &SaturationProgram,
) -> Result<EgglogRunReport, String> {
    if program.backend == SaturationBackend::EggFallback {
        return Ok(EgglogRunReport {
            tuple_count: 0,
            output_count: 0,
        });
    }
    let mut egraph = egglog::EGraph::default();
    let source = egglog_program_with_facts(facts, &program.program);
    let outputs = egraph
        .parse_and_run_program(Some("rustyred-thg-saturation.egg".to_string()), &source)
        .map_err(|err| err.to_string())?;
    Ok(EgglogRunReport {
        tuple_count: egraph.num_tuples(),
        output_count: outputs.len(),
    })
}

fn egglog_program_with_facts(facts: &SaturationFacts, program: &str) -> String {
    let mut source = String::with_capacity(program.len() + facts.facts.len() * 96 + 64);
    source.push_str(program.trim());
    source.push('\n');
    for line in egglog_fact_lines(facts) {
        source.push_str(&line);
        source.push('\n');
    }
    source.push_str(&format!("(run equivalence {MAX_SATURATION_PATH_LEN})\n"));
    source.push_str(&format!("(run derivation {MAX_SATURATION_PATH_LEN})\n"));
    source
}

fn egglog_fact_lines(facts: &SaturationFacts) -> Vec<String> {
    let mut lines = Vec::new();
    for edge in support_edges_from_facts(&facts.facts) {
        lines.push(format!(
            "(support-edge {} {} {} {})",
            egglog_string(&edge.from_id),
            egglog_string(&edge.to_id),
            egglog_f64(edge.confidence),
            egglog_string(&edge.dependency_fact_id)
        ));
    }
    for edge in contradiction_edges_from_facts(&facts.facts) {
        lines.push(format!(
            "(contradiction-edge {} {} {} {})",
            egglog_string(&edge.from_id),
            egglog_string(&edge.to_id),
            egglog_f64(edge.confidence),
            egglog_string(&edge.dependency_fact_id)
        ));
    }
    lines.extend(egglog_shadow_lines(facts));
    lines.sort();
    lines.dedup();
    lines
}

fn egglog_shadow_lines(facts: &SaturationFacts) -> Vec<String> {
    let nodes_by_id = facts
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut lines = Vec::new();
    for shadow in facts.nodes.iter().filter(|node| {
        node.labels
            .iter()
            .any(|label| label == EPISTEMIC_SHADOW_LABEL)
    }) {
        let content_id =
            prop_str(&shadow.properties, "content_node_id").unwrap_or_else(|| shadow.id.clone());
        let Some(content_node) = nodes_by_id.get(content_id.as_str()) else {
            continue;
        };
        let Some(form) = claim_logical_form(&claim_text(content_node)) else {
            continue;
        };
        let status = grounded_status_symbol(
            &prop_str(&shadow.properties, "grounded_extension_status")
                .unwrap_or_else(|| "undecided".to_string()),
        );
        lines.push(format!(
            "(let {} (Shadow {} {}))",
            egglog_symbol(&shadow.id),
            egglog_string(status),
            egglog_prop_term(&form)
        ));
    }
    lines
}

fn egglog_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn egglog_f64(value: f64) -> String {
    let value = if value.is_finite() { value } else { 0.0 };
    let mut rendered = format!("{value:.17}");
    while rendered.contains('.') && rendered.ends_with('0') {
        rendered.pop();
    }
    if rendered.ends_with('.') {
        rendered.push('0');
    }
    rendered
}

fn egglog_symbol(value: &str) -> String {
    let mut out = String::from("shadow_");
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

fn egglog_prop_term(form: &ClaimForm) -> String {
    let mut term = format!("(Atom {})", egglog_string(&form.prop));
    for _ in 0..form.neg_count {
        term = format!("(Not {term})");
    }
    term
}

fn derive_shared_subset(
    index: &BTreeMap<String, Vec<Value>>,
    statements: &mut BTreeMap<String, SaturationDerivedStatement>,
    program: &SaturationProgram,
) {
    for dep in relation(index, "claim_dependency") {
        let claim_id = attr_string(dep, "claim_id");
        if claim_id.is_empty() {
            continue;
        }
        if let Ok(statement) = exact_derived_statement(
            StatementSeed {
                rule_id: "dependent_claim",
                relation: "dependent_claim",
                subject_id: &claim_id,
                reason: "This claim depends on another graph object for its justification.",
                dependency_fact_ids: vec![object_field(dep, "fact_id").to_string()],
                attributes: json!({
                    "depends_on_object_id": attr_value_or(dep, "depends_on_object_id", json!("")),
                    "justification_type": attr_value_or(dep, "justification_type", json!("")),
                    "strength": attr_value_or(dep, "strength", json!(0.0)),
                }),
                confidence: 1.0,
                writeback_policy: "read-only",
            },
            &program.engine,
        ) {
            merge_statement(statements, statement);
        }
    }

    for path in relation(index, "evidence_path") {
        let path_length = integer_attr(path, "path_length", 0);
        let max_length = 3_i64;
        if path_length <= max_length {
            continue;
        }
        if let Ok(statement) = exact_derived_statement(
            StatementSeed {
                rule_id: "evidence_path_too_long",
                relation: "evidence_path_too_long",
                subject_id: object_field(path, "entity_id"),
                reason: "The evidence path exceeds the configured symbolic derivation depth.",
                dependency_fact_ids: vec![object_field(path, "fact_id").to_string()],
                attributes: json!({"path_length": path_length, "max_length": max_length}),
                confidence: 0.9,
                writeback_policy: "read-only",
            },
            &program.engine,
        ) {
            merge_statement(statements, statement);
        }
    }

    for edge in relation(index, "edge") {
        let edge_type = attr_string(edge, "edge_type").to_lowercase();
        let status = attr_string(edge, "acceptance_status").to_lowercase();
        if edge_type != "contradicts" && status != "contested" {
            continue;
        }
        for object_id in [
            attr_string(edge, "from_object_id"),
            attr_string(edge, "to_object_id"),
        ] {
            if object_id.is_empty() {
                continue;
            }
            if let Ok(statement) = exact_derived_statement(
                StatementSeed {
                    rule_id: "object_in_unresolved_tension_neighborhood",
                    relation: "object_in_unresolved_tension_neighborhood",
                    subject_id: &object_id,
                    reason: "This object is adjacent to a contradicting or contested edge.",
                    dependency_fact_ids: vec![object_field(edge, "fact_id").to_string()],
                    attributes: json!({
                        "edge_id": object_field(edge, "entity_id"),
                        "edge_type": edge_type,
                        "acceptance_status": status,
                    }),
                    confidence: 0.8,
                    writeback_policy: "read-only",
                },
                &program.engine,
            ) {
                merge_statement(statements, statement);
            }
        }
    }
}

fn derive_support_iterations(edges: &[SupportEdge]) -> usize {
    if edges.is_empty() {
        0
    } else {
        support_paths(edges).1
    }
}

fn derive_support_fixpoint(
    edges: &[SupportEdge],
    statements: &mut BTreeMap<String, SaturationDerivedStatement>,
    program: &SaturationProgram,
) {
    let (paths, _) = support_paths(edges);
    for path in paths {
        if path.path_length < 2 {
            continue;
        }
        let statement = saturation_statement(
            StatementSeed {
                rule_id: SUPPORT_REACHABLE_RULE,
                relation: "support_reachable",
                subject_id: &path.from_id,
                reason:
                    "Support reaches this target through an evidence or claim-dependency chain.",
                dependency_fact_ids: path.dependency_fact_ids,
                attributes: json!({
                    "target_id": path.to_id,
                    "path_length": path.path_length,
                }),
                confidence: path.confidence,
                writeback_policy: "read-only",
            },
            &program.engine,
        );
        merge_statement(statements, statement);
    }
}

fn derive_contradiction_fixpoint(
    support_edges: &[SupportEdge],
    contradiction_edges: &[ContradictionEdge],
    statements: &mut BTreeMap<String, SaturationDerivedStatement>,
    program: &SaturationProgram,
) -> usize {
    let mut iterations = 0;
    let support_by_from = support_edges.iter().fold(
        BTreeMap::<String, Vec<&SupportEdge>>::new(),
        |mut acc, edge| {
            acc.entry(edge.from_id.clone()).or_default().push(edge);
            acc
        },
    );
    let mut queue = VecDeque::new();
    let mut paths = Vec::new();
    for edge in contradiction_edges {
        let state = PathState {
            from_id: edge.from_id.clone(),
            to_id: edge.to_id.clone(),
            path_length: 1,
            confidence: edge.confidence,
            dependency_fact_ids: vec![edge.dependency_fact_id.clone()],
        };
        queue.push_back((state, vec![edge.from_id.clone(), edge.to_id.clone()]));
    }

    while let Some((state, path_nodes)) = queue.pop_front() {
        iterations = iterations.max(state.path_length);
        paths.push(state.clone());
        if state.path_length >= MAX_SATURATION_PATH_LEN {
            continue;
        }
        if let Some(next_edges) = support_by_from.get(&state.to_id) {
            for edge in next_edges {
                if path_nodes.contains(&edge.to_id) {
                    continue;
                }
                let mut dependency_fact_ids = state.dependency_fact_ids.clone();
                dependency_fact_ids.push(edge.dependency_fact_id.clone());
                dependency_fact_ids = sorted_strings(dependency_fact_ids);
                let next = PathState {
                    from_id: state.from_id.clone(),
                    to_id: edge.to_id.clone(),
                    path_length: state.path_length + 1,
                    confidence: state.confidence * edge.confidence * CONTRADICTION_ATTENUATION,
                    dependency_fact_ids,
                };
                let mut next_nodes = path_nodes.clone();
                next_nodes.push(edge.to_id.clone());
                queue.push_back((next, next_nodes));
            }
        }
    }

    for path in paths {
        let statement = saturation_statement(
            StatementSeed {
                rule_id: CONTRADICTION_REACHES_RULE,
                relation: "contradiction_reaches",
                subject_id: &path.from_id,
                reason: "A contradiction propagates across support and dependency reachability.",
                dependency_fact_ids: path.dependency_fact_ids,
                attributes: json!({
                    "target_id": path.to_id,
                    "path_length": path.path_length,
                }),
                confidence: path.confidence,
                writeback_policy: "read-only",
            },
            &program.engine,
        );
        merge_statement(statements, statement);
    }
    iterations
}

fn support_paths(edges: &[SupportEdge]) -> (Vec<PathState>, usize) {
    let mut by_from = BTreeMap::<String, Vec<&SupportEdge>>::new();
    for edge in edges {
        by_from.entry(edge.from_id.clone()).or_default().push(edge);
    }

    let mut queue = VecDeque::new();
    let mut paths = Vec::new();
    for edge in edges {
        let state = PathState {
            from_id: edge.from_id.clone(),
            to_id: edge.to_id.clone(),
            path_length: 1,
            confidence: edge.confidence,
            dependency_fact_ids: vec![edge.dependency_fact_id.clone()],
        };
        queue.push_back((state, vec![edge.from_id.clone(), edge.to_id.clone()]));
    }

    let mut iterations = 0;
    while let Some((state, path_nodes)) = queue.pop_front() {
        iterations = iterations.max(state.path_length);
        paths.push(state.clone());
        if state.path_length >= MAX_SATURATION_PATH_LEN {
            continue;
        }
        if let Some(next_edges) = by_from.get(&state.to_id) {
            for edge in next_edges {
                if state.dependency_fact_ids.contains(&edge.dependency_fact_id) {
                    continue;
                }
                if path_nodes.contains(&edge.to_id) {
                    continue;
                }
                let mut dependency_fact_ids = state.dependency_fact_ids.clone();
                dependency_fact_ids.push(edge.dependency_fact_id.clone());
                dependency_fact_ids = sorted_strings(dependency_fact_ids);
                let next = PathState {
                    from_id: state.from_id.clone(),
                    to_id: edge.to_id.clone(),
                    path_length: state.path_length + 1,
                    confidence: state.confidence * edge.confidence * SUPPORT_ATTENUATION,
                    dependency_fact_ids,
                };
                let mut next_nodes = path_nodes.clone();
                next_nodes.push(edge.to_id.clone());
                queue.push_back((next, next_nodes));
            }
        }
    }
    (paths, iterations)
}

fn derive_equivalence_classes(facts: &SaturationFacts) -> Vec<SaturationEquivalenceClass> {
    let nodes_by_id = facts
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut candidates = Vec::new();
    for shadow in facts.nodes.iter().filter(|node| {
        node.labels
            .iter()
            .any(|label| label == EPISTEMIC_SHADOW_LABEL)
    }) {
        let content_id =
            prop_str(&shadow.properties, "content_node_id").unwrap_or_else(|| shadow.id.clone());
        let Some(content_node) = nodes_by_id.get(content_id.as_str()) else {
            continue;
        };
        let Some(form) = claim_logical_form(&claim_text(content_node)) else {
            continue;
        };
        let status = grounded_status_symbol(
            &prop_str(&shadow.properties, "grounded_extension_status")
                .unwrap_or_else(|| "undecided".to_string()),
        );
        let term_str = format!("(shadow {status} {})", form.claim_term());
        let Ok(term) = term_str.parse::<RecExpr<SymbolLang>>() else {
            continue;
        };
        candidates.push(EqualityCandidate {
            shadow_id: shadow.id.clone(),
            content_id,
            form,
            term,
        });
    }

    if candidates.is_empty() {
        return Vec::new();
    }
    let class_roots = egraph_class_roots(&candidates);
    let mut groups: BTreeMap<Id, Vec<usize>> = BTreeMap::new();
    for (idx, root) in class_roots.into_iter().enumerate() {
        groups.entry(root).or_default().push(idx);
    }

    let mut classes = Vec::new();
    for members in groups.into_values() {
        if members.len() < 2 {
            continue;
        }
        let mut sorted = members.clone();
        sorted.sort_by(|&a, &b| {
            candidates[a]
                .content_id
                .cmp(&candidates[b].content_id)
                .then_with(|| candidates[a].shadow_id.cmp(&candidates[b].shadow_id))
        });
        let rep = &candidates[sorted[0]];
        let mut member_content_ids = sorted
            .iter()
            .map(|&idx| candidates[idx].content_id.clone())
            .collect::<Vec<_>>();
        let mut member_shadow_ids = sorted
            .iter()
            .map(|&idx| candidates[idx].shadow_id.clone())
            .collect::<Vec<_>>();
        member_content_ids.sort();
        member_content_ids.dedup();
        member_shadow_ids.sort();
        member_shadow_ids.dedup();
        classes.push(SaturationEquivalenceClass {
            class_id: format!("eclass:{}", stable_hash(json!(member_shadow_ids))),
            canonical_form: rep.form.canonical_label(),
            representative_content_id: rep.content_id.clone(),
            representative_shadow_id: rep.shadow_id.clone(),
            member_content_ids,
            member_shadow_ids,
        });
    }
    classes.sort_by(|left, right| left.class_id.cmp(&right.class_id));
    classes
}

fn egraph_class_roots(candidates: &[EqualityCandidate]) -> Vec<Id> {
    let rules: Vec<egg::Rewrite<SymbolLang, ()>> =
        vec![egg::rewrite!("epistemic-double-negation"; "(not (not ?x))" => "?x")];
    let mut runner: Runner<SymbolLang, ()> = Runner::default()
        .with_node_limit(usize::MAX)
        .with_iter_limit(usize::MAX)
        .with_time_limit(std::time::Duration::from_secs(3600));
    for candidate in candidates {
        runner = runner.with_expr(&candidate.term);
    }
    let runner = runner.run(&rules);
    runner
        .roots
        .iter()
        .map(|root| runner.egraph.find(*root))
        .collect()
}

fn materialize_same_eclass<S: GraphStore>(
    store: &mut S,
    class: &SaturationEquivalenceClass,
    config: &SaturationConfig,
    _mutations: &mut Vec<GraphMutation>,
) -> GraphStoreResult<usize> {
    let mut written = 0;
    for member_shadow_id in &class.member_shadow_ids {
        if member_shadow_id == &class.representative_shadow_id {
            continue;
        }
        let edge = EdgeRecord::new(
            same_eclass_edge_id(
                member_shadow_id,
                &class.representative_shadow_id,
                crate::epistemic::DEFAULT_EPISTEMIC_ENGINE_VERSION,
            ),
            member_shadow_id,
            SAME_ECLASS,
            &class.representative_shadow_id,
            json!({
                "class_id": class.class_id,
                "canonical_form": class.canonical_form,
                "confidence": 1.0,
                "evidence": "egraph_congruence",
                "source_kind": "structural",
                "engine": EGRAPH_EPISTEMIC_ENGINE,
                "engine_version": crate::epistemic::DEFAULT_EPISTEMIC_ENGINE_VERSION,
                "computed_at": config.computed_at,
                "quarantine": true,
            }),
        )
        .with_confidence(1.0)
        .with_provenance(Provenance {
            source_id: Some(EGRAPH_EPISTEMIC_ENGINE.to_string()),
            timestamp: Some(config.computed_at.to_string()),
            method: Some("egglog_saturation_equivalence".to_string()),
        });
        match upsert_edge_if_changed(store, edge.clone())? {
            WriteDisposition::Written => {
                written += 1;
            }
            WriteDisposition::Skipped => {}
        }
    }
    Ok(written)
}

fn prune_stale_derived_artifacts<S: GraphStore>(
    store: &mut S,
    component_key: &str,
    live_statement_ids: &BTreeSet<String>,
    retracted_fact_ids: &[String],
    mutations: &mut Vec<GraphMutation>,
) -> GraphStoreResult<(usize, usize)> {
    let retracted = retracted_fact_ids.iter().cloned().collect::<BTreeSet<_>>();
    let mut nodes_tombstoned = 0;
    let mut edges_tombstoned = 0;
    let existing = store.query_nodes(
        NodeQuery::label(SATURATION_DERIVED_STATEMENT_LABEL).with_limit(SATURATION_FACT_LIMIT),
    );
    for node in existing {
        if prop_str(&node.properties, "engine_version").as_deref()
            != Some(SATURATION_ENGINE_VERSION)
        {
            continue;
        }
        if prop_str(&node.properties, "component_key").as_deref() != Some(component_key) {
            continue;
        }
        if live_statement_ids.contains(&node.id) {
            continue;
        }
        let dependency_fact_ids = prop_string_vec(&node.properties, "dependency_fact_ids");
        let touched_retraction = dependency_fact_ids
            .iter()
            .any(|dep| retracted.contains(dep) || retracted.contains(&node.id));
        if !retracted.is_empty() && !touched_retraction {
            continue;
        }

        let outgoing = store.neighbors(NeighborQuery::out(&node.id).with_include_expired(true));
        for hit in outgoing {
            if let Some(edge) = store.get_edge(&hit.edge_id).cloned() {
                let mut tombstone = edge.clone();
                tombstone.tombstone = true;
                match upsert_edge_if_changed(store, tombstone.clone())? {
                    WriteDisposition::Written => {
                        edges_tombstoned += 1;
                        mutations.push(GraphMutation::EdgeUpsert(tombstone));
                    }
                    WriteDisposition::Skipped => {}
                }
            }
        }

        let mut tombstone = node.clone();
        tombstone.tombstone = true;
        match upsert_node_if_changed(store, tombstone.clone())? {
            WriteDisposition::Written => {
                nodes_tombstoned += 1;
                mutations.push(GraphMutation::NodeUpsert(tombstone));
            }
            WriteDisposition::Skipped => {}
        }
    }
    Ok((nodes_tombstoned, edges_tombstoned))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriteDisposition {
    Written,
    Skipped,
}

fn upsert_node_if_changed<S: GraphStore>(
    store: &mut S,
    node: NodeRecord,
) -> GraphStoreResult<WriteDisposition> {
    if let Some(existing) = store.get_node(&node.id) {
        if same_node_payload(existing, &node) {
            return Ok(WriteDisposition::Skipped);
        }
    }
    store.upsert_node(node)?;
    Ok(WriteDisposition::Written)
}

fn upsert_edge_if_changed<S: GraphStore>(
    store: &mut S,
    edge: EdgeRecord,
) -> GraphStoreResult<WriteDisposition> {
    if let Some(existing) = store.get_edge(&edge.id) {
        if same_edge_payload(existing, &edge) {
            return Ok(WriteDisposition::Skipped);
        }
    }
    store.upsert_edge(edge)?;
    Ok(WriteDisposition::Written)
}

fn same_node_payload(left: &NodeRecord, right: &NodeRecord) -> bool {
    left.id == right.id
        && left.labels == right.labels
        && left.properties == right.properties
        && left.tombstone == right.tombstone
}

fn same_edge_payload(left: &EdgeRecord, right: &EdgeRecord) -> bool {
    left.id == right.id
        && left.from_id == right.from_id
        && left.to_id == right.to_id
        && left.edge_type == right.edge_type
        && left.properties == right.properties
        && left.tombstone == right.tombstone
        && left.confidence == right.confidence
        && left.epistemic_type == right.epistemic_type
        && left.provenance == right.provenance
}

fn derived_statement_node<S: GraphStore>(
    store: &S,
    statement: &SaturationDerivedStatement,
    component_key: &str,
    config: &SaturationConfig,
) -> NodeRecord {
    let (object_ref, object_value) = derived_statement_object_ref(store, statement);
    let field_provenance = StatementFieldProvenance::new(
        crate::epistemic::EpistemicSourceKind::Structural,
        config.engine.clone(),
        config.engine_version.clone(),
        config.computed_at,
    );
    let provenance = StatementProvenance::new(
        field_provenance,
        statement.dependency_fact_ids.clone(),
        statement.rule_id.clone(),
        StatementSemiring::Viterbi,
    );
    let mut properties = json!({
            "fact_id": statement.fact_id,
            "rule_id": statement.rule_id,
            "relation": statement.relation,
            "subject_id": statement.subject_id,
            "subject_ref": statement.subject_id,
            "object_ref": object_ref.clone(),
            "reason": statement.reason,
            "dependency_fact_ids": statement.dependency_fact_ids,
            "contributors": statement.contributors,
            "attributes": statement.attributes,
            "confidence": statement.confidence,
            "writeback_policy": statement.writeback_policy,
            "engine": config.engine,
            "engine_version": config.engine_version,
            "computed_at": config.computed_at,
            "component_key": component_key,
            "quarantine": true,
            "field_provenance": {
                "confidence": {
                    "source_kind": "structural",
                    "engine": config.engine,
                    "engine_version": config.engine_version,
                    "computed_at": config.computed_at
                },
                "dependency_fact_ids": {
                    "source_kind": "structural",
                    "engine": config.engine,
                    "engine_version": config.engine_version,
                    "computed_at": config.computed_at
                }
            }
    });
    if let Some(object_value) = object_value {
        if let Some(object) = properties.as_object_mut() {
            object.insert("object_value".to_string(), object_value);
        }
    }
    let mut node = StatementRecord::derive(
        &statement.subject_id,
        &statement.relation,
        &object_ref,
        provenance,
        properties,
    );
    if !node
        .labels
        .iter()
        .any(|label| label == SATURATION_DERIVED_STATEMENT_LABEL)
    {
        node.labels
            .push(SATURATION_DERIVED_STATEMENT_LABEL.to_string());
        node.labels.sort();
        node.labels.dedup();
    }
    node
}

fn derived_statement_edge(
    statement: &SaturationDerivedStatement,
    statement_node_id: &str,
    config: &SaturationConfig,
) -> EdgeRecord {
    EdgeRecord::new(
        statement_edge_id(&statement.fact_id, &statement.subject_id),
        statement_node_id,
        SATURATION_DERIVES_EDGE,
        &statement.subject_id,
        json!({
            "fact_id": statement.fact_id,
            "statement_id": statement_node_id,
            "rule_id": statement.rule_id,
            "confidence": statement.confidence,
            "dependency_fact_ids": statement.dependency_fact_ids,
            "engine": config.engine,
            "engine_version": config.engine_version,
            "computed_at": config.computed_at,
            "quarantine": true,
        }),
    )
    .with_confidence(statement.confidence)
    .with_epistemic_type(EpistemicType::Derives)
    .with_provenance(Provenance {
        source_id: Some(config.engine.clone()),
        timestamp: Some(config.computed_at.to_string()),
        method: Some("egglog_saturation".to_string()),
    })
}

fn derived_statement_hyper_id<S: GraphStore>(
    store: &S,
    statement: &SaturationDerivedStatement,
) -> String {
    let (object_ref, _) = derived_statement_object_ref(store, statement);
    hyper_statement_id(&statement.subject_id, &statement.relation, &object_ref)
}

fn derived_statement_object_ref<S: GraphStore>(
    store: &S,
    statement: &SaturationDerivedStatement,
) -> (String, Option<Value>) {
    for key in ["target_id", "depends_on_object_id", "artifact_id"] {
        let candidate = prop_str(&statement.attributes, key);
        if let Some(candidate) = candidate.filter(|candidate| store.get_node(candidate).is_some()) {
            return (candidate, None);
        }
    }
    if let Some(edge_id) = prop_str(&statement.attributes, "edge_id") {
        if store.get_node(&edge_id).is_some() {
            return (edge_id, None);
        }
        let value = json!({
            "edge_id": edge_id,
            "attributes": statement.attributes,
        });
        return (literal_ref(&value), Some(value));
    }
    for key in ["target_id", "depends_on_object_id", "artifact_id"] {
        if let Some(candidate) = prop_str(&statement.attributes, key) {
            let value = Value::String(candidate);
            return (literal_ref(&value), Some(value));
        }
    }
    let value = if statement.attributes.is_null() {
        Value::String(statement.reason.clone())
    } else {
        statement.attributes.clone()
    };
    (literal_ref(&value), Some(value))
}

fn statement_edge_id(fact_id: &str, subject_id: &str) -> String {
    format!(
        "saturation:edge:{}",
        stable_hash(json!({ "fact_id": fact_id, "subject_id": subject_id }))
    )
}

fn exact_derived_statement(
    seed: StatementSeed<'_>,
    engine: &str,
) -> Result<SaturationDerivedStatement, String> {
    let dependency_fact_ids = sorted_strings(seed.dependency_fact_ids);
    let hash_payload = json!({
        "rule_id": seed.rule_id,
        "relation": seed.relation,
        "subject_id": seed.subject_id,
        "attributes": seed.attributes,
        "dependency_fact_ids": dependency_fact_ids,
    });
    let fact_id = stable_hash_value(&hash_payload)?;
    Ok(SaturationDerivedStatement {
        fact_id,
        rule_id: seed.rule_id.to_string(),
        relation: seed.relation.to_string(),
        subject_id: seed.subject_id.to_string(),
        reason: seed.reason.to_string(),
        dependency_fact_ids: value_string_vec(&hash_payload["dependency_fact_ids"]),
        contributors: vec![SaturationContributor {
            dependency_fact_ids: value_string_vec(&hash_payload["dependency_fact_ids"]),
            confidence: seed.confidence,
        }],
        attributes: hash_payload["attributes"].clone(),
        confidence: seed.confidence,
        writeback_policy: seed.writeback_policy.to_string(),
        engine: engine.to_string(),
    })
}

fn saturation_statement(seed: StatementSeed<'_>, engine: &str) -> SaturationDerivedStatement {
    let dependency_fact_ids = sorted_strings(seed.dependency_fact_ids);
    let hash_payload = json!({
        "rule_id": seed.rule_id,
        "relation": seed.relation,
        "subject_id": seed.subject_id,
        "attributes": seed.attributes,
    });
    SaturationDerivedStatement {
        fact_id: stable_hash(hash_payload),
        rule_id: seed.rule_id.to_string(),
        relation: seed.relation.to_string(),
        subject_id: seed.subject_id.to_string(),
        reason: seed.reason.to_string(),
        dependency_fact_ids: dependency_fact_ids.clone(),
        contributors: vec![SaturationContributor {
            dependency_fact_ids,
            confidence: seed.confidence,
        }],
        attributes: seed.attributes,
        confidence: seed.confidence,
        writeback_policy: seed.writeback_policy.to_string(),
        engine: engine.to_string(),
    }
}

fn merge_statement(
    statements: &mut BTreeMap<String, SaturationDerivedStatement>,
    mut statement: SaturationDerivedStatement,
) {
    statement.dependency_fact_ids = sorted_strings(statement.dependency_fact_ids);
    if let Some(existing) = statements.get_mut(&statement.fact_id) {
        existing.confidence = existing.confidence.max(statement.confidence);
        existing.dependency_fact_ids = sorted_strings(
            existing
                .dependency_fact_ids
                .iter()
                .cloned()
                .chain(statement.dependency_fact_ids.iter().cloned())
                .collect(),
        );
        existing.contributors.extend(statement.contributors);
        existing.contributors.sort_by(|left, right| {
            left.dependency_fact_ids
                .cmp(&right.dependency_fact_ids)
                .then_with(|| left.confidence.total_cmp(&right.confidence))
        });
        existing.contributors.dedup_by(|left, right| {
            left.dependency_fact_ids == right.dependency_fact_ids
                && (left.confidence - right.confidence).abs() < f64::EPSILON
        });
    } else {
        statements.insert(statement.fact_id.clone(), statement);
    }
}

fn datalog_facts_from_graph_records(nodes: &[NodeRecord], edges: &[EdgeRecord]) -> Vec<Value> {
    let mut facts = Vec::new();
    for node in nodes {
        if node.labels.iter().any(|label| label == "Claim")
            || prop_str(&node.properties, "claim_text").is_some()
        {
            facts.push(graph_fact(
                "claim",
                &node.id,
                &node.id,
                json!({
                    "status": prop_value(&node.properties, "status").unwrap_or_else(|| json!("")),
                    "claim_text": claim_text(node),
                }),
                "graph-node",
            ));
        }
    }
    for edge in edges {
        let edge_type = edge.edge_type.as_str();
        if is_claim_dependency_edge(edge_type) {
            facts.push(graph_fact(
                "claim_dependency",
                &edge.id,
                &edge.id,
                json!({
                    "claim_id": edge.from_id,
                    "depends_on_object_id": edge.to_id,
                    "justification_type": prop_value(&edge.properties, "justification_type").unwrap_or_else(|| json!("dependency")),
                    "strength": prop_value(&edge.properties, "strength").unwrap_or_else(|| json!(edge.effective_confidence())),
                }),
                "graph-edge",
            ));
        } else if is_evidence_link_edge(edge_type) {
            facts.push(graph_fact(
                "evidence_link",
                &edge.id,
                &edge.id,
                json!({
                    "claim_id": edge.from_id,
                    "artifact_id": edge.to_id,
                    "strength": prop_value(&edge.properties, "strength").unwrap_or_else(|| json!(edge.effective_confidence())),
                }),
                "graph-edge",
            ));
        } else if is_contradiction_edge(edge_type) {
            facts.push(graph_fact(
                "edge",
                &edge.id,
                &edge.id,
                json!({
                    "edge_type": "contradicts",
                    "from_object_id": edge.from_id,
                    "to_object_id": edge.to_id,
                    "acceptance_status": prop_value(&edge.properties, "acceptance_status").unwrap_or_else(|| json!("contested")),
                    "confidence": edge.effective_confidence(),
                }),
                "graph-edge",
            ));
        }
    }
    facts.extend(statement_facts_from_graph_records(nodes, edges));
    facts.sort_by(|left, right| object_field(left, "fact_id").cmp(object_field(right, "fact_id")));
    facts
}

fn statement_facts_from_graph_records(nodes: &[NodeRecord], edges: &[EdgeRecord]) -> Vec<Value> {
    let mut subject_by_statement = BTreeMap::<String, String>::new();
    let mut object_by_statement = BTreeMap::<String, String>::new();
    for edge in edges {
        match edge.edge_type.as_str() {
            HAS_SUBJECT => {
                subject_by_statement.insert(edge.from_id.clone(), edge.to_id.clone());
            }
            HAS_OBJECT => {
                object_by_statement.insert(edge.from_id.clone(), edge.to_id.clone());
            }
            _ => {}
        }
    }

    let mut facts = Vec::new();
    for node in nodes {
        if !node.labels.iter().any(|label| label == STATEMENT_LABEL) {
            continue;
        }
        if prop_bool(&node.properties, "derived") {
            continue;
        }
        let relation = prop_str(&node.properties, "relation")
            .or_else(|| prop_str(&node.properties, "predicate_key"))
            .unwrap_or_default();
        if relation.is_empty() {
            continue;
        }
        let subject_id = subject_by_statement
            .get(&node.id)
            .cloned()
            .or_else(|| prop_str(&node.properties, "subject_ref"))
            .or_else(|| prop_str(&node.properties, "subject_id"))
            .unwrap_or_default();
        if subject_id.is_empty() {
            continue;
        }
        let object_id = object_by_statement
            .get(&node.id)
            .cloned()
            .or_else(|| {
                prop_str(&node.properties, "object_ref").filter(|value| !value.starts_with("lit:"))
            })
            .unwrap_or_default();
        let confidence = node
            .properties
            .get("confidence")
            .and_then(value_to_f64)
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);

        if is_claim_dependency_edge(&relation) {
            if object_id.is_empty() {
                continue;
            }
            facts.push(graph_fact(
                "claim_dependency",
                &node.id,
                &node.id,
                json!({
                    "claim_id": subject_id,
                    "depends_on_object_id": object_id,
                    "justification_type": prop_value(&node.properties, "justification_type").unwrap_or_else(|| json!("dependency")),
                    "strength": confidence,
                }),
                &prop_str(&node.properties, "source_ref").unwrap_or_else(|| "statement".to_string()),
            ));
        } else if is_evidence_link_edge(&relation) {
            if object_id.is_empty() {
                continue;
            }
            facts.push(graph_fact(
                "evidence_link",
                &node.id,
                &node.id,
                json!({
                    "claim_id": subject_id,
                    "artifact_id": object_id,
                    "strength": confidence,
                }),
                &prop_str(&node.properties, "source_ref")
                    .unwrap_or_else(|| "statement".to_string()),
            ));
        } else if is_contradiction_edge(&relation) {
            if object_id.is_empty() {
                continue;
            }
            facts.push(graph_fact(
                "edge",
                &node.id,
                &node.id,
                json!({
                    "edge_type": "contradicts",
                    "from_object_id": subject_id,
                    "to_object_id": object_id,
                    "acceptance_status": prop_value(&node.properties, "acceptance_status").unwrap_or_else(|| json!("contested")),
                    "confidence": confidence,
                }),
                &prop_str(&node.properties, "source_ref").unwrap_or_else(|| "statement".to_string()),
            ));
        }
    }
    facts
}

fn graph_fact(
    relation: &str,
    fact_id: &str,
    entity_id: &str,
    attributes: Value,
    source_ref: &str,
) -> Value {
    let source_ref = source_ref.to_string();
    json!({
        "fact_id": fact_id,
        "relation": relation,
        "entity_id": entity_id,
        "attributes": attributes,
        "source_ref": source_ref,
    })
}

fn support_edges_from_facts(facts: &[Value]) -> Vec<SupportEdge> {
    let mut out = Vec::new();
    for fact in facts {
        match object_field(fact, "relation") {
            "claim_dependency" => {
                let from_id = attr_string(fact, "claim_id");
                let to_id = attr_string(fact, "depends_on_object_id");
                if from_id.is_empty() || to_id.is_empty() {
                    continue;
                }
                out.push(SupportEdge {
                    from_id,
                    to_id,
                    confidence: numeric_attr(fact, "strength", 1.0).clamp(0.0, 1.0),
                    dependency_fact_id: object_field(fact, "fact_id").to_string(),
                });
            }
            "evidence_link" => {
                let from_id = attr_string(fact, "claim_id");
                let to_id = attr_string(fact, "artifact_id");
                if from_id.is_empty() || to_id.is_empty() {
                    continue;
                }
                out.push(SupportEdge {
                    from_id,
                    to_id,
                    confidence: numeric_attr(fact, "strength", 1.0).clamp(0.0, 1.0),
                    dependency_fact_id: object_field(fact, "fact_id").to_string(),
                });
            }
            _ => {}
        }
    }
    out.sort_by(|left, right| {
        (&left.from_id, &left.to_id, &left.dependency_fact_id).cmp(&(
            &right.from_id,
            &right.to_id,
            &right.dependency_fact_id,
        ))
    });
    out
}

fn contradiction_edges_from_facts(facts: &[Value]) -> Vec<ContradictionEdge> {
    let mut out = Vec::new();
    for fact in facts {
        if object_field(fact, "relation") != "edge" {
            continue;
        }
        let edge_type = attr_string(fact, "edge_type").to_lowercase();
        let status = attr_string(fact, "acceptance_status").to_lowercase();
        if edge_type != "contradicts" && edge_type != "undercuts" && status != "contested" {
            continue;
        }
        let from_id = attr_string(fact, "from_object_id");
        let to_id = attr_string(fact, "to_object_id");
        if from_id.is_empty() || to_id.is_empty() {
            continue;
        }
        out.push(ContradictionEdge {
            from_id,
            to_id,
            confidence: numeric_attr(fact, "confidence", 1.0).clamp(0.0, 1.0),
            dependency_fact_id: object_field(fact, "fact_id").to_string(),
        });
    }
    out.sort_by(|left, right| {
        (&left.from_id, &left.to_id, &left.dependency_fact_id).cmp(&(
            &right.from_id,
            &right.to_id,
            &right.dependency_fact_id,
        ))
    });
    out
}

fn is_claim_dependency_edge(edge_type: &str) -> bool {
    matches!(
        edge_type,
        "claim_dependency" | "ClaimDependency" | "CLAIM_DEPENDENCY" | "DEPENDS_ON" | "DependsOn"
    )
}

fn is_evidence_link_edge(edge_type: &str) -> bool {
    matches!(
        edge_type,
        "evidence_link" | "EvidenceLink" | "EVIDENCE_LINK" | "SUPPORTS" | "Supports" | "supports"
    )
}

fn is_contradiction_edge(edge_type: &str) -> bool {
    matches!(
        edge_type,
        "CONTRADICTS" | "Contradicts" | "contradicts" | "UNDERCUTS" | "Undercuts" | "undercuts"
    )
}

fn facts_by_relation(facts: &[Value]) -> BTreeMap<String, Vec<Value>> {
    let mut index = BTreeMap::<String, Vec<Value>>::new();
    for fact in facts {
        index
            .entry(object_field(fact, "relation").to_string())
            .or_default()
            .push(fact.clone());
    }
    for facts in index.values_mut() {
        facts.sort_by(|left, right| {
            object_field(left, "fact_id").cmp(object_field(right, "fact_id"))
        });
    }
    index
}

fn relation<'a>(index: &'a BTreeMap<String, Vec<Value>>, relation: &str) -> &'a [Value] {
    index.get(relation).map(Vec::as_slice).unwrap_or(&[])
}

fn payload_facts(payload: &Value) -> Result<&Vec<Value>, String> {
    if let Some(facts) = payload.as_array() {
        return Ok(facts);
    }
    payload
        .get("facts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "datalog payload expected a JSON array or an object with facts array".to_string()
        })
}

fn normalize_datalog_fact(raw: &Value) -> Result<Value, String> {
    let relation = field_string(raw, "relation");
    let entity_id = field_string(raw, "entity_id");
    if relation.trim().is_empty() {
        return Err("DatalogFact requires relation".to_string());
    }
    if entity_id.trim().is_empty() {
        return Err("DatalogFact requires entity_id".to_string());
    }
    let attributes = raw
        .get("attributes")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let source_ref = field_string(raw, "source_ref");
    let fallback_id = stable_hash_value(&json!({
        "relation": relation,
        "entity_id": entity_id,
        "attributes": attributes,
        "source_ref": source_ref,
    }))?;
    let fact_id = field_string(raw, "fact_id");
    Ok(json!({
        "fact_id": if fact_id.is_empty() { fallback_id } else { fact_id },
        "relation": field_string(raw, "relation"),
        "entity_id": field_string(raw, "entity_id"),
        "attributes": raw
            .get("attributes")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({})),
        "source_ref": field_string(raw, "source_ref"),
    }))
}

fn shared_subset_payload(payload: &Value) -> Result<Value, String> {
    Ok(json!({
        "facts": payload_facts(payload)?.clone(),
        "rule_ids": SATURATION_SHARED_RULE_IDS,
    }))
}

fn fact_ids_from_receipt(receipt: &Value) -> BTreeSet<String> {
    receipt
        .get("derived_facts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|fact| fact.get("fact_id").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

fn is_saturation_artifact_node(node: &NodeRecord) -> bool {
    node.labels
        .iter()
        .any(|label| label == SATURATION_DERIVED_STATEMENT_LABEL)
        || prop_str(&node.properties, "engine").as_deref() == Some(SATURATION_ENGINE)
}

fn is_saturation_artifact_edge(edge: &EdgeRecord) -> bool {
    edge.edge_type == SATURATION_DERIVES_EDGE
        || prop_str(&edge.properties, "engine").as_deref() == Some(SATURATION_ENGINE)
}

fn claim_logical_form(text: &str) -> Option<ClaimForm> {
    let normalized = normalize_claim(text);
    let prop = without_negation(&normalized);
    if prop.is_empty() {
        return None;
    }
    Some(ClaimForm {
        prop,
        neg_count: count_negations(&normalized),
    })
}

fn normalize_claim(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn without_negation(normalized: &str) -> String {
    normalized
        .split_whitespace()
        .filter(|token| !matches!(*token, "not" | "no" | "never" | "without"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn count_negations(normalized: &str) -> usize {
    normalized
        .split_whitespace()
        .filter(|token| matches!(*token, "not" | "no" | "never" | "without"))
        .count()
}

fn grounded_status_symbol(raw: &str) -> &'static str {
    match raw.to_lowercase().as_str() {
        "in" => "s_in",
        "out" => "s_out",
        _ => "s_undecided",
    }
}

fn claim_text(node: &NodeRecord) -> String {
    ["claim_text", "text", "title", "body", "content"]
        .iter()
        .find_map(|key| prop_str(&node.properties, key))
        .unwrap_or_else(|| node.id.clone())
}

fn graph_compile_options(message: &str) -> GraphCompileOptions {
    GraphCompileOptions {
        name: Some("saturation-revision".to_string()),
        branch: Some("saturation".to_string()),
        author: Some(SATURATION_ENGINE.to_string()),
        message: Some(message.to_string()),
        ..GraphCompileOptions::default()
    }
}

fn sorted_strings(values: Vec<String>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn value_string_vec(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn prop_string_vec(value: &Value, key: &str) -> Vec<String> {
    value.get(key).map(value_string_vec).unwrap_or_default()
}

fn field_string(value: &Value, key: &str) -> String {
    value.get(key).map(value_to_string).unwrap_or_default()
}

fn object_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn attr<'a>(fact: &'a Value, key: &str) -> Option<&'a Value> {
    fact.get("attributes")
        .and_then(Value::as_object)
        .and_then(|attributes| attributes.get(key))
}

fn attr_string(fact: &Value, key: &str) -> String {
    attr(fact, key).map(value_to_string).unwrap_or_default()
}

fn attr_value_or(fact: &Value, key: &str, fallback: Value) -> Value {
    attr(fact, key).cloned().unwrap_or(fallback)
}

fn numeric_attr(fact: &Value, key: &str, fallback: f64) -> f64 {
    attr(fact, key).and_then(value_to_f64).unwrap_or(fallback)
}

fn integer_attr(fact: &Value, key: &str, fallback: i64) -> i64 {
    attr(fact, key)
        .and_then(|value| match value {
            Value::Number(number) => number
                .as_i64()
                .or_else(|| number.as_u64().map(|v| v as i64)),
            Value::String(text) => text.parse::<i64>().ok(),
            _ => None,
        })
        .unwrap_or(fallback)
}

fn prop_str(value: &Value, key: &str) -> Option<String> {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .map(value_to_string)
        .filter(|value| !value.trim().is_empty())
}

fn prop_value(value: &Value, key: &str) -> Option<Value> {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .cloned()
}

fn prop_bool(value: &Value, key: &str) -> bool {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn value_to_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}
