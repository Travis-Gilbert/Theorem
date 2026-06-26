//! Minimal THIR lifting for binary reconstruction.

use std::collections::BTreeSet;

use rustyred_thg_binformat::{BinaryLoadReport, BinarySymbol};
use rustyred_thg_core::{stable_hash, EdgeRecord, GraphStore, GraphStoreResult, NodeRecord};
use rustyred_thg_disasm::{DisassemblyReport, InstructionFact};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const THIR_FUNCTION_LABEL: &str = "ThirFunction";
pub const THIR_BLOCK_LABEL: &str = "ThirBasicBlock";
pub const THIR_STATEMENT_LABEL: &str = "ThirStmt";

pub const FUNCTION_HAS_BLOCK: &str = "FUNCTION_HAS_BLOCK";
pub const BLOCK_HAS_STATEMENT: &str = "BLOCK_HAS_STATEMENT";
pub const STATEMENT_FROM_INSTRUCTION: &str = "STATEMENT_FROM_INSTRUCTION";

pub const LIFT_SOURCE: &str = "rustyred-thg-lift";
pub const LIFT_VERSION: &str = "rustyred-thg-lift-v0";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ThirProgram {
    pub artifact_id: String,
    pub functions: Vec<ThirFunction>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ThirFunction {
    pub function_id: String,
    pub artifact_id: String,
    pub address: u64,
    pub name: Option<String>,
    pub confidence: f64,
    pub blocks: Vec<ThirBasicBlock>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ThirBasicBlock {
    pub block_id: String,
    pub function_id: String,
    pub address: u64,
    pub statements: Vec<ThirStmt>,
    pub successors: Vec<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ThirStmt {
    Assign {
        stmt_id: String,
        instruction_id: String,
        address: u64,
        text: String,
    },
    Call {
        stmt_id: String,
        instruction_id: String,
        address: u64,
        target: Option<u64>,
        text: String,
    },
    Branch {
        stmt_id: String,
        instruction_id: String,
        address: u64,
        condition: Option<String>,
        target: Option<u64>,
        fallthrough: Option<u64>,
        text: String,
    },
    Return {
        stmt_id: String,
        instruction_id: String,
        address: u64,
        text: String,
    },
    Raw {
        stmt_id: String,
        instruction_id: String,
        address: u64,
        text: String,
    },
}

impl ThirStmt {
    pub fn stmt_id(&self) -> &str {
        match self {
            ThirStmt::Assign { stmt_id, .. }
            | ThirStmt::Call { stmt_id, .. }
            | ThirStmt::Branch { stmt_id, .. }
            | ThirStmt::Return { stmt_id, .. }
            | ThirStmt::Raw { stmt_id, .. } => stmt_id,
        }
    }

    pub fn instruction_id(&self) -> &str {
        match self {
            ThirStmt::Assign { instruction_id, .. }
            | ThirStmt::Call { instruction_id, .. }
            | ThirStmt::Branch { instruction_id, .. }
            | ThirStmt::Return { instruction_id, .. }
            | ThirStmt::Raw { instruction_id, .. } => instruction_id,
        }
    }

    pub fn address(&self) -> u64 {
        match self {
            ThirStmt::Assign { address, .. }
            | ThirStmt::Call { address, .. }
            | ThirStmt::Branch { address, .. }
            | ThirStmt::Return { address, .. }
            | ThirStmt::Raw { address, .. } => *address,
        }
    }
}

pub fn lift_to_thir(load: &BinaryLoadReport, disasm: &DisassemblyReport) -> ThirProgram {
    let mut instructions = disasm.instructions.clone();
    instructions.sort_by_key(|instruction| instruction.address);
    let entries = function_entries(load, &instructions);
    let functions = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let next = entries.get(index + 1).map(|entry| entry.address);
            let body = instructions
                .iter()
                .filter(|instruction| {
                    instruction.address >= entry.address
                        && next.map(|end| instruction.address < end).unwrap_or(true)
                })
                .cloned()
                .collect::<Vec<_>>();
            (!body.is_empty()).then(|| lift_function(&load.artifact.artifact_id, entry, body))
        })
        .collect();
    ThirProgram {
        artifact_id: load.artifact.artifact_id.clone(),
        functions,
    }
}

pub fn write_thir_in_store<S: GraphStore>(
    store: &mut S,
    program: &ThirProgram,
) -> GraphStoreResult<()> {
    for function in &program.functions {
        store.upsert_node(function_node(function))?;
        for block in &function.blocks {
            store.upsert_node(block_node(block, function))?;
            store.upsert_edge(EdgeRecord::new(
                edge_id(&function.function_id, FUNCTION_HAS_BLOCK, &block.block_id),
                &function.function_id,
                FUNCTION_HAS_BLOCK,
                &block.block_id,
                json!({"authority": "derived_fact", "source": LIFT_SOURCE, "version": LIFT_VERSION}),
            ))?;
            for statement in &block.statements {
                store.upsert_node(statement_node(statement, block, function))?;
                store.upsert_edge(EdgeRecord::new(
                    edge_id(&block.block_id, BLOCK_HAS_STATEMENT, statement.stmt_id()),
                    &block.block_id,
                    BLOCK_HAS_STATEMENT,
                    statement.stmt_id(),
                    json!({"authority": "derived_fact", "source": LIFT_SOURCE, "version": LIFT_VERSION}),
                ))?;
                store.upsert_edge(EdgeRecord::new(
                    edge_id(statement.stmt_id(), STATEMENT_FROM_INSTRUCTION, statement.instruction_id()),
                    statement.stmt_id(),
                    STATEMENT_FROM_INSTRUCTION,
                    statement.instruction_id(),
                    json!({"authority": "derived_fact", "source": LIFT_SOURCE, "version": LIFT_VERSION}),
                ))?;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct FunctionEntry {
    address: u64,
    name: Option<String>,
    confidence: f64,
}

fn function_entries(
    load: &BinaryLoadReport,
    instructions: &[InstructionFact],
) -> Vec<FunctionEntry> {
    let mut seen = BTreeSet::new();
    let mut entries = load
        .symbols
        .iter()
        .filter(|symbol| symbol.address > 0 && is_function_symbol(symbol))
        .filter_map(|symbol| {
            seen.insert(symbol.address).then(|| FunctionEntry {
                address: symbol.address,
                name: (!symbol.name.is_empty()).then(|| symbol.name.clone()),
                confidence: 0.86,
            })
        })
        .collect::<Vec<_>>();
    for entrypoint in &load.entrypoints {
        if seen.insert(entrypoint.address) {
            entries.push(FunctionEntry {
                address: entrypoint.address,
                name: Some(entrypoint.kind.clone()),
                confidence: 0.72,
            });
        }
    }
    if entries.is_empty() {
        if let Some(first) = instructions.first() {
            entries.push(FunctionEntry {
                address: first.address,
                name: Some("entry".to_string()),
                confidence: 0.4,
            });
        }
    }
    entries.sort_by_key(|entry| entry.address);
    entries
}

fn is_function_symbol(symbol: &BinarySymbol) -> bool {
    symbol.kind == "Text" || symbol.kind == "Label" || symbol.name.starts_with("_start")
}

fn lift_function(
    artifact_id: &str,
    entry: &FunctionEntry,
    instructions: Vec<InstructionFact>,
) -> ThirFunction {
    let function_id = format!("thir:function:{artifact_id}:{:x}", entry.address);
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    let mut block_start = instructions
        .first()
        .map(|instruction| instruction.address)
        .unwrap_or(entry.address);
    for instruction in instructions {
        if current.is_empty() {
            block_start = instruction.address;
        }
        let terminal = is_terminal(&instruction);
        current.push(instruction);
        if terminal {
            blocks.push(lift_block(
                &function_id,
                block_start,
                std::mem::take(&mut current),
            ));
        }
    }
    if !current.is_empty() {
        blocks.push(lift_block(&function_id, block_start, current));
    }
    ThirFunction {
        function_id,
        artifact_id: artifact_id.to_string(),
        address: entry.address,
        name: entry.name.clone(),
        confidence: entry.confidence,
        blocks,
    }
}

fn lift_block(
    function_id: &str,
    address: u64,
    instructions: Vec<InstructionFact>,
) -> ThirBasicBlock {
    let mut successors = BTreeSet::new();
    let statements = instructions
        .iter()
        .map(|instruction| {
            if let Some(target) = instruction.branch_target {
                successors.insert(target);
            }
            if instruction.flow_control == "ConditionalBranch" {
                successors.insert(instruction.address + instruction.size as u64);
            }
            lift_statement(instruction)
        })
        .collect();
    ThirBasicBlock {
        block_id: format!("thir:block:{function_id}:{address:x}"),
        function_id: function_id.to_string(),
        address,
        statements,
        successors: successors.into_iter().collect(),
    }
}

fn lift_statement(instruction: &InstructionFact) -> ThirStmt {
    let stmt_id = format!(
        "thir:stmt:{}",
        stable_hash(json!([
            instruction.instruction_id,
            instruction.address,
            instruction.text
        ]))
    );
    if instruction.effects.iter().any(|effect| effect == "calls") {
        ThirStmt::Call {
            stmt_id,
            instruction_id: instruction.instruction_id.clone(),
            address: instruction.address,
            target: instruction.branch_target,
            text: instruction.text.clone(),
        }
    } else if instruction
        .effects
        .iter()
        .any(|effect| effect == "branches")
    {
        ThirStmt::Branch {
            stmt_id,
            instruction_id: instruction.instruction_id.clone(),
            address: instruction.address,
            condition: (instruction.flow_control == "ConditionalBranch")
                .then(|| instruction.mnemonic.clone()),
            target: instruction.branch_target,
            fallthrough: Some(instruction.address + instruction.size as u64),
            text: instruction.text.clone(),
        }
    } else if instruction.effects.iter().any(|effect| effect == "returns") {
        ThirStmt::Return {
            stmt_id,
            instruction_id: instruction.instruction_id.clone(),
            address: instruction.address,
            text: instruction.text.clone(),
        }
    } else if instruction.effects.iter().any(|effect| effect == "assigns") {
        ThirStmt::Assign {
            stmt_id,
            instruction_id: instruction.instruction_id.clone(),
            address: instruction.address,
            text: instruction.text.clone(),
        }
    } else {
        ThirStmt::Raw {
            stmt_id,
            instruction_id: instruction.instruction_id.clone(),
            address: instruction.address,
            text: instruction.text.clone(),
        }
    }
}

fn is_terminal(instruction: &InstructionFact) -> bool {
    instruction
        .effects
        .iter()
        .any(|effect| effect == "branches" || effect == "returns")
}

fn function_node(function: &ThirFunction) -> NodeRecord {
    NodeRecord::new(
        &function.function_id,
        [THIR_FUNCTION_LABEL],
        json!({
            "artifact_id": function.artifact_id,
            "address": function.address,
            "name": function.name,
            "confidence": function.confidence,
            "block_count": function.blocks.len(),
            "authority": "derived_fact",
            "source": LIFT_SOURCE,
            "version": LIFT_VERSION,
        }),
    )
}

fn block_node(block: &ThirBasicBlock, function: &ThirFunction) -> NodeRecord {
    NodeRecord::new(
        &block.block_id,
        [THIR_BLOCK_LABEL],
        json!({
            "artifact_id": function.artifact_id,
            "function_id": block.function_id,
            "address": block.address,
            "statement_count": block.statements.len(),
            "successors": block.successors,
            "authority": "derived_fact",
            "source": LIFT_SOURCE,
            "version": LIFT_VERSION,
        }),
    )
}

fn statement_node(
    statement: &ThirStmt,
    block: &ThirBasicBlock,
    function: &ThirFunction,
) -> NodeRecord {
    NodeRecord::new(
        statement.stmt_id(),
        [THIR_STATEMENT_LABEL],
        json!({
            "artifact_id": function.artifact_id,
            "function_id": function.function_id,
            "block_id": block.block_id,
            "address": statement.address(),
            "statement": statement,
            "authority": "derived_fact",
            "source": LIFT_SOURCE,
            "version": LIFT_VERSION,
        }),
    )
}

fn edge_id(from: &str, edge_type: &str, to: &str) -> String {
    format!("thir:edge:{}", stable_hash(json!([from, edge_type, to])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_binformat::{
        write_binary_facts_in_store, BinaryArtifact, BinaryEntrypoint, BinaryLoadReport,
        BinarySection,
    };
    use rustyred_thg_core::{InMemoryGraphStore, NodeQuery};
    use rustyred_thg_disasm::{decode_instructions, write_instruction_facts_in_store};

    fn fixture_load_report() -> BinaryLoadReport {
        BinaryLoadReport {
            artifact: BinaryArtifact {
                artifact_id: "sha256:test".to_string(),
                sha256: "test".to_string(),
                name: "fixture".to_string(),
                format: "Elf".to_string(),
                arch: "X86_64".to_string(),
                endian: "Little".to_string(),
                entrypoint: 0x1000,
                byte_len: 3,
            },
            sections: vec![BinarySection {
                section_id: "section:text".to_string(),
                artifact_id: "sha256:test".to_string(),
                index: 0,
                name: ".text".to_string(),
                address: 0x1000,
                size: 3,
                kind: "Text".to_string(),
                executable: true,
                bytes: vec![0x90, 0x90, 0xc3],
            }],
            symbols: Vec::new(),
            strings: Vec::new(),
            relocations: Vec::new(),
            entrypoints: vec![BinaryEntrypoint {
                entrypoint_id: "entry".to_string(),
                artifact_id: "sha256:test".to_string(),
                address: 0x1000,
                kind: "entry".to_string(),
            }],
        }
    }

    fn fixture_load_report_with_bytes(bytes: Vec<u8>) -> BinaryLoadReport {
        let mut load = fixture_load_report();
        load.artifact.byte_len = bytes.len();
        load.sections[0].size = bytes.len() as u64;
        load.sections[0].bytes = bytes;
        load
    }

    #[test]
    fn lifts_instruction_facts_to_thir() {
        let load = fixture_load_report();
        let disasm = decode_instructions(&load).unwrap();
        let program = lift_to_thir(&load, &disasm);
        assert_eq!(program.functions.len(), 1);
        assert_eq!(program.functions[0].blocks.len(), 1);
        assert!(matches!(
            program.functions[0].blocks[0].statements.last().unwrap(),
            ThirStmt::Return { .. }
        ));
    }

    #[test]
    fn writes_thir_nodes() {
        let load = fixture_load_report();
        let disasm = decode_instructions(&load).unwrap();
        let program = lift_to_thir(&load, &disasm);
        let mut store = InMemoryGraphStore::new();
        write_binary_facts_in_store(&mut store, &load).unwrap();
        write_instruction_facts_in_store(&mut store, &disasm).unwrap();
        write_thir_in_store(&mut store, &program).unwrap();
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(THIR_FUNCTION_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(THIR_STATEMENT_LABEL))
                .len(),
            3
        );
    }

    #[test]
    fn conditional_branch_block_records_target_and_fallthrough_successors() {
        let load = fixture_load_report_with_bytes(vec![0x74, 0x01, 0x90, 0xc3]);
        let disasm = decode_instructions(&load).unwrap();
        let program = lift_to_thir(&load, &disasm);
        let successors = &program.functions[0].blocks[0].successors;

        assert!(successors.contains(&0x1002));
        assert!(successors.contains(&0x1003));
    }
}
