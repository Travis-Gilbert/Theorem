//! Per-binding charter compilation over registered affordances.
//!
//! The composed-agent spec requires zero silent capability: the binding charter
//! must enumerate what the agent can see and call before the reasoning loop
//! starts. This module is the pure compiler for that plane. It does not call
//! models, resolve credentials, or invoke tools; it turns the current
//! affordance graph plus an `AgentBinding` into deterministic payloads for
//! `CHARTER.COMPILED` and `CAPABILITIES.SELECTED`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use theorem_harness_core::{
    stable_value_hash, ActionTierPolicy, AgentBinding, BindingTransitionInput, Payload,
};

use crate::outcomes::{affordance_nodes, effective_affordance_fitness_from_node};
use crate::types::{Affordance, AffordanceGraphStore, CapabilityScope, DEFAULT_MIN_FITNESS};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct BindingCharterRequest {
    pub tenant_id: String,
    pub stance: String,
    pub task_type: String,
    #[serde(default)]
    pub scope: CapabilityScope,
    #[serde(default)]
    pub max_visible_tools: usize,
    #[serde(default)]
    pub min_fitness: Option<f32>,
}

impl BindingCharterRequest {
    pub fn normalized(mut self, binding: &AgentBinding) -> Self {
        self.tenant_id = self.tenant_id.trim().to_string();
        if self.tenant_id.is_empty() {
            self.tenant_id = binding.identity.agent_id.trim().to_string();
        }
        self.stance = self.stance.trim().to_string();
        self.task_type = self.task_type.trim().to_string();
        if self.scope.agent_id.trim().is_empty() {
            self.scope.agent_id = binding.identity.agent_id.clone();
        }
        if self.max_visible_tools == 0 {
            self.max_visible_tools = 64;
        }
        self.min_fitness = self
            .min_fitness
            .map(|value| value.clamp(0.0, 1.0))
            .or(Some(DEFAULT_MIN_FITNESS));
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BindingCharter {
    pub agent_id: String,
    pub owner_id: String,
    pub agent_name: String,
    pub composition_hash: String,
    pub stance: String,
    pub task_type: String,
    pub charter_hash: String,
    pub capability_scope_hash: String,
    pub visible_tools: Vec<CharterTool>,
    pub callable_tools: Vec<String>,
    pub confirmation_gated_tools: Vec<String>,
    pub binding_private_tools: Vec<String>,
    pub action_tiers: Vec<ActionTierPolicy>,
}

impl BindingCharter {
    pub fn charter_compiled_transition(&self) -> BindingTransitionInput {
        BindingTransitionInput::new("CHARTER.COMPILED", self.charter_compiled_payload())
    }

    pub fn capabilities_selected_transition(&self) -> BindingTransitionInput {
        BindingTransitionInput::new(
            "CAPABILITIES.SELECTED",
            self.capabilities_selected_payload(),
        )
    }

    pub fn charter_compiled_payload(&self) -> Payload {
        payload(json!({
            "charter_hash": self.charter_hash,
            "stance": self.stance,
            "task_type": self.task_type,
            "agent_id": self.agent_id,
            "composition_hash": self.composition_hash,
            "visible_tool_count": self.visible_tools.len(),
        }))
    }

    pub fn capabilities_selected_payload(&self) -> Payload {
        payload(json!({
            "capability_scope_hash": self.capability_scope_hash,
            "visible_tools": self.visible_tool_ids(),
            "callable_tools": self.callable_tools,
            "confirmation_gated_tools": self.confirmation_gated_tools,
            "binding_private_tools": self.binding_private_tools,
        }))
    }

    pub fn visible_tool_ids(&self) -> Vec<String> {
        self.visible_tools
            .iter()
            .map(|tool| tool.affordance_id.clone())
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CharterTool {
    pub affordance_id: String,
    pub server_id: String,
    pub family: String,
    pub label: String,
    pub description: String,
    pub input_schema: Value,
    pub permissions: Vec<String>,
    pub writeback_policy: String,
    pub tags: Vec<String>,
    pub fitness: f32,
    pub execution_surface: String,
    pub parity_status: String,
    pub source_module: String,
}

impl CharterTool {
    fn from_affordance(affordance: Affordance) -> Self {
        let execution_surface = affordance
            .cost
            .get("execution_surface")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let parity_status = affordance
            .cost
            .get("parity_status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let source_module = affordance
            .cost
            .get("source_module")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Self {
            affordance_id: affordance.affordance_id,
            server_id: affordance.server_id,
            family: affordance.family,
            label: affordance.label,
            description: affordance.description,
            input_schema: affordance.input_schema,
            permissions: affordance.permissions,
            writeback_policy: affordance.writeback_policy,
            tags: affordance.tags,
            fitness: affordance.fitness,
            execution_surface,
            parity_status,
            source_module,
        }
    }
}

pub fn compile_binding_charter_from_store<S: AffordanceGraphStore>(
    store: &S,
    binding: &AgentBinding,
    request: BindingCharterRequest,
) -> rustyred_thg_core::ThgResult<BindingCharter> {
    let affordances = affordance_nodes(store)?
        .into_iter()
        .map(|node| {
            let mut affordance = Affordance::from_node_record(&node)?;
            affordance.fitness = effective_affordance_fitness_from_node(&node);
            Ok(affordance)
        })
        .collect::<Result<Vec<_>, _>>()?;
    compile_binding_charter(binding, request, affordances)
        .map_err(|message| rustyred_thg_core::ThgError::new("invalid_binding_charter", message))
}

pub fn compile_binding_charter(
    binding: &AgentBinding,
    request: BindingCharterRequest,
    affordances: Vec<Affordance>,
) -> Result<BindingCharter, String> {
    let request = request.normalized(binding);
    if request.stance.is_empty() {
        return Err("charter stance is required".to_string());
    }

    let min_fitness = request.min_fitness.unwrap_or(DEFAULT_MIN_FITNESS);
    let mut visible_tools = affordances
        .into_iter()
        .filter(|affordance| affordance.tenant_id == request.tenant_id)
        .filter(|affordance| request.scope.admits(affordance))
        .filter(|affordance| affordance.fitness >= min_fitness)
        .map(CharterTool::from_affordance)
        .collect::<Vec<_>>();
    visible_tools.sort_by(|left, right| {
        left.family
            .cmp(&right.family)
            .then_with(|| left.server_id.cmp(&right.server_id))
            .then_with(|| left.affordance_id.cmp(&right.affordance_id))
    });
    visible_tools.truncate(request.max_visible_tools);

    let callable_tools = visible_tools
        .iter()
        .map(|tool| tool.affordance_id.clone())
        .collect::<Vec<_>>();
    let confirmation_gated_tools = visible_tools
        .iter()
        .filter(|tool| requires_confirmation(tool))
        .map(|tool| tool.affordance_id.clone())
        .collect::<Vec<_>>();
    let binding_private_tools = visible_tools
        .iter()
        .filter(|tool| {
            tool.tags.iter().any(|tag| {
                matches!(
                    tag.as_str(),
                    "binding_private" | "private" | "scratchpad" | "credential"
                )
            })
        })
        .map(|tool| tool.affordance_id.clone())
        .collect::<Vec<_>>();
    let action_tiers = binding.capability_scope.action_tiers.clone();
    let capability_scope_hash = stable_value_hash(&json!({
        "agent_id": binding.identity.agent_id,
        "visible_tools": callable_tools,
        "confirmation_gated_tools": confirmation_gated_tools,
        "binding_private_tools": binding_private_tools,
        "action_tiers": action_tiers,
    }));
    let charter_hash = stable_value_hash(&json!({
        "agent_id": binding.identity.agent_id,
        "owner_id": binding.identity.owner_id,
        "agent_name": binding.identity.agent_name,
        "composition_hash": binding.identity.composition_hash,
        "stance": request.stance,
        "task_type": request.task_type,
        "visible_tools": visible_tools,
        "capability_scope_hash": capability_scope_hash,
    }));

    Ok(BindingCharter {
        agent_id: binding.identity.agent_id.clone(),
        owner_id: binding.identity.owner_id.clone(),
        agent_name: binding.identity.agent_name.clone(),
        composition_hash: binding.identity.composition_hash.clone(),
        stance: request.stance,
        task_type: request.task_type,
        charter_hash,
        capability_scope_hash,
        visible_tools,
        callable_tools,
        confirmation_gated_tools,
        binding_private_tools,
        action_tiers,
    })
}

fn requires_confirmation(tool: &CharterTool) -> bool {
    if tool.writeback_policy != "read-only" {
        return true;
    }
    tool.permissions
        .iter()
        .chain(tool.tags.iter())
        .any(|value| matches!(value.as_str(), "write" | "writeback" | "external_action"))
}

fn payload(value: Value) -> Payload {
    match value {
        Value::Object(map) => map,
        _ => Payload::new(),
    }
}
