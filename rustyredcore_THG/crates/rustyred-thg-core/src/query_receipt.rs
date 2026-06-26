use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::planner::{AccessPathTrace, PlanTrace};
use crate::state::stable_hash;

pub type ReceiptScope = BTreeMap<String, String>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryKind {
    Search,
    CrawlFrontier,
    ContextCompile,
    GraphExpand,
    CodeLookup,
    ArtifactList,
    ToolSelect,
    AdapterRoute,
    TrainingExport,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryOutcomeLabel {
    Success,
    Failure,
    Partial,
    Inconclusive,
}

impl Default for QueryOutcomeLabel {
    fn default() -> Self {
        Self::Inconclusive
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AccessPathReceipt {
    pub relation: String,
    pub alias: String,
    pub predicate: String,
    pub method: String,
    pub est_rows: f64,
    pub est_work: f64,
    pub returned_rows: usize,
    pub visited_rows: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_id: Option<String>,
}

impl AccessPathReceipt {
    pub fn from_trace(path: &AccessPathTrace) -> Self {
        let manifest_id = (!is_scan_method(&path.method)).then(|| {
            format!(
                "{}:{}:{}",
                path.method.trim(),
                path.relation.trim(),
                path.predicate.trim()
            )
        });
        Self {
            relation: path.relation.clone(),
            alias: path.alias.clone(),
            predicate: path.predicate.clone(),
            method: path.method.clone(),
            est_rows: path.est_rows,
            est_work: path.est_work,
            returned_rows: path.returned_rows,
            visited_rows: path.visited_rows,
            manifest_id,
        }
    }

    pub fn stage_key(&self) -> String {
        format!("{}:{}:{}", self.alias, self.predicate, self.method)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct QueryReceipt {
    pub id: String,
    pub query_signature: String,
    pub query_kind: QueryKind,
    pub scope: ReceiptScope,
    #[serde(default)]
    pub access_paths_considered: Vec<AccessPathReceipt>,
    #[serde(default)]
    pub access_paths_used: Vec<AccessPathReceipt>,
    #[serde(default)]
    pub indexes_used: Vec<String>,
    #[serde(default)]
    pub candidate_counts_by_stage: BTreeMap<String, usize>,
    pub full_scan_count: usize,
    pub hydrated_object_count: usize,
    pub graph_expansion_count: usize,
    #[serde(default)]
    pub latency_by_stage_ms: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_cost: Option<u64>,
    pub result_count: usize,
    #[serde(default)]
    pub accepted_result_ids: Vec<String>,
    #[serde(default)]
    pub cited_result_ids: Vec<String>,
    #[serde(default)]
    pub dismissed_result_ids: Vec<String>,
    pub outcome_label: QueryOutcomeLabel,
    pub created_at: i64,
}

impl QueryReceipt {
    pub fn from_plan_trace(
        id: impl Into<String>,
        query_kind: QueryKind,
        scope: ReceiptScope,
        trace: &PlanTrace,
        result_count: usize,
        created_at: i64,
    ) -> Self {
        let access_paths_considered = trace
            .access_paths
            .iter()
            .map(AccessPathReceipt::from_trace)
            .collect::<Vec<_>>();
        let access_paths_used = access_paths_considered.clone();
        let mut indexes_used = access_paths_used
            .iter()
            .filter_map(|path| path.manifest_id.clone())
            .collect::<Vec<_>>();
        indexes_used.sort();
        indexes_used.dedup();

        let mut candidate_counts_by_stage = BTreeMap::new();
        for path in &access_paths_used {
            candidate_counts_by_stage.insert(path.stage_key(), path.returned_rows);
        }
        for ranker in &trace.rankers {
            candidate_counts_by_stage.insert(
                format!(
                    "ranker:{}:{}:{}",
                    ranker.alias, ranker.predicate, ranker.method
                ),
                ranker.contributed_rows,
            );
        }
        candidate_counts_by_stage.insert("candidate_set".to_string(), trace.candidate_set_size);

        let graph_expansion_count = access_paths_used
            .iter()
            .filter(|path| is_graph_method(&path.method))
            .map(|path| path.visited_rows)
            .sum();
        let query_signature = Self::signature_for_plan_trace(&query_kind, &scope, trace);

        Self {
            id: id.into(),
            query_signature,
            query_kind,
            scope,
            access_paths_considered,
            access_paths_used,
            indexes_used,
            candidate_counts_by_stage,
            full_scan_count: trace.full_relation_scans,
            hydrated_object_count: result_count,
            graph_expansion_count,
            latency_by_stage_ms: BTreeMap::new(),
            token_cost: None,
            result_count,
            accepted_result_ids: Vec::new(),
            cited_result_ids: Vec::new(),
            dismissed_result_ids: Vec::new(),
            outcome_label: QueryOutcomeLabel::Inconclusive,
            created_at,
        }
    }

    pub fn signature_for_plan_trace(
        query_kind: &QueryKind,
        scope: &ReceiptScope,
        trace: &PlanTrace,
    ) -> String {
        #[derive(Serialize)]
        struct AccessShape<'a> {
            relation: &'a str,
            alias: &'a str,
            predicate: &'a str,
            method: &'a str,
        }

        #[derive(Serialize)]
        struct RankerShape<'a> {
            relation: &'a str,
            alias: &'a str,
            predicate: &'a str,
            method: &'a str,
            score_source: &'a str,
        }

        #[derive(Serialize)]
        struct SignatureInput<'a> {
            query_kind: &'a QueryKind,
            scope: &'a ReceiptScope,
            access_paths: Vec<AccessShape<'a>>,
            rankers: Vec<RankerShape<'a>>,
            fusion: &'a str,
            knn_strategy: &'a Option<String>,
            cascade_rule_order: &'a [String],
        }

        let access_paths = trace
            .access_paths
            .iter()
            .map(|path| AccessShape {
                relation: &path.relation,
                alias: &path.alias,
                predicate: &path.predicate,
                method: &path.method,
            })
            .collect::<Vec<_>>();
        let rankers = trace
            .rankers
            .iter()
            .map(|ranker| RankerShape {
                relation: &ranker.relation,
                alias: &ranker.alias,
                predicate: &ranker.predicate,
                method: &ranker.method,
                score_source: &ranker.score_source,
            })
            .collect::<Vec<_>>();

        stable_hash(SignatureInput {
            query_kind,
            scope,
            access_paths,
            rankers,
            fusion: &trace.fusion,
            knn_strategy: &trace.knn_strategy,
            cascade_rule_order: &trace.cascade_rule_order,
        })
    }

    pub fn explain(&self) -> QueryExplain {
        QueryExplain::from(self)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct QueryExplain {
    pub query_signature: String,
    #[serde(default)]
    pub access_paths_considered: Vec<String>,
    #[serde(default)]
    pub access_paths_selected: Vec<String>,
    #[serde(default)]
    pub candidate_counts: BTreeMap<String, usize>,
    pub full_scans: usize,
    pub graph_expansions: usize,
    pub bm25_hits: usize,
    pub vector_hits: usize,
    pub multi_vector_rerank_count: usize,
    #[serde(default)]
    pub ppr_seed_set: Vec<String>,
    #[serde(default)]
    pub policy_exclusions: Vec<String>,
    #[serde(default)]
    pub ttl_tombstone_exclusions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_cost: Option<u64>,
    #[serde(default)]
    pub cache_skip_hits: BTreeMap<String, usize>,
    #[serde(default)]
    pub latency: BTreeMap<String, f64>,
    #[serde(default)]
    pub included_atoms: Vec<String>,
    pub outcome_label: QueryOutcomeLabel,
    #[serde(default)]
    pub advisor_notes: Vec<String>,
}

impl From<&QueryReceipt> for QueryExplain {
    fn from(receipt: &QueryReceipt) -> Self {
        let bm25_hits = receipt
            .candidate_counts_by_stage
            .iter()
            .filter(|(stage, _)| stage.contains("fulltext") || stage.contains("bm25"))
            .map(|(_, count)| *count)
            .sum();
        let vector_hits = receipt
            .candidate_counts_by_stage
            .iter()
            .filter(|(stage, _)| stage.contains("vector") || stage.contains("knn"))
            .map(|(_, count)| *count)
            .sum();
        let multi_vector_rerank_count = receipt
            .candidate_counts_by_stage
            .iter()
            .filter(|(stage, _)| stage.contains("multi_vector") || stage.contains("maxsim"))
            .map(|(_, count)| *count)
            .sum();

        Self {
            query_signature: receipt.query_signature.clone(),
            access_paths_considered: receipt
                .access_paths_considered
                .iter()
                .map(access_path_label)
                .collect(),
            access_paths_selected: receipt
                .access_paths_used
                .iter()
                .map(access_path_label)
                .collect(),
            candidate_counts: receipt.candidate_counts_by_stage.clone(),
            full_scans: receipt.full_scan_count,
            graph_expansions: receipt.graph_expansion_count,
            bm25_hits,
            vector_hits,
            multi_vector_rerank_count,
            ppr_seed_set: Vec::new(),
            policy_exclusions: Vec::new(),
            ttl_tombstone_exclusions: Vec::new(),
            token_cost: receipt.token_cost,
            cache_skip_hits: BTreeMap::new(),
            latency: receipt.latency_by_stage_ms.clone(),
            included_atoms: receipt.cited_result_ids.clone(),
            outcome_label: receipt.outcome_label.clone(),
            advisor_notes: Vec::new(),
        }
    }
}

fn access_path_label(path: &AccessPathReceipt) -> String {
    format!(
        "{}:{}:{}:{}",
        path.relation, path.alias, path.predicate, path.method
    )
}

fn is_scan_method(method: &str) -> bool {
    let method = method.to_ascii_lowercase();
    method.contains("scan") || method == "full"
}

fn is_graph_method(method: &str) -> bool {
    let method = method.to_ascii_lowercase();
    method.contains("graph") || method.contains("ppr") || method.contains("expand")
}
