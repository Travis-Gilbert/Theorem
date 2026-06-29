use rustyred_thg_core::{EdgeRecord, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CODE_COMPILER_VERSION: &str = "rustyred-code-compiler-v0";
pub const CODE_COMPILER_FEATURE_VERSION: &str = "code-compiler-ir-v1";
pub const DEFAULT_CODE_COMPILER_SYMBOL_LIMIT: usize = 100_000;

pub const CODE_SPEC_LABEL: &str = "CodeSpecification";
pub const CODE_COMPILER_DRIFT_LABEL: &str = "CodeDriftFinding";
pub const SPECIFIES_CODE: &str = "SPECIFIES_CODE";
pub const DRIFT_FOR_SPEC: &str = "DRIFT_FOR_SPEC";
pub const DRIFT_FOR_CODE: &str = "DRIFT_FOR_CODE";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CodeSpecCompileInput {
    pub tenant_id: String,
    pub repo_id: String,
    pub repo_label: Option<String>,
    pub spec_id: Option<String>,
    pub spec_title: Option<String>,
    pub max_symbols: usize,
}

impl CodeSpecCompileInput {
    pub fn new(tenant_id: impl Into<String>, repo_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            repo_label: None,
            spec_id: None,
            spec_title: None,
            max_symbols: DEFAULT_CODE_COMPILER_SYMBOL_LIMIT,
        }
    }

    pub fn symbol_limit(&self) -> usize {
        if self.max_symbols == 0 {
            DEFAULT_CODE_COMPILER_SYMBOL_LIMIT
        } else {
            self.max_symbols
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CodeSpecDriftInput {
    pub tenant_id: String,
    pub repo_id: String,
    pub spec_node_id: String,
    pub max_symbols: usize,
}

impl CodeSpecDriftInput {
    pub fn new(
        tenant_id: impl Into<String>,
        repo_id: impl Into<String>,
        spec_node_id: impl Into<String>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            spec_node_id: spec_node_id.into(),
            max_symbols: DEFAULT_CODE_COMPILER_SYMBOL_LIMIT,
        }
    }

    pub fn symbol_limit(&self) -> usize {
        if self.max_symbols == 0 {
            DEFAULT_CODE_COMPILER_SYMBOL_LIMIT
        } else {
            self.max_symbols
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodeFileSnapshot {
    pub file_id: String,
    pub path: String,
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodeSymbolSnapshot {
    pub symbol_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    pub file_path: String,
    pub kind: String,
    pub name: String,
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default)]
    pub call_names: Vec<String>,
    #[serde(default)]
    pub dependency_names: Vec<String>,
    #[serde(default)]
    pub parser_backed: bool,
}

impl CodeSymbolSnapshot {
    pub fn symbol_key(&self) -> String {
        format!("{}\u{0}{}\u{0}{}", self.file_path, self.kind, self.name)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodeDependencySnapshot {
    pub from_symbol_id: String,
    pub to_symbol_id: String,
    pub edge_type: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CodeSpecCompileOutput {
    pub spec_node: NodeRecord,
    pub spec_edges: Vec<EdgeRecord>,
    pub files: Vec<CodeFileSnapshot>,
    pub symbols: Vec<CodeSymbolSnapshot>,
    pub dependency_edges: Vec<CodeDependencySnapshot>,
    pub file_count: usize,
    pub symbol_count: usize,
    pub structure_count: usize,
    pub member_count: usize,
    pub dependency_edge_count: usize,
    pub artifact_hash: String,
    pub spec_body: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeSpecDriftKind {
    MissingSymbol,
    UndocumentedSymbol,
    SignatureChanged,
}

impl CodeSpecDriftKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingSymbol => "missing_symbol",
            Self::UndocumentedSymbol => "undocumented_symbol",
            Self::SignatureChanged => "signature_changed",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CodeSpecDriftFinding {
    pub finding_id: String,
    pub drift_kind: CodeSpecDriftKind,
    pub severity: String,
    pub symbol_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_id: Option<String>,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub expected: Value,
    pub actual: Value,
    pub suggested_next_step: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CodeSpecDriftReport {
    pub tenant_id: String,
    pub repo_id: String,
    pub spec_node_id: String,
    pub findings: Vec<CodeSpecDriftFinding>,
}
