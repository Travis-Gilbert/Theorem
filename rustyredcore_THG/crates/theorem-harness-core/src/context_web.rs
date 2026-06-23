use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

const GENERATED_ARTIFACT_PREFIXES: &[&str] = &[
    "graphify-out/",
    "dist/",
    "build/",
    ".next/",
    "node_modules/",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextWebBudget {
    #[serde(default = "default_max_tokens")]
    pub max_tokens: i64,
    #[serde(default = "default_max_atoms")]
    pub max_atoms: usize,
    #[serde(default = "default_max_edges")]
    pub max_edges: usize,
    #[serde(default = "default_max_paths")]
    pub max_paths: usize,
    #[serde(default = "default_max_tools")]
    pub max_tools: usize,
}

impl Default for ContextWebBudget {
    fn default() -> Self {
        Self {
            max_tokens: default_max_tokens(),
            max_atoms: default_max_atoms(),
            max_edges: default_max_edges(),
            max_paths: default_max_paths(),
            max_tools: default_max_tools(),
        }
    }
}

impl ContextWebBudget {
    pub fn capped_for_mode(&self, mode: &str) -> Self {
        if mode.trim().eq_ignore_ascii_case("mini") {
            Self {
                max_tokens: self.max_tokens.min(300),
                max_atoms: self.max_atoms.min(6),
                ..self.clone()
            }
        } else {
            self.clone()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextWebCitation {
    pub source_id: String,
    pub source_type: String,
    #[serde(default)]
    pub locator: String,
    #[serde(default)]
    pub excerpt_hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextWebValidatorFinding {
    pub validator_id: String,
    #[serde(default = "low_severity")]
    pub severity: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub affected_atom_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextWebValidationSummary {
    #[serde(default)]
    pub findings: Vec<ContextWebValidatorFinding>,
    #[serde(default)]
    pub scores: BTreeMap<String, f64>,
    #[serde(default = "default_true")]
    pub passed: bool,
}

impl Default for ContextWebValidationSummary {
    fn default() -> Self {
        Self {
            findings: Vec::new(),
            scores: BTreeMap::new(),
            passed: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextWebEvaluation {
    #[serde(default)]
    pub naive_tokens: i64,
    #[serde(default)]
    pub context_web_tokens: i64,
    #[serde(default)]
    pub compression_ratio: f64,
    #[serde(default)]
    pub graph_overhead: i64,
    #[serde(default)]
    pub trivial_change_penalty: i64,
    #[serde(default = "default_useful_when")]
    pub useful_when: Vec<String>,
    #[serde(default = "default_not_useful_when")]
    pub not_useful_when: Vec<String>,
}

impl Default for ContextWebEvaluation {
    fn default() -> Self {
        Self {
            naive_tokens: 0,
            context_web_tokens: 0,
            compression_ratio: 0.0,
            graph_overhead: 0,
            trivial_change_penalty: 0,
            useful_when: default_useful_when(),
            not_useful_when: default_not_useful_when(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextWebIndex {
    #[serde(default)]
    pub repo_id: String,
    #[serde(default)]
    pub commit_sha: String,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default)]
    pub file_hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub symbol_hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub last_incremental_update: String,
    #[serde(default)]
    pub graph_state_hash: String,
    #[serde(default)]
    pub index_state_hash: String,
    #[serde(default = "incremental_strategy")]
    pub update_strategy: String,
}

impl Default for ContextWebIndex {
    fn default() -> Self {
        Self {
            repo_id: String::new(),
            commit_sha: String::new(),
            changed_files: Vec::new(),
            file_hashes: BTreeMap::new(),
            symbol_hashes: BTreeMap::new(),
            last_incremental_update: String::new(),
            graph_state_hash: String::new(),
            index_state_hash: String::new(),
            update_strategy: incremental_strategy(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextWebAtom {
    pub id: String,
    #[serde(default = "file_kind")]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub source_ref: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub estimated_tokens: i64,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub citations: Vec<ContextWebCitation>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub trigger_description: String,
    #[serde(default)]
    pub why_relevant: String,
    #[serde(default = "summary_hydration")]
    pub hydration_level: String,
    #[serde(default)]
    pub hydration_handle: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextWebEdge {
    pub from_id: String,
    pub to_id: String,
    pub relation: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub score: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextWebPath {
    pub node_ids: Vec<String>,
    #[serde(default)]
    pub edge_relations: Vec<String>,
    #[serde(default)]
    pub score: f64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct ContextWebTokenLedger {
    #[serde(default)]
    pub raw_candidate_tokens: i64,
    #[serde(default)]
    pub packed_tokens: i64,
    #[serde(default)]
    pub saved_tokens: i64,
    #[serde(default)]
    pub tool_schema_tokens_avoided: i64,
    #[serde(default)]
    pub hydration_tokens_avoided: i64,
    #[serde(default)]
    pub cache_hits: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ContextWebSpendPlan {
    #[serde(default)]
    pub spend_plan_id: String,
    #[serde(default)]
    pub budget_allocation: BTreeMap<String, i64>,
    #[serde(default)]
    pub hydration_policy: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub expected_savings: BTreeMap<String, Value>,
    #[serde(default)]
    pub cache_keys: BTreeMap<String, Value>,
    #[serde(default)]
    pub degradations: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ContextWebStructuralBankResult {
    pub bank_id: String,
    pub task_signature: String,
    #[serde(default)]
    pub anchor_nodes: Vec<String>,
    #[serde(default)]
    pub bridge_nodes: Vec<String>,
    #[serde(default)]
    pub ancestor_paths: Vec<Vec<String>>,
    #[serde(default)]
    pub sibling_clusters: Vec<Vec<String>>,
    #[serde(default)]
    pub blast_radius_candidates: Vec<String>,
    #[serde(default)]
    pub structural_scores: BTreeMap<String, f64>,
    #[serde(default)]
    pub explanation_lines: Vec<String>,
    #[serde(default)]
    pub citation_handles: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextWebPolicy {
    #[serde(default = "default_generated_artifact_paths")]
    pub generated_artifact_paths: Vec<String>,
    #[serde(default)]
    pub allow_generated_artifacts: bool,
    #[serde(default)]
    pub explicit_targets: Vec<String>,
}

impl Default for ContextWebPolicy {
    fn default() -> Self {
        Self {
            generated_artifact_paths: default_generated_artifact_paths(),
            allow_generated_artifacts: false,
            explicit_targets: Vec::new(),
        }
    }
}

impl ContextWebPolicy {
    pub fn allows_atom(&self, atom: &ContextWebAtom) -> bool {
        if !is_generated_artifact(&atom.id, &atom.labels) {
            return true;
        }
        self.allow_generated_artifacts || self.explicit_targets.contains(&atom.id)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextWebPack {
    pub run_id: String,
    pub query: String,
    #[serde(default = "standard_mode")]
    pub mode: String,
    #[serde(default)]
    pub budget: ContextWebBudget,
    #[serde(default)]
    pub atoms: Vec<ContextWebAtom>,
    #[serde(default)]
    pub edges: Vec<ContextWebEdge>,
    #[serde(default)]
    pub paths: Vec<ContextWebPath>,
    #[serde(default)]
    pub tools_used: Vec<Map<String, Value>>,
    #[serde(default)]
    pub source_mix: BTreeMap<String, i64>,
    #[serde(default)]
    pub token_ledger: ContextWebTokenLedger,
    #[serde(default)]
    pub provenance: Map<String, Value>,
    #[serde(default)]
    pub spend_plan: ContextWebSpendPlan,
    #[serde(default)]
    pub validation: ContextWebValidationSummary,
    #[serde(default)]
    pub evaluation: ContextWebEvaluation,
    #[serde(default)]
    pub index: ContextWebIndex,
    #[serde(default)]
    pub structural_bank: Vec<ContextWebStructuralBankResult>,
    #[serde(default)]
    pub solution_cards: Vec<Map<String, Value>>,
    #[serde(default)]
    pub deferred_ingestion: Vec<Map<String, Value>>,
    #[serde(default)]
    pub state_hash: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ContextWebPackInput {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub budget: ContextWebBudget,
    #[serde(default)]
    pub atoms: Vec<ContextWebAtom>,
    #[serde(default)]
    pub edges: Vec<ContextWebEdge>,
    #[serde(default)]
    pub paths: Vec<ContextWebPath>,
    #[serde(default)]
    pub tools_used: Vec<Map<String, Value>>,
    #[serde(default, alias = "token_ledger")]
    pub ledger: ContextWebTokenLedger,
    #[serde(default)]
    pub policy: ContextWebPolicy,
}

impl ContextWebPackInput {
    pub fn into_pack_and_policy(self) -> (ContextWebPack, ContextWebPolicy) {
        (
            ContextWebPack {
                run_id: "run-context-fixture".to_string(),
                query: "fixture query".to_string(),
                mode: self.mode.unwrap_or_else(standard_mode),
                budget: self.budget,
                atoms: self.atoms,
                edges: self.edges,
                paths: self.paths,
                tools_used: self.tools_used,
                source_mix: BTreeMap::new(),
                token_ledger: self.ledger,
                provenance: Map::new(),
                spend_plan: ContextWebSpendPlan::default(),
                validation: ContextWebValidationSummary::default(),
                evaluation: ContextWebEvaluation::default(),
                index: ContextWebIndex::default(),
                structural_bank: Vec::new(),
                solution_cards: Vec::new(),
                deferred_ingestion: Vec::new(),
                state_hash: String::new(),
            },
            self.policy,
        )
    }
}

impl ContextWebPack {
    pub fn bounded(&self, policy: Option<&ContextWebPolicy>) -> Self {
        let effective_budget = self.budget.capped_for_mode(&self.mode);
        let default_policy = ContextWebPolicy::default();
        let policy = policy.unwrap_or(&default_policy);
        let mut selected = Vec::new();
        let mut packed_tokens = 0;
        let mut hydration_tokens_avoided = self.token_ledger.hydration_tokens_avoided;
        let mut why_included = Map::new();
        let mut why_excluded = Map::new();
        let (mut atoms, candidate_edges, merged_atom_ids) =
            merge_duplicate_atoms_and_edges(self.atoms.clone(), self.edges.clone());

        for atom in &mut atoms {
            if atom.estimated_tokens <= 0 {
                atom.estimated_tokens = calibrated_atom_tokens(atom);
            }
        }
        atoms.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.id.cmp(&right.id))
        });

        for atom in atoms {
            if !policy.allows_atom(&atom) {
                why_excluded.insert(
                    atom.id.clone(),
                    Value::String("generated_artifact_quarantined".to_string()),
                );
                continue;
            }
            let next_tokens = atom.estimated_tokens.max(0);
            if selected.len() >= effective_budget.max_atoms {
                why_excluded.insert(
                    atom.id.clone(),
                    Value::String("atom_budget_exhausted".to_string()),
                );
                continue;
            }
            if packed_tokens + next_tokens > effective_budget.max_tokens {
                if let Some(summary_atom) = summary_fidelity_atom(&atom) {
                    let summary_tokens = summary_atom.estimated_tokens.max(0);
                    if packed_tokens + summary_tokens <= effective_budget.max_tokens {
                        hydration_tokens_avoided += (next_tokens - summary_tokens).max(0);
                        packed_tokens += summary_tokens;
                        why_included.insert(
                            summary_atom.id.clone(),
                            Value::String("included_as_summary".to_string()),
                        );
                        selected.push(summary_atom);
                        continue;
                    }
                }
                why_excluded.insert(
                    atom.id.clone(),
                    Value::String("token_budget_exhausted".to_string()),
                );
                continue;
            }
            packed_tokens += next_tokens;
            why_included.insert(
                atom.id.clone(),
                Value::String("ranked_within_budget".to_string()),
            );
            selected.push(atom);
        }

        let selected_ids = selected
            .iter()
            .map(|atom| atom.id.as_str())
            .collect::<Vec<_>>();
        let edges = candidate_edges
            .iter()
            .take(effective_budget.max_edges)
            .filter(|edge| {
                selected_ids.contains(&edge.from_id.as_str())
                    && selected_ids.contains(&edge.to_id.as_str())
            })
            .cloned()
            .collect::<Vec<_>>();
        let paths = self
            .paths
            .iter()
            .take(effective_budget.max_paths)
            .filter(|path| {
                path.node_ids
                    .iter()
                    .all(|node_id| selected_ids.contains(&node_id.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();

        let raw_tokens = if self.token_ledger.raw_candidate_tokens != 0 {
            self.token_ledger.raw_candidate_tokens
        } else {
            self.atoms
                .iter()
                .map(|atom| atom.estimated_tokens.max(0))
                .sum()
        };
        let ledger = ContextWebTokenLedger {
            raw_candidate_tokens: raw_tokens,
            packed_tokens,
            saved_tokens: (raw_tokens - packed_tokens).max(0),
            tool_schema_tokens_avoided: self.token_ledger.tool_schema_tokens_avoided,
            hydration_tokens_avoided,
            cache_hits: self.token_ledger.cache_hits,
        };

        let mut provenance = self.provenance.clone();
        let mut policies = existing_string_list(&provenance, "policies_applied");
        push_unique(&mut policies, "generated_artifact_quarantine".to_string());
        push_unique(&mut policies, "token_budget".to_string());
        if hydration_tokens_avoided > self.token_ledger.hydration_tokens_avoided {
            push_unique(&mut policies, "summary_hydration".to_string());
        }
        if !merged_atom_ids.is_empty() {
            push_unique(&mut policies, "atom_dedup_merge".to_string());
        }
        let policies_applied = policies.into_iter().map(Value::String).collect::<Vec<_>>();
        let external_quarantined = why_excluded
            .values()
            .any(|reason| reason.as_str() == Some("generated_artifact_quarantined"));
        provenance.insert("why_included".to_string(), Value::Object(why_included));
        provenance.insert("why_excluded".to_string(), Value::Object(why_excluded));
        provenance.insert(
            "policies_applied".to_string(),
            Value::Array(policies_applied),
        );
        provenance.insert(
            "external_content_quarantined".to_string(),
            Value::Bool(external_quarantined),
        );
        if !merged_atom_ids.is_empty() {
            provenance.insert(
                "merged_atom_ids".to_string(),
                Value::Array(merged_atom_ids.into_iter().map(Value::String).collect()),
            );
        }

        Self {
            run_id: self.run_id.clone(),
            query: self.query.clone(),
            mode: if self.mode.is_empty() {
                standard_mode()
            } else {
                self.mode.clone()
            },
            budget: effective_budget.clone(),
            atoms: selected.clone(),
            edges: edges.clone(),
            paths: paths.clone(),
            tools_used: self
                .tools_used
                .iter()
                .take(effective_budget.max_tools)
                .cloned()
                .collect(),
            source_mix: source_mix(&selected),
            token_ledger: ledger,
            provenance: provenance.clone(),
            spend_plan: self.spend_plan.clone(),
            validation: validation_summary(&selected, packed_tokens, &effective_budget),
            evaluation: evaluation_summary(raw_tokens, packed_tokens, &edges, &paths, &provenance),
            index: self.index.clone(),
            structural_bank: self.structural_bank.clone(),
            solution_cards: self.solution_cards.clone(),
            deferred_ingestion: self.deferred_ingestion.clone(),
            state_hash: self.state_hash.clone(),
        }
    }
}

fn merge_duplicate_atoms_and_edges(
    atoms: Vec<ContextWebAtom>,
    edges: Vec<ContextWebEdge>,
) -> (Vec<ContextWebAtom>, Vec<ContextWebEdge>, Vec<String>) {
    let mut merged_by_key: BTreeMap<String, ContextWebAtom> = BTreeMap::new();
    let mut representative_by_id: HashMap<String, String> = HashMap::new();
    let mut merged_atom_ids = Vec::new();

    for mut atom in atoms {
        let key = merge_key(&atom);
        if let Some(existing) = merged_by_key.get_mut(&key) {
            let representative_id = existing.id.clone();
            representative_by_id.insert(atom.id.clone(), representative_id);
            push_unique(&mut merged_atom_ids, atom.id.clone());
            push_unique(&mut merged_atom_ids, existing.id.clone());
            merge_atom(existing, &mut atom);
        } else {
            representative_by_id.insert(atom.id.clone(), atom.id.clone());
            merged_by_key.insert(key, atom);
        }
    }

    let mut merged_edges = Vec::new();
    for mut edge in edges {
        if let Some(from_id) = representative_by_id.get(&edge.from_id) {
            edge.from_id = from_id.clone();
        }
        if let Some(to_id) = representative_by_id.get(&edge.to_id) {
            edge.to_id = to_id.clone();
        }
        if edge.from_id == edge.to_id {
            continue;
        }
        if !merged_edges.iter().any(|existing: &ContextWebEdge| {
            existing.from_id == edge.from_id
                && existing.to_id == edge.to_id
                && existing.relation == edge.relation
        }) {
            merged_edges.push(edge);
        }
    }

    (
        merged_by_key.into_values().collect(),
        merged_edges,
        merged_atom_ids,
    )
}

fn merge_key(atom: &ContextWebAtom) -> String {
    let source_ref = atom.source_ref.trim();
    if !source_ref.is_empty() {
        return format!("source:{source_ref}");
    }
    let title = atom.title.trim();
    if !title.is_empty() {
        return format!("title:{}", title.to_lowercase());
    }
    format!("id:{}", atom.id)
}

fn merge_atom(target: &mut ContextWebAtom, source: &mut ContextWebAtom) {
    if source.score > target.score {
        target.score = source.score;
    }
    if target.summary.trim().is_empty()
        || source.summary.len() > target.summary.len()
            && target.hydration_level.eq_ignore_ascii_case("summary")
    {
        target.summary = source.summary.clone();
    }
    if target.source_ref.trim().is_empty() {
        target.source_ref = source.source_ref.clone();
    }
    if target.why_relevant.trim().is_empty() {
        target.why_relevant = source.why_relevant.clone();
    }
    if target.trigger_description.trim().is_empty() {
        target.trigger_description = source.trigger_description.clone();
    }
    if target.hydration_handle.trim().is_empty() {
        target.hydration_handle = source.hydration_handle.clone();
    }
    for channel in source.channels.drain(..) {
        push_unique(&mut target.channels, channel);
    }
    for label in source.labels.drain(..) {
        push_unique(&mut target.labels, label);
    }
    for citation in source.citations.drain(..) {
        if !target.citations.contains(&citation) {
            target.citations.push(citation);
        }
    }
}

fn summary_fidelity_atom(atom: &ContextWebAtom) -> Option<ContextWebAtom> {
    if atom.summary.trim().is_empty() || atom.hydration_level.eq_ignore_ascii_case("summary") {
        return None;
    }
    let mut summary = atom.clone();
    summary.hydration_level = "summary".to_string();
    summary.estimated_tokens = calibrated_text_tokens(&summary.summary)
        .saturating_add(calibrated_text_tokens(&summary.title))
        .max(1);
    Some(summary)
}

fn calibrated_atom_tokens(atom: &ContextWebAtom) -> i64 {
    let citation_tokens = (atom.citations.len() as i64) * 8;
    let label_tokens = (atom.labels.len() as i64) * 2;
    let body_tokens = calibrated_text_tokens(&atom.title)
        + calibrated_text_tokens(&atom.summary)
        + calibrated_text_tokens(&atom.source_ref)
        + calibrated_text_tokens(&atom.why_relevant)
        + calibrated_text_tokens(&atom.trigger_description);
    (body_tokens + citation_tokens + label_tokens).max(1)
}

fn calibrated_text_tokens(value: &str) -> i64 {
    let bytes = value.trim().len() as i64;
    if bytes == 0 {
        0
    } else {
        ((bytes + 3) / 4).max(1)
    }
}

pub fn normalize_context_web_node_id(raw: &str) -> String {
    let value = raw.trim();
    if value.is_empty() {
        return String::new();
    }
    if let Some(path) = value.strip_prefix("file:") {
        if is_generated_path(path) {
            return format!("generated:{path}");
        }
        return value.to_string();
    }
    if value.starts_with("generated:") {
        return value.to_string();
    }
    if let Some(path) = value.strip_prefix("file/") {
        if is_generated_path(path) {
            return format!("generated:{path}");
        }
        if path.starts_with("apps/") {
            return format!("file:{path}");
        }
        return format!("file:apps/{path}");
    }
    if is_generated_path(value) {
        return format!("generated:{value}");
    }
    value.to_string()
}

pub fn is_generated_artifact(identifier: &str, labels: &[String]) -> bool {
    if labels.iter().any(|label| {
        let lowered = label.to_lowercase();
        lowered == "generatedartifact" || lowered == "generated_artifact"
    }) {
        return true;
    }
    let normalized = normalize_context_web_node_id(identifier);
    if normalized.starts_with("generated:") {
        return true;
    }
    if let Some(path) = normalized.strip_prefix("file:") {
        return is_generated_path(path);
    }
    is_generated_path(&normalized)
}

fn source_mix(atoms: &[ContextWebAtom]) -> BTreeMap<String, i64> {
    let mut mix = BTreeMap::from([
        ("artifact".to_string(), 0),
        ("claim".to_string(), 0),
        ("code".to_string(), 0),
        ("postmortem".to_string(), 0),
        ("skill".to_string(), 0),
        ("web".to_string(), 0),
    ]);
    for atom in atoms {
        let bucket = match atom.kind.as_str() {
            "file" | "symbol" | "test" => Some("code"),
            "webdoc" | "browser_snapshot" => Some("web"),
            "postmortem" => Some("postmortem"),
            "claim" | "tension" => Some("claim"),
            "context_artifact" | "tool_output" => Some("artifact"),
            "skill" | "plugin_method" => Some("skill"),
            _ => None,
        };
        if let Some(bucket) = bucket {
            *mix.entry(bucket.to_string()).or_insert(0) += 1;
        }
    }
    mix
}

fn validation_summary(
    atoms: &[ContextWebAtom],
    packed_tokens: i64,
    budget: &ContextWebBudget,
) -> ContextWebValidationSummary {
    let mut findings = Vec::new();
    let mut scores = BTreeMap::new();

    if atoms.len() >= 8 && packed_tokens >= (budget.max_tokens as f64 * 0.7) as i64 {
        scores.insert("lost_in_middle_risk".to_string(), 0.72);
        findings.push(ContextWebValidatorFinding {
            validator_id: "lost_in_middle_risk".to_string(),
            severity: "medium".to_string(),
            score: 0.72,
            summary: "Many atoms survived packing, so middle-context dilution risk is elevated."
                .to_string(),
            affected_atom_ids: atoms[2..atoms.len().saturating_sub(2)]
                .iter()
                .map(|atom| atom.id.clone())
                .collect(),
        });
    } else {
        scores.insert("lost_in_middle_risk".to_string(), 0.12);
    }

    let top_external = atoms
        .iter()
        .take(3)
        .filter(|atom| {
            atom.channels
                .iter()
                .any(|channel| channel == "external_advisory")
        })
        .map(|atom| atom.id.clone())
        .collect::<Vec<_>>();
    if top_external.is_empty() {
        scores.insert("context_poisoning_risk".to_string(), 0.08);
    } else {
        let score = if atoms
            .first()
            .is_some_and(|atom| top_external.iter().any(|id| id == &atom.id))
        {
            0.85
        } else {
            0.56
        };
        scores.insert("context_poisoning_risk".to_string(), score);
        findings.push(ContextWebValidatorFinding {
            validator_id: "context_poisoning_risk".to_string(),
            severity: if score >= 0.8 { "high" } else { "medium" }.to_string(),
            score,
            summary: "External advisory atoms ranked near the top of the context pack.".to_string(),
            affected_atom_ids: top_external,
        });
    }

    let mut kinds = atoms
        .iter()
        .map(|atom| atom.kind.as_str())
        .collect::<Vec<_>>();
    kinds.sort_unstable();
    kinds.dedup();
    let distraction_score = (kinds.len() as f64 / atoms.len().max(1) as f64).min(1.0);
    scores.insert("context_distraction_score".to_string(), distraction_score);
    if kinds.len() >= 5 && atoms.len() >= 6 {
        findings.push(ContextWebValidatorFinding {
            validator_id: "context_distraction_score".to_string(),
            severity: "medium".to_string(),
            score: distraction_score,
            summary: "The selected atom mix spans many kinds, which may reduce focus.".to_string(),
            affected_atom_ids: atoms.iter().map(|atom| atom.id.clone()).collect(),
        });
    }

    let clashes = clashing_atom_ids(atoms);
    let clash_score = if clashes.is_empty() { 0.05 } else { 0.61 };
    scores.insert("context_clash_detector".to_string(), clash_score);
    if !clashes.is_empty() {
        findings.push(ContextWebValidatorFinding {
            validator_id: "context_clash_detector".to_string(),
            severity: "medium".to_string(),
            score: clash_score,
            summary: "Multiple selected atoms share the same source ref or title across kinds."
                .to_string(),
            affected_atom_ids: clashes,
        });
    }

    let passed = !findings.iter().any(|finding| finding.severity == "high");
    ContextWebValidationSummary {
        findings,
        scores,
        passed,
    }
}

fn clashing_atom_ids(atoms: &[ContextWebAtom]) -> Vec<String> {
    let mut seen: HashMap<(String, String), String> = HashMap::new();
    let mut clashes = Vec::new();
    for atom in atoms {
        let key = (
            non_empty_or(&atom.source_ref, &atom.id),
            non_empty_or(&atom.title, &atom.id),
        );
        if let Some(existing) = seen.get(&key) {
            if existing != &atom.id {
                push_unique(&mut clashes, existing.clone());
                push_unique(&mut clashes, atom.id.clone());
            }
        } else {
            seen.insert(key, atom.id.clone());
        }
    }
    clashes
}

fn evaluation_summary(
    raw_tokens: i64,
    packed_tokens: i64,
    edges: &[ContextWebEdge],
    paths: &[ContextWebPath],
    provenance: &Map<String, Value>,
) -> ContextWebEvaluation {
    let graph_overhead = edges.len() as i64 * 12 + paths.len() as i64 * 24;
    let changed_files = provenance
        .get("mode_semantics")
        .and_then(Value::as_object)
        .and_then(|value| value.get("changed_files"))
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let trivial_change_penalty = if changed_files <= 1 && raw_tokens <= 600 {
        graph_overhead
    } else {
        0
    };
    ContextWebEvaluation {
        naive_tokens: raw_tokens,
        context_web_tokens: packed_tokens,
        compression_ratio: if packed_tokens == 0 {
            0.0
        } else {
            round3(raw_tokens as f64 / packed_tokens as f64)
        },
        graph_overhead,
        trivial_change_penalty,
        useful_when: default_useful_when(),
        not_useful_when: default_not_useful_when(),
    }
}

fn existing_string_list(map: &Map<String, Value>, key: &str) -> Vec<String> {
    map.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn is_generated_path(value: &str) -> bool {
    let path = value.trim_start_matches('/');
    GENERATED_ARTIFACT_PREFIXES.iter().any(|prefix| {
        let trimmed_prefix = prefix.trim_end_matches('/');
        path == trimmed_prefix || path.starts_with(prefix)
    })
}

fn non_empty_or(value: &str, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn default_max_tokens() -> i64 {
    4000
}

fn default_max_atoms() -> usize {
    24
}

fn default_max_edges() -> usize {
    48
}

fn default_max_paths() -> usize {
    8
}

fn default_max_tools() -> usize {
    5
}

fn low_severity() -> String {
    "low".to_string()
}

fn default_true() -> bool {
    true
}

fn default_useful_when() -> Vec<String> {
    vec![
        "multi_file".to_string(),
        "cross_module".to_string(),
        "risky_change".to_string(),
    ]
}

fn default_not_useful_when() -> Vec<String> {
    vec!["tiny_one_file_edit".to_string()]
}

fn incremental_strategy() -> String {
    "incremental".to_string()
}

fn file_kind() -> String {
    "file".to_string()
}

fn summary_hydration() -> String {
    "summary".to_string()
}

fn default_generated_artifact_paths() -> Vec<String> {
    vec![
        "graphify-out/**".to_string(),
        "dist/**".to_string(),
        "build/**".to_string(),
        ".next/**".to_string(),
        "node_modules/**".to_string(),
    ]
}

fn standard_mode() -> String {
    "standard".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(id: &str, kind: &str, source_ref: &str, title: &str, score: f64) -> ContextWebAtom {
        ContextWebAtom {
            id: id.to_string(),
            kind: kind.to_string(),
            source_ref: source_ref.to_string(),
            title: title.to_string(),
            summary: format!("summary for {title}"),
            score,
            estimated_tokens: 1_000,
            hydration_level: "full".to_string(),
            hydration_handle: format!("hydrate:{id}"),
            channels: Vec::new(),
            citations: Vec::new(),
            labels: Vec::new(),
            trigger_description: String::new(),
            why_relevant: String::new(),
        }
    }

    #[test]
    fn bounded_merges_duplicate_sources_and_repoints_edges() {
        let pack = ContextWebPack {
            run_id: "run".to_string(),
            query: "query".to_string(),
            mode: "standard".to_string(),
            budget: ContextWebBudget {
                max_tokens: 100,
                max_atoms: 10,
                max_edges: 10,
                max_paths: 10,
                max_tools: 5,
            },
            atoms: vec![
                atom("a", "file", "src/lib.rs", "Lib", 0.9),
                atom("b", "symbol", "src/lib.rs", "Lib", 0.7),
                atom("c", "file", "src/main.rs", "Main", 0.8),
            ],
            edges: vec![ContextWebEdge {
                from_id: "b".to_string(),
                to_id: "c".to_string(),
                relation: "MENTIONS".to_string(),
                reason: String::new(),
                score: 1.0,
            }],
            paths: Vec::new(),
            tools_used: Vec::new(),
            source_mix: BTreeMap::new(),
            token_ledger: ContextWebTokenLedger::default(),
            provenance: Map::new(),
            spend_plan: ContextWebSpendPlan::default(),
            validation: ContextWebValidationSummary::default(),
            evaluation: ContextWebEvaluation::default(),
            index: ContextWebIndex::default(),
            structural_bank: Vec::new(),
            solution_cards: Vec::new(),
            deferred_ingestion: Vec::new(),
            state_hash: String::new(),
        };

        let bounded = pack.bounded(None);

        assert_eq!(bounded.atoms.len(), 2);
        assert!(bounded
            .edges
            .iter()
            .any(|edge| edge.from_id == "a" && edge.to_id == "c"));
        assert!(!bounded
            .validation
            .findings
            .iter()
            .any(|finding| finding.validator_id == "context_clash_detector"));
    }

    #[test]
    fn bounded_tries_summary_hydration_before_dropping_atom() {
        let mut large = atom("large", "file", "src/large.rs", "Large", 1.0);
        large.summary = "short".to_string();
        large.trigger_description = "x".repeat(8_000);
        large.estimated_tokens = 8_000;

        let pack = ContextWebPack {
            run_id: "run".to_string(),
            query: "query".to_string(),
            mode: "standard".to_string(),
            budget: ContextWebBudget {
                max_tokens: 20,
                max_atoms: 2,
                max_edges: 0,
                max_paths: 0,
                max_tools: 0,
            },
            atoms: vec![large],
            edges: Vec::new(),
            paths: Vec::new(),
            tools_used: Vec::new(),
            source_mix: BTreeMap::new(),
            token_ledger: ContextWebTokenLedger::default(),
            provenance: Map::new(),
            spend_plan: ContextWebSpendPlan::default(),
            validation: ContextWebValidationSummary::default(),
            evaluation: ContextWebEvaluation::default(),
            index: ContextWebIndex::default(),
            structural_bank: Vec::new(),
            solution_cards: Vec::new(),
            deferred_ingestion: Vec::new(),
            state_hash: String::new(),
        };

        let bounded = pack.bounded(None);

        assert_eq!(bounded.atoms.len(), 1);
        assert_eq!(bounded.atoms[0].hydration_level, "summary");
        assert!(bounded.token_ledger.hydration_tokens_avoided > 0);
        assert_eq!(
            bounded.provenance["why_included"]["large"].as_str(),
            Some("included_as_summary")
        );
    }
}
