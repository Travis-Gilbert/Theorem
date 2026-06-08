use std::collections::{BTreeMap, BTreeSet};

use rustyred_thg_core::{stable_hash, GraphStore, NodeQuery};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const OPEN_WEB_UNVERIFIED_LAYER: &str = "open_web_unverified";
pub const DEFAULT_CONFIDENCE_CEILING: f32 = 0.35;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSurface {
    BrowseWithMe,
    BrowseForMe,
    WebConsume,
}

impl Default for BrowserSurface {
    fn default() -> Self {
        Self::BrowseForMe
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalMode {
    LocalOnly,
    LocalFirst,
    WebAllowed,
    WebRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskMode {
    ReadOnly,
    ConfirmBeforeWrite,
    SupervisedAction,
    Private,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputTarget {
    Answer,
    Artifact,
    ActionPlan,
    CitationMap,
    CodePatch,
    Task,
    Report,
    GraphTrace,
    BrowserChrome,
    HarnessReceipt,
}

impl Default for OutputTarget {
    fn default() -> Self {
        Self::Answer
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphLayer {
    MemgraphCanonical,
    ThgHot,
    FalkorHot,
    RustyredHot,
    RedisHot,
    LocalWebdocs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolName {
    Ask,
    Browser,
    Capture,
    WebResearch,
    Code,
    Files,
    Calendar,
    Email,
    Agents,
    Theorem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PermissionPolicy {
    pub allow_read: bool,
    pub allow_write_hot_graph: bool,
    pub allow_write_canonical: bool,
    pub allow_remember: bool,
    pub allow_external_web: bool,
    pub allow_disclosure: bool,
    pub allow_agent_execution: bool,
    pub require_confirmation_for_write: bool,
    pub require_receipt: bool,
}

impl PermissionPolicy {
    pub fn read_only() -> Self {
        Self {
            allow_read: true,
            allow_write_hot_graph: false,
            allow_write_canonical: false,
            allow_remember: false,
            allow_external_web: false,
            allow_disclosure: true,
            allow_agent_execution: false,
            require_confirmation_for_write: true,
            require_receipt: true,
        }
    }

    pub fn web_consume() -> Self {
        Self {
            allow_read: true,
            allow_write_hot_graph: true,
            allow_write_canonical: false,
            allow_remember: false,
            allow_external_web: true,
            allow_disclosure: true,
            allow_agent_execution: false,
            require_confirmation_for_write: true,
            require_receipt: true,
        }
    }
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        Self::read_only()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RetrievalPolicy {
    pub mode: RetrievalMode,
    #[serde(default = "default_max_known_objects")]
    pub max_known_objects: usize,
    #[serde(default = "default_max_hot_context")]
    pub max_hot_context: usize,
    #[serde(default = "default_max_web_candidates")]
    pub max_web_candidates: usize,
    pub freshness_required: bool,
    pub include_counterevidence: bool,
    pub include_tensions: bool,
    pub include_user_priors: bool,
    #[serde(default = "default_true")]
    pub include_browser_hot_graph: bool,
}

impl Default for RetrievalPolicy {
    fn default() -> Self {
        Self {
            mode: RetrievalMode::LocalFirst,
            max_known_objects: default_max_known_objects(),
            max_hot_context: default_max_hot_context(),
            max_web_candidates: default_max_web_candidates(),
            freshness_required: false,
            include_counterevidence: true,
            include_tensions: true,
            include_user_priors: true,
            include_browser_hot_graph: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TracePolicy {
    #[serde(rename = "include_graph_trace", alias = "graph_trace")]
    pub graph_trace: bool,
    #[serde(rename = "include_receipts", alias = "receipts")]
    pub receipts: bool,
    #[serde(rename = "include_context_preview", alias = "context_preview")]
    pub context_preview: bool,
    #[serde(default = "default_true")]
    pub include_excluded_refs: bool,
    #[serde(default = "default_true")]
    pub include_permission_explanations: bool,
}

impl Default for TracePolicy {
    fn default() -> Self {
        Self {
            graph_trace: true,
            receipts: true,
            context_preview: true,
            include_excluded_refs: true,
            include_permission_explanations: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextReference {
    pub kind: String,
    pub id: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub source: Option<String>,
    #[serde(default = "default_salience")]
    pub salience: f64,
    #[serde(default)]
    pub required: bool,
    #[serde(default = "empty_object")]
    pub metadata: Value,
}

impl Default for ContextReference {
    fn default() -> Self {
        Self {
            kind: "object".to_string(),
            id: String::new(),
            title: None,
            url: None,
            source: None,
            salience: default_salience(),
            required: false,
            metadata: empty_object(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextCommandState {
    pub command_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub raw_request: String,
    #[serde(default, skip_serializing_if = "is_default_browser_surface")]
    pub surface: BrowserSurface,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub query: String,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub folio_id: Option<String>,
    pub notebook_id: Option<String>,
    pub project_id: Option<String>,
    pub current_page: Option<ContextReference>,
    pub selected_text: Option<String>,
    #[serde(default)]
    pub working_set: Vec<ContextReference>,
    #[serde(default)]
    pub exclusions: Vec<ContextReference>,
    #[serde(default)]
    pub hot_context: Vec<ContextReference>,
    #[serde(default)]
    pub canonical_context: Vec<ContextReference>,
    #[serde(default = "default_memory_scope")]
    pub memory_scope: String,
    pub permission_policy: PermissionPolicy,
    pub retrieval_policy: RetrievalPolicy,
    pub risk_mode: RiskMode,
    pub output_target: OutputTarget,
    pub graph_layers: Vec<GraphLayer>,
    pub tool_scope: Vec<ToolName>,
    pub trace_policy: TracePolicy,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default = "empty_object")]
    pub metadata: Value,
}

impl ContextCommandState {
    pub fn can_write_hot_graph(&self) -> bool {
        self.permission_policy.allow_write_hot_graph && self.risk_mode != RiskMode::ReadOnly
    }

    pub fn can_execute_web_action(&self) -> bool {
        self.permission_policy.allow_external_web
            && self.permission_policy.allow_agent_execution
            && self.risk_mode != RiskMode::ReadOnly
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextCommandRequest {
    pub raw_request: String,
    #[serde(default)]
    pub surface: Option<BrowserSurface>,
    #[serde(default)]
    pub retrieval_mode: Option<RetrievalMode>,
    #[serde(default)]
    pub risk_mode: Option<RiskMode>,
    #[serde(default)]
    pub output_target: Option<OutputTarget>,
    #[serde(default)]
    pub allow_external_web: Option<bool>,
    #[serde(default)]
    pub allow_agent_execution: Option<bool>,
    #[serde(default)]
    pub private: bool,
}

pub fn resolve_context_command(request: ContextCommandRequest) -> ContextCommandState {
    let surface = request.surface.unwrap_or(BrowserSurface::BrowseForMe);
    let mut risk_mode = request.risk_mode.unwrap_or(match surface {
        BrowserSurface::BrowseWithMe => RiskMode::SupervisedAction,
        BrowserSurface::BrowseForMe => RiskMode::ConfirmBeforeWrite,
        BrowserSurface::WebConsume => RiskMode::ConfirmBeforeWrite,
    });
    if request.private {
        risk_mode = RiskMode::Private;
    }

    let mut permission_policy = match risk_mode {
        RiskMode::ReadOnly => PermissionPolicy::read_only(),
        RiskMode::Private => PermissionPolicy {
            allow_disclosure: false,
            ..PermissionPolicy::web_consume()
        },
        RiskMode::ConfirmBeforeWrite | RiskMode::SupervisedAction => PermissionPolicy::web_consume(),
    };
    permission_policy.allow_external_web = request
        .allow_external_web
        .unwrap_or(!matches!(risk_mode, RiskMode::ReadOnly));
    permission_policy.allow_agent_execution = request.allow_agent_execution.unwrap_or(matches!(
        surface,
        BrowserSurface::BrowseForMe | BrowserSurface::BrowseWithMe
    ));

    let retrieval_mode = request.retrieval_mode.unwrap_or_else(|| {
        if permission_policy.allow_external_web {
            RetrievalMode::WebAllowed
        } else {
            RetrievalMode::LocalOnly
        }
    });
    let raw_request = request.raw_request;
    let warnings = if permission_policy.allow_external_web {
        Vec::new()
    } else {
        vec!["web disabled by policy".to_string()]
    };

    ContextCommandState {
        command_id: format!(
            "context-command:{}",
            stable_hash(json!({
                "request": raw_request,
                "surface": surface,
                "risk": risk_mode,
                "retrieval": retrieval_mode,
                "private": request.private
            }))
        ),
        raw_request: raw_request.clone(),
        surface,
        goal: raw_request.clone(),
        query: raw_request,
        user_id: None,
        session_id: None,
        folio_id: None,
        notebook_id: None,
        project_id: None,
        current_page: None,
        selected_text: None,
        working_set: Vec::new(),
        exclusions: Vec::new(),
        hot_context: Vec::new(),
        canonical_context: Vec::new(),
        memory_scope: default_memory_scope(),
        permission_policy,
        retrieval_policy: RetrievalPolicy {
            mode: retrieval_mode,
            freshness_required: matches!(retrieval_mode, RetrievalMode::WebRequired),
            ..RetrievalPolicy::default()
        },
        risk_mode,
        output_target: request.output_target.unwrap_or(OutputTarget::HarnessReceipt),
        graph_layers: vec![
            GraphLayer::RustyredHot,
            GraphLayer::RedisHot,
            GraphLayer::LocalWebdocs,
            GraphLayer::MemgraphCanonical,
        ],
        tool_scope: vec![
            ToolName::Ask,
            ToolName::Browser,
            ToolName::Capture,
            ToolName::WebResearch,
            ToolName::Theorem,
        ],
        trace_policy: TracePolicy::default(),
        warnings,
        metadata: empty_object(),
    }
}

fn default_max_known_objects() -> usize {
    40
}

fn default_max_hot_context() -> usize {
    30
}

fn default_max_web_candidates() -> usize {
    20
}

fn default_true() -> bool {
    true
}

fn default_salience() -> f64 {
    1.0
}

fn default_memory_scope() -> String {
    "session".to_string()
}

fn empty_object() -> Value {
    json!({})
}

fn is_default_browser_surface(surface: &BrowserSurface) -> bool {
    *surface == BrowserSurface::default()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerceptionMode {
    Ask,
    Browse,
    Capture,
    Compare,
    Verify,
    Monitor,
    Act,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerceptionCandidateKind {
    Object,
    Claim,
    Webdoc,
    Url,
    Tab,
    File,
    Action,
    Tool,
    Memory,
    Counterevidence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerceptionCandidateStatus {
    Known,
    Local,
    ExternalUnfetched,
    FetchedUnadmitted,
    Admitted,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PerceptionCandidate {
    pub id: String,
    pub kind: PerceptionCandidateKind,
    pub status: PerceptionCandidateStatus,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoverageDiagnosis {
    pub has_known_context: bool,
    pub has_browser_context: bool,
    pub needs_web: bool,
    pub needs_counterevidence: bool,
    pub needs_freshness: bool,
    pub confidence: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PerceptionBundle {
    pub mode: PerceptionMode,
    pub candidates: Vec<PerceptionCandidate>,
    pub coverage: CoverageDiagnosis,
    pub actions: Vec<ActionCandidate>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PerceptionInput {
    pub mode: PerceptionMode,
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<PageObservation>,
    #[serde(default)]
    pub seed_urls: Vec<String>,
}

impl Default for PerceptionMode {
    fn default() -> Self {
        Self::Browse
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PageObservation {
    pub url: String,
    pub title: String,
    pub distilled_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tab_id: Option<String>,
    #[serde(default)]
    pub interactive_elements: Vec<ObservedElement>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservedElement {
    pub element_id: String,
    pub role: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default)]
    pub visible: bool,
    #[serde(default)]
    pub degraded: bool,
}

pub fn perceive_with_graph<S: GraphStore>(
    store: &S,
    command: &ContextCommandState,
    input: PerceptionInput,
) -> PerceptionBundle {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();

    for node in store.query_nodes(NodeQuery {
        label: Some("Page".to_string()),
        limit: Some(8),
        ..NodeQuery::default()
    }) {
        let url = node
            .properties
            .get("url")
            .or_else(|| node.properties.get("canonical_url"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let label = node
            .properties
            .get("title")
            .and_then(Value::as_str)
            .or_else(|| url.as_deref())
            .unwrap_or(&node.id)
            .to_string();
        if seen.insert(node.id.clone()) {
            candidates.push(PerceptionCandidate {
                id: node.id,
                kind: PerceptionCandidateKind::Webdoc,
                status: PerceptionCandidateStatus::Admitted,
                label,
                url,
                confidence: DEFAULT_CONFIDENCE_CEILING,
                metadata: json!({ "source": "rustyred_hot" }),
            });
        }
    }

    if let Some(page) = &input.page {
        candidates.push(PerceptionCandidate {
            id: format!("tab:{}", stable_hash(json!({ "url": page.url }))),
            kind: PerceptionCandidateKind::Tab,
            status: PerceptionCandidateStatus::FetchedUnadmitted,
            label: if page.title.is_empty() {
                page.url.clone()
            } else {
                page.title.clone()
            },
            url: Some(page.url.clone()),
            confidence: 0.5,
            metadata: json!({
                "interactive_elements": page.interactive_elements.len(),
                "text_bytes": page.distilled_text.len()
            }),
        });
        for element in &page.interactive_elements {
            candidates.push(PerceptionCandidate {
                id: format!("element:{}", element.element_id),
                kind: PerceptionCandidateKind::Action,
                status: PerceptionCandidateStatus::FetchedUnadmitted,
                label: element.name.clone(),
                url: element.value.clone().filter(|value| value.starts_with("http")),
                confidence: 0.4,
                metadata: json!({
                    "role": element.role,
                    "visible": element.visible
                }),
            });
        }
    }

    for url in &input.seed_urls {
        candidates.push(PerceptionCandidate {
            id: format!("url:{}", stable_hash(json!({ "url": url }))),
            kind: PerceptionCandidateKind::Url,
            status: PerceptionCandidateStatus::ExternalUnfetched,
            label: url.clone(),
            url: Some(url.clone()),
            confidence: 0.25,
            metadata: json!({ "source": "seed_url" }),
        });
    }

    let has_known_context = candidates
        .iter()
        .any(|candidate| matches!(candidate.status, PerceptionCandidateStatus::Admitted));
    let has_browser_context = input.page.is_some();
    let needs_web = matches!(command.retrieval_policy.mode, RetrievalMode::WebRequired)
        || (!has_known_context
            && !has_browser_context
            && command.permission_policy.allow_external_web
            && !matches!(command.retrieval_policy.mode, RetrievalMode::LocalOnly));
    let needs_counterevidence = command.retrieval_policy.include_counterevidence
        && matches!(input.mode, PerceptionMode::Verify | PerceptionMode::Compare);
    let needs_freshness = command.retrieval_policy.freshness_required;
    let confidence = if has_known_context && has_browser_context {
        0.72
    } else if has_known_context || has_browser_context {
        0.5
    } else {
        0.2
    };
    let coverage = CoverageDiagnosis {
        has_known_context,
        has_browser_context,
        needs_web,
        needs_counterevidence,
        needs_freshness,
        confidence,
    };
    let actions = perception_actions(command, &coverage, input.page.as_ref());

    PerceptionBundle {
        mode: input.mode,
        candidates,
        coverage,
        actions,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    SummarizePage,
    SummarizeSelection,
    ExplainSelection,
    AskWithContext,
    CapturePage,
    ExtractClaims,
    CompareToGraph,
    FindCounterevidence,
    VerifyClaim,
    InspectSourceQuality,
    ShowRelatedObjects,
    OpenRelatedSources,
    CreateReport,
    DraftMemo,
    MonitorPage,
    RememberToProject,
    MarkSourceTrusted,
    MarkSourceNoisy,
    ExcludeSource,
    InspectPermissions,
    SwitchPrivateMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionCategory {
    Read,
    Navigate,
    Extract,
    Compare,
    Writeback,
    Monitor,
    Permission,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionRisk {
    ReadOnly,
    ExternalWeb,
    HotGraphWrite,
    CanonicalWrite,
    Remember,
    StateChanging,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Ready,
    NeedsConfirmation,
    BlockedPolicy,
    NotImplemented,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionRoute {
    AskPipeline,
    CaptureApi,
    WebApi,
    ContextCommandApi,
    MonitorApi,
    WritebackApi,
    FrontendOnly,
    NotImplemented,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionCandidate {
    pub id: String,
    pub action_type: ActionType,
    pub category: ActionCategory,
    pub risk: ActionRisk,
    pub status: ActionStatus,
    pub execution_route: ExecutionRoute,
    pub label: String,
    #[serde(default)]
    pub target: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionRailBundle {
    pub actions: Vec<ActionCandidate>,
    pub groups: BTreeMap<String, Vec<String>>,
}

pub fn build_action_rail(
    command: &ContextCommandState,
    perception: &PerceptionBundle,
) -> ActionRailBundle {
    let mut actions = Vec::new();
    actions.push(action(
        ActionType::InspectPermissions,
        ActionCategory::Permission,
        ActionRisk::ReadOnly,
        ExecutionRoute::FrontendOnly,
        "Inspect active browsing permissions",
        json!({ "command_id": command.command_id }),
        ActionStatus::Ready,
    ));

    for candidate in &perception.actions {
        actions.push(gate_action(command, candidate.clone()));
    }

    if perception.coverage.has_browser_context {
        actions.push(gate_action(
            command,
            action(
                ActionType::ExtractClaims,
                ActionCategory::Extract,
                ActionRisk::ReadOnly,
                ExecutionRoute::AskPipeline,
                "Extract claims from the observed page",
                json!({}),
                ActionStatus::Ready,
            ),
        ));
        actions.push(gate_action(
            command,
            action(
                ActionType::CapturePage,
                ActionCategory::Writeback,
                ActionRisk::HotGraphWrite,
                ExecutionRoute::CaptureApi,
                "Capture page into the quarantined web graph",
                json!({ "graph_layer": OPEN_WEB_UNVERIFIED_LAYER }),
                ActionStatus::Ready,
            ),
        ));
    }

    if perception.coverage.needs_counterevidence {
        actions.push(gate_action(
            command,
            action(
                ActionType::FindCounterevidence,
                ActionCategory::Navigate,
                ActionRisk::ExternalWeb,
                ExecutionRoute::WebApi,
                "Find counterevidence on the web",
                json!({}),
                ActionStatus::Ready,
            ),
        ));
    }

    actions.sort_by(|left, right| {
        left.status
            .cmp(&right.status)
            .then_with(|| left.category.cmp(&right.category))
            .then_with(|| left.action_type.cmp(&right.action_type))
    });
    let groups = group_actions(&actions);
    ActionRailBundle { actions, groups }
}

pub fn gate_action(command: &ContextCommandState, mut candidate: ActionCandidate) -> ActionCandidate {
    candidate.status = match candidate.risk {
        ActionRisk::ReadOnly => ActionStatus::Ready,
        ActionRisk::ExternalWeb => {
            if command.permission_policy.allow_external_web {
                if command.permission_policy.allow_agent_execution {
                    ActionStatus::Ready
                } else {
                    ActionStatus::NeedsConfirmation
                }
            } else {
                ActionStatus::BlockedPolicy
            }
        }
        ActionRisk::HotGraphWrite => {
            if !command.can_write_hot_graph() {
                ActionStatus::BlockedPolicy
            } else if command.permission_policy.require_confirmation_for_write {
                ActionStatus::NeedsConfirmation
            } else {
                ActionStatus::Ready
            }
        }
        ActionRisk::CanonicalWrite => {
            if command.permission_policy.allow_write_canonical {
                ActionStatus::NeedsConfirmation
            } else {
                ActionStatus::BlockedPolicy
            }
        }
        ActionRisk::Remember => {
            if command.permission_policy.allow_remember {
                ActionStatus::NeedsConfirmation
            } else {
                ActionStatus::BlockedPolicy
            }
        }
        ActionRisk::StateChanging => {
            if matches!(
                command.risk_mode,
                RiskMode::ConfirmBeforeWrite | RiskMode::SupervisedAction
            ) {
                ActionStatus::NeedsConfirmation
            } else {
                ActionStatus::BlockedPolicy
            }
        }
    };
    candidate
}

fn perception_actions(
    command: &ContextCommandState,
    coverage: &CoverageDiagnosis,
    page: Option<&PageObservation>,
) -> Vec<ActionCandidate> {
    let mut actions = Vec::new();
    if coverage.has_known_context || coverage.has_browser_context {
        actions.push(action(
            ActionType::AskWithContext,
            ActionCategory::Read,
            ActionRisk::ReadOnly,
            ExecutionRoute::AskPipeline,
            "Answer with current context",
            json!({}),
            ActionStatus::Ready,
        ));
        actions.push(action(
            ActionType::CompareToGraph,
            ActionCategory::Compare,
            ActionRisk::ReadOnly,
            ExecutionRoute::ContextCommandApi,
            "Compare page context to the graph",
            json!({}),
            ActionStatus::Ready,
        ));
    }
    if coverage.needs_web && command.permission_policy.allow_external_web {
        actions.push(action(
            ActionType::OpenRelatedSources,
            ActionCategory::Navigate,
            ActionRisk::ExternalWeb,
            ExecutionRoute::WebApi,
            "Open related web sources",
            json!({}),
            ActionStatus::Ready,
        ));
        actions.push(action(
            ActionType::VerifyClaim,
            ActionCategory::Navigate,
            ActionRisk::ExternalWeb,
            ExecutionRoute::WebApi,
            "Verify the request against fresh web sources",
            json!({}),
            ActionStatus::Ready,
        ));
    }
    if let Some(page) = page {
        actions.push(action(
            ActionType::SummarizePage,
            ActionCategory::Read,
            ActionRisk::ReadOnly,
            ExecutionRoute::AskPipeline,
            "Summarize the current page",
            json!({ "url": page.url }),
            ActionStatus::Ready,
        ));
        for element in page
            .interactive_elements
            .iter()
            .filter(|element| element.role == "link")
            .take(3)
        {
            actions.push(action(
                ActionType::OpenRelatedSources,
                ActionCategory::Navigate,
                ActionRisk::ExternalWeb,
                ExecutionRoute::WebApi,
                format!("Open {}", element.name),
                json!({ "element_id": element.element_id, "href": element.value }),
                ActionStatus::Ready,
            ));
        }
    }
    actions
}

pub fn action(
    action_type: ActionType,
    category: ActionCategory,
    risk: ActionRisk,
    execution_route: ExecutionRoute,
    label: impl Into<String>,
    target: Value,
    status: ActionStatus,
) -> ActionCandidate {
    let label = label.into();
    let id = format!(
        "action:{}",
        stable_hash(json!({
            "action_type": action_type,
            "category": category,
            "risk": risk,
            "route": execution_route,
            "label": label,
            "target": target
        }))
    );
    ActionCandidate {
        id,
        action_type,
        category,
        risk,
        status,
        execution_route,
        label,
        target,
    }
}

fn group_actions(actions: &[ActionCandidate]) -> BTreeMap<String, Vec<String>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for action in actions {
        groups
            .entry(format!("{:?}", action.category).to_ascii_lowercase())
            .or_default()
            .push(action.id.clone());
    }
    groups
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BrowserPlaybook {
    pub intent: String,
    pub title: String,
    pub skill_markdown: String,
}

pub fn default_browser_playbooks() -> Vec<BrowserPlaybook> {
    [
        (
            "deep_research",
            "Deep Research",
            "Prefer WebDoc ingestion, preserve source URLs privately, and emit trace events before training export.",
        ),
        (
            "docs_search",
            "Documentation Search",
            "Resolve official docs first, keep version and package identifiers in the receipt, and capture examples as claims.",
        ),
        (
            "paper_search",
            "Paper Search",
            "Track title, venue, authors, year, and citation links before summarizing claims.",
        ),
        (
            "pricing_tracker",
            "Pricing Tracker",
            "Capture price, plan name, region, timestamp, and the page section that stated the price.",
        ),
        (
            "product_research",
            "Product Research",
            "Compare first-party pages with independent sources and mark promotional claims as unverified.",
        ),
        (
            "source_verification",
            "Source Verification",
            "Find counterevidence, inspect source quality, and keep disputed claims below canonical promotion.",
        ),
    ]
    .into_iter()
    .map(|(intent, title, body)| BrowserPlaybook {
        intent: intent.to_string(),
        title: title.to_string(),
        skill_markdown: format!("# {title}\n\n{body}\n"),
    })
    .collect()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrowsingRunReceipt {
    pub run_id: String,
    pub surface: BrowserSurface,
    pub context_command_id: String,
    pub pages_reached: Vec<String>,
    pub actions_applied: Vec<ActionCandidate>,
    pub data_extracted: Value,
    pub playbooks_used: Vec<String>,
    pub playbooks_created: Vec<String>,
    pub confidence_ceiling: f32,
    pub quarantine_layer: String,
    pub events: Vec<String>,
}

pub fn browsing_run_receipt(
    run_id: impl Into<String>,
    command: &ContextCommandState,
    perception: &PerceptionBundle,
    rail: &ActionRailBundle,
) -> BrowsingRunReceipt {
    let pages_reached = perception
        .candidates
        .iter()
        .filter_map(|candidate| candidate.url.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let actions_applied = rail
        .actions
        .iter()
        .filter(|action| action.status == ActionStatus::Ready)
        .take(3)
        .cloned()
        .collect::<Vec<_>>();
    BrowsingRunReceipt {
        run_id: run_id.into(),
        surface: command.surface,
        context_command_id: command.command_id.clone(),
        pages_reached,
        actions_applied,
        data_extracted: json!({
            "candidate_count": perception.candidates.len(),
            "coverage": perception.coverage
        }),
        playbooks_used: vec!["source_verification".to_string()],
        playbooks_created: Vec::new(),
        confidence_ceiling: DEFAULT_CONFIDENCE_CEILING,
        quarantine_layer: OPEN_WEB_UNVERIFIED_LAYER.to_string(),
        events: vec![
            "context_command.resolved".to_string(),
            "perception.bundle.created".to_string(),
            "action_rail.bundle.created".to_string(),
            "browsing_run.receipt.emitted".to_string(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::{GraphStore, InMemoryGraphStore, NodeRecord};

    #[test]
    fn context_command_round_trips_json() {
        let state = resolve_context_command(ContextCommandRequest {
            raw_request: "verify this source".to_string(),
            surface: Some(BrowserSurface::BrowseForMe),
            retrieval_mode: Some(RetrievalMode::WebRequired),
            ..ContextCommandRequest::default()
        });
        let json = serde_json::to_string(&state).expect("serialize");
        let decoded: ContextCommandState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.retrieval_policy.mode, RetrievalMode::WebRequired);
        assert_eq!(decoded.surface, BrowserSurface::BrowseForMe);
    }

    #[test]
    fn python_context_command_fixture_round_trips_without_losing_shape() {
        let fixture = json!({
            "command_id": "cmd_0001",
            "goal": "Compare pricing across vendors",
            "query": "vendor pricing 2026",
            "user_id": "user_42",
            "session_id": "sess_7",
            "folio_id": null,
            "notebook_id": "nb_3",
            "project_id": null,
            "current_page": {
                "kind": "page",
                "id": "https://example.com/pricing",
                "title": "Pricing",
                "url": "https://example.com/pricing",
                "source": "example.com",
                "salience": 1.0,
                "required": false,
                "metadata": {}
            },
            "selected_text": "Team plan $20/seat",
            "working_set": [{
                "kind": "object",
                "id": "obj_1",
                "title": "Vendor A",
                "url": null,
                "source": null,
                "salience": 1.0,
                "required": false,
                "metadata": {}
            }],
            "exclusions": [],
            "hot_context": [{
                "kind": "webdoc",
                "id": "wd_9",
                "title": null,
                "url": "https://a.test",
                "source": null,
                "salience": 0.7,
                "required": false,
                "metadata": {}
            }],
            "canonical_context": [],
            "memory_scope": "session",
            "graph_layers": [
                "memgraph_canonical",
                "rustyred_hot",
                "redis_hot",
                "local_webdocs"
            ],
            "tool_scope": ["ask", "browser", "capture", "web_research"],
            "retrieval_policy": {
                "mode": "local_first",
                "freshness_required": false,
                "max_known_objects": 40,
                "max_hot_context": 30,
                "max_web_candidates": 20,
                "include_counterevidence": true,
                "include_tensions": true,
                "include_user_priors": true,
                "include_browser_hot_graph": true
            },
            "output_target": "answer",
            "risk_mode": "confirm_before_write",
            "permission_policy": {
                "allow_read": true,
                "allow_write_canonical": false,
                "allow_write_hot_graph": true,
                "allow_remember": false,
                "allow_external_web": false,
                "allow_disclosure": false,
                "allow_agent_execution": false,
                "require_confirmation_for_write": true,
                "require_receipt": true
            },
            "trace_policy": {
                "include_graph_trace": true,
                "include_receipts": true,
                "include_context_preview": true,
                "include_excluded_refs": true,
                "include_permission_explanations": true
            },
            "warnings": ["web disabled by policy"],
            "metadata": { "origin": "unit-fixture" }
        });
        let decoded: ContextCommandState =
            serde_json::from_value(fixture.clone()).expect("python fixture");
        assert_eq!(decoded.output_target, OutputTarget::Answer);
        assert!(decoded.graph_layers.contains(&GraphLayer::MemgraphCanonical));
        assert_eq!(serde_json::to_value(decoded).expect("encoded"), fixture);
    }

    #[test]
    fn read_only_command_blocks_graph_write_actions() {
        let command = resolve_context_command(ContextCommandRequest {
            raw_request: "summarize only".to_string(),
            risk_mode: Some(RiskMode::ReadOnly),
            allow_external_web: Some(false),
            ..ContextCommandRequest::default()
        });
        let mut store = InMemoryGraphStore::default();
        GraphStore::upsert_node(
            &mut store,
            NodeRecord::new(
                "page:one",
                ["Page"],
                json!({ "url": "https://example.com", "title": "Example" }),
            ),
        )
        .expect("page");
        let perception = perceive_with_graph(
            &store,
            &command,
            PerceptionInput {
                mode: PerceptionMode::Capture,
                query: "summarize".to_string(),
                page: Some(PageObservation {
                    url: "https://example.com".to_string(),
                    title: "Example".to_string(),
                    distilled_text: "hello".to_string(),
                    active_tab_id: None,
                    interactive_elements: Vec::new(),
                }),
                seed_urls: Vec::new(),
            },
        );
        let rail = build_action_rail(&command, &perception);
        let capture = rail
            .actions
            .iter()
            .find(|action| action.action_type == ActionType::CapturePage)
            .expect("capture action");
        assert_eq!(capture.status, ActionStatus::BlockedPolicy);
    }

    #[test]
    fn missing_context_requests_web_when_policy_allows_it() {
        let command = resolve_context_command(ContextCommandRequest {
            raw_request: "find source".to_string(),
            retrieval_mode: Some(RetrievalMode::WebAllowed),
            ..ContextCommandRequest::default()
        });
        let store = InMemoryGraphStore::default();
        let perception = perceive_with_graph(
            &store,
            &command,
            PerceptionInput {
                mode: PerceptionMode::Browse,
                query: "find source".to_string(),
                ..PerceptionInput::default()
            },
        );
        assert!(perception.coverage.needs_web);
        let rail = build_action_rail(&command, &perception);
        assert!(rail
            .actions
            .iter()
            .any(|action| action.execution_route == ExecutionRoute::WebApi));
    }

    #[test]
    fn browse_with_me_uses_supervised_action_policy() {
        let command = resolve_context_command(ContextCommandRequest {
            raw_request: "help me browse".to_string(),
            surface: Some(BrowserSurface::BrowseWithMe),
            ..ContextCommandRequest::default()
        });
        assert_eq!(command.risk_mode, RiskMode::SupervisedAction);
        assert!(command.permission_policy.allow_agent_execution);
        assert!(command.permission_policy.require_confirmation_for_write);
    }
}
