use crate::types::Payload;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const DEFAULT_PERMISSIONS: &[&str] = &["web_browse", "graph_read"];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolSelectionState {
    pub tool_id: String,
    pub name: String,
    pub reason: String,
    pub cost: String,
    pub permissions: Vec<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlockedTool {
    pub tool_id: String,
    pub name: String,
    pub missing_permissions: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompiledToolkit {
    pub task_type: String,
    pub selected_tools: Vec<ToolSelectionState>,
    pub blocked_tools: Vec<BlockedTool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolContract {
    pub tool_id: String,
    pub name: String,
    pub skills: Vec<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub cost: String,
    pub permissions: Vec<String>,
    pub validator: String,
    pub failure_modes: Vec<String>,
    pub task_types: Vec<String>,
}

impl ToolContract {
    fn from_static(definition: StaticToolContract) -> Self {
        Self {
            tool_id: definition.tool_id.to_string(),
            name: definition.name.to_string(),
            skills: strings(definition.skills),
            inputs: strings(definition.inputs),
            outputs: strings(definition.outputs),
            cost: definition.cost.to_string(),
            permissions: strings(definition.permissions),
            validator: definition.validator.to_string(),
            failure_modes: strings(definition.failure_modes),
            task_types: strings(definition.task_types),
        }
    }

    fn to_selection(&self, reason: String) -> ToolSelectionState {
        ToolSelectionState {
            tool_id: self.tool_id.clone(),
            name: self.name.clone(),
            reason,
            cost: self.cost.clone(),
            permissions: self.permissions.clone(),
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct StaticToolContract {
    tool_id: &'static str,
    name: &'static str,
    skills: &'static [&'static str],
    inputs: &'static [&'static str],
    outputs: &'static [&'static str],
    cost: &'static str,
    permissions: &'static [&'static str],
    validator: &'static str,
    failure_modes: &'static [&'static str],
    task_types: &'static [&'static str],
}

pub fn compile_task_toolkit(
    task_type: &str,
    permissions: Option<Vec<String>>,
    scope: Option<Payload>,
) -> CompiledToolkit {
    let normalized = normalize_task_type(task_type);
    let scope = scope.unwrap_or_default();
    let permission_set = permissions
        .or_else(|| scope.get("permissions").map(normalize_permissions_value))
        .unwrap_or_else(|| strings(DEFAULT_PERMISSIONS))
        .into_iter()
        .collect::<BTreeSet<_>>();

    let catalog = default_tools();
    let mut candidate_ids = catalog
        .iter()
        .filter(|tool| tool.task_types.iter().any(|task| task == &normalized))
        .map(|tool| tool.tool_id.clone())
        .collect::<Vec<_>>();

    for requested in requested_tools(&scope) {
        if catalog.iter().any(|tool| tool.tool_id == requested)
            && !candidate_ids.contains(&requested)
        {
            candidate_ids.push(requested);
        }
    }
    if !candidate_ids
        .iter()
        .any(|tool_id| tool_id == "context_artifact_compile")
    {
        candidate_ids.push("context_artifact_compile".to_string());
    }

    let mut selected_tools = Vec::new();
    let mut blocked_tools = Vec::new();
    for tool_id in candidate_ids {
        let tool = catalog
            .iter()
            .find(|candidate| candidate.tool_id == tool_id)
            .expect("candidate tool id should exist in default catalog");
        let missing = missing_permissions(tool, &permission_set);
        if missing.is_empty() {
            selected_tools.push(tool.to_selection(selection_reason(tool, &normalized)));
        } else {
            blocked_tools.push(BlockedTool {
                tool_id: tool.tool_id.clone(),
                name: tool.name.clone(),
                missing_permissions: missing,
                reason: "Tool permissions are not available for this run.".to_string(),
            });
        }
    }

    CompiledToolkit {
        task_type: normalized,
        selected_tools,
        blocked_tools,
    }
}

pub fn select_tools(task_type: &str, scope: Option<Payload>) -> Vec<ToolSelectionState> {
    let permissions = scope
        .as_ref()
        .and_then(|payload| payload.get("permissions").map(normalize_permissions_value));
    compile_task_toolkit(task_type, permissions, scope).selected_tools
}

pub fn catalog_as_dicts() -> Vec<ToolContract> {
    default_tools()
}

pub fn normalize_permissions(value: Option<Value>) -> Vec<String> {
    value
        .as_ref()
        .map(normalize_permissions_value)
        .unwrap_or_else(|| strings(DEFAULT_PERMISSIONS))
}

fn normalize_permissions_value(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => split_permission_string(text),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        Value::Null => strings(DEFAULT_PERMISSIONS),
        other => {
            let text = other.to_string();
            split_permission_string(&text)
        }
    }
}

fn split_permission_string(text: &str) -> Vec<String> {
    if text.contains(',') {
        text.split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            Vec::new()
        } else {
            vec![trimmed.to_string()]
        }
    }
}

fn normalize_task_type(value: &str) -> String {
    let normalized = value.trim().to_lowercase();
    match normalized.as_str() {
        "search" | "research" | "plan" | "fix" | "review" | "design" | "remember" | "memory" => {
            normalized
        }
        _ => "other".to_string(),
    }
}

fn requested_tools(scope: &Payload) -> Vec<String> {
    match scope.get("tool_scope") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
        Some(Value::String(item)) => {
            let trimmed = item.trim();
            if trimmed.is_empty() {
                Vec::new()
            } else {
                vec![trimmed.to_string()]
            }
        }
        _ => Vec::new(),
    }
}

fn missing_permissions(tool: &ToolContract, permissions: &BTreeSet<String>) -> Vec<String> {
    let mut missing = tool
        .permissions
        .iter()
        .filter(|permission| !permissions.contains(*permission))
        .cloned()
        .collect::<Vec<_>>();
    missing.sort();
    missing
}

fn selection_reason(tool: &ToolContract, task_type: &str) -> String {
    if tool.tool_id == "native_search" {
        return format!("{task_type} benefits from fresh evidence acquisition.");
    }
    if tool.tool_id == "fractal_expansion" {
        return "Fractal expansion uses gap-frontier search with Rust PPR.".to_string();
    }
    if tool.tool_id.starts_with("context_web.") {
        return format!("{task_type} benefits from bounded context-web packing.");
    }
    if tool.tool_id.starts_with("theseus_browser.") {
        return "Research mode can use browser evidence as advisory context.".to_string();
    }
    if tool.tool_id == "neural_search.search" {
        return "Neural retrieval can rank candidate atoms before hydration.".to_string();
    }
    if tool.tool_id == "memory_patch_validation" {
        return "Memory changes must stay proposals until validated.".to_string();
    }
    "Harness runs should produce a reusable artifact.".to_string()
}

fn default_tools() -> Vec<ToolContract> {
    DEFAULT_TOOLS
        .iter()
        .copied()
        .map(ToolContract::from_static)
        .collect()
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

const DEFAULT_TOOLS: &[StaticToolContract] = &[
    StaticToolContract {
        tool_id: "native_search",
        name: "Native Search",
        skills: &["local_webdoc_search", "redis_priors", "graph_candidates"],
        inputs: &["query", "scope", "budget"],
        outputs: &["ranked_results", "search_trace_id", "graph_candidates"],
        cost: "low",
        permissions: &["web_browse", "graph_read"],
        validator: "ranked_results_json",
        failure_modes: &["disabled", "empty_index", "redis_unavailable"],
        task_types: &["search", "research", "plan", "fix", "review", "design"],
    },
    StaticToolContract {
        tool_id: "fractal_expansion",
        name: "Fractal Expansion",
        skills: &["fractal_expansion", "gap_frontier_search", "rust_ppr"],
        inputs: &["query", "scope", "budget"],
        outputs: &["ranked_results", "search_trace_id", "gap_walk_metadata"],
        cost: "medium",
        permissions: &["graph_read"],
        validator: "gap_walk_results_json",
        failure_modes: &[
            "rust_ppr_unavailable",
            "insufficient_communities",
            "no_ppr_frontier",
        ],
        task_types: &["search", "research", "plan", "review", "design"],
    },
    StaticToolContract {
        tool_id: "context_web.mini",
        name: "Context-Web Mini",
        skills: &["minimal_context_probe", "generated_artifact_quarantine"],
        inputs: &["run_id", "query", "budget_tokens"],
        outputs: &["context_web_pack", "token_ledger", "provenance"],
        cost: "low",
        permissions: &["graph_read"],
        validator: "context_web_pack_bounded",
        failure_modes: &["empty_run", "thg_unavailable"],
        task_types: &[
            "search", "research", "plan", "fix", "review", "design", "other",
        ],
    },
    StaticToolContract {
        tool_id: "context_web.pack",
        name: "Context-Web Pack",
        skills: &["bounded_context_packing", "thg_graph_algorithms"],
        inputs: &["run_id", "query", "mode", "budget_tokens"],
        outputs: &["context_web_pack", "spend_plan", "token_ledger"],
        cost: "medium",
        permissions: &["graph_read"],
        validator: "context_web_pack_bounded",
        failure_modes: &["budget_exhausted", "thg_unavailable"],
        task_types: &["research", "review", "plan", "design"],
    },
    StaticToolContract {
        tool_id: "theseus_browser.search",
        name: "Theseus Browser Search",
        skills: &["browser_search", "webdoc_capture_policy"],
        inputs: &["query", "mode", "capture"],
        outputs: &["context_atoms", "captured_webdocs", "source_quality"],
        cost: "medium",
        permissions: &["web_browse"],
        validator: "external_content_advisory",
        failure_modes: &["network_unavailable", "capture_denied"],
        task_types: &[],
    },
    StaticToolContract {
        tool_id: "theseus_browser.read",
        name: "Theseus Browser Read",
        skills: &["browser_read", "claim_extraction_policy"],
        inputs: &["url", "capture", "extract_claims"],
        outputs: &["webdoc_id", "page_snapshot_id", "extracted_atoms"],
        cost: "medium",
        permissions: &["web_browse"],
        validator: "external_content_advisory",
        failure_modes: &["fetch_failed", "capture_denied"],
        task_types: &[],
    },
    StaticToolContract {
        tool_id: "neural_search.search",
        name: "Neural Search",
        skills: &["hybrid_neural_search", "graph_score_fusion"],
        inputs: &["query", "spaces", "top_k", "hydrate"],
        outputs: &["candidates", "embedding_scores", "graph_scores"],
        cost: "medium",
        permissions: &["graph_read"],
        validator: "context_candidate_shape",
        failure_modes: &["index_unavailable", "heavy_runtime_disabled"],
        task_types: &[],
    },
    StaticToolContract {
        tool_id: "context_web.explain_inclusion",
        name: "Context-Web Explain Inclusion",
        skills: &["context_provenance", "why_included"],
        inputs: &["run_id", "pack_id", "atom_id"],
        outputs: &["why_included", "why_excluded", "policies_applied"],
        cost: "low",
        permissions: &["graph_read"],
        validator: "context_web_provenance_present",
        failure_modes: &["pack_missing"],
        task_types: &[],
    },
    StaticToolContract {
        tool_id: "context_artifact_compile",
        name: "Context Artifact Compile",
        skills: &["capsule_packing", "token_ledger", "artifact_export"],
        inputs: &["run_id", "task", "budget_tokens"],
        outputs: &["context_artifact", "token_ledger", "provenance"],
        cost: "medium",
        permissions: &["graph_read"],
        validator: "context_artifact_compiled",
        failure_modes: &["budget_exhausted", "empty_run"],
        task_types: &[
            "search", "research", "plan", "fix", "review", "design", "remember", "other",
        ],
    },
    StaticToolContract {
        tool_id: "memory_patch_validation",
        name: "Memory Patch Validation",
        skills: &["proposal_review", "provenance_check"],
        inputs: &["run_id", "patch"],
        outputs: &["validation_result"],
        cost: "low",
        permissions: &["graph_read"],
        validator: "human_review_required",
        failure_modes: &["missing_patch", "unsafe_canonical_write"],
        task_types: &["remember", "memory"],
    },
];
