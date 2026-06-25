use std::collections::{BTreeMap, BTreeSet, VecDeque};

use rustyred_thg_core::{
    stable_hash, EdgeRecord, GraphStore, GraphStoreResult, NeighborQuery, NodeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::code_to_spec::collect_code_symbols;
use super::ir::{CodeSymbolSnapshot, CODE_COMPILER_FEATURE_VERSION, CODE_COMPILER_VERSION};
use crate::{CALLS_SYMBOL, DEPENDS_ON_SYMBOL, SOURCE};

pub const CODE_PROCESS_LABEL: &str = "CodeProcessFlow";
pub const PROCESS_ENTRYPOINT: &str = "PROCESS_ENTRYPOINT";
pub const PROCESS_TOUCHES_CODE: &str = "PROCESS_TOUCHES_CODE";

const DEFAULT_MAX_PROCESSES: usize = 32;
const DEFAULT_MAX_STEPS: usize = 32;
const DEFAULT_PROCESS_DEPTH: usize = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeProcessDetectInput {
    pub tenant_id: String,
    pub repo_id: String,
    pub max_processes: usize,
    pub max_steps: usize,
    pub max_depth: usize,
}

impl CodeProcessDetectInput {
    pub fn new(tenant_id: impl Into<String>, repo_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            max_processes: DEFAULT_MAX_PROCESSES,
            max_steps: DEFAULT_MAX_STEPS,
            max_depth: DEFAULT_PROCESS_DEPTH,
        }
    }

    fn process_limit(&self) -> usize {
        if self.max_processes == 0 {
            DEFAULT_MAX_PROCESSES
        } else {
            self.max_processes
        }
    }

    fn step_limit(&self) -> usize {
        if self.max_steps == 0 {
            DEFAULT_MAX_STEPS
        } else {
            self.max_steps
        }
    }

    fn depth_limit(&self) -> usize {
        if self.max_depth == 0 {
            DEFAULT_PROCESS_DEPTH
        } else {
            self.max_depth
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodeProcessStep {
    pub symbol_id: String,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    pub depth: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CodeProcessFlow {
    pub process_id: String,
    pub entry_symbol_id: String,
    pub entry_name: String,
    pub entry_file_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_line: Option<u64>,
    pub trigger: String,
    pub confidence: f64,
    pub steps: Vec<CodeProcessStep>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodeProcessDetectOutput {
    pub tenant_id: String,
    pub repo_id: String,
    pub processes: Vec<CodeProcessFlow>,
}

pub fn detect_code_processes_in_store<S: GraphStore>(
    store: &mut S,
    input: CodeProcessDetectInput,
) -> GraphStoreResult<CodeProcessDetectOutput> {
    let symbols = collect_code_symbols(store, &input.tenant_id, &input.repo_id, 100_000);
    let symbols_by_id = symbols
        .iter()
        .cloned()
        .map(|symbol| (symbol.symbol_id.clone(), symbol))
        .collect::<BTreeMap<_, _>>();
    let mut entrypoints = symbols
        .iter()
        .filter_map(|symbol| entrypoint_trigger(symbol).map(|trigger| (symbol, trigger)))
        .collect::<Vec<_>>();
    entrypoints.sort_by(|left, right| {
        left.0
            .file_path
            .cmp(&right.0.file_path)
            .then_with(|| left.0.line.cmp(&right.0.line))
            .then_with(|| left.0.name.cmp(&right.0.name))
    });
    entrypoints.truncate(input.process_limit());

    let mut processes = Vec::new();
    for (entry, trigger) in entrypoints {
        let flow = build_process_flow(store, &input, &symbols_by_id, entry, trigger);
        store.upsert_node(process_node(&input, &flow))?;
        store.upsert_edge(EdgeRecord::new(
            process_edge_id(&flow.process_id, PROCESS_ENTRYPOINT, &flow.entry_symbol_id),
            &flow.process_id,
            PROCESS_ENTRYPOINT,
            &flow.entry_symbol_id,
            json!({
                "tenant_id": &input.tenant_id,
                "repo_id": &input.repo_id,
                "compiler_version": CODE_COMPILER_VERSION,
                "feature_version": CODE_COMPILER_FEATURE_VERSION,
                "source": SOURCE,
            }),
        ))?;
        for step in &flow.steps {
            store.upsert_edge(EdgeRecord::new(
                process_edge_id(&flow.process_id, PROCESS_TOUCHES_CODE, &step.symbol_id),
                &flow.process_id,
                PROCESS_TOUCHES_CODE,
                &step.symbol_id,
                json!({
                    "tenant_id": &input.tenant_id,
                    "repo_id": &input.repo_id,
                    "depth": step.depth,
                    "compiler_version": CODE_COMPILER_VERSION,
                    "feature_version": CODE_COMPILER_FEATURE_VERSION,
                    "source": SOURCE,
                }),
            ))?;
        }
        processes.push(flow);
    }

    Ok(CodeProcessDetectOutput {
        tenant_id: input.tenant_id,
        repo_id: input.repo_id,
        processes,
    })
}

pub(super) fn count_processes<S: GraphStore>(store: &S, tenant_id: &str, repo_id: &str) -> usize {
    store
        .query_nodes(
            rustyred_thg_core::NodeQuery::label(CODE_PROCESS_LABEL)
                .with_property("tenant_id", json!(tenant_id))
                .with_property("repo_id", json!(repo_id))
                .with_limit(100_000),
        )
        .into_iter()
        .filter(|node| !node.tombstone)
        .count()
}

fn build_process_flow<S: GraphStore>(
    store: &S,
    input: &CodeProcessDetectInput,
    symbols_by_id: &BTreeMap<String, CodeSymbolSnapshot>,
    entry: &CodeSymbolSnapshot,
    trigger: &'static str,
) -> CodeProcessFlow {
    let mut steps = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([(entry.symbol_id.clone(), 0usize)]);
    while let Some((symbol_id, depth)) = queue.pop_front() {
        if steps.len() >= input.step_limit() || !seen.insert(symbol_id.clone()) {
            continue;
        }
        if let Some(symbol) = symbols_by_id.get(&symbol_id) {
            steps.push(CodeProcessStep {
                symbol_id: symbol.symbol_id.clone(),
                name: symbol.name.clone(),
                kind: symbol.kind.clone(),
                file_path: symbol.file_path.clone(),
                line: symbol.line,
                depth,
            });
        }
        if depth >= input.depth_limit() {
            continue;
        }
        for edge_type in [CALLS_SYMBOL, DEPENDS_ON_SYMBOL] {
            for hit in store
                .neighbors(NeighborQuery::out(&symbol_id).with_edge_type(edge_type))
                .into_iter()
                .filter(|hit| symbols_by_id.contains_key(&hit.node_id))
            {
                queue.push_back((hit.node_id, depth + 1));
            }
        }
    }
    steps.sort_by(|left, right| {
        left.depth
            .cmp(&right.depth)
            .then_with(|| left.file_path.cmp(&right.file_path))
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.name.cmp(&right.name))
    });
    let process_id = format!(
        "code:process:{}",
        stable_hash(json!([&input.tenant_id, &input.repo_id, &entry.symbol_id]))
    );
    let confidence = if trigger == "main_entrypoint" {
        0.95
    } else if trigger == "handler_entrypoint" {
        0.85
    } else {
        0.75
    };
    CodeProcessFlow {
        process_id,
        entry_symbol_id: entry.symbol_id.clone(),
        entry_name: entry.name.clone(),
        entry_file_path: entry.file_path.clone(),
        entry_line: entry.line,
        trigger: trigger.to_string(),
        confidence,
        steps,
    }
}

fn process_node(input: &CodeProcessDetectInput, flow: &CodeProcessFlow) -> NodeRecord {
    NodeRecord::new(
        &flow.process_id,
        [CODE_PROCESS_LABEL],
        json!({
            "tenant_id": &input.tenant_id,
            "repo_id": &input.repo_id,
            "entry_symbol_id": &flow.entry_symbol_id,
            "entry_name": &flow.entry_name,
            "entry_file_path": &flow.entry_file_path,
            "entry_line": flow.entry_line,
            "trigger": &flow.trigger,
            "confidence": flow.confidence,
            "steps": &flow.steps,
            "step_count": flow.steps.len(),
            "compiler_version": CODE_COMPILER_VERSION,
            "feature_version": CODE_COMPILER_FEATURE_VERSION,
            "source": SOURCE,
        }),
    )
}

fn entrypoint_trigger(symbol: &CodeSymbolSnapshot) -> Option<&'static str> {
    let name = symbol.name.to_ascii_lowercase();
    let signature = symbol
        .signature
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    if name == "main" || signature.contains("#[tokio::main]") || signature.contains("#[main]") {
        return Some("main_entrypoint");
    }
    if name.ends_with("_handler")
        || name.contains("handler")
        || name.starts_with("handle_")
        || name.starts_with("serve")
    {
        return Some("handler_entrypoint");
    }
    if matches!(
        name.as_str(),
        "run" | "start" | "execute" | "process" | "dispatch" | "bootstrap"
    ) {
        return Some("workflow_entrypoint");
    }
    None
}

fn process_edge_id(from: &str, edge_type: &str, to: &str) -> String {
    format!(
        "code:edge:process:{}",
        stable_hash(json!([from, edge_type, to]))
    )
}
