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
pub const PCODE_FACT_LABEL: &str = "PcodeFact";

pub const FUNCTION_HAS_BLOCK: &str = "FUNCTION_HAS_BLOCK";
pub const BLOCK_HAS_STATEMENT: &str = "BLOCK_HAS_STATEMENT";
pub const STATEMENT_FROM_INSTRUCTION: &str = "STATEMENT_FROM_INSTRUCTION";
pub const STATEMENT_HAS_PCODE: &str = "STATEMENT_HAS_PCODE";

pub const LIFT_SOURCE: &str = "rustyred-thg-lift";
pub const LIFT_VERSION: &str = "rustyred-thg-lift-v0";
pub const PCODE_VERSION: &str = "ghidra-pcode-aligned-v0";
pub const GHIDRA_PCODE_MAX: u8 = 75;

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

#[repr(u8)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PcodeOpcode {
    Unimplemented = 0,
    Copy = 1,
    Load = 2,
    Store = 3,
    Branch = 4,
    Cbranch = 5,
    Branchind = 6,
    Call = 7,
    Callind = 8,
    Callother = 9,
    Return = 10,
    IntEqual = 11,
    IntNotequal = 12,
    IntSless = 13,
    IntSlessequal = 14,
    IntLess = 15,
    IntLessequal = 16,
    IntZext = 17,
    IntSext = 18,
    IntAdd = 19,
    IntSub = 20,
    IntCarry = 21,
    IntScarry = 22,
    IntSborrow = 23,
    Int2comp = 24,
    IntNegate = 25,
    IntXor = 26,
    IntAnd = 27,
    IntOr = 28,
    IntLeft = 29,
    IntRight = 30,
    IntSright = 31,
    IntMult = 32,
    IntDiv = 33,
    IntSdiv = 34,
    IntRem = 35,
    IntSrem = 36,
    BoolNegate = 37,
    BoolXor = 38,
    BoolAnd = 39,
    BoolOr = 40,
    FloatEqual = 41,
    FloatNotequal = 42,
    FloatLess = 43,
    FloatLessequal = 44,
    FloatNan = 46,
    FloatAdd = 47,
    FloatDiv = 48,
    FloatMult = 49,
    FloatSub = 50,
    FloatNeg = 51,
    FloatAbs = 52,
    FloatSqrt = 53,
    FloatInt2float = 54,
    FloatFloat2float = 55,
    FloatTrunc = 56,
    FloatCeil = 57,
    FloatFloor = 58,
    FloatRound = 59,
    Multiequal = 60,
    Indirect = 61,
    Piece = 62,
    Subpiece = 63,
    Cast = 64,
    Ptradd = 65,
    Ptrsub = 66,
    Segmentop = 67,
    Cpoolref = 68,
    New = 69,
    Insert = 70,
    Zpull = 71,
    Popcount = 72,
    Lzcount = 73,
    Spull = 74,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PcodeOpcodeCategory {
    Placeholder,
    Copy,
    Memory,
    ControlFlow,
    IntegerComparison,
    IntegerExtension,
    IntegerArithmetic,
    IntegerLogical,
    IntegerShift,
    Boolean,
    FloatComparison,
    FloatArithmetic,
    FloatConversion,
    InternalDataFlow,
    TypePointer,
    SegmentConstantPool,
    Allocation,
    BitRange,
    BitCount,
}

impl PcodeOpcode {
    pub fn ghidra_opcode_id(self) -> u8 {
        self as u8
    }

    pub fn from_ghidra_opcode_id(opcode_id: u8) -> Option<Self> {
        match opcode_id {
            0 => Some(Self::Unimplemented),
            1 => Some(Self::Copy),
            2 => Some(Self::Load),
            3 => Some(Self::Store),
            4 => Some(Self::Branch),
            5 => Some(Self::Cbranch),
            6 => Some(Self::Branchind),
            7 => Some(Self::Call),
            8 => Some(Self::Callind),
            9 => Some(Self::Callother),
            10 => Some(Self::Return),
            11 => Some(Self::IntEqual),
            12 => Some(Self::IntNotequal),
            13 => Some(Self::IntSless),
            14 => Some(Self::IntSlessequal),
            15 => Some(Self::IntLess),
            16 => Some(Self::IntLessequal),
            17 => Some(Self::IntZext),
            18 => Some(Self::IntSext),
            19 => Some(Self::IntAdd),
            20 => Some(Self::IntSub),
            21 => Some(Self::IntCarry),
            22 => Some(Self::IntScarry),
            23 => Some(Self::IntSborrow),
            24 => Some(Self::Int2comp),
            25 => Some(Self::IntNegate),
            26 => Some(Self::IntXor),
            27 => Some(Self::IntAnd),
            28 => Some(Self::IntOr),
            29 => Some(Self::IntLeft),
            30 => Some(Self::IntRight),
            31 => Some(Self::IntSright),
            32 => Some(Self::IntMult),
            33 => Some(Self::IntDiv),
            34 => Some(Self::IntSdiv),
            35 => Some(Self::IntRem),
            36 => Some(Self::IntSrem),
            37 => Some(Self::BoolNegate),
            38 => Some(Self::BoolXor),
            39 => Some(Self::BoolAnd),
            40 => Some(Self::BoolOr),
            41 => Some(Self::FloatEqual),
            42 => Some(Self::FloatNotequal),
            43 => Some(Self::FloatLess),
            44 => Some(Self::FloatLessequal),
            46 => Some(Self::FloatNan),
            47 => Some(Self::FloatAdd),
            48 => Some(Self::FloatDiv),
            49 => Some(Self::FloatMult),
            50 => Some(Self::FloatSub),
            51 => Some(Self::FloatNeg),
            52 => Some(Self::FloatAbs),
            53 => Some(Self::FloatSqrt),
            54 => Some(Self::FloatInt2float),
            55 => Some(Self::FloatFloat2float),
            56 => Some(Self::FloatTrunc),
            57 => Some(Self::FloatCeil),
            58 => Some(Self::FloatFloor),
            59 => Some(Self::FloatRound),
            60 => Some(Self::Multiequal),
            61 => Some(Self::Indirect),
            62 => Some(Self::Piece),
            63 => Some(Self::Subpiece),
            64 => Some(Self::Cast),
            65 => Some(Self::Ptradd),
            66 => Some(Self::Ptrsub),
            67 => Some(Self::Segmentop),
            68 => Some(Self::Cpoolref),
            69 => Some(Self::New),
            70 => Some(Self::Insert),
            71 => Some(Self::Zpull),
            72 => Some(Self::Popcount),
            73 => Some(Self::Lzcount),
            74 => Some(Self::Spull),
            _ => None,
        }
    }

    pub fn ghidra_mnemonic(self) -> &'static str {
        match self {
            Self::Unimplemented => "UNIMPLEMENTED",
            Self::Copy => "COPY",
            Self::Load => "LOAD",
            Self::Store => "STORE",
            Self::Branch => "BRANCH",
            Self::Cbranch => "CBRANCH",
            Self::Branchind => "BRANCHIND",
            Self::Call => "CALL",
            Self::Callind => "CALLIND",
            Self::Callother => "CALLOTHER",
            Self::Return => "RETURN",
            Self::IntEqual => "INT_EQUAL",
            Self::IntNotequal => "INT_NOTEQUAL",
            Self::IntSless => "INT_SLESS",
            Self::IntSlessequal => "INT_SLESSEQUAL",
            Self::IntLess => "INT_LESS",
            Self::IntLessequal => "INT_LESSEQUAL",
            Self::IntZext => "INT_ZEXT",
            Self::IntSext => "INT_SEXT",
            Self::IntAdd => "INT_ADD",
            Self::IntSub => "INT_SUB",
            Self::IntCarry => "INT_CARRY",
            Self::IntScarry => "INT_SCARRY",
            Self::IntSborrow => "INT_SBORROW",
            Self::Int2comp => "INT_2COMP",
            Self::IntNegate => "INT_NEGATE",
            Self::IntXor => "INT_XOR",
            Self::IntAnd => "INT_AND",
            Self::IntOr => "INT_OR",
            Self::IntLeft => "INT_LEFT",
            Self::IntRight => "INT_RIGHT",
            Self::IntSright => "INT_SRIGHT",
            Self::IntMult => "INT_MULT",
            Self::IntDiv => "INT_DIV",
            Self::IntSdiv => "INT_SDIV",
            Self::IntRem => "INT_REM",
            Self::IntSrem => "INT_SREM",
            Self::BoolNegate => "BOOL_NEGATE",
            Self::BoolXor => "BOOL_XOR",
            Self::BoolAnd => "BOOL_AND",
            Self::BoolOr => "BOOL_OR",
            Self::FloatEqual => "FLOAT_EQUAL",
            Self::FloatNotequal => "FLOAT_NOTEQUAL",
            Self::FloatLess => "FLOAT_LESS",
            Self::FloatLessequal => "FLOAT_LESSEQUAL",
            Self::FloatNan => "FLOAT_NAN",
            Self::FloatAdd => "FLOAT_ADD",
            Self::FloatDiv => "FLOAT_DIV",
            Self::FloatMult => "FLOAT_MULT",
            Self::FloatSub => "FLOAT_SUB",
            Self::FloatNeg => "FLOAT_NEG",
            Self::FloatAbs => "FLOAT_ABS",
            Self::FloatSqrt => "FLOAT_SQRT",
            Self::FloatInt2float => "INT2FLOAT",
            Self::FloatFloat2float => "FLOAT2FLOAT",
            Self::FloatTrunc => "TRUNC",
            Self::FloatCeil => "CEIL",
            Self::FloatFloor => "FLOOR",
            Self::FloatRound => "ROUND",
            Self::Multiequal => "MULTIEQUAL",
            Self::Indirect => "INDIRECT",
            Self::Piece => "PIECE",
            Self::Subpiece => "SUBPIECE",
            Self::Cast => "CAST",
            Self::Ptradd => "PTRADD",
            Self::Ptrsub => "PTRSUB",
            Self::Segmentop => "SEGMENTOP",
            Self::Cpoolref => "CPOOLREF",
            Self::New => "NEW",
            Self::Insert => "INSERT",
            Self::Zpull => "ZPULL",
            Self::Popcount => "POPCOUNT",
            Self::Lzcount => "LZCOUNT",
            Self::Spull => "SPULL",
        }
    }

    pub fn from_ghidra_mnemonic(mnemonic: &str) -> Option<Self> {
        match mnemonic.trim().to_ascii_uppercase().as_str() {
            "UNIMPLEMENTED" => Some(Self::Unimplemented),
            "COPY" => Some(Self::Copy),
            "LOAD" => Some(Self::Load),
            "STORE" => Some(Self::Store),
            "BRANCH" => Some(Self::Branch),
            "CBRANCH" => Some(Self::Cbranch),
            "BRANCHIND" => Some(Self::Branchind),
            "CALL" => Some(Self::Call),
            "CALLIND" => Some(Self::Callind),
            "CALLOTHER" => Some(Self::Callother),
            "RETURN" => Some(Self::Return),
            "INT_EQUAL" => Some(Self::IntEqual),
            "INT_NOTEQUAL" => Some(Self::IntNotequal),
            "INT_SLESS" => Some(Self::IntSless),
            "INT_SLESSEQUAL" => Some(Self::IntSlessequal),
            "INT_LESS" => Some(Self::IntLess),
            "INT_LESSEQUAL" => Some(Self::IntLessequal),
            "INT_ZEXT" => Some(Self::IntZext),
            "INT_SEXT" => Some(Self::IntSext),
            "INT_ADD" => Some(Self::IntAdd),
            "INT_SUB" => Some(Self::IntSub),
            "INT_CARRY" => Some(Self::IntCarry),
            "INT_SCARRY" => Some(Self::IntScarry),
            "INT_SBORROW" => Some(Self::IntSborrow),
            "INT_2COMP" => Some(Self::Int2comp),
            "INT_NEGATE" => Some(Self::IntNegate),
            "INT_XOR" => Some(Self::IntXor),
            "INT_AND" => Some(Self::IntAnd),
            "INT_OR" => Some(Self::IntOr),
            "INT_LEFT" => Some(Self::IntLeft),
            "INT_RIGHT" => Some(Self::IntRight),
            "INT_SRIGHT" => Some(Self::IntSright),
            "INT_MULT" => Some(Self::IntMult),
            "INT_DIV" => Some(Self::IntDiv),
            "INT_SDIV" => Some(Self::IntSdiv),
            "INT_REM" => Some(Self::IntRem),
            "INT_SREM" => Some(Self::IntSrem),
            "BOOL_NEGATE" => Some(Self::BoolNegate),
            "BOOL_XOR" => Some(Self::BoolXor),
            "BOOL_AND" => Some(Self::BoolAnd),
            "BOOL_OR" => Some(Self::BoolOr),
            "FLOAT_EQUAL" => Some(Self::FloatEqual),
            "FLOAT_NOTEQUAL" => Some(Self::FloatNotequal),
            "FLOAT_LESS" => Some(Self::FloatLess),
            "FLOAT_LESSEQUAL" => Some(Self::FloatLessequal),
            "FLOAT_NAN" => Some(Self::FloatNan),
            "FLOAT_ADD" => Some(Self::FloatAdd),
            "FLOAT_DIV" => Some(Self::FloatDiv),
            "FLOAT_MULT" => Some(Self::FloatMult),
            "FLOAT_SUB" => Some(Self::FloatSub),
            "FLOAT_NEG" => Some(Self::FloatNeg),
            "FLOAT_ABS" => Some(Self::FloatAbs),
            "FLOAT_SQRT" => Some(Self::FloatSqrt),
            "INT2FLOAT" | "FLOAT_INT2FLOAT" => Some(Self::FloatInt2float),
            "FLOAT2FLOAT" | "FLOAT_FLOAT2FLOAT" => Some(Self::FloatFloat2float),
            "TRUNC" | "FLOAT_TRUNC" => Some(Self::FloatTrunc),
            "CEIL" | "FLOAT_CEIL" => Some(Self::FloatCeil),
            "FLOOR" | "FLOAT_FLOOR" => Some(Self::FloatFloor),
            "ROUND" | "FLOAT_ROUND" => Some(Self::FloatRound),
            "MULTIEQUAL" | "BUILD" => Some(Self::Multiequal),
            "INDIRECT" | "DELAY_SLOT" => Some(Self::Indirect),
            "PIECE" => Some(Self::Piece),
            "SUBPIECE" => Some(Self::Subpiece),
            "CAST" => Some(Self::Cast),
            "PTRADD" | "LABEL" => Some(Self::Ptradd),
            "PTRSUB" | "CROSSBUILD" => Some(Self::Ptrsub),
            "SEGMENTOP" => Some(Self::Segmentop),
            "CPOOLREF" => Some(Self::Cpoolref),
            "NEW" => Some(Self::New),
            "INSERT" => Some(Self::Insert),
            "ZPULL" => Some(Self::Zpull),
            "POPCOUNT" => Some(Self::Popcount),
            "LZCOUNT" => Some(Self::Lzcount),
            "SPULL" => Some(Self::Spull),
            _ => None,
        }
    }

    pub fn category(self) -> PcodeOpcodeCategory {
        match self {
            Self::Unimplemented => PcodeOpcodeCategory::Placeholder,
            Self::Copy => PcodeOpcodeCategory::Copy,
            Self::Load | Self::Store => PcodeOpcodeCategory::Memory,
            Self::Branch
            | Self::Cbranch
            | Self::Branchind
            | Self::Call
            | Self::Callind
            | Self::Callother
            | Self::Return => PcodeOpcodeCategory::ControlFlow,
            Self::IntEqual
            | Self::IntNotequal
            | Self::IntSless
            | Self::IntSlessequal
            | Self::IntLess
            | Self::IntLessequal => PcodeOpcodeCategory::IntegerComparison,
            Self::IntZext | Self::IntSext => PcodeOpcodeCategory::IntegerExtension,
            Self::IntAdd
            | Self::IntSub
            | Self::IntCarry
            | Self::IntScarry
            | Self::IntSborrow
            | Self::Int2comp
            | Self::IntMult
            | Self::IntDiv
            | Self::IntSdiv
            | Self::IntRem
            | Self::IntSrem => PcodeOpcodeCategory::IntegerArithmetic,
            Self::IntNegate | Self::IntXor | Self::IntAnd | Self::IntOr => {
                PcodeOpcodeCategory::IntegerLogical
            }
            Self::IntLeft | Self::IntRight | Self::IntSright => PcodeOpcodeCategory::IntegerShift,
            Self::BoolNegate | Self::BoolXor | Self::BoolAnd | Self::BoolOr => {
                PcodeOpcodeCategory::Boolean
            }
            Self::FloatEqual
            | Self::FloatNotequal
            | Self::FloatLess
            | Self::FloatLessequal
            | Self::FloatNan => PcodeOpcodeCategory::FloatComparison,
            Self::FloatAdd
            | Self::FloatDiv
            | Self::FloatMult
            | Self::FloatSub
            | Self::FloatNeg
            | Self::FloatAbs
            | Self::FloatSqrt => PcodeOpcodeCategory::FloatArithmetic,
            Self::FloatInt2float
            | Self::FloatFloat2float
            | Self::FloatTrunc
            | Self::FloatCeil
            | Self::FloatFloor
            | Self::FloatRound => PcodeOpcodeCategory::FloatConversion,
            Self::Multiequal | Self::Indirect | Self::Piece | Self::Subpiece => {
                PcodeOpcodeCategory::InternalDataFlow
            }
            Self::Cast | Self::Ptradd | Self::Ptrsub => PcodeOpcodeCategory::TypePointer,
            Self::Segmentop | Self::Cpoolref => PcodeOpcodeCategory::SegmentConstantPool,
            Self::New => PcodeOpcodeCategory::Allocation,
            Self::Insert | Self::Zpull | Self::Spull => PcodeOpcodeCategory::BitRange,
            Self::Popcount | Self::Lzcount => PcodeOpcodeCategory::BitCount,
        }
    }

    pub fn is_commutative(self) -> bool {
        matches!(
            self,
            Self::IntEqual
                | Self::IntNotequal
                | Self::IntAdd
                | Self::IntXor
                | Self::IntAnd
                | Self::IntOr
                | Self::IntMult
                | Self::BoolXor
                | Self::BoolAnd
                | Self::BoolOr
                | Self::FloatEqual
                | Self::FloatNotequal
                | Self::FloatAdd
                | Self::FloatMult
                | Self::IntCarry
                | Self::IntScarry
        )
    }

    pub fn is_control_flow(self) -> bool {
        self.category() == PcodeOpcodeCategory::ControlFlow
    }

    pub fn is_memory_access(self) -> bool {
        self.category() == PcodeOpcodeCategory::Memory
    }

    pub fn is_internal(self) -> bool {
        self.category() == PcodeOpcodeCategory::InternalDataFlow
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PcodeOperand {
    Address { value: u64 },
    Condition { value: String },
    Text { value: String },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PcodeFact {
    pub pcode_id: String,
    pub artifact_id: String,
    pub function_id: String,
    pub block_id: String,
    pub statement_id: String,
    pub instruction_id: String,
    pub address: u64,
    pub sequence: usize,
    pub opcode: PcodeOpcode,
    pub ghidra_opcode_id: u8,
    pub inputs: Vec<PcodeOperand>,
    pub output: Option<PcodeOperand>,
    pub source_text: String,
    pub authority: String,
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

pub fn lower_thir_to_pcode(program: &ThirProgram) -> Vec<PcodeFact> {
    program
        .functions
        .iter()
        .flat_map(|function| {
            function.blocks.iter().flat_map(move |block| {
                block
                    .statements
                    .iter()
                    .enumerate()
                    .map(move |(sequence, statement)| {
                        pcode_fact_for_statement(function, block, statement, sequence)
                    })
            })
        })
        .collect()
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
            for (sequence, statement) in block.statements.iter().enumerate() {
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
                let pcode = pcode_fact_for_statement(function, block, statement, sequence);
                store.upsert_node(pcode_node(&pcode))?;
                store.upsert_edge(EdgeRecord::new(
                    edge_id(statement.stmt_id(), STATEMENT_HAS_PCODE, &pcode.pcode_id),
                    statement.stmt_id(),
                    STATEMENT_HAS_PCODE,
                    &pcode.pcode_id,
                    json!({"authority": "derived_fact", "source": LIFT_SOURCE, "version": LIFT_VERSION, "pcode_version": PCODE_VERSION}),
                ))?;
            }
        }
    }
    Ok(())
}

pub fn write_pcode_facts_in_store<S: GraphStore>(
    store: &mut S,
    facts: &[PcodeFact],
) -> GraphStoreResult<()> {
    for fact in facts {
        store.upsert_node(pcode_node(fact))?;
        store.upsert_edge(EdgeRecord::new(
            edge_id(&fact.statement_id, STATEMENT_HAS_PCODE, &fact.pcode_id),
            &fact.statement_id,
            STATEMENT_HAS_PCODE,
            &fact.pcode_id,
            json!({"authority": &fact.authority, "source": LIFT_SOURCE, "version": LIFT_VERSION, "pcode_version": PCODE_VERSION}),
        ))?;
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
        .filter(|symbol| seen.insert(symbol.address))
        .map(|symbol| FunctionEntry {
            address: symbol.address,
            name: (!symbol.name.is_empty()).then(|| symbol.name.clone()),
            confidence: 0.86,
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
            fallthrough: (instruction.flow_control == "ConditionalBranch")
                .then_some(instruction.address + instruction.size as u64),
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

fn pcode_fact_for_statement(
    function: &ThirFunction,
    block: &ThirBasicBlock,
    statement: &ThirStmt,
    sequence: usize,
) -> PcodeFact {
    let (opcode, inputs, output, source_text) = pcode_shape_for_statement(statement);
    let ghidra_opcode_id = opcode.ghidra_opcode_id();
    let pcode_id = format!(
        "pcode:fact:{}",
        stable_hash(json!([
            PCODE_VERSION,
            statement.stmt_id(),
            sequence,
            ghidra_opcode_id,
            &inputs,
            &output
        ]))
    );
    PcodeFact {
        pcode_id,
        artifact_id: function.artifact_id.clone(),
        function_id: function.function_id.clone(),
        block_id: block.block_id.clone(),
        statement_id: statement.stmt_id().to_string(),
        instruction_id: statement.instruction_id().to_string(),
        address: statement.address(),
        sequence,
        opcode,
        ghidra_opcode_id,
        inputs,
        output,
        source_text,
        authority: "derived_fact".to_string(),
    }
}

fn pcode_shape_for_statement(
    statement: &ThirStmt,
) -> (PcodeOpcode, Vec<PcodeOperand>, Option<PcodeOperand>, String) {
    match statement {
        ThirStmt::Assign { text, .. } => (
            PcodeOpcode::Copy,
            vec![PcodeOperand::Text {
                value: text.clone(),
            }],
            None,
            text.clone(),
        ),
        ThirStmt::Call { target, text, .. } => {
            let input = target
                .map(|value| PcodeOperand::Address { value })
                .unwrap_or_else(|| PcodeOperand::Text {
                    value: text.clone(),
                });
            (
                if target.is_some() {
                    PcodeOpcode::Call
                } else {
                    PcodeOpcode::Callind
                },
                vec![input],
                None,
                text.clone(),
            )
        }
        ThirStmt::Branch {
            condition,
            target,
            text,
            ..
        } => {
            // Match Ghidra's branch operand layout: the destination is input0 and
            // CBRANCH carries the condition as input1. Fallthrough is the implicit
            // next instruction, not a p-code operand, so it is not emitted.
            let opcode = if condition.is_some() {
                PcodeOpcode::Cbranch
            } else if target.is_some() {
                PcodeOpcode::Branch
            } else {
                PcodeOpcode::Branchind
            };
            let mut inputs = Vec::new();
            if let Some(target) = target {
                inputs.push(PcodeOperand::Address { value: *target });
            }
            if let Some(condition) = condition {
                inputs.push(PcodeOperand::Condition {
                    value: condition.clone(),
                });
            }
            if inputs.is_empty() {
                inputs.push(PcodeOperand::Text {
                    value: text.clone(),
                });
            }
            (opcode, inputs, None, text.clone())
        }
        ThirStmt::Return { text, .. } => (
            PcodeOpcode::Return,
            vec![PcodeOperand::Text {
                value: text.clone(),
            }],
            None,
            text.clone(),
        ),
        ThirStmt::Raw { text, .. } => raw_pcode_shape(text),
    }
}

fn raw_pcode_shape(text: &str) -> (PcodeOpcode, Vec<PcodeOperand>, Option<PcodeOperand>, String) {
    let opcode = pcode_opcode_from_text(text).unwrap_or(PcodeOpcode::Unimplemented);
    (
        opcode,
        vec![PcodeOperand::Text {
            value: text.to_string(),
        }],
        None,
        text.to_string(),
    )
}

fn pcode_opcode_from_text(text: &str) -> Option<PcodeOpcode> {
    let mnemonic = text
        .trim_start()
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect::<String>();
    PcodeOpcode::from_ghidra_mnemonic(&mnemonic)
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

fn pcode_node(fact: &PcodeFact) -> NodeRecord {
    NodeRecord::new(
        &fact.pcode_id,
        [PCODE_FACT_LABEL],
        json!({
            "artifact_id": fact.artifact_id,
            "function_id": fact.function_id,
            "block_id": fact.block_id,
            "statement_id": fact.statement_id,
            "instruction_id": fact.instruction_id,
            "address": fact.address,
            "sequence": fact.sequence,
            "opcode": fact.opcode,
            "ghidra_opcode_id": fact.ghidra_opcode_id,
            "ghidra_mnemonic": fact.opcode.ghidra_mnemonic(),
            "opcode_category": fact.opcode.category(),
            "is_commutative": fact.opcode.is_commutative(),
            "is_control_flow": fact.opcode.is_control_flow(),
            "is_memory_access": fact.opcode.is_memory_access(),
            "is_internal": fact.opcode.is_internal(),
            "inputs": fact.inputs,
            "output": fact.output,
            "source_text": fact.source_text,
            "authority": fact.authority,
            "source": LIFT_SOURCE,
            "version": LIFT_VERSION,
            "pcode_version": PCODE_VERSION,
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
            language_specs: Vec::new(),
        }
    }

    fn fixture_load_report_with_bytes(bytes: Vec<u8>) -> BinaryLoadReport {
        let mut load = fixture_load_report();
        load.artifact.byte_len = bytes.len();
        load.sections[0].size = bytes.len() as u64;
        load.sections[0].bytes = bytes;
        load
    }

    fn synthetic_program(statements: Vec<ThirStmt>) -> ThirProgram {
        ThirProgram {
            artifact_id: "sha256:test".to_string(),
            functions: vec![ThirFunction {
                function_id: "thir:function:test:1000".to_string(),
                artifact_id: "sha256:test".to_string(),
                address: 0x1000,
                name: Some("entry".to_string()),
                confidence: 0.8,
                blocks: vec![ThirBasicBlock {
                    block_id: "thir:block:test:1000".to_string(),
                    function_id: "thir:function:test:1000".to_string(),
                    address: 0x1000,
                    statements,
                    successors: vec![0x1010],
                }],
            }],
        }
    }

    fn call_statement(stmt_id: &str, target: Option<u64>) -> ThirStmt {
        ThirStmt::Call {
            stmt_id: stmt_id.to_string(),
            instruction_id: format!("{stmt_id}:inst"),
            address: 0x1000,
            target,
            text: "call rax".to_string(),
        }
    }

    fn branch_statement(stmt_id: &str, condition: Option<&str>, target: Option<u64>) -> ThirStmt {
        ThirStmt::Branch {
            stmt_id: stmt_id.to_string(),
            instruction_id: format!("{stmt_id}:inst"),
            address: 0x1001,
            condition: condition.map(str::to_string),
            target,
            fallthrough: condition.map(|_| 0x1003),
            text: "jcc 0x1010".to_string(),
        }
    }

    fn return_statement(stmt_id: &str) -> ThirStmt {
        ThirStmt::Return {
            stmt_id: stmt_id.to_string(),
            instruction_id: format!("{stmt_id}:inst"),
            address: 0x1002,
            text: "ret".to_string(),
        }
    }

    fn raw_statement(stmt_id: &str) -> ThirStmt {
        raw_statement_with_text(stmt_id, "db 0xff")
    }

    fn raw_statement_with_text(stmt_id: &str, text: &str) -> ThirStmt {
        ThirStmt::Raw {
            stmt_id: stmt_id.to_string(),
            instruction_id: format!("{stmt_id}:inst"),
            address: 0x1003,
            text: text.to_string(),
        }
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
        assert_eq!(
            store.query_nodes(NodeQuery::label(PCODE_FACT_LABEL)).len(),
            3
        );
    }

    #[test]
    fn pcode_opcode_table_matches_ghidra_mnemonics_and_aliases() {
        assert_eq!(GHIDRA_PCODE_MAX, 75);
        assert_eq!(
            PcodeOpcode::from_ghidra_opcode_id(54),
            Some(PcodeOpcode::FloatInt2float)
        );
        assert_eq!(PcodeOpcode::from_ghidra_opcode_id(45), None);
        assert_eq!(PcodeOpcode::FloatInt2float.ghidra_mnemonic(), "INT2FLOAT");
        assert_eq!(
            PcodeOpcode::from_ghidra_mnemonic("INT2FLOAT"),
            Some(PcodeOpcode::FloatInt2float)
        );
        assert_eq!(
            PcodeOpcode::from_ghidra_mnemonic("BUILD"),
            Some(PcodeOpcode::Multiequal)
        );
        assert_eq!(
            PcodeOpcode::from_ghidra_mnemonic("DELAY_SLOT"),
            Some(PcodeOpcode::Indirect)
        );
        assert_eq!(
            PcodeOpcode::from_ghidra_mnemonic("LABEL"),
            Some(PcodeOpcode::Ptradd)
        );
        assert_eq!(
            PcodeOpcode::from_ghidra_mnemonic("CROSSBUILD"),
            Some(PcodeOpcode::Ptrsub)
        );
    }

    #[test]
    fn pcode_opcode_classification_matches_ghidra_reference_shape() {
        assert!(PcodeOpcode::IntAdd.is_commutative());
        assert!(PcodeOpcode::IntScarry.is_commutative());
        assert!(!PcodeOpcode::IntSub.is_commutative());
        assert_eq!(
            PcodeOpcode::Cbranch.category(),
            PcodeOpcodeCategory::ControlFlow
        );
        assert_eq!(PcodeOpcode::Load.category(), PcodeOpcodeCategory::Memory);
        assert_eq!(
            PcodeOpcode::IntLess.category(),
            PcodeOpcodeCategory::IntegerComparison
        );
        assert_eq!(
            PcodeOpcode::Multiequal.category(),
            PcodeOpcodeCategory::InternalDataFlow
        );
        assert!(PcodeOpcode::Call.is_control_flow());
        assert!(PcodeOpcode::Store.is_memory_access());
        assert!(PcodeOpcode::Indirect.is_internal());
    }

    #[test]
    fn lowers_thir_statements_to_ghidra_aligned_pcode_opcodes() {
        let program = synthetic_program(vec![
            call_statement("stmt:call-direct", Some(0x2000)),
            call_statement("stmt:call-indirect", None),
            branch_statement("stmt:branch-conditional", Some("zf == 1"), Some(0x1010)),
            branch_statement("stmt:branch-direct", None, Some(0x1010)),
            branch_statement("stmt:branch-indirect", None, None),
            return_statement("stmt:return"),
            raw_statement("stmt:raw"),
            raw_statement_with_text("stmt:callother", "CALLOTHER cpuid leaf"),
        ]);

        let facts = lower_thir_to_pcode(&program);
        let opcodes = facts.iter().map(|fact| fact.opcode).collect::<Vec<_>>();
        let opcode_ids = facts
            .iter()
            .map(|fact| fact.ghidra_opcode_id)
            .collect::<Vec<_>>();

        assert_eq!(
            opcodes,
            vec![
                PcodeOpcode::Call,
                PcodeOpcode::Callind,
                PcodeOpcode::Cbranch,
                PcodeOpcode::Branch,
                PcodeOpcode::Branchind,
                PcodeOpcode::Return,
                PcodeOpcode::Unimplemented,
                PcodeOpcode::Callother,
            ]
        );
        assert_eq!(opcode_ids, vec![7, 8, 5, 4, 6, 10, 0, 9]);
        // Ghidra CBRANCH order: destination input0, condition input1.
        assert!(matches!(
            facts[2].inputs.first(),
            Some(PcodeOperand::Address { value }) if *value == 0x1010
        ));
        assert!(matches!(
            facts[2].inputs.get(1),
            Some(PcodeOperand::Condition { value }) if value == "zf == 1"
        ));
        assert_eq!(facts[0].sequence, 0);
        assert_eq!(facts[6].sequence, 6);
        assert_eq!(facts[7].source_text, "CALLOTHER cpuid leaf");
    }

    #[test]
    fn writes_pcode_nodes_with_ghidra_opcode_metadata() {
        let program = synthetic_program(vec![branch_statement(
            "stmt:branch-conditional",
            Some("zf == 1"),
            Some(0x1010),
        )]);
        let facts = lower_thir_to_pcode(&program);
        let mut store = InMemoryGraphStore::new();
        store.upsert_node(pcode_node(&facts[0])).unwrap();

        let nodes = store.query_nodes(NodeQuery::label(PCODE_FACT_LABEL));
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].properties["ghidra_opcode_id"], json!(5));
        assert_eq!(nodes[0].properties["ghidra_mnemonic"], json!("CBRANCH"));
        assert_eq!(
            nodes[0].properties["opcode_category"],
            json!(PcodeOpcodeCategory::ControlFlow)
        );
        assert_eq!(nodes[0].properties["is_control_flow"], json!(true));
        assert_eq!(nodes[0].properties["is_memory_access"], json!(false));
        assert_eq!(nodes[0].properties["is_commutative"], json!(false));
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

    #[test]
    fn unconditional_branch_statement_has_no_fallthrough() {
        let load = fixture_load_report_with_bytes(vec![0xeb, 0x01, 0x90, 0xc3]);
        let disasm = decode_instructions(&load).unwrap();
        let program = lift_to_thir(&load, &disasm);
        let block = &program.functions[0].blocks[0];

        assert_eq!(block.successors, vec![0x1003]);
        assert!(matches!(
            block.statements.last().unwrap(),
            ThirStmt::Branch {
                fallthrough: None,
                ..
            }
        ));
    }
}
