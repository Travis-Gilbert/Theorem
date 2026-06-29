//! Programmable Harness capability primitives.
//!
//! This crate is the runtime boundary for agent-authored capability plugins. It
//! has two tiers:
//! - declarative skill plugins compose existing affordances as data;
//! - WASM plugins run through Extism with memory/time/fuel limits and explicit
//!   host-function grants.

#![forbid(unsafe_code)]

use extism::{CurrentPlugin, Function, Manifest, Plugin, PluginBuilder, UserData, Val, Wasm, PTR};
use rustyred_thg_core::{now_ms, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use theorem_harness_core::agent_binding::{
    BindingCapabilityScope, BindingError, PROGRAMMABLE_CAPABILITY_ACTION_TIER,
};
use theorem_harness_core::types::Payload;
use theorem_harness_core::{evaluate_publication, stable_value_hash, ActionTierPolicy};

pub type PluginResult<T> = Result<T, PluginError>;

pub const PROGRAMMABLE_PLUGIN_SOURCE: &str = "rustyred-plugin";
pub const DEFAULT_PLUGIN_TIMEOUT_MS: u64 = 1_000;
pub const DEFAULT_PLUGIN_MEMORY_MAX_PAGES: u32 = 32;
pub const DEFAULT_PLUGIN_FUEL: u64 = 2_000_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PluginError {
    Extism(String),
    InvalidInput { field: String, message: String },
    Denied { grant: String },
    TestFailed(String),
    NotFound(String),
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Extism(message) => write!(f, "extism error: {message}"),
            Self::InvalidInput { field, message } => write!(f, "invalid {field}: {message}"),
            Self::Denied { grant } => write!(f, "host function grant denied: {grant}"),
            Self::TestFailed(message) => write!(f, "capability test failed: {message}"),
            Self::NotFound(message) => write!(f, "not found: {message}"),
        }
    }
}

impl Error for PluginError {}

impl From<extism::Error> for PluginError {
    fn from(value: extism::Error) -> Self {
        Self::Extism(value.root_cause().to_string())
    }
}

impl From<serde_json::Error> for PluginError {
    fn from(value: serde_json::Error) -> Self {
        Self::InvalidInput {
            field: "json".to_string(),
            message: value.to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostFunctionGrant {
    GraphRead,
    FactWrite,
    AffordanceRegister,
}

impl HostFunctionGrant {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GraphRead => "graph_read",
            Self::FactWrite => "fact_write",
            Self::AffordanceRegister => "affordance_register",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WasmPluginSource {
    Bytes(Vec<u8>),
    Wat(String),
    File(PathBuf),
}

impl WasmPluginSource {
    pub fn content_hash(&self) -> PluginResult<String> {
        let mut hasher = Sha256::new();
        match self {
            Self::Bytes(bytes) => hasher.update(bytes),
            Self::Wat(wat) => hasher.update(wat.as_bytes()),
            Self::File(path) => {
                let bytes = std::fs::read(path).map_err(|error| PluginError::InvalidInput {
                    field: "source".to_string(),
                    message: format!("failed to read {}: {error}", path.display()),
                })?;
                hasher.update(bytes);
            }
        }
        Ok(format!("{:x}", hasher.finalize()))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginLimits {
    #[serde(default = "default_memory_pages")]
    pub memory_max_pages: u32,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_fuel")]
    pub fuel: u64,
    #[serde(default)]
    pub with_wasi: bool,
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub allowed_paths: Vec<AllowedPath>,
}

impl Default for PluginLimits {
    fn default() -> Self {
        Self {
            memory_max_pages: DEFAULT_PLUGIN_MEMORY_MAX_PAGES,
            timeout_ms: DEFAULT_PLUGIN_TIMEOUT_MS,
            fuel: DEFAULT_PLUGIN_FUEL,
            with_wasi: false,
            allowed_hosts: Vec::new(),
            allowed_paths: Vec::new(),
        }
    }
}

fn default_memory_pages() -> u32 {
    DEFAULT_PLUGIN_MEMORY_MAX_PAGES
}

fn default_timeout_ms() -> u64 {
    DEFAULT_PLUGIN_TIMEOUT_MS
}

fn default_fuel() -> u64 {
    DEFAULT_PLUGIN_FUEL
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AllowedPath {
    pub host_path: String,
    pub guest_path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginExportSpec {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub writeback_policy: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl PluginExportSpec {
    pub fn normalized(mut self) -> Self {
        self.name = self.name.trim().to_string();
        if self.label.trim().is_empty() {
            self.label = self.name.clone();
        } else {
            self.label = self.label.trim().to_string();
        }
        if self.writeback_policy.trim().is_empty() {
            self.writeback_policy = "read-only".to_string();
        }
        self.permissions = clean_strings(self.permissions);
        self.tags = clean_strings(self.tags);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WasmPluginSpec {
    pub plugin_id: String,
    pub tenant_id: String,
    pub source: WasmPluginSource,
    #[serde(default)]
    pub exports: Vec<PluginExportSpec>,
    #[serde(default)]
    pub grants: Vec<HostFunctionGrant>,
    #[serde(default)]
    pub limits: PluginLimits,
    #[serde(default)]
    pub declared_tests: Vec<CapabilityTest>,
    #[serde(default)]
    pub provenance: CapabilityProvenance,
}

impl WasmPluginSpec {
    pub fn normalized(mut self) -> Self {
        self.plugin_id = self.plugin_id.trim().to_string();
        self.tenant_id = self.tenant_id.trim().to_string();
        self.exports = self
            .exports
            .into_iter()
            .map(PluginExportSpec::normalized)
            .collect();
        self.grants.sort();
        self.grants.dedup();
        self
    }

    pub fn validate(&self) -> PluginResult<()> {
        if self.plugin_id.trim().is_empty() {
            return Err(PluginError::InvalidInput {
                field: "plugin_id".to_string(),
                message: "plugin_id is required".to_string(),
            });
        }
        if self.tenant_id.trim().is_empty() {
            return Err(PluginError::InvalidInput {
                field: "tenant_id".to_string(),
                message: "tenant_id is required".to_string(),
            });
        }
        for export in &self.exports {
            if export.name.trim().is_empty() {
                return Err(PluginError::InvalidInput {
                    field: "exports.name".to_string(),
                    message: "export name is required".to_string(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapabilityProvenance {
    #[serde(default)]
    pub corpus_segment_ids: Vec<String>,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub authored_by: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceFact {
    pub fact_id: String,
    pub tenant_id: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub plugin_id: String,
    pub function_name: String,
    pub provenance: CapabilityProvenance,
    pub written_at_ms: i64,
}

impl ProvenanceFact {
    pub fn to_node_record(&self) -> NodeRecord {
        NodeRecord::new(
            format!("plugin_fact:{}:{}", self.tenant_id, self.fact_id),
            ["PluginFact"],
            json!({
                "fact": self,
                "provenance": {
                    "source_id": self.plugin_id,
                    "timestamp": self.written_at_ms.to_string(),
                    "method": PROGRAMMABLE_PLUGIN_SOURCE,
                },
            }),
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PluginRuntimeState {
    pub graph_seed: Vec<Value>,
    pub graph_reads: Vec<Value>,
    pub fact_writes: Vec<ProvenanceFact>,
    pub registered_affordances: Vec<PluginExportSpec>,
}

#[derive(Clone)]
struct HostFunctionState {
    plugin_id: String,
    tenant_id: String,
    function_name: String,
    grants: BTreeSet<HostFunctionGrant>,
    provenance: CapabilityProvenance,
    state: Arc<Mutex<PluginRuntimeState>>,
}

impl HostFunctionState {
    fn allows(&self, grant: HostFunctionGrant) -> bool {
        self.grants.contains(&grant)
    }

    fn denied(grant: HostFunctionGrant) -> String {
        json!({
            "ok": false,
            "error": "grant_denied",
            "grant": grant.as_str(),
        })
        .to_string()
    }
}

pub struct LoadedWasmPlugin {
    spec: WasmPluginSpec,
    plugin: Plugin,
    state: Arc<Mutex<PluginRuntimeState>>,
}

impl LoadedWasmPlugin {
    pub fn spec(&self) -> &WasmPluginSpec {
        &self.spec
    }

    pub fn runtime_state(&self) -> PluginRuntimeState {
        self.state.lock().expect("plugin runtime state").clone()
    }

    pub fn invoke(&mut self, function_name: &str, input: &str) -> PluginResult<String> {
        let function_name = function_name.trim();
        if !self.plugin.function_exists(function_name) {
            return Err(PluginError::NotFound(format!(
                "plugin export {function_name} does not exist"
            )));
        }
        Ok(self.plugin.call::<&str, String>(function_name, input)?)
    }
}

#[derive(Clone, Debug, Default)]
pub struct PluginHost {
    graph_seed: Vec<Value>,
}

impl PluginHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_graph_seed(mut self, graph_seed: Vec<Value>) -> Self {
        self.graph_seed = graph_seed;
        self
    }

    pub fn load(&self, spec: WasmPluginSpec) -> PluginResult<LoadedWasmPlugin> {
        let spec = spec.normalized();
        spec.validate()?;
        let state = Arc::new(Mutex::new(PluginRuntimeState {
            graph_seed: self.graph_seed.clone(),
            ..PluginRuntimeState::default()
        }));
        let host_state = HostFunctionState {
            plugin_id: spec.plugin_id.clone(),
            tenant_id: spec.tenant_id.clone(),
            function_name: String::new(),
            grants: spec.grants.iter().cloned().collect(),
            provenance: spec.provenance.clone(),
            state: Arc::clone(&state),
        };

        let manifest = manifest_for_source(&spec.source, &spec.limits)?;
        let mut builder = PluginBuilder::new(manifest).with_wasi(spec.limits.with_wasi);
        if spec.limits.fuel > 0 {
            builder = builder.with_fuel_limit(spec.limits.fuel);
        }
        let functions = host_functions(host_state);
        let plugin = builder.with_functions(functions).build()?;
        Ok(LoadedWasmPlugin {
            spec,
            plugin,
            state,
        })
    }
}

fn manifest_for_source(source: &WasmPluginSource, limits: &PluginLimits) -> PluginResult<Manifest> {
    let wasm = match source {
        WasmPluginSource::Bytes(bytes) => Wasm::data(bytes.clone()),
        WasmPluginSource::Wat(wat) => {
            let bytes = wat::parse_str(wat).map_err(|error| PluginError::InvalidInput {
                field: "source".to_string(),
                message: format!("failed to parse WAT source: {error}"),
            })?;
            Wasm::data(bytes)
        }
        WasmPluginSource::File(path) => Wasm::file(path),
    };
    let mut manifest = Manifest::new([wasm])
        .with_memory_max(limits.memory_max_pages)
        .with_timeout(Duration::from_millis(limits.timeout_ms));
    for host in &limits.allowed_hosts {
        manifest = manifest.with_allowed_host(host);
    }
    for allowed_path in &limits.allowed_paths {
        manifest =
            manifest.with_allowed_path(allowed_path.host_path.clone(), &allowed_path.guest_path);
    }
    Ok(manifest)
}

fn host_functions(host_state: HostFunctionState) -> Vec<Function> {
    vec![
        Function::new(
            "thg_graph_read",
            [PTR],
            [PTR],
            UserData::new(host_state.clone()),
            host_graph_read,
        ),
        Function::new(
            "thg_fact_write",
            [PTR],
            [PTR],
            UserData::new(host_state.clone()),
            host_fact_write,
        ),
        Function::new(
            "thg_affordance_register",
            [PTR],
            [PTR],
            UserData::new(host_state),
            host_affordance_register,
        ),
    ]
}

fn host_graph_read(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostFunctionState>,
) -> Result<(), extism::Error> {
    let state = user_data.get()?;
    let state = state.lock().expect("host function state").clone();
    let response = if !state.allows(HostFunctionGrant::GraphRead) {
        HostFunctionState::denied(HostFunctionGrant::GraphRead)
    } else {
        let request: String = plugin.memory_get_val(&inputs[0])?;
        let mut runtime = state.state.lock().expect("plugin runtime state");
        let request_value = serde_json::from_str::<Value>(&request).unwrap_or_else(|_| {
            json!({
                "query": request,
            })
        });
        runtime.graph_reads.push(request_value.clone());
        json!({
            "ok": true,
            "plugin_id": state.plugin_id,
            "tenant_id": state.tenant_id,
            "request": request_value,
            "facts": runtime.graph_seed,
        })
        .to_string()
    };
    let handle = plugin.memory_new(response.as_str())?;
    outputs[0] = plugin.memory_to_val(handle);
    Ok(())
}

fn host_fact_write(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostFunctionState>,
) -> Result<(), extism::Error> {
    let state = user_data.get()?;
    let state = state.lock().expect("host function state").clone();
    let response = if !state.allows(HostFunctionGrant::FactWrite) {
        HostFunctionState::denied(HostFunctionGrant::FactWrite)
    } else {
        let request: String = plugin.memory_get_val(&inputs[0])?;
        let request = serde_json::from_str::<Value>(&request).unwrap_or_else(|_| {
            json!({
                "object": request,
            })
        });
        let subject = text_field(&request, "subject").unwrap_or_else(|| "plugin".to_string());
        let predicate = text_field(&request, "predicate").unwrap_or_else(|| "asserts".to_string());
        let object = text_field(&request, "object").unwrap_or_else(|| request.to_string());
        let fact_id = stable_value_hash(&json!({
            "tenant_id": state.tenant_id,
            "plugin_id": state.plugin_id,
            "subject": subject,
            "predicate": predicate,
            "object": object,
            "time": now_ms(),
        }));
        let fact = ProvenanceFact {
            fact_id,
            tenant_id: state.tenant_id.clone(),
            subject,
            predicate,
            object,
            plugin_id: state.plugin_id.clone(),
            function_name: state.function_name.clone(),
            provenance: state.provenance.clone(),
            written_at_ms: now_ms(),
        };
        state
            .state
            .lock()
            .expect("plugin runtime state")
            .fact_writes
            .push(fact.clone());
        json!({
            "ok": true,
            "fact": fact,
        })
        .to_string()
    };
    let handle = plugin.memory_new(response.as_str())?;
    outputs[0] = plugin.memory_to_val(handle);
    Ok(())
}

fn host_affordance_register(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostFunctionState>,
) -> Result<(), extism::Error> {
    let state = user_data.get()?;
    let state = state.lock().expect("host function state").clone();
    let response = if !state.allows(HostFunctionGrant::AffordanceRegister) {
        HostFunctionState::denied(HostFunctionGrant::AffordanceRegister)
    } else {
        let request: String = plugin.memory_get_val(&inputs[0])?;
        let export = serde_json::from_str::<PluginExportSpec>(&request)
            .unwrap_or_else(|_| PluginExportSpec {
                name: request.trim().to_string(),
                label: request.trim().to_string(),
                description: String::new(),
                input_schema: json!({}),
                permissions: Vec::new(),
                writeback_policy: "read-only".to_string(),
                tags: Vec::new(),
            })
            .normalized();
        state
            .state
            .lock()
            .expect("plugin runtime state")
            .registered_affordances
            .push(export.clone());
        json!({
            "ok": true,
            "export": export,
        })
        .to_string()
    };
    let handle = plugin.memory_new(response.as_str())?;
    outputs[0] = plugin.memory_to_val(handle);
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeclarativeSkillDefinition {
    pub skill_id: String,
    pub tenant_id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub parameters_schema: Value,
    pub steps: Vec<DeclarativeSkillStep>,
    #[serde(default)]
    pub declared_tests: Vec<CapabilityTest>,
    #[serde(default)]
    pub provenance: CapabilityProvenance,
}

impl DeclarativeSkillDefinition {
    pub fn validate(&self) -> PluginResult<()> {
        if self.skill_id.trim().is_empty() {
            return Err(PluginError::InvalidInput {
                field: "skill_id".to_string(),
                message: "skill_id is required".to_string(),
            });
        }
        if self.steps.is_empty() {
            return Err(PluginError::InvalidInput {
                field: "steps".to_string(),
                message: "declarative skill requires at least one affordance step".to_string(),
            });
        }
        for step in &self.steps {
            if step.affordance_id.trim().is_empty() {
                return Err(PluginError::InvalidInput {
                    field: "steps.affordance_id".to_string(),
                    message: "step affordance_id is required".to_string(),
                });
            }
        }
        Ok(())
    }

    pub fn to_skill_pack_value(&self) -> PluginResult<Value> {
        self.validate()?;
        Ok(json!({
            "kind": "skill_pack",
            "id": self.skill_id,
            "title": self.title,
            "description": self.description,
            "capabilities": [self.skill_id],
            "parameters_schema": self.parameters_schema,
            "spec": {
                "kind": "programmable_declarative_skill",
                "skill_id": self.skill_id,
                "steps": self.steps,
                "provenance": self.provenance,
            },
            "validators": self.declared_tests.iter().map(CapabilityTest::to_validator_value).collect::<Vec<_>>(),
            "metadata": {
                "programmable_capability": true,
                "capability_kind": "declarative_skill",
                "provenance": self.provenance,
            }
        }))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeclarativeSkillStep {
    pub affordance_id: String,
    #[serde(default)]
    pub arguments: Value,
}

pub trait DeclarativeAffordanceInvoker {
    fn invoke_affordance(&mut self, affordance_id: &str, arguments: Value) -> PluginResult<Value>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeclarativeSkillInvokeReceipt {
    pub skill_id: String,
    pub status: String,
    pub step_results: Vec<Value>,
}

pub fn invoke_declarative_skill<I: DeclarativeAffordanceInvoker>(
    definition: &DeclarativeSkillDefinition,
    invoker: &mut I,
) -> PluginResult<DeclarativeSkillInvokeReceipt> {
    definition.validate()?;
    let mut step_results = Vec::with_capacity(definition.steps.len());
    for step in &definition.steps {
        let result = invoker.invoke_affordance(&step.affordance_id, step.arguments.clone())?;
        step_results.push(result);
    }
    Ok(DeclarativeSkillInvokeReceipt {
        skill_id: definition.skill_id.clone(),
        status: "applied".to_string(),
        step_results,
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeclarativeSkillPublishOptions {
    #[serde(default)]
    pub actor_id: String,
    #[serde(default = "validated_status")]
    pub status: String,
    #[serde(default)]
    pub source_content_hash: String,
    #[serde(default)]
    pub artifact_hashes: Vec<String>,
    #[serde(default)]
    pub created_at: String,
}

fn validated_status() -> String {
    "validated".to_string()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeclarativeSkillPublishRequest {
    pub tenant_slug: String,
    pub actor_id: String,
    pub source_content_hash: String,
    pub artifact_hashes: Vec<String>,
    pub status: String,
    pub created_at: String,
    pub pack: Value,
}

impl DeclarativeSkillDefinition {
    pub fn to_skill_publish_request(
        &self,
        options: DeclarativeSkillPublishOptions,
    ) -> PluginResult<DeclarativeSkillPublishRequest> {
        self.validate()?;
        let pack = self.to_skill_pack_value()?;
        let source_content_hash = if options.source_content_hash.trim().is_empty() {
            stable_value_hash(&json!({
                "skill_id": self.skill_id,
                "tenant_id": self.tenant_id,
                "provenance": self.provenance,
            }))
        } else {
            options.source_content_hash.trim().to_string()
        };
        let status = if options.status.trim().is_empty() {
            validated_status()
        } else {
            options.status.trim().to_string()
        };
        Ok(DeclarativeSkillPublishRequest {
            tenant_slug: self.tenant_id.clone(),
            actor_id: options.actor_id,
            source_content_hash,
            artifact_hashes: options.artifact_hashes,
            status,
            created_at: options.created_at,
            pack,
        })
    }
}

pub fn declarative_skill_publish_request(
    definition: &DeclarativeSkillDefinition,
    options: DeclarativeSkillPublishOptions,
) -> PluginResult<DeclarativeSkillPublishRequest> {
    definition.validate()?;
    definition.to_skill_publish_request(options)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityTestStatus {
    Passed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityTest {
    pub test_id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub expected: Value,
}

impl CapabilityTest {
    fn to_validator_value(&self) -> Value {
        json!({
            "id": self.test_id,
            "kind": if self.kind.trim().is_empty() { "always_pass" } else { self.kind.as_str() },
            "expected": self.expected,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityTestReceipt {
    pub test_id: String,
    pub status: CapabilityTestStatus,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityGateRequest {
    pub capability_id: String,
    pub capability_kind: String,
    #[serde(default = "programmable_action_tier")]
    pub action_tier: String,
    #[serde(default)]
    pub human_authorized: bool,
    #[serde(default)]
    pub test_receipts: Vec<CapabilityTestReceipt>,
    #[serde(default)]
    pub git_ref: String,
}

fn programmable_action_tier() -> String {
    PROGRAMMABLE_CAPABILITY_ACTION_TIER.to_string()
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityExposureDecision {
    HoldForApproval,
    TestFailed,
    Expose,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityGateReceipt {
    pub capability_id: String,
    pub decision: CapabilityExposureDecision,
    pub action_tier: String,
    pub test_status: CapabilityTestStatus,
    pub message: String,
}

pub fn evaluate_capability_gate(mut req: CapabilityGateRequest) -> CapabilityGateReceipt {
    req.action_tier = PROGRAMMABLE_CAPABILITY_ACTION_TIER.to_string();
    let test_failed = req
        .test_receipts
        .iter()
        .any(|receipt| receipt.status == CapabilityTestStatus::Failed);
    if test_failed || req.test_receipts.is_empty() {
        return CapabilityGateReceipt {
            capability_id: req.capability_id,
            decision: CapabilityExposureDecision::TestFailed,
            action_tier: req.action_tier,
            test_status: CapabilityTestStatus::Failed,
            message: "capability exposure requires a passing declared test".to_string(),
        };
    }

    let tiers = programmable_action_tiers();
    let payload = publication_payload(&req);
    let heads = [
        "programmable-harness".to_string(),
        "programmable-harness-review".to_string(),
    ];
    match evaluate_publication(&heads, &tiers, &payload) {
        Ok(()) => CapabilityGateReceipt {
            capability_id: req.capability_id,
            decision: CapabilityExposureDecision::Expose,
            action_tier: req.action_tier,
            test_status: CapabilityTestStatus::Passed,
            message: "declared test passed and tier-two authorization is present".to_string(),
        },
        Err(BindingError::Guard(violation)) => CapabilityGateReceipt {
            capability_id: req.capability_id,
            decision: CapabilityExposureDecision::HoldForApproval,
            action_tier: req.action_tier,
            test_status: CapabilityTestStatus::Passed,
            message: violation.message,
        },
    }
}

pub fn programmable_action_tiers() -> Vec<ActionTierPolicy> {
    BindingCapabilityScope::for_agent("programmable-harness").action_tiers
}

fn publication_payload(req: &CapabilityGateRequest) -> Payload {
    let value = json!({
        "action_tier": req.action_tier,
        "human_authorized": req.human_authorized,
        "claims": [{
            "statement": format!("programmable capability {} may be exposed", req.capability_id),
            "provenance": if req.git_ref.trim().is_empty() {
                "programmable-harness/test-gate".to_string()
            } else {
                format!("git:{}", req.git_ref)
            },
        }],
    });
    match value {
        Value::Object(map) => map,
        _ => Payload::new(),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityRollbackReceipt {
    pub capability_id: String,
    pub status: String,
    pub git_ref: String,
    pub exposed: bool,
}

pub fn rollback_capability(capability_id: &str, git_ref: &str) -> CapabilityRollbackReceipt {
    CapabilityRollbackReceipt {
        capability_id: capability_id.trim().to_string(),
        status: "rolled_back".to_string(),
        git_ref: git_ref.trim().to_string(),
        exposed: false,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BehaviorPatternCandidate {
    pub pattern_id: String,
    pub weight: f32,
    pub summary: String,
    pub recommended_kind: CapabilityKind,
    #[serde(default)]
    pub corpus_segment_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    DeclarativeSkill,
    WasmPlugin,
}

pub fn surface_crystallization_candidate(
    patterns: &[BehaviorPatternCandidate],
    min_weight: f32,
) -> Option<BehaviorPatternCandidate> {
    patterns
        .iter()
        .filter(|pattern| pattern.weight >= min_weight)
        .max_by(|left, right| {
            left.weight
                .partial_cmp(&right.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.pattern_id.cmp(&right.pattern_id).reverse())
        })
        .cloned()
}

pub fn crystallize_pattern_to_declarative_skill(
    pattern: &BehaviorPatternCandidate,
    skill_id: &str,
    steps: Vec<DeclarativeSkillStep>,
) -> DeclarativeSkillDefinition {
    DeclarativeSkillDefinition {
        skill_id: skill_id.trim().to_string(),
        tenant_id: String::new(),
        title: pattern.summary.clone(),
        description: format!("Crystallized from behavior pattern {}", pattern.pattern_id),
        parameters_schema: json!({}),
        steps,
        declared_tests: vec![CapabilityTest {
            test_id: "crystallized-skill-smoke".to_string(),
            kind: "always_pass".to_string(),
            expected: json!({}),
        }],
        provenance: CapabilityProvenance {
            corpus_segment_ids: pattern.corpus_segment_ids.clone(),
            source_refs: vec![format!("behavior_pattern:{}", pattern.pattern_id)],
            authored_by: "behavior-corpus-flywheel".to_string(),
        },
    }
}

fn clean_strings(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn echo_wat(function: &str) -> String {
        format!(
            r#"
            (module
                (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
                (import "extism:host/env" "length" (func $length (param i64) (result i64)))
                (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
                (import "extism:host/user" "{function}" (func $host (param i64) (result i64)))
                (func (export "run") (result i32)
                    (local $input i64)
                    (local $output i64)
                    (local.set $input (call $input_offset))
                    (local.set $output (call $host (local.get $input)))
                    (call $output_set (local.get $output) (call $length (local.get $output)))
                    (i32.const 0)
                )
            )
            "#
        )
    }

    fn loop_wat() -> String {
        r#"
            (module
                (func (export "run") (result i32)
                    (loop $forever
                        br $forever
                    )
                    (i32.const 0)
                )
            )
        "#
        .to_string()
    }

    fn grow_memory_wat() -> String {
        r#"
            (module
                (memory (export "memory") 1)
                (func (export "run") (result i32)
                    (drop (memory.grow (i32.const 64)))
                    (i32.const 0)
                )
            )
        "#
        .to_string()
    }

    fn spec_with(function: &str, grants: Vec<HostFunctionGrant>) -> WasmPluginSpec {
        WasmPluginSpec {
            plugin_id: format!("test.{function}"),
            tenant_id: "tenant".to_string(),
            source: WasmPluginSource::Wat(echo_wat(function)),
            exports: vec![PluginExportSpec {
                name: "run".to_string(),
                label: "Run".to_string(),
                description: "test export".to_string(),
                input_schema: json!({}),
                permissions: vec![],
                writeback_policy: "read-only".to_string(),
                tags: vec![],
            }],
            grants,
            limits: PluginLimits::default(),
            declared_tests: vec![],
            provenance: CapabilityProvenance {
                corpus_segment_ids: vec!["seg:1".to_string()],
                source_refs: vec!["test".to_string()],
                authored_by: "test".to_string(),
            },
        }
    }

    #[test]
    fn wasm_plugin_loads_from_bytes_path_and_runs_sandboxed_call() {
        let wat = echo_wat("thg_graph_read");
        let wasm = wat::parse_str(&wat).unwrap();
        let mut loaded = PluginHost::new()
            .with_graph_seed(vec![json!({"fact": "seed"})])
            .load(WasmPluginSpec {
                source: WasmPluginSource::Bytes(wasm.clone()),
                ..spec_with("thg_graph_read", vec![HostFunctionGrant::GraphRead])
            })
            .unwrap();
        let output = loaded.invoke("run", r#"{"query":"facts"}"#).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["facts"][0]["fact"], "seed");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugin.wasm");
        fs::write(&path, wasm).unwrap();
        let mut loaded_from_file = PluginHost::new()
            .load(WasmPluginSpec {
                source: WasmPluginSource::File(path),
                ..spec_with("thg_graph_read", vec![HostFunctionGrant::GraphRead])
            })
            .unwrap();
        assert!(loaded_from_file
            .invoke("run", "{}")
            .unwrap()
            .contains("\"ok\":true"));
    }

    #[test]
    fn wasm_plugin_limiters_terminate_runaway_and_memory_growth() {
        let mut timeout_plugin = PluginHost::new()
            .load(WasmPluginSpec {
                plugin_id: "limit.loop".to_string(),
                tenant_id: "tenant".to_string(),
                source: WasmPluginSource::Wat(loop_wat()),
                exports: vec![],
                grants: vec![],
                limits: PluginLimits {
                    timeout_ms: 10,
                    fuel: 1_000,
                    ..PluginLimits::default()
                },
                declared_tests: vec![],
                provenance: CapabilityProvenance::default(),
            })
            .unwrap();
        let error = timeout_plugin.invoke("run", "").unwrap_err().to_string();
        assert!(
            error.contains("timeout") || error.contains("fuel"),
            "runaway plugin should be terminated, got {error}"
        );

        let mut memory_plugin = PluginHost::new()
            .load(WasmPluginSpec {
                plugin_id: "limit.memory".to_string(),
                tenant_id: "tenant".to_string(),
                source: WasmPluginSource::Wat(grow_memory_wat()),
                exports: vec![],
                grants: vec![],
                limits: PluginLimits {
                    memory_max_pages: 1,
                    ..PluginLimits::default()
                },
                declared_tests: vec![],
                provenance: CapabilityProvenance::default(),
            })
            .unwrap();
        let error = memory_plugin.invoke("run", "").unwrap_err().to_string();
        assert!(
            error.contains("oom"),
            "memory limiter should trap, got {error}"
        );
    }

    #[test]
    fn granted_host_function_writes_provenance_fact_and_denied_call_does_not() {
        let mut granted = PluginHost::new()
            .load(spec_with(
                "thg_fact_write",
                vec![HostFunctionGrant::FactWrite],
            ))
            .unwrap();
        let output = granted
            .invoke(
                "run",
                r#"{"subject":"cap","predicate":"emits","object":"fact"}"#,
            )
            .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["ok"], true);
        let state = granted.runtime_state();
        assert_eq!(state.fact_writes.len(), 1);
        assert_eq!(
            state.fact_writes[0].provenance.corpus_segment_ids,
            vec!["seg:1"]
        );

        let mut denied = PluginHost::new()
            .load(spec_with("thg_fact_write", vec![]))
            .unwrap();
        let output = denied.invoke("run", "{}").unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["ok"], false);
        assert_eq!(value["grant"], "fact_write");
        assert!(denied.runtime_state().fact_writes.is_empty());
    }

    #[derive(Default)]
    struct MockInvoker {
        calls: Vec<String>,
    }

    impl DeclarativeAffordanceInvoker for MockInvoker {
        fn invoke_affordance(
            &mut self,
            affordance_id: &str,
            arguments: Value,
        ) -> PluginResult<Value> {
            self.calls.push(affordance_id.to_string());
            Ok(json!({
                "affordance_id": affordance_id,
                "arguments": arguments,
                "ok": true,
            }))
        }
    }

    #[test]
    fn declarative_skill_composes_two_affordances_and_has_pack_shape() {
        let definition = DeclarativeSkillDefinition {
            skill_id: "skill.two-step".to_string(),
            tenant_id: "tenant".to_string(),
            title: "Two step".to_string(),
            description: "compose two existing affordances".to_string(),
            parameters_schema: json!({"type":"object"}),
            steps: vec![
                DeclarativeSkillStep {
                    affordance_id: "search.code".to_string(),
                    arguments: json!({"q": "plugin"}),
                },
                DeclarativeSkillStep {
                    affordance_id: "graph.write".to_string(),
                    arguments: json!({"node": "receipt"}),
                },
            ],
            declared_tests: vec![CapabilityTest {
                test_id: "decl-smoke".to_string(),
                kind: "always_pass".to_string(),
                expected: json!({}),
            }],
            provenance: CapabilityProvenance::default(),
        };
        let pack = definition.to_skill_pack_value().unwrap();
        assert_eq!(pack["kind"], "skill_pack");
        assert_eq!(pack["metadata"]["capability_kind"], "declarative_skill");

        let mut invoker = MockInvoker::default();
        let receipt = invoke_declarative_skill(&definition, &mut invoker).unwrap();
        assert_eq!(receipt.status, "applied");
        assert_eq!(invoker.calls, vec!["search.code", "graph.write"]);

        let publish_request = definition
            .to_skill_publish_request(DeclarativeSkillPublishOptions {
                actor_id: "codex".to_string(),
                created_at: "t1".to_string(),
                ..DeclarativeSkillPublishOptions::default()
            })
            .unwrap();
        assert_eq!(publish_request.tenant_slug, "tenant");
        assert_eq!(publish_request.actor_id, "codex");
        assert_eq!(publish_request.status, "validated");
        assert_eq!(publish_request.created_at, "t1");
        assert_eq!(publish_request.pack["kind"], "skill_pack");
        assert_eq!(publish_request.pack["id"], "skill.two-step");
        assert_eq!(
            publish_request.pack["capabilities"],
            json!(["skill.two-step"])
        );
        assert!(!publish_request.source_content_hash.is_empty());
    }

    #[test]
    fn safety_gate_holds_fails_exposes_and_rolls_back() {
        let passed = CapabilityTestReceipt {
            test_id: "smoke".to_string(),
            status: CapabilityTestStatus::Passed,
            message: "ok".to_string(),
        };
        let hold = evaluate_capability_gate(CapabilityGateRequest {
            capability_id: "cap.new".to_string(),
            capability_kind: "wasm_plugin".to_string(),
            human_authorized: false,
            test_receipts: vec![passed.clone()],
            git_ref: "abc123".to_string(),
            action_tier: PROGRAMMABLE_CAPABILITY_ACTION_TIER.to_string(),
        });
        assert_eq!(hold.decision, CapabilityExposureDecision::HoldForApproval);

        let spoofed_tier = evaluate_capability_gate(CapabilityGateRequest {
            capability_id: "cap.new".to_string(),
            capability_kind: "wasm_plugin".to_string(),
            human_authorized: false,
            test_receipts: vec![passed.clone()],
            git_ref: "abc123".to_string(),
            action_tier: "tier_one".to_string(),
        });
        assert_eq!(
            spoofed_tier.decision,
            CapabilityExposureDecision::HoldForApproval
        );
        assert_eq!(
            spoofed_tier.action_tier,
            PROGRAMMABLE_CAPABILITY_ACTION_TIER
        );

        let failed = evaluate_capability_gate(CapabilityGateRequest {
            capability_id: "cap.new".to_string(),
            capability_kind: "wasm_plugin".to_string(),
            human_authorized: true,
            test_receipts: vec![CapabilityTestReceipt {
                status: CapabilityTestStatus::Failed,
                ..passed.clone()
            }],
            git_ref: "abc123".to_string(),
            action_tier: PROGRAMMABLE_CAPABILITY_ACTION_TIER.to_string(),
        });
        assert_eq!(failed.decision, CapabilityExposureDecision::TestFailed);

        let exposed = evaluate_capability_gate(CapabilityGateRequest {
            capability_id: "cap.new".to_string(),
            capability_kind: "wasm_plugin".to_string(),
            human_authorized: true,
            test_receipts: vec![passed],
            git_ref: "abc123".to_string(),
            action_tier: PROGRAMMABLE_CAPABILITY_ACTION_TIER.to_string(),
        });
        assert_eq!(exposed.decision, CapabilityExposureDecision::Expose);

        let rollback = rollback_capability("cap.new", "HEAD~1");
        assert!(!rollback.exposed);
        assert_eq!(rollback.status, "rolled_back");
    }

    #[test]
    fn corpus_flywheel_surfaces_candidate_and_records_provenance() {
        let pattern = surface_crystallization_candidate(
            &[
                BehaviorPatternCandidate {
                    pattern_id: "low".to_string(),
                    weight: 0.2,
                    summary: "low".to_string(),
                    recommended_kind: CapabilityKind::DeclarativeSkill,
                    corpus_segment_ids: vec!["seg-low".to_string()],
                },
                BehaviorPatternCandidate {
                    pattern_id: "high".to_string(),
                    weight: 0.91,
                    summary: "repeat winning sequence".to_string(),
                    recommended_kind: CapabilityKind::DeclarativeSkill,
                    corpus_segment_ids: vec!["seg-a".to_string(), "seg-b".to_string()],
                },
            ],
            0.8,
        )
        .unwrap();
        let skill = crystallize_pattern_to_declarative_skill(
            &pattern,
            "skill.crystallized",
            vec![DeclarativeSkillStep {
                affordance_id: "search.code".to_string(),
                arguments: json!({}),
            }],
        );
        assert_eq!(skill.provenance.corpus_segment_ids, vec!["seg-a", "seg-b"]);
        assert_eq!(skill.provenance.source_refs, vec!["behavior_pattern:high"]);
    }
}
