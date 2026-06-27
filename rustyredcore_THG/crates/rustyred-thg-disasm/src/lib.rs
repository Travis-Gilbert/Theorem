//! Instruction fact decoding for Theorem reconstruction.

use iced_x86::{Decoder, DecoderOptions, FlowControl, Formatter, Instruction, NasmFormatter};
use rustyred_thg_binformat::{BinaryLoadReport, BinarySection};
use rustyred_thg_core::{
    stable_hash, EdgeRecord, GraphStore, GraphStoreError, GraphStoreResult, NodeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const INSTRUCTION_FACT_LABEL: &str = "InstructionFact";
pub const FUNCTION_START_CANDIDATE_LABEL: &str = "FunctionStartCandidate";
pub const SECTION_HAS_INSTRUCTION: &str = "SECTION_HAS_INSTRUCTION";
pub const ARTIFACT_HAS_INSTRUCTION: &str = "ARTIFACT_HAS_INSTRUCTION";
pub const SECTION_HAS_FUNCTION_START_CANDIDATE: &str = "SECTION_HAS_FUNCTION_START_CANDIDATE";
pub const ARTIFACT_HAS_FUNCTION_START_CANDIDATE: &str = "ARTIFACT_HAS_FUNCTION_START_CANDIDATE";
pub const DISASM_SOURCE: &str = "rustyred-thg-disasm";
pub const DISASM_VERSION: &str = "rustyred-thg-disasm-v0";
pub const FUNCTION_START_PATTERN_VERSION: &str = "ghidra-byte-pattern-function-start-v0";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InstructionFact {
    pub instruction_id: String,
    pub artifact_id: String,
    pub section_id: String,
    pub address: u64,
    pub size: u32,
    pub bytes: Vec<u8>,
    pub mnemonic: String,
    pub operands: String,
    pub text: String,
    pub flow_control: String,
    pub branch_target: Option<u64>,
    pub effects: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionStartCandidateAction {
    FunctionStart,
    PossibleFunctionStart,
    CodeBoundary,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FunctionStartCandidate {
    pub candidate_id: String,
    pub artifact_id: String,
    pub section_id: String,
    pub address: u64,
    pub offset: usize,
    pub action: FunctionStartCandidateAction,
    pub pattern_id: String,
    pub pattern_description: String,
    pub pattern_bytes: String,
    pub pattern_mask: String,
    pub constraints: Vec<String>,
    pub confidence: f64,
    pub evidence: Vec<String>,
    pub authority: String,
}

#[derive(Clone, Debug)]
struct FunctionStartPattern {
    pattern_id: &'static str,
    action: FunctionStartCandidateAction,
    bytes: &'static [u8],
    mask: &'static [u8],
    description: &'static str,
    constraints: &'static [&'static str],
    confidence: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DisassemblyReport {
    pub artifact_id: String,
    pub decoder: String,
    pub instructions: Vec<InstructionFact>,
    pub function_start_candidates: Vec<FunctionStartCandidate>,
}

pub fn decode_instructions(report: &BinaryLoadReport) -> GraphStoreResult<DisassemblyReport> {
    if !report.artifact.arch.contains("X86_64") && !report.artifact.arch.contains("X86-64") {
        return Err(GraphStoreError::new(
            "unsupported_binary_arch",
            format!(
                "rustyred-thg-disasm currently supports x86-64 only, got {}",
                report.artifact.arch
            ),
        ));
    }

    let mut instructions = Vec::new();
    let mut function_start_candidates = Vec::new();
    for section in report.sections.iter().filter(|section| section.executable) {
        instructions.extend(decode_section(&report.artifact.artifact_id, section)?);
        function_start_candidates.extend(scan_function_start_candidates_for_section(
            &report.artifact.artifact_id,
            section,
        ));
    }
    Ok(DisassemblyReport {
        artifact_id: report.artifact.artifact_id.clone(),
        decoder: "iced-x86".to_string(),
        instructions,
        function_start_candidates,
    })
}

pub fn scan_function_start_candidates(report: &BinaryLoadReport) -> Vec<FunctionStartCandidate> {
    let mut candidates = report
        .sections
        .iter()
        .filter(|section| section.executable)
        .flat_map(|section| {
            scan_function_start_candidates_for_section(&report.artifact.artifact_id, section)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.address
            .cmp(&right.address)
            .then_with(|| left.pattern_id.cmp(&right.pattern_id))
    });
    candidates.dedup_by(|left, right| {
        left.address == right.address && left.pattern_id == right.pattern_id
    });
    candidates
}

pub fn decode_section(
    artifact_id: &str,
    section: &BinarySection,
) -> GraphStoreResult<Vec<InstructionFact>> {
    let mut decoder = Decoder::with_ip(64, &section.bytes, section.address, DecoderOptions::NONE);
    let mut formatter = NasmFormatter::new();
    let mut facts = Vec::new();
    while decoder.can_decode() {
        let position_before = decoder.position();
        let ip_before = decoder.ip();
        let instruction = decoder.decode();
        if instruction.is_invalid() {
            if decoder.position() == position_before {
                decoder
                    .set_position(position_before.saturating_add(1))
                    .map_err(|error| {
                        GraphStoreError::new(
                            "invalid_instruction_skip_failed",
                            format!("failed to skip invalid instruction byte: {error}"),
                        )
                    })?;
                decoder.set_ip(ip_before.saturating_add(1));
            }
            continue;
        }
        let fact = instruction_fact(artifact_id, section, &instruction, &mut formatter)?;
        facts.push(fact);
    }
    Ok(facts)
}

fn scan_function_start_candidates_for_section(
    artifact_id: &str,
    section: &BinarySection,
) -> Vec<FunctionStartCandidate> {
    let patterns = ghidra_x86_64_function_start_patterns();
    let mut candidates = Vec::new();
    for offset in 0..section.bytes.len() {
        for pattern in &patterns {
            if !matches_pattern(&section.bytes, offset, pattern) {
                continue;
            }
            let address = section.address + offset as u64;
            let confidence = if offset == 0 {
                (pattern.confidence + 0.05).min(0.95)
            } else {
                pattern.confidence
            };
            candidates.push(FunctionStartCandidate {
                candidate_id: format!(
                    "funcstart:{}",
                    stable_hash(json!([
                        FUNCTION_START_PATTERN_VERSION,
                        artifact_id,
                        &section.section_id,
                        address,
                        pattern.pattern_id
                    ]))
                ),
                artifact_id: artifact_id.to_string(),
                section_id: section.section_id.clone(),
                address,
                offset,
                action: pattern.action.clone(),
                pattern_id: pattern.pattern_id.to_string(),
                pattern_description: pattern.description.to_string(),
                pattern_bytes: hex_bytes(pattern.bytes),
                pattern_mask: hex_bytes(pattern.mask),
                constraints: pattern
                    .constraints
                    .iter()
                    .map(|constraint| (*constraint).to_string())
                    .collect(),
                confidence,
                evidence: vec![
                    section.section_id.clone(),
                    format!("pattern:{}", pattern.pattern_id),
                ],
                authority: "candidate_evidence".to_string(),
            });
        }
    }
    candidates.sort_by(|left, right| {
        left.address
            .cmp(&right.address)
            .then_with(|| left.pattern_id.cmp(&right.pattern_id))
    });
    candidates
}

fn matches_pattern(bytes: &[u8], offset: usize, pattern: &FunctionStartPattern) -> bool {
    if pattern.bytes.len() != pattern.mask.len() {
        return false;
    }
    let Some(window) = bytes.get(offset..offset.saturating_add(pattern.bytes.len())) else {
        return false;
    };
    if window.len() != pattern.bytes.len() {
        return false;
    }
    window
        .iter()
        .zip(pattern.bytes.iter().zip(pattern.mask.iter()))
        .all(|(byte, (expected, mask))| (*byte & *mask) == (*expected & *mask))
}

fn ghidra_x86_64_function_start_patterns() -> Vec<FunctionStartPattern> {
    vec![
        FunctionStartPattern {
            pattern_id: "ghidra:x86-64gcc:funcstart:push-rbp-mov-rbp-rsp",
            action: FunctionStartCandidateAction::FunctionStart,
            bytes: &[0x55, 0x48, 0x89, 0xe5],
            mask: &[0xff, 0xff, 0xff, 0xff],
            description: "PUSH RBP; MOV RBP, RSP",
            constraints: &["after=defined_or_block_start"],
            confidence: 0.82,
        },
        FunctionStartPattern {
            pattern_id: "ghidra:x86-64gcc:funcstart:push-rbp-mov-ebp-esp",
            action: FunctionStartCandidateAction::FunctionStart,
            bytes: &[0x55, 0x89, 0xe5],
            mask: &[0xff, 0xff, 0xff],
            description: "PUSH RBP; MOV EBP, ESP",
            constraints: &["after=defined_or_block_start"],
            confidence: 0.78,
        },
        FunctionStartPattern {
            pattern_id: "ghidra:x86-64gcc:funcstart:sub-rsp-imm8",
            action: FunctionStartCandidateAction::FunctionStart,
            bytes: &[0x48, 0x83, 0xec, 0x00],
            mask: &[0xff, 0xff, 0xff, 0x00],
            description: "SUB RSP, imm8",
            constraints: &["after=defined_or_block_start", "validcode>=10"],
            confidence: 0.68,
        },
        FunctionStartPattern {
            pattern_id: "ghidra:x86-64gcc:funcstart:sub-rsp-imm32",
            action: FunctionStartCandidateAction::FunctionStart,
            bytes: &[0x48, 0x81, 0xec, 0x00, 0x00, 0x00, 0x00],
            mask: &[0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00],
            description: "SUB RSP, imm32",
            constraints: &["after=defined_or_block_start", "validcode>=10"],
            confidence: 0.68,
        },
        FunctionStartPattern {
            pattern_id: "ghidra:x86-64gcc:codeboundary:endbr64",
            action: FunctionStartCandidateAction::CodeBoundary,
            bytes: &[0xf3, 0x0f, 0x1e, 0xfa],
            mask: &[0xff, 0xff, 0xff, 0xff],
            description: "ENDBR64 code boundary",
            constraints: &["codeboundary", "may_be_exception_handler"],
            confidence: 0.56,
        },
    ]
}

pub fn write_instruction_facts_in_store<S: GraphStore>(
    store: &mut S,
    report: &DisassemblyReport,
) -> GraphStoreResult<()> {
    for instruction in &report.instructions {
        store.upsert_node(instruction_node(instruction))?;
        store.upsert_edge(EdgeRecord::new(
            edge_id(
                &instruction.section_id,
                SECTION_HAS_INSTRUCTION,
                &instruction.instruction_id,
            ),
            &instruction.section_id,
            SECTION_HAS_INSTRUCTION,
            &instruction.instruction_id,
            json!({
                "authority": "observed_fact",
                "source": DISASM_SOURCE,
                "version": DISASM_VERSION,
            }),
        ))?;
        store.upsert_edge(EdgeRecord::new(
            edge_id(
                &instruction.artifact_id,
                ARTIFACT_HAS_INSTRUCTION,
                &instruction.instruction_id,
            ),
            &instruction.artifact_id,
            ARTIFACT_HAS_INSTRUCTION,
            &instruction.instruction_id,
            json!({
                "authority": "observed_fact",
                "source": DISASM_SOURCE,
                "version": DISASM_VERSION,
            }),
        ))?;
    }
    for candidate in &report.function_start_candidates {
        store.upsert_node(function_start_candidate_node(candidate))?;
        store.upsert_edge(EdgeRecord::new(
            edge_id(
                &candidate.section_id,
                SECTION_HAS_FUNCTION_START_CANDIDATE,
                &candidate.candidate_id,
            ),
            &candidate.section_id,
            SECTION_HAS_FUNCTION_START_CANDIDATE,
            &candidate.candidate_id,
            json!({
                "authority": "candidate_evidence",
                "source": DISASM_SOURCE,
                "version": DISASM_VERSION,
                "pattern_version": FUNCTION_START_PATTERN_VERSION,
            }),
        ))?;
        store.upsert_edge(EdgeRecord::new(
            edge_id(
                &candidate.artifact_id,
                ARTIFACT_HAS_FUNCTION_START_CANDIDATE,
                &candidate.candidate_id,
            ),
            &candidate.artifact_id,
            ARTIFACT_HAS_FUNCTION_START_CANDIDATE,
            &candidate.candidate_id,
            json!({
                "authority": "candidate_evidence",
                "source": DISASM_SOURCE,
                "version": DISASM_VERSION,
                "pattern_version": FUNCTION_START_PATTERN_VERSION,
            }),
        ))?;
    }
    Ok(())
}

fn instruction_fact(
    artifact_id: &str,
    section: &BinarySection,
    instruction: &Instruction,
    formatter: &mut NasmFormatter,
) -> GraphStoreResult<InstructionFact> {
    let address = instruction.ip();
    let offset = address.checked_sub(section.address).ok_or_else(|| {
        GraphStoreError::new(
            "instruction_address_underflow",
            format!(
                "instruction {address:x} is before section {}",
                section.section_id
            ),
        )
    })? as usize;
    let size = instruction.len();
    let bytes = section
        .bytes
        .get(offset..offset.saturating_add(size))
        .unwrap_or_default()
        .to_vec();
    let mut text = String::new();
    formatter.format(instruction, &mut text);
    let mnemonic = format!("{:?}", instruction.mnemonic()).to_ascii_lowercase();
    let operands = text
        .strip_prefix(&mnemonic)
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let flow_control = format!("{:?}", instruction.flow_control());
    let branch_target = branch_target(instruction);
    let effects = effects_for(instruction);
    Ok(InstructionFact {
        instruction_id: format!(
            "instr:{}",
            stable_hash(json!([artifact_id, section.section_id, address, &bytes]))
        ),
        artifact_id: artifact_id.to_string(),
        section_id: section.section_id.clone(),
        address,
        size: size as u32,
        bytes,
        mnemonic,
        operands,
        text,
        flow_control,
        branch_target,
        effects,
    })
}

fn branch_target(instruction: &Instruction) -> Option<u64> {
    match instruction.flow_control() {
        FlowControl::UnconditionalBranch
        | FlowControl::ConditionalBranch
        | FlowControl::IndirectBranch
        | FlowControl::Call
        | FlowControl::IndirectCall => {
            let target = instruction.near_branch_target();
            (target != 0).then_some(target)
        }
        _ => None,
    }
}

fn effects_for(instruction: &Instruction) -> Vec<String> {
    match instruction.flow_control() {
        FlowControl::Call | FlowControl::IndirectCall => vec!["calls".to_string()],
        FlowControl::Return => vec!["returns".to_string()],
        FlowControl::UnconditionalBranch
        | FlowControl::ConditionalBranch
        | FlowControl::IndirectBranch => vec!["branches".to_string()],
        _ => {
            let mnemonic = format!("{:?}", instruction.mnemonic()).to_ascii_lowercase();
            if mnemonic.starts_with("mov") || mnemonic.starts_with("lea") {
                vec!["assigns".to_string()]
            } else if mnemonic.starts_with("cmp") || mnemonic.starts_with("test") {
                vec!["compares".to_string()]
            } else {
                Vec::new()
            }
        }
    }
}

fn instruction_node(instruction: &InstructionFact) -> NodeRecord {
    NodeRecord::new(
        &instruction.instruction_id,
        [INSTRUCTION_FACT_LABEL],
        json!({
            "artifact_id": instruction.artifact_id,
            "section_id": instruction.section_id,
            "address": instruction.address,
            "size": instruction.size,
            "bytes_hex": hex_bytes(&instruction.bytes),
            "mnemonic": instruction.mnemonic,
            "operands": instruction.operands,
            "text": instruction.text,
            "flow_control": instruction.flow_control,
            "branch_target": instruction.branch_target,
            "effects": instruction.effects,
            "authority": "observed_fact",
            "source": DISASM_SOURCE,
            "version": DISASM_VERSION,
        }),
    )
}

fn function_start_candidate_node(candidate: &FunctionStartCandidate) -> NodeRecord {
    NodeRecord::new(
        &candidate.candidate_id,
        [FUNCTION_START_CANDIDATE_LABEL],
        json!({
            "artifact_id": candidate.artifact_id,
            "section_id": candidate.section_id,
            "address": candidate.address,
            "offset": candidate.offset,
            "action": candidate.action,
            "pattern_id": candidate.pattern_id,
            "pattern_description": candidate.pattern_description,
            "pattern_bytes": candidate.pattern_bytes,
            "pattern_mask": candidate.pattern_mask,
            "constraints": candidate.constraints,
            "confidence": candidate.confidence,
            "evidence": candidate.evidence,
            "authority": candidate.authority,
            "source": DISASM_SOURCE,
            "version": DISASM_VERSION,
            "pattern_version": FUNCTION_START_PATTERN_VERSION,
        }),
    )
}

fn edge_id(from: &str, edge_type: &str, to: &str) -> String {
    format!("instr:edge:{}", stable_hash(json!([from, edge_type, to])))
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_binformat::{
        write_binary_facts_in_store, BinaryArtifact, BinaryLoadReport, BinarySection,
    };
    use rustyred_thg_core::{InMemoryGraphStore, NodeQuery};

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
            entrypoints: Vec::new(),
            language_specs: Vec::new(),
        }
    }

    #[test]
    fn decodes_x86_64_instruction_facts() {
        let report = decode_instructions(&fixture_load_report()).unwrap();
        assert_eq!(report.instructions.len(), 3);
        assert_eq!(report.instructions[0].mnemonic, "nop");
        assert_eq!(report.instructions[2].effects, vec!["returns"]);
    }

    #[test]
    fn skips_invalid_bytes_and_keeps_later_instructions() {
        let mut load = fixture_load_report();
        load.sections[0].bytes = vec![0x90, 0xf0, 0x90, 0xc3];
        load.sections[0].size = load.sections[0].bytes.len() as u64;

        let report = decode_instructions(&load).unwrap();

        assert_eq!(report.instructions.first().unwrap().mnemonic, "nop");
        assert_eq!(report.instructions.last().unwrap().mnemonic, "ret");
    }

    #[test]
    fn finds_ghidra_style_function_start_candidates() {
        let mut load = fixture_load_report();
        load.sections[0].bytes = vec![0x55, 0x48, 0x89, 0xe5, 0xc3];
        load.sections[0].size = load.sections[0].bytes.len() as u64;

        let candidates = scan_function_start_candidates(&load);

        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].action,
            FunctionStartCandidateAction::FunctionStart
        );
        assert_eq!(candidates[0].address, 0x1000);
        assert_eq!(candidates[0].pattern_bytes, "554889e5");
        assert!(candidates[0]
            .constraints
            .contains(&"after=defined_or_block_start".to_string()));
        assert_eq!(candidates[0].authority, "candidate_evidence");
    }

    #[test]
    fn records_endbr64_as_code_boundary_candidate() {
        let mut load = fixture_load_report();
        load.sections[0].bytes = vec![0xf3, 0x0f, 0x1e, 0xfa, 0xc3];
        load.sections[0].size = load.sections[0].bytes.len() as u64;

        let report = decode_instructions(&load).unwrap();

        assert!(report.function_start_candidates.iter().any(|candidate| {
            candidate.action == FunctionStartCandidateAction::CodeBoundary
                && candidate.pattern_id == "ghidra:x86-64gcc:codeboundary:endbr64"
                && candidate.constraints.contains(&"codeboundary".to_string())
        }));
    }

    #[test]
    fn writes_instruction_nodes() {
        let load = fixture_load_report();
        let report = decode_instructions(&load).unwrap();
        let mut store = InMemoryGraphStore::new();
        write_binary_facts_in_store(&mut store, &load).unwrap();
        write_instruction_facts_in_store(&mut store, &report).unwrap();
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(INSTRUCTION_FACT_LABEL))
                .len(),
            3
        );
    }

    #[test]
    fn writes_function_start_candidate_nodes() {
        let mut load = fixture_load_report();
        load.sections[0].bytes = vec![0x55, 0x48, 0x89, 0xe5, 0xc3];
        load.sections[0].size = load.sections[0].bytes.len() as u64;
        let report = decode_instructions(&load).unwrap();
        let mut store = InMemoryGraphStore::new();
        write_binary_facts_in_store(&mut store, &load).unwrap();
        write_instruction_facts_in_store(&mut store, &report).unwrap();

        let candidates = store.query_nodes(NodeQuery::label(FUNCTION_START_CANDIDATE_LABEL));
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].properties["authority"],
            json!("candidate_evidence")
        );
        assert_eq!(
            candidates[0].properties["pattern_version"],
            json!(FUNCTION_START_PATTERN_VERSION)
        );
    }
}
