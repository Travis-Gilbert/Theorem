//! Harness capability pack for binary reconstruction.

use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::time::Duration;

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
    compile_reconstruction_analysis, tenant_scoped_reconstruction_id, validate_instruction,
    write_reconstruction_analysis_in_store, write_reconstruction_analysis_in_store_for_tenant,
    write_validation_receipt_in_store_for_tenant, ReconstructionAnalysis,
    ReconstructionInstruction, ReconstructionPlan, RECONSTRUCTION_INSTRUCTION_LABEL,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const RECONSTRUCT_CAPABILITY_PACK: &str = "theorem.reconstruct.binary";
const MAX_RECONSTRUCTION_INSTRUCTION_LIMIT: u64 = 100;

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
    pub function_signature_count: usize,
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
                summary: "Parse a binary artifact (from bytes_hex or url) and write observed artifact facts.",
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
                "Parses a binary artifact (from bytes_hex or url) and stores artifact facts.",
                true,
            ),
            tool(
                "reconstruct.analyze",
                "Runs loader, decoder, lifting, semantic signatures, and recovery.",
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
        function_signature_count: analysis.signatures.len(),
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

/// Hard ceiling for a fetched artifact body. A caller may pass a smaller
/// `max_bytes`, but request input can never raise the effective limit above
/// this, so a hostile or mis-specified size cannot drive an unbounded download.
pub const DEFAULT_MAX_FETCH_BYTES: usize = 256 * 1024 * 1024;

/// URL front door for the engine: fetch a binary artifact over http(s) and run
/// the full reconstruction pipeline on the fetched bytes. Callers that only have
/// a URL (the harness reverse-engineer tool) reach reconstruction without
/// pre-downloading and hex-encoding the artifact first.
pub fn run_reconstruction_pipeline_from_url(
    name: Option<String>,
    url: &str,
    max_bytes: usize,
) -> GraphStoreResult<ReconstructionPipelineOutput> {
    let bytes = fetch_binary_bytes(url, max_bytes)?;
    let name = name.unwrap_or_else(|| default_artifact_name_from_url(url));
    run_reconstruction_pipeline(name, &bytes)
}

/// Download an artifact body over http(s) with a scheme guard, an SSRF host
/// guard, a global timeout, and a hard size ceiling so a hostile or oversized
/// response cannot reach internal services or exhaust memory.
pub fn fetch_binary_bytes(url: &str, max_bytes: usize) -> GraphStoreResult<Vec<u8>> {
    fetch_binary_bytes_with_policy(url, max_bytes, false)
}

fn fetch_binary_bytes_with_policy(
    url: &str,
    max_bytes: usize,
    allow_private_hosts: bool,
) -> GraphStoreResult<Vec<u8>> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(GraphStoreError::new(
            "invalid_url_scheme",
            "url must start with http:// or https://",
        ));
    }
    if !allow_private_hosts {
        guard_url_host(url)?;
    }
    // Clamp to the hard ceiling so request input cannot raise the limit, and use
    // saturating arithmetic so the read bound cannot overflow or wrap.
    let limit = max_bytes.min(DEFAULT_MAX_FETCH_BYTES);
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(120)))
            .build(),
    );
    let mut response = agent.get(url).call().map_err(|error| {
        GraphStoreError::new("fetch_failed", format!("url fetch failed: {error}"))
    })?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take((limit as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            GraphStoreError::new("fetch_read_failed", format!("url body read failed: {error}"))
        })?;
    if bytes.len() > limit {
        return Err(GraphStoreError::new(
            "fetch_too_large",
            format!("fetched artifact exceeds the {limit} byte limit"),
        ));
    }
    if bytes.is_empty() {
        return Err(GraphStoreError::new(
            "fetch_empty",
            "fetched artifact body was empty",
        ));
    }
    Ok(bytes)
}

/// SSRF guard for the agent-facing fetch path: reject URLs whose host is
/// loopback, private, link-local, carrier-grade-NAT, or otherwise internal,
/// before any connection is made. Hostnames are resolved so a public name that
/// points at an internal address (including the cloud metadata endpoint, which
/// is link-local) is also rejected.
fn guard_url_host(url: &str) -> GraphStoreResult<()> {
    let host = url_host(url)
        .ok_or_else(|| GraphStoreError::new("invalid_url_host", "url has no host"))?;
    let lowered = host.to_ascii_lowercase();
    if lowered == "localhost" || lowered.ends_with(".localhost") {
        return Err(GraphStoreError::new(
            "blocked_url_host",
            "refusing to fetch from localhost",
        ));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return reject_internal_ip(&host, ip);
    }
    let resolved = (host.as_str(), 0u16).to_socket_addrs().map_err(|error| {
        GraphStoreError::new(
            "url_host_unresolved",
            format!("could not resolve host {host}: {error}"),
        )
    })?;
    let mut any = false;
    for addr in resolved {
        any = true;
        reject_internal_ip(&host, addr.ip())?;
    }
    if !any {
        return Err(GraphStoreError::new(
            "url_host_unresolved",
            format!("host {host} resolved to no addresses"),
        ));
    }
    Ok(())
}

fn reject_internal_ip(host: &str, ip: IpAddr) -> GraphStoreResult<()> {
    if is_internal_ip(ip) {
        return Err(GraphStoreError::new(
            "blocked_url_host",
            format!("refusing to fetch host {host} resolving to internal address {ip}"),
        ));
    }
    Ok(())
}

fn is_internal_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.octets()[0] == 0
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                || v6
                    .to_ipv4_mapped()
                    .map(|v4| is_internal_ip(IpAddr::V4(v4)))
                    .unwrap_or(false)
        }
    }
}

/// Extract the host from an http(s) URL: strip scheme, path/query/fragment,
/// userinfo, and port, handling bracketed IPv6 literals.
fn url_host(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1)?;
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .filter(|authority| !authority.is_empty())?;
    let host_port = authority.rsplit('@').next()?;
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        rest.split(']').next()?.to_string()
    } else {
        host_port.split(':').next()?.to_string()
    };
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// Derive a stable artifact name from a URL's last path segment so fetched
/// artifacts get a meaningful identifier without a caller-supplied name.
fn default_artifact_name_from_url(url: &str) -> String {
    url.split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or("artifact.bin")
        .to_string()
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
    let bytes = input.resolve_bytes()?;
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
    let bytes = input.resolve_bytes()?;
    let output = run_reconstruction_pipeline(input.artifact_name, &bytes)?;
    write_pipeline_output_in_store_for_tenant(context.store, &output, context.tenant_id)?;
    Ok(json!(output.report))
}

fn lift_handler(context: PluginOperationContext<'_>, arguments: Value) -> GraphStoreResult<Value> {
    let input = BinaryBytesInput::from_value(arguments)?;
    let bytes = input.resolve_bytes()?;
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
    let bytes = input.resolve_bytes()?;
    let output = run_reconstruction_pipeline(input.artifact_name, &bytes)?;
    write_pipeline_output_in_store_for_tenant(context.store, &output, context.tenant_id)?;
    Ok(json!({
        "artifact_id": output.load.artifact.artifact_id,
        "function_signature_count": output.analysis.signatures.len(),
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
        .clamp(1, MAX_RECONSTRUCTION_INSTRUCTION_LIMIT) as usize;
    let nodes = if let Some(instruction_id) = instruction_id {
        find_reconstruction_instruction_for_tenant(
            context.store,
            context.tenant_id,
            &instruction_id,
        )
        .into_iter()
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
    find_reconstruction_instruction_for_tenant(context.store, context.tenant_id, &instruction.id)
        .ok_or_else(|| {
        GraphStoreError::new(
            "missing_instruction",
            format!(
                "instruction {} is not available for this tenant",
                instruction.id
            ),
        )
    })?;
    let receipt = validate_instruction(&instruction, observed);
    write_validation_receipt_in_store_for_tenant(context.store, &receipt, context.tenant_id)?;
    Ok(json!(receipt))
}

fn find_reconstruction_instruction_for_tenant<S: GraphStore>(
    store: &S,
    tenant_id: &str,
    instruction_id: &str,
) -> Option<NodeRecord> {
    let scoped_instruction_id = tenant_scoped_reconstruction_id(tenant_id, instruction_id);
    for candidate_id in [instruction_id, scoped_instruction_id.as_str()] {
        if let Some(node) = GraphStore::get_node_record(store, candidate_id) {
            if is_reconstruction_instruction_for_tenant(&node, tenant_id) {
                return Some(node);
            }
        }
    }
    GraphStore::query_nodes(
        store,
        NodeQuery::label(RECONSTRUCTION_INSTRUCTION_LABEL)
            .with_property("tenant_id", json!(tenant_id))
            .with_property("original_id", json!(instruction_id))
            .with_limit(1),
    )
    .into_iter()
    .next()
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
    bytes_hex: Option<String>,
    url: Option<String>,
    max_bytes: Option<usize>,
}

impl BinaryBytesInput {
    fn from_value(value: Value) -> GraphStoreResult<Self> {
        let bytes_hex = value
            .get("bytes_hex")
            .or_else(|| value.get("bytesHex"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let url = value
            .get("url")
            .or_else(|| value.get("source_url"))
            .or_else(|| value.get("sourceUrl"))
            .and_then(Value::as_str)
            .map(str::to_string);
        if bytes_hex.is_none() && url.is_none() {
            return Err(GraphStoreError::new(
                "missing_binary_source",
                "one of bytes_hex or url is required",
            ));
        }
        let artifact_name = value
            .get("artifact_name")
            .or_else(|| value.get("artifactName"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| match &url {
                Some(url) => default_artifact_name_from_url(url),
                None => "artifact.bin".to_string(),
            });
        let max_bytes = value
            .get("max_bytes")
            .or_else(|| value.get("maxBytes"))
            .and_then(Value::as_u64)
            .map(|value| value as usize);
        Ok(Self {
            artifact_name,
            bytes_hex,
            url,
            max_bytes,
        })
    }

    /// Resolve the artifact bytes from whichever source was supplied. Hex bytes
    /// win when both are present (a caller that already has the bytes never
    /// triggers a network fetch).
    fn resolve_bytes(&self) -> GraphStoreResult<Vec<u8>> {
        if let Some(bytes_hex) = &self.bytes_hex {
            return decode_hex(bytes_hex);
        }
        if let Some(url) = &self.url {
            return fetch_binary_bytes(url, self.max_bytes.unwrap_or(DEFAULT_MAX_FETCH_BYTES));
        }
        Err(GraphStoreError::new(
            "missing_binary_source",
            "one of bytes_hex or url is required",
        ))
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
        let stored_instruction_id =
            tenant_scoped_reconstruction_id("Travis-Gilbert", "recon:instr:test");
        store
            .upsert_node(NodeRecord::new(
                &stored_instruction_id,
                [RECONSTRUCTION_INSTRUCTION_LABEL],
                json!({
                    "tenant_id": "Travis-Gilbert",
                    "original_id": "recon:instr:test",
                    "source_artifact": "sha256:test"
                }),
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
    fn validate_rejects_instruction_from_another_tenant() {
        let instruction = ReconstructionInstruction {
            id: "recon:instr:other".to_string(),
            source_artifact: "sha256:other".to_string(),
            target: rustyred_thg_reconstruct::ReconstructionTarget {
                kind: "component".to_string(),
                id: "component:other".to_string(),
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
                "recon:instr:other",
                [RECONSTRUCTION_INSTRUCTION_LABEL],
                json!({
                    "tenant_id": "Other-Tenant",
                    "source_artifact": "sha256:other"
                }),
            ))
            .unwrap();

        let error = registry
            .execute(
                &mut store,
                "Travis-Gilbert",
                "reconstruct.validate",
                json!({"instruction": instruction, "observed": "pong"}),
            )
            .unwrap_err();
        assert_eq!(error.code, "missing_instruction");
    }

    #[test]
    fn instruction_get_filters_by_tenant_and_id_before_limit() {
        let mut registry = PluginRegistry::new();
        registry.register(ReconstructionHarnessPlugin);
        let mut store = RedCoreGraphStore::memory();
        let other_instruction_id =
            tenant_scoped_reconstruction_id("Other-Tenant", "recon:instr:other");
        let target_instruction_id =
            tenant_scoped_reconstruction_id("Travis-Gilbert", "recon:instr:target");
        store
            .upsert_node(NodeRecord::new(
                &other_instruction_id,
                [RECONSTRUCTION_INSTRUCTION_LABEL],
                json!({
                    "tenant_id": "Other-Tenant",
                    "original_id": "recon:instr:other",
                    "source_artifact": "sha256:other"
                }),
            ))
            .unwrap();
        store
            .upsert_node(NodeRecord::new(
                &target_instruction_id,
                [RECONSTRUCTION_INSTRUCTION_LABEL],
                json!({
                    "tenant_id": "Travis-Gilbert",
                    "original_id": "recon:instr:target",
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
            json!(&target_instruction_id)
        );

        let tenant_page = registry
            .execute(
                &mut store,
                "Travis-Gilbert",
                "reconstruct.instruction.get",
                json!({"limit": u64::MAX}),
            )
            .unwrap();
        let instructions = tenant_page.result["instructions"].as_array().unwrap();
        assert_eq!(instructions.len(), 1);
        assert_eq!(instructions[0]["id"], json!(&target_instruction_id));
    }

    #[test]
    fn fetch_rejects_non_http_scheme() {
        let error = fetch_binary_bytes("file:///etc/passwd", 1024).unwrap_err();
        assert_eq!(error.code, "invalid_url_scheme");
    }

    #[test]
    fn binary_input_requires_a_source() {
        let error = BinaryBytesInput::from_value(json!({})).unwrap_err();
        assert_eq!(error.code, "missing_binary_source");
    }

    #[test]
    fn binary_input_prefers_bytes_hex_over_url() {
        // Both supplied: hex wins, so resolve_bytes never touches the network.
        let input = BinaryBytesInput::from_value(json!({
            "bytes_hex": "90c3",
            "url": "http://127.0.0.1:1/never"
        }))
        .unwrap();
        assert_eq!(input.resolve_bytes().unwrap(), vec![0x90, 0xc3]);
    }

    #[test]
    fn binary_input_derives_name_from_url() {
        let input =
            BinaryBytesInput::from_value(json!({"url": "https://host/dl/app.elf?v=2"})).unwrap();
        assert_eq!(input.artifact_name, "app.elf");
    }

    #[test]
    fn fetch_reads_exact_bytes_over_http() {
        let body: Vec<u8> = (0u8..200).collect();
        let (url, handle) = serve_once(body.clone());
        // Loopback is blocked by the SSRF guard on the public path, so the
        // real-HTTP positive test uses the private-host policy bypass.
        let fetched = fetch_binary_bytes_with_policy(&url, 1 << 20, true).unwrap();
        handle.join().ok();
        assert_eq!(fetched, body);
    }

    #[test]
    fn fetch_rejects_oversized_body() {
        let (url, handle) = serve_once(vec![0xabu8; 4096]);
        let error = fetch_binary_bytes_with_policy(&url, 1024, true).unwrap_err();
        handle.join().ok();
        assert_eq!(error.code, "fetch_too_large");
    }

    #[test]
    fn pipeline_from_url_matches_direct_bytes() {
        // Arbitrary (non-object) bytes: both paths must reach load_binary and
        // fail identically, proving the URL front door fetches then pipes.
        let body = b"theorem-not-an-object-file".to_vec();
        let (url, handle) = serve_once(body.clone());
        let fetched = fetch_binary_bytes_with_policy(&url, 1 << 20, true).unwrap();
        handle.join().ok();
        let via_url = run_reconstruction_pipeline("artifact.bin", &fetched);
        let via_bytes = run_reconstruction_pipeline("artifact.bin", &body);
        assert_eq!(result_code(&via_url), result_code(&via_bytes));
    }

    #[test]
    fn public_fetch_rejects_localhost_name() {
        let error = fetch_binary_bytes("http://localhost/x", 1024).unwrap_err();
        assert_eq!(error.code, "blocked_url_host");
    }

    #[test]
    fn public_fetch_rejects_internal_hosts() {
        // Loopback, RFC1918, IPv6 loopback, and the cloud metadata endpoint
        // (link-local) must all be rejected before any connection is made.
        for url in [
            "http://127.0.0.1:9/x",
            "http://10.0.0.5/x",
            "http://192.168.1.1/x",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/x",
        ] {
            assert_eq!(
                fetch_binary_bytes(url, 1024).unwrap_err().code,
                "blocked_url_host",
                "expected {url} to be blocked"
            );
        }
    }

    #[test]
    fn pipeline_from_url_inherits_ssrf_guard() {
        let error = run_reconstruction_pipeline_from_url(None, "http://127.0.0.1:9/x", 1 << 20)
            .unwrap_err();
        assert_eq!(error.code, "blocked_url_host");
    }

    fn result_code(result: &GraphStoreResult<ReconstructionPipelineOutput>) -> String {
        match result {
            Ok(output) => format!("ok:{}", output.report.artifact_id),
            Err(error) => format!("err:{}", error.code),
        }
    }

    fn serve_once(body: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
        use std::io::Write;
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut scratch = [0u8; 1024];
                let _ = stream.read(&mut scratch);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        (format!("http://127.0.0.1:{port}/artifact.bin"), handle)
    }
}
