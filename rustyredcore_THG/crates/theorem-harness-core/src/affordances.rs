use crate::state_hash::stable_value_hash;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AffordanceContract {
    pub affordance_id: String,
    pub engine_id: String,
    pub family: String,
    pub label: String,
    pub input_shape: String,
    pub output_shape: String,
    pub writeback_policy: String,
    pub execution_surface: String,
    pub parity_status: String,
    pub source_module: String,
    pub permissions: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AffordanceReceipt {
    pub engine_id: String,
    pub affordance_id: String,
    pub receipt_hash: String,
    pub input_hash: String,
    #[serde(default)]
    pub input_node_refs: Vec<String>,
    #[serde(default)]
    pub payload: Map<String, Value>,
    #[serde(default = "read_only_policy")]
    pub writeback_policy: String,
    #[serde(default)]
    pub provenance: Map<String, Value>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

impl AffordanceReceipt {
    pub fn new(
        engine_id: impl Into<String>,
        affordance_id: impl Into<String>,
        input_hash: impl Into<String>,
        payload: Map<String, Value>,
    ) -> Self {
        let mut receipt = Self {
            engine_id: engine_id.into(),
            affordance_id: affordance_id.into(),
            receipt_hash: String::new(),
            input_hash: input_hash.into(),
            input_node_refs: Vec::new(),
            payload,
            writeback_policy: read_only_policy(),
            provenance: Map::new(),
            metadata: Map::new(),
        };
        receipt.receipt_hash = receipt.computed_receipt_hash();
        receipt
    }

    pub fn with_input_node_refs(mut self, refs: Vec<String>) -> Self {
        self.input_node_refs = refs.into_iter().filter(|item| !item.is_empty()).collect();
        self.refresh_hash();
        self
    }

    pub fn with_writeback_policy(mut self, policy: impl Into<String>) -> Self {
        self.writeback_policy = policy.into();
        self.refresh_hash();
        self
    }

    pub fn with_provenance(mut self, provenance: Map<String, Value>) -> Self {
        self.provenance = provenance;
        self.refresh_hash();
        self
    }

    pub fn with_metadata(mut self, metadata: Map<String, Value>) -> Self {
        self.metadata = metadata;
        self.refresh_hash();
        self
    }

    pub fn refresh_hash(&mut self) {
        self.receipt_hash = self.computed_receipt_hash();
    }

    pub fn computed_receipt_hash(&self) -> String {
        stable_value_hash(&json!({
            "engine_id": self.engine_id,
            "affordance_id": self.affordance_id,
            "input_hash": self.input_hash,
            "input_node_refs": self.input_node_refs,
            "payload": self.payload,
            "writeback_policy": self.writeback_policy,
            "provenance": self.provenance,
            "metadata": self.metadata,
        }))
    }
}

pub fn default_affordance_registry() -> Vec<AffordanceContract> {
    STATIC_AFFORDANCES
        .iter()
        .copied()
        .map(AffordanceContract::from_static)
        .collect()
}

pub fn affordance_by_id(affordance_id: &str) -> Option<AffordanceContract> {
    default_affordance_registry()
        .into_iter()
        .find(|contract| contract.affordance_id == affordance_id)
}

pub fn affordance_ids() -> Vec<String> {
    default_affordance_registry()
        .into_iter()
        .map(|contract| contract.affordance_id)
        .collect()
}

pub fn validate_affordance_registry() -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for contract in default_affordance_registry() {
        if contract.affordance_id.trim().is_empty() {
            return Err("affordance_id must not be empty".to_string());
        }
        if !seen.insert(contract.affordance_id.clone()) {
            return Err(format!(
                "duplicate affordance_id {}",
                contract.affordance_id
            ));
        }
        if contract.engine_id.trim().is_empty() {
            return Err(format!("{} has empty engine_id", contract.affordance_id));
        }
        if contract.input_shape.trim().is_empty() || contract.output_shape.trim().is_empty() {
            return Err(format!(
                "{} must declare input/output shapes",
                contract.affordance_id
            ));
        }
    }
    Ok(())
}

impl AffordanceContract {
    fn from_static(definition: StaticAffordanceContract) -> Self {
        Self {
            affordance_id: definition.affordance_id.to_string(),
            engine_id: definition.engine_id.to_string(),
            family: definition.family.to_string(),
            label: definition.label.to_string(),
            input_shape: definition.input_shape.to_string(),
            output_shape: definition.output_shape.to_string(),
            writeback_policy: definition.writeback_policy.to_string(),
            execution_surface: definition.execution_surface.to_string(),
            parity_status: definition.parity_status.to_string(),
            source_module: definition.source_module.to_string(),
            permissions: strings(definition.permissions),
            tags: strings(definition.tags),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct StaticAffordanceContract {
    affordance_id: &'static str,
    engine_id: &'static str,
    family: &'static str,
    label: &'static str,
    input_shape: &'static str,
    output_shape: &'static str,
    writeback_policy: &'static str,
    execution_surface: &'static str,
    parity_status: &'static str,
    source_module: &'static str,
    permissions: &'static [&'static str],
    tags: &'static [&'static str],
}

const STATIC_AFFORDANCES: &[StaticAffordanceContract] = &[
    StaticAffordanceContract {
        affordance_id: "datalog.derive",
        engine_id: "native-datalog",
        family: "datalog",
        label: "Datalog derivation",
        input_shape: "substrate_fact_pack",
        output_shape: "datalog_receipt",
        writeback_policy: "read-only",
        execution_surface: "rustyred-thg-core",
        parity_status: "native-parity",
        source_module: "apps.notebook.inference_engines.affordances.run_datalog_affordance",
        permissions: &["graph_read"],
        tags: &["bgi", "symbolic", "facts"],
    },
    StaticAffordanceContract {
        affordance_id: "probabilistic.source_reliability",
        engine_id: "native-probabilistic",
        family: "probabilistic",
        label: "Source reliability",
        input_shape: "substrate_evidence_records",
        output_shape: "probabilistic_receipt",
        writeback_policy: "read-only",
        execution_surface: "rustyred-thg-core",
        parity_status: "native-parity",
        source_module:
            "apps.notebook.inference_engines.affordances.run_probabilistic_source_reliability",
        permissions: &["graph_read"],
        tags: &["bgi", "symbolic", "evidence"],
    },
    StaticAffordanceContract {
        affordance_id: "probabilistic.expected_value_of_information",
        engine_id: "native-probabilistic",
        family: "probabilistic",
        label: "Expected value of information",
        input_shape: "substrate_validator_records",
        output_shape: "probabilistic_receipt",
        writeback_policy: "read-only",
        execution_surface: "rustyred-thg-core",
        parity_status: "native-parity",
        source_module:
            "apps.notebook.inference_engines.affordances.run_probabilistic_expected_value",
        permissions: &["graph_read"],
        tags: &["bgi", "symbolic", "validators"],
    },
    StaticAffordanceContract {
        affordance_id: "causal.intervention_effect",
        engine_id: "assumption-bound-causal-fallback",
        family: "causal",
        label: "Causal intervention effect",
        input_shape: "causal_intervention",
        output_shape: "causal_receipt",
        writeback_policy: "proposal-only",
        execution_surface: "runtime-adapter",
        parity_status: "python-reference-projection",
        source_module: "apps.notebook.inference_engines.affordances_engines.run_causal_affordance",
        permissions: &["graph_read"],
        tags: &["bgi", "symbolic", "assumptions"],
    },
    StaticAffordanceContract {
        affordance_id: "evolution.archive",
        engine_id: "native-evolution",
        family: "evolution",
        label: "Quality-diversity archive",
        input_shape: "evolution_candidates",
        output_shape: "evolution_archive_receipt",
        writeback_policy: "read-only",
        execution_surface: "rustyred-thg-core",
        parity_status: "native-parity",
        source_module:
            "apps.notebook.inference_engines.affordances_engines.run_evolution_affordance",
        permissions: &["graph_read"],
        tags: &["bgi", "symbolic", "archive"],
    },
    StaticAffordanceContract {
        affordance_id: "proof.create_obligation",
        engine_id: "proof-obligation-tracker",
        family: "proof",
        label: "Proof obligation",
        input_shape: "proof_obligation",
        output_shape: "proof_receipt",
        writeback_policy: "proposal-only",
        execution_surface: "runtime-adapter",
        parity_status: "python-reference-projection",
        source_module: "apps.notebook.inference_engines.affordances_engines.run_proof_affordance",
        permissions: &["graph_read"],
        tags: &["bgi", "symbolic", "proof"],
    },
    StaticAffordanceContract {
        affordance_id: "optimizer.optimize",
        engine_id: "python-deterministic-optimizer",
        family: "optimizer",
        label: "Constrained optimizer",
        input_shape: "optimization_problem",
        output_shape: "optimization_receipt",
        writeback_policy: "read-only",
        execution_surface: "runtime-adapter",
        parity_status: "python-reference-projection",
        source_module:
            "apps.notebook.inference_engines.affordances_engines.run_optimizer_affordance",
        permissions: &["graph_read"],
        tags: &["bgi", "symbolic", "selection"],
    },
    StaticAffordanceContract {
        affordance_id: "expression.render",
        engine_id: "deterministic_brief",
        family: "expression",
        label: "Expression render",
        input_shape: "expression_result",
        output_shape: "expression_receipt",
        writeback_policy: "read-only",
        execution_surface: "runtime-adapter",
        parity_status: "deterministic-reference",
        source_module:
            "apps.notebook.inference_engines.affordances_engines.run_expression_affordance",
        permissions: &["graph_read"],
        tags: &["bgi", "symbolic", "render"],
    },
    StaticAffordanceContract {
        affordance_id: "egraph.extract",
        engine_id: "egraph-theorem",
        family: "egraph",
        label: "E-graph extraction",
        input_shape: "egraph_expression",
        output_shape: "egraph_receipt",
        writeback_policy: "read-only",
        execution_surface: "runtime-adapter",
        parity_status: "python-reference-projection",
        source_module: "apps.notebook.inference_engines.affordances_engines.run_egraph_affordance",
        permissions: &["graph_read"],
        tags: &["bgi", "symbolic", "rewrite"],
    },
    StaticAffordanceContract {
        affordance_id: "simulation.dry_run",
        engine_id: "simulation-receipt-fallback",
        family: "simulation",
        label: "Simulation dry run",
        input_shape: "simulation_dry_run",
        output_shape: "simulation_receipt",
        writeback_policy: "read-only",
        execution_surface: "runtime-adapter",
        parity_status: "python-reference-projection",
        source_module:
            "apps.notebook.inference_engines.affordances_engines.run_simulation_affordance",
        permissions: &["graph_read"],
        tags: &["bgi", "symbolic", "validation"],
    },
    StaticAffordanceContract {
        affordance_id: "solver.check",
        engine_id: "z3",
        family: "solver",
        label: "Constraint solver",
        input_shape: "solver_problem",
        output_shape: "solver_receipt",
        writeback_policy: "proposal-only",
        execution_surface: "runtime-adapter",
        parity_status: "python-reference-projection",
        source_module: "apps.notebook.inference_engines.affordances_engines.run_solver_affordance",
        permissions: &["graph_read"],
        tags: &["bgi", "symbolic", "constraints"],
    },
];

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn read_only_policy() -> String {
    "read-only".to_string()
}
