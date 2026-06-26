//! Harness capability pack for binary reconstruction.

use rustyred_thg_binformat::{load_binary, write_binary_facts_in_store, BinaryLoadReport};
use rustyred_thg_core::{
    GraphStore, GraphStoreError, GraphStoreResult, NodeQuery, NodeRecord, PluginCapability,
    PluginCapabilityKind, PluginOperationContext, PluginOperationRegistration, RedCoreGraphStore,
    RustyRedPlugin,
};
use rustyred_thg_disasm::{
    decode_instructions, write_instruction_facts_in_store, DisassemblyReport,
};
use rustyred_thg_lift::{lift_to_thir, write_thir_in_store, ThirProgram};
use rustyred_thg_reconstruct::{
    compile_reconstruction_analysis, validate_instruction, write_reconstruction_analysis_in_store,
    write_reconstruction_analysis_in_store_for_tenant,
    write_validation_receipt_in_store_for_tenant, ReconstructionAnalysis,
    ReconstructionInstruction, ReconstructionPlan, RECONSTRUCTION_INSTRUCTION_LABEL,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const RECONSTRUCT_CAPABILITY_PACK: &str = "theorem.reconstruct.binary";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReconstructToolSpec {
    pub name: String,
    pub summary: String,
    pub writes_graph: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReconstructCapabilityPack {
    pub capability: String,
    pub tools: Vec<ReconstructToolSpec>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ReconstructionPipelineReport {
    pub artifact_id: String,
    pub section_count: usize,
    pub symbol_count: usize,
    pub string_count: usize,
    pub instruction_count: usize,
    pub function_count: usize,
    pub component_count: usize,
    pub instruction_obligation_count: usize,
    pub plan: ReconstructionPlan,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ReconstructionPipelineOutput {
    pub load: BinaryLoadReport,
    pub disassembly: DisassemblyReport,
    pub thir: ThirProgram,
    pub analysis: ReconstructionAnalysis,
    pub report: ReconstructionPipelineReport,
}

#[derive(Clone, Debug, Default)]
pub struct ReconstructionHarnessPlugin;

impl RustyRedPlugin for ReconstructionHarnessPlugin {
    fn name(&self) -> &'static str {
        RECONSTRUCT_CAPABILITY_PACK
    }

    fn capabilities(&self) -> Vec<PluginCapability> {
        capability_pack()
            .tools
            .into_iter()
            .map(|tool| PluginCapability {
                kind: PluginCapabilityKind::Operation,
                name: tool.name,
            })
            .collect()
    }

    fn operations(&self) -> Vec<PluginOperationRegistration> {
        vec![
            PluginOperationRegistration {
                operation: "reconstruct.load",
                command: "reconstruct.load",
                aliases: &["theorem.reconstruct.binary.load"],
                summary: "Parse binary bytes and write observed artifact facts.",
                writes_graph: true,
                handler: load_handler,
            },
            PluginOperationRegistration {
                operation: "reconstruct.analyze",
                command: "reconstruct.analyze",
                aliases: &["theorem.reconstruct.binary.analyze"],
                summary:
                    "Run loader, decoder, lifter, semantic recovery, and instruction compiler.",
                writes_graph: true,
                handler: analyze_handler,
            },
            PluginOperationRegistration {
                operation: "reconstruct.lift",
                command: "reconstruct.lift",
                aliases: &["theorem.reconstruct.binary.lift"],
                summary: "Produce THIR functions and statements from binary instruction facts.",
                writes_graph: true,
                handler: lift_handler,
            },
            PluginOperationRegistration {
                operation: "reconstruct.graph.write",
                command: "reconstruct.graph.write",
                aliases: &["theorem.reconstruct.binary.graph.write"],
                summary: "Commit reconstruction facts and semantic graph nodes to GraphStore.",
                writes_graph: true,
                handler: analyze_handler,
            },
            PluginOperationRegistration {
                operation: "reconstruct.components.recover",
                command: "reconstruct.components.recover",
                aliases: &["theorem.reconstruct.binary.components.recover"],
                summary: "Recover component hypotheses from lifted binary graph evidence.",
                writes_graph: true,
                handler: components_handler,
            },
            PluginOperationRegistration {
                operation: "reconstruct.plan.compile",
                command: "reconstruct.plan.compile",
                aliases: &["theorem.reconstruct.binary.plan"],
                summary: "Compile reconstruction instructions from binary evidence.",
                writes_graph: true,
                handler: analyze_handler,
            },
            PluginOperationRegistration {
                operation: "reconstruct.instruction.get",
                command: "reconstruct.instruction.get",
                aliases: &["theorem.reconstruct.binary.instruction.get"],
                summary: "Return bounded reconstruction instructions for agent execution.",
                writes_graph: false,
                handler: instruction_get_handler,
            },
            PluginOperationRegistration {
                operation: "reconstruct.validate",
                command: "reconstruct.validate",
                aliases: &["theorem.reconstruct.binary.validate"],
                summary: "Run an instruction validator and write a validation receipt.",
                writes_graph: true,
                handler: validate_handler,
            },
            PluginOperationRegistration {
                operation: "reconstruct.receipt.write",
                command: "reconstruct.receipt.write",
                aliases: &["theorem.reconstruct.binary.receipt.write"],
                summary: "Write a validation receipt for a reconstruction instruction.",
                writes_graph: true,
                handler: validate_handler,
            },
        ]
    }
}

pub fn capability_pack() -> ReconstructCapabilityPack {
    ReconstructCapabilityPack {
        capability: RECONSTRUCT_CAPABILITY_PACK.to_string(),
        tools: vec![
            tool(
                "reconstruct.load",
                "Parses binary bytes and stores artifact facts.",
                true,
            ),
            tool(
                "reconstruct.analyze",
                "Runs loader and decoder recovery.",
                true,
            ),
            tool(
                "reconstruct.lift",
                "Produces THIR functions and statements.",
                true,
            ),
            tool(
                "reconstruct.graph.write",
                "Commits semantic graph facts to GraphStore.",
                true,
            ),
            tool(
                "reconstruct.components.recover",
                "Recovers component hypotheses.",
                true,
            ),
            tool(
                "reconstruct.plan.compile",
                "Emits reconstruction plans and instructions.",
                true,
            ),
            tool(
                "reconstruct.instruction.get",
                "Returns one bounded instruction.",
                false,
            ),
            tool(
                "reconstruct.validate",
                "Runs validators against implementation output.",
                true,
            ),
            tool(
                "reconstruct.receipt.write",
                "Stores validation results.",
                true,
            ),
        ],
    }
}

pub fn run_reconstruction_pipeline(
    name: impl Into<String>,
    bytes: &[u8],
) -> GraphStoreResult<ReconstructionPipelineOutput> {
    let load = load_binary(name, bytes)?;
    let disassembly = decode_instructions(&load)?;
    let thir = lift_to_thir(&load, &disassembly);
    let analysis = compile_reconstruction_analysis(&load, &thir);
    let report = ReconstructionPipelineReport {
        artifact_id: load.artifact.artifact_id.clone(),
        section_count: load.sections.len(),
        symbol_count: load.symbols.len(),
        string_count: load.strings.len(),
        instruction_count: disassembly.instructions.len(),
        function_count: thir.functions.len(),
        component_count: analysis.components.len(),
        instruction_obligation_count: analysis.plan.instructions.len(),
        plan: analysis.plan.clone(),
    };
    Ok(ReconstructionPipelineOutput {
        load,
        disassembly,
        thir,
        analysis,
        report,
    })
}

pub fn write_pipeline_output_in_store<S: GraphStore>(
    store: &mut S,
    output: &ReconstructionPipelineOutput,
) -> GraphStoreResult<()> {
    write_pipeline_output_in_store_scoped(store, output, None)
}

pub fn write_pipeline_output_in_store_for_tenant<S: GraphStore>(
    store: &mut S,
    output: &ReconstructionPipelineOutput,
    tenant_id: &str,
) -> GraphStoreResult<()> {
    write_pipeline_output_in_store_scoped(store, output, Some(tenant_id))
}

fn write_pipeline_output_in_store_scoped<S: GraphStore>(
    store: &mut S,
    output: &ReconstructionPipelineOutput,
    tenant_id: Option<&str>,
) -> GraphStoreResult<()> {
    write_binary_facts_in_store(store, &output.load)?;
    write_instruction_facts_in_store(store, &output.disassembly)?;
    write_thir_in_store(store, &output.thir)?;
    if let Some(tenant_id) = tenant_id {
        write_reconstruction_analysis_in_store_for_tenant(store, &output.analysis, tenant_id)?;
    } else {
        write_reconstruction_analysis_in_store(store, &output.analysis)?;
    }
    Ok(())
}

fn tool(name: &str, summary: &str, writes_graph: bool) -> ReconstructToolSpec {
    ReconstructToolSpec {
        name: name.to_string(),
        summary: summary.to_string(),
        writes_graph,
    }
}

fn load_handler(context: PluginOperationContext<'_>, arguments: Value) -> GraphStoreResult<Value> {
    let input = BinaryBytesInput::from_value(arguments)?;
    let bytes = decode_hex(&input.bytes_hex)?;
    let load = load_binary(input.artifact_name, &bytes)?;
    write_binary_facts_in_store(context.store, &load)?;
    Ok(json!({
        "artifact_id": load.artifact.artifact_id,
        "section_count": load.sections.len(),
        "symbol_count": load.symbols.len(),
        "string_count": load.strings.len(),
    }))
}

fn analyze_handler(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let input = BinaryBytesInput::from_value(arguments)?;
    let bytes = decode_hex(&input.bytes_hex)?;
    let output = run_reconstruction_pipeline(input.artifact_name, &bytes)?;
    write_pipeline_output_in_store_for_tenant(context.store, &output, context.tenant_id)?;
    Ok(json!(output.report))
}

fn lift_handler(context: PluginOperationContext<'_>, arguments: Value) -> GraphStoreResult<Value> {
    let input = BinaryBytesInput::from_value(arguments)?;
    let bytes = decode_hex(&input.bytes_hex)?;
    let load = load_binary(input.artifact_name, &bytes)?;
    let disassembly = decode_instructions(&load)?;
    let thir = lift_to_thir(&load, &disassembly);
    write_binary_facts_in_store(context.store, &load)?;
    write_instruction_facts_in_store(context.store, &disassembly)?;
    write_thir_in_store(context.store, &thir)?;
    Ok(json!({
        "artifact_id": load.artifact.artifact_id,
        "instruction_count": disassembly.instructions.len(),
        "function_count": thir.functions.len(),
        "thir_function_ids": thir.functions.iter().map(|function| function.function_id.clone()).collect::<Vec<_>>(),
    }))
}

fn components_handler(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let input = BinaryBytesInput::from_value(arguments)?;
    let bytes = decode_hex(&input.bytes_hex)?;
    let output = run_reconstruction_pipeline(input.artifact_name, &bytes)?;
    write_pipeline_output_in_store_for_tenant(context.store, &output, context.tenant_id)?;
    Ok(json!({
        "artifact_id": output.load.artifact.artifact_id,
        "component_count": output.analysis.components.len(),
        "components": output.analysis.components,
    }))
}

fn instruction_get_handler(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let instruction_id = arguments
        .get("instruction_id")
        .or_else(|| arguments.get("instructionId"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let nodes = if let Some(instruction_id) = instruction_id {
        GraphStore::get_node_record(context.store, &instruction_id)
            .into_iter()
            .filter(|node| is_reconstruction_instruction_for_tenant(node, context.tenant_id))
            .collect::<Vec<_>>()
    } else {
        GraphStore::query_nodes(
            context.store,
            NodeQuery::label(RECONSTRUCTION_INSTRUCTION_LABEL)
                .with_property("tenant_id", json!(context.tenant_id))
                .with_limit(limit),
        )
    };
    Ok(json!({
        "instructions": nodes.into_iter().map(|node| json!({
            "id": node.id,
            "properties": node.properties,
        })).collect::<Vec<_>>()
    }))
}

fn validate_handler(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let instruction_value = arguments
        .get("instruction")
        .cloned()
        .ok_or_else(|| GraphStoreError::new("missing_instruction", "instruction is required"))?;
    let instruction: ReconstructionInstruction = serde_json::from_value(instruction_value)
        .map_err(|error| {
            GraphStoreError::new(
                "invalid_instruction",
                format!("failed to decode reconstruction instruction: {error}"),
            )
        })?;
    let observed = arguments.get("observed").cloned().unwrap_or(Value::Null);
    let receipt = validate_instruction(&instruction, observed);
    write_validation_receipt_in_store_for_tenant(context.store, &receipt, context.tenant_id)?;
    Ok(json!(receipt))
}

fn is_reconstruction_instruction_for_tenant(node: &NodeRecord, tenant_id: &str) -> bool {
    node.labels
        .iter()
        .any(|label| label == RECONSTRUCTION_INSTRUCTION_LABEL)
        && node.properties.get("tenant_id").and_then(Value::as_str) == Some(tenant_id)
}

fn decode_hex(bytes_hex: &str) -> GraphStoreResult<Vec<u8>> {
    hex::decode(bytes_hex.trim()).map_err(|error| {
        GraphStoreError::new(
            "invalid_binary_hex",
            format!("bytes_hex must be hex-encoded binary bytes: {error}"),
        )
    })
}

#[derive(Clone, Debug, Deserialize)]
struct BinaryBytesInput {
    artifact_name: String,
    bytes_hex: String,
}

impl BinaryBytesInput {
    fn from_value(value: Value) -> GraphStoreResult<Self> {
        let artifact_name = value
            .get("artifact_name")
            .or_else(|| value.get("artifactName"))
            .and_then(Value::as_str)
            .unwrap_or("artifact.bin")
            .to_string();
        let bytes_hex = value
            .get("bytes_hex")
            .or_else(|| value.get("bytesHex"))
            .and_then(Value::as_str)
            .ok_or_else(|| GraphStoreError::new("missing_bytes_hex", "bytes_hex is required"))?
            .to_string();
        Ok(Self {
            artifact_name,
            bytes_hex,
        })
    }
}

#[allow(dead_code)]
fn _assert_store_bound(_: &mut RedCoreGraphStore) {}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::{NodeRecord, PluginRegistry, RedCoreGraphStore};

    #[test]
    fn capability_pack_lists_agent_tools_without_raw_disassembly_surface() {
        let pack = capability_pack();
        assert_eq!(pack.capability, RECONSTRUCT_CAPABILITY_PACK);
        assert!(pack
            .tools
            .iter()
            .any(|tool| tool.name == "reconstruct.instruction.get"));
        assert!(!pack
            .tools
            .iter()
            .any(|tool| tool.name.contains("raw_disassembly")));
    }

    #[test]
    fn plugin_registers_reconstruct_operations() {
        let mut registry = PluginRegistry::new();
        registry.register(ReconstructionHarnessPlugin);
        assert!(registry.operation("reconstruct.analyze").is_some());
        assert!(registry
            .operation("theorem.reconstruct.binary.instruction.get")
            .is_some());
    }

    #[test]
    fn validate_operation_writes_receipt() {
        let instruction = ReconstructionInstruction {
            id: "recon:instr:test".to_string(),
            source_artifact: "sha256:test".to_string(),
            target: rustyred_thg_reconstruct::ReconstructionTarget {
                kind: "component".to_string(),
                id: "component:test".to_string(),
                language: "rust".to_string(),
                runtime: "axum".to_string(),
            },
            action: rustyred_thg_reconstruct::ReconstructionAction::ImplementComponent,
            requirements: Vec::new(),
            validators: vec![rustyred_thg_reconstruct::ValidatorSpec::GoldenFixture {
                input: "ping".to_string(),
                expected: "pong".to_string(),
            }],
            evidence: Vec::new(),
            confidence: 0.8,
            uncertainty: Vec::new(),
        };
        let mut registry = PluginRegistry::new();
        registry.register(ReconstructionHarnessPlugin);
        let mut store = RedCoreGraphStore::memory();
        store
            .upsert_node(NodeRecord::new(
                "recon:instr:test",
                [RECONSTRUCTION_INSTRUCTION_LABEL],
                json!({"source_artifact": "sha256:test"}),
            ))
            .unwrap();
        let output = registry
            .execute(
                &mut store,
                "Travis-Gilbert",
                "reconstruct.validate",
                json!({"instruction": instruction, "observed": "pong"}),
            )
            .unwrap();
        assert_eq!(output.result["passed"], true);
    }

    #[test]
    fn instruction_get_filters_by_tenant_and_id_before_limit() {
        let mut registry = PluginRegistry::new();
        registry.register(ReconstructionHarnessPlugin);
        let mut store = RedCoreGraphStore::memory();
        store
            .upsert_node(NodeRecord::new(
                "recon:instr:other",
                [RECONSTRUCTION_INSTRUCTION_LABEL],
                json!({
                    "tenant_id": "Other-Tenant",
                    "source_artifact": "sha256:other"
                }),
            ))
            .unwrap();
        store
            .upsert_node(NodeRecord::new(
                "recon:instr:target",
                [RECONSTRUCTION_INSTRUCTION_LABEL],
                json!({
                    "tenant_id": "Travis-Gilbert",
                    "source_artifact": "sha256:target"
                }),
            ))
            .unwrap();

        let by_id = registry
            .execute(
                &mut store,
                "Travis-Gilbert",
                "reconstruct.instruction.get",
                json!({"instruction_id": "recon:instr:target"}),
            )
            .unwrap();
        assert_eq!(by_id.result["instructions"].as_array().unwrap().len(), 1);
        assert_eq!(
            by_id.result["instructions"][0]["id"],
            json!("recon:instr:target")
        );

        let tenant_page = registry
            .execute(
                &mut store,
                "Travis-Gilbert",
                "reconstruct.instruction.get",
                json!({"limit": 10}),
            )
            .unwrap();
        let instructions = tenant_page.result["instructions"].as_array().unwrap();
        assert_eq!(instructions.len(), 1);
        assert_eq!(instructions[0]["id"], json!("recon:instr:target"));
    }
}
