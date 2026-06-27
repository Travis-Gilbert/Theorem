//! Binary artifact loading for the Theorem reconstruction pipeline.
//!
//! This crate owns observed binary facts only: artifact identity, file format,
//! architecture, sections, symbols, relocations when available, entrypoints,
//! and printable strings. Downstream crates decode, lift, infer, and compile
//! reconstruction obligations from these facts.

use object::{Object, ObjectSection, ObjectSymbol, Relocation, RelocationTarget};
use quick_xml::{
    events::{BytesStart, Event},
    name::QName,
    Reader,
};
use rustyred_thg_core::{
    stable_hash, EdgeRecord, GraphStore, GraphStoreError, GraphStoreResult, NodeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

pub const BINARY_ARTIFACT_LABEL: &str = "BinaryArtifact";
pub const BINARY_SECTION_LABEL: &str = "BinarySection";
pub const BINARY_SYMBOL_LABEL: &str = "BinarySymbol";
pub const BINARY_STRING_LABEL: &str = "BinaryString";
pub const BINARY_RELOCATION_LABEL: &str = "BinaryRelocation";
pub const BINARY_ENTRYPOINT_LABEL: &str = "BinaryEntrypoint";
pub const PROGRAM_GRAPH_LABEL: &str = "ProgramGraph";
pub const PROGRAM_MANAGER_LABEL: &str = "ProgramManager";
pub const PROGRAM_LANGUAGE_SPEC_LABEL: &str = "ProgramLanguageSpec";
pub const PROGRAM_COMPILER_SPEC_LABEL: &str = "ProgramCompilerSpec";
pub const PROGRAM_PROCESSOR_SPEC_LABEL: &str = "ProgramProcessorSpec";

pub const ARTIFACT_HAS_SECTION: &str = "ARTIFACT_HAS_SECTION";
pub const ARTIFACT_HAS_SYMBOL: &str = "ARTIFACT_HAS_SYMBOL";
pub const ARTIFACT_HAS_STRING: &str = "ARTIFACT_HAS_STRING";
pub const ARTIFACT_HAS_RELOCATION: &str = "ARTIFACT_HAS_RELOCATION";
pub const ARTIFACT_HAS_ENTRYPOINT: &str = "ARTIFACT_HAS_ENTRYPOINT";
pub const PROGRAM_FOR_ARTIFACT: &str = "PROGRAM_FOR_ARTIFACT";
pub const PROGRAM_HAS_MANAGER: &str = "PROGRAM_HAS_MANAGER";
pub const PROGRAM_HAS_LANGUAGE_SPEC: &str = "PROGRAM_HAS_LANGUAGE_SPEC";
pub const LANGUAGE_HAS_COMPILER_SPEC: &str = "LANGUAGE_HAS_COMPILER_SPEC";
pub const LANGUAGE_HAS_PROCESSOR_SPEC: &str = "LANGUAGE_HAS_PROCESSOR_SPEC";
pub const MANAGER_HAS_FACT: &str = "MANAGER_HAS_FACT";

pub const BINFORMAT_SOURCE: &str = "rustyred-thg-binformat";
pub const BINFORMAT_VERSION: &str = "rustyred-thg-binformat-v0";
pub const PROGRAM_GRAPH_VERSION: &str = "ghidra-program-graph-v0";
pub const PROGRAM_LANGUAGE_PACK_VERSION: &str = "ghidra-language-pack-v0";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BinaryArtifact {
    pub artifact_id: String,
    pub sha256: String,
    pub name: String,
    pub format: String,
    pub arch: String,
    pub endian: String,
    pub entrypoint: u64,
    pub byte_len: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BinarySection {
    pub section_id: String,
    pub artifact_id: String,
    pub index: usize,
    pub name: String,
    pub address: u64,
    pub size: u64,
    pub kind: String,
    pub executable: bool,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BinarySymbol {
    pub symbol_id: String,
    pub artifact_id: String,
    pub index: usize,
    pub name: String,
    pub address: u64,
    pub size: u64,
    pub kind: String,
    pub scope: String,
    pub is_definition: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BinaryString {
    pub string_id: String,
    pub artifact_id: String,
    pub offset: usize,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BinaryRelocation {
    pub relocation_id: String,
    pub artifact_id: String,
    pub section_index: usize,
    pub offset: u64,
    pub kind: String,
    pub target: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BinaryEntrypoint {
    pub entrypoint_id: String,
    pub artifact_id: String,
    pub address: u64,
    pub kind: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramManagerKind {
    Memory,
    Code,
    Symbol,
    Namespace,
    Function,
    External,
    Reference,
    DataType,
    Equate,
    Bookmark,
    Context,
    Property,
    Tree,
    Relocation,
    SourceFile,
    String,
    Entrypoint,
}

impl ProgramManagerKind {
    pub fn stable_name(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Code => "code",
            Self::Symbol => "symbol",
            Self::Namespace => "namespace",
            Self::Function => "function",
            Self::External => "external",
            Self::Reference => "reference",
            Self::DataType => "data_type",
            Self::Equate => "equate",
            Self::Bookmark => "bookmark",
            Self::Context => "context",
            Self::Property => "property",
            Self::Tree => "tree",
            Self::Relocation => "relocation",
            Self::SourceFile => "source_file",
            Self::String => "string",
            Self::Entrypoint => "entrypoint",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Memory => "Memory",
            Self::Code => "Code",
            Self::Symbol => "Symbol",
            Self::Namespace => "Namespace",
            Self::Function => "Function",
            Self::External => "External",
            Self::Reference => "Reference",
            Self::DataType => "DataType",
            Self::Equate => "Equate",
            Self::Bookmark => "Bookmark",
            Self::Context => "Context",
            Self::Property => "Property",
            Self::Tree => "Tree",
            Self::Relocation => "Relocation",
            Self::SourceFile => "SourceFile",
            Self::String => "String",
            Self::Entrypoint => "Entrypoint",
        }
    }

    pub fn ghidra_manager_order(self) -> Option<usize> {
        match self {
            Self::Memory => Some(0),
            Self::Code => Some(1),
            Self::Symbol => Some(2),
            Self::Namespace => Some(3),
            Self::Function => Some(4),
            Self::External => Some(5),
            Self::Reference => Some(6),
            Self::DataType => Some(7),
            Self::Equate => Some(8),
            Self::Bookmark => Some(9),
            Self::Context => Some(10),
            Self::Property => Some(11),
            Self::Tree => Some(12),
            Self::Relocation => Some(13),
            Self::SourceFile => Some(14),
            Self::String | Self::Entrypoint => None,
        }
    }

    fn all_program_managers() -> [Self; 17] {
        [
            Self::Memory,
            Self::Code,
            Self::Symbol,
            Self::Namespace,
            Self::Function,
            Self::External,
            Self::Reference,
            Self::DataType,
            Self::Equate,
            Self::Bookmark,
            Self::Context,
            Self::Property,
            Self::Tree,
            Self::Relocation,
            Self::SourceFile,
            Self::String,
            Self::Entrypoint,
        ]
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgramManager {
    pub manager_id: String,
    pub program_id: String,
    pub kind: ProgramManagerKind,
    pub display_name: String,
    pub ghidra_manager_order: Option<usize>,
    pub fact_count: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgramGraph {
    pub program_id: String,
    pub artifact_id: String,
    pub name: String,
    pub executable_format: String,
    pub architecture: String,
    pub endian: String,
    pub entrypoint: u64,
    pub byte_len: usize,
    pub managers: Vec<ProgramManager>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgramLanguageExternalName {
    pub tool: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgramProcessorProperty {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgramProcessorContextValue {
    pub space: String,
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgramProcessorRegisterGroup {
    pub group: String,
    pub exemplar_registers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgramProcessorSpec {
    pub processor_spec_id: String,
    pub language_spec_id: String,
    pub spec_file: String,
    pub program_counter_register: String,
    pub properties: Vec<ProgramProcessorProperty>,
    pub context_values: Vec<ProgramProcessorContextValue>,
    pub tracked_values: Vec<ProgramProcessorContextValue>,
    pub register_groups: Vec<ProgramProcessorRegisterGroup>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgramCompilerDataOrganization {
    pub pointer_size: u8,
    pub default_pointer_alignment: u8,
    pub machine_alignment: u8,
    pub default_alignment: u8,
    pub integer_size: u8,
    pub long_size: u8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgramCompilerInjectParameter {
    pub name: String,
    pub byte_len: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgramCompilerCallFixup {
    pub name: String,
    pub targets: Vec<String>,
    pub param_shift: i32,
    pub dynamic: bool,
    pub incidental_copy: bool,
    pub input_parameters: Vec<ProgramCompilerInjectParameter>,
    pub output_parameters: Vec<ProgramCompilerInjectParameter>,
    pub pcode_body: Option<String>,
    pub pcode_body_hash: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgramCompilerCallOtherFixup {
    pub targetop: String,
    pub param_shift: i32,
    pub dynamic: bool,
    pub incidental_copy: bool,
    pub input_parameters: Vec<ProgramCompilerInjectParameter>,
    pub output_parameters: Vec<ProgramCompilerInjectParameter>,
    pub pcode_body: Option<String>,
    pub pcode_body_hash: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgramCompilerSpec {
    pub compiler_spec_id: String,
    pub language_spec_id: String,
    pub compiler_id: String,
    pub name: String,
    pub spec_file: String,
    pub stack_pointer_register: Option<String>,
    pub stack_pointer_space: Option<String>,
    pub default_proto: Option<String>,
    pub prototype_names: Vec<String>,
    pub data_organization: Option<ProgramCompilerDataOrganization>,
    pub callfixups: Vec<ProgramCompilerCallFixup>,
    pub callotherfixups: Vec<ProgramCompilerCallOtherFixup>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgramLanguageSpec {
    pub language_spec_id: String,
    pub artifact_id: String,
    pub language_id: String,
    pub processor: String,
    pub endian: String,
    pub size_bits: u16,
    pub variant: String,
    pub version: String,
    pub sla_file: String,
    pub processor_spec_file: String,
    pub manual_index_file: Option<String>,
    pub description: String,
    pub external_names: Vec<ProgramLanguageExternalName>,
    pub compiler_specs: Vec<ProgramCompilerSpec>,
    pub processor_spec: ProgramProcessorSpec,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BinaryLoadReport {
    pub artifact: BinaryArtifact,
    pub sections: Vec<BinarySection>,
    pub symbols: Vec<BinarySymbol>,
    pub strings: Vec<BinaryString>,
    pub relocations: Vec<BinaryRelocation>,
    pub entrypoints: Vec<BinaryEntrypoint>,
    pub language_specs: Vec<ProgramLanguageSpec>,
}

pub fn load_binary(name: impl Into<String>, bytes: &[u8]) -> GraphStoreResult<BinaryLoadReport> {
    let name = name.into();
    let sha256 = sha256_hex(bytes);
    let artifact_id = format!("sha256:{sha256}");
    let parsed = object::File::parse(bytes).map_err(|error| {
        GraphStoreError::new(
            "binary_parse_failed",
            format!("failed to parse binary artifact {name}: {error}"),
        )
    })?;
    let artifact = BinaryArtifact {
        artifact_id: artifact_id.clone(),
        sha256,
        name,
        format: format!("{:?}", parsed.format()),
        arch: format!("{:?}", parsed.architecture()),
        endian: format!("{:?}", parsed.endianness()),
        entrypoint: parsed.entry(),
        byte_len: bytes.len(),
    };

    let mut sections = Vec::new();
    for (index, section) in parsed.sections().enumerate() {
        let name = section.name().unwrap_or("").to_string();
        let data = section.data().unwrap_or(&[]);
        let kind = format!("{:?}", section.kind());
        let executable = kind == "Text" || name == ".text" || name.contains("text");
        sections.push(BinarySection {
            section_id: format!("bin:section:{}:{index}", artifact.sha256),
            artifact_id: artifact_id.clone(),
            index,
            name,
            address: section.address(),
            size: section.size(),
            kind,
            executable,
            bytes: data.to_vec(),
        });
    }

    let mut symbols = Vec::new();
    for (index, symbol) in parsed.symbols().chain(parsed.dynamic_symbols()).enumerate() {
        let name = symbol.name().unwrap_or("").to_string();
        if name.is_empty() && symbol.address() == 0 && symbol.size() == 0 {
            continue;
        }
        symbols.push(BinarySymbol {
            symbol_id: format!("bin:symbol:{}:{index}", artifact.sha256),
            artifact_id: artifact_id.clone(),
            index,
            name,
            address: symbol.address(),
            size: symbol.size(),
            kind: format!("{:?}", symbol.kind()),
            scope: format!("{:?}", symbol.scope()),
            is_definition: symbol.is_definition(),
        });
    }

    let strings = extract_ascii_strings(bytes, 4)
        .into_iter()
        .map(|(offset, value)| BinaryString {
            string_id: format!(
                "bin:string:{}",
                stable_hash(json!([&artifact_id, offset, &value]))
            ),
            artifact_id: artifact_id.clone(),
            offset,
            value,
        })
        .collect::<Vec<_>>();
    let relocations = collect_relocations(&parsed, &artifact_id, &artifact.sha256);

    let entrypoints = if artifact.entrypoint > 0 {
        vec![BinaryEntrypoint {
            entrypoint_id: format!("bin:entry:{}:{:x}", artifact.sha256, artifact.entrypoint),
            artifact_id: artifact_id.clone(),
            address: artifact.entrypoint,
            kind: "object_entrypoint".to_string(),
        }]
    } else {
        symbols
            .iter()
            .find(|symbol| symbol.kind == "Text" || symbol.kind == "Label")
            .map(|symbol| BinaryEntrypoint {
                entrypoint_id: format!("bin:entry:{}:{:x}", artifact.sha256, symbol.address),
                artifact_id: artifact_id.clone(),
                address: symbol.address,
                kind: "symbol_entrypoint".to_string(),
            })
            .into_iter()
            .collect()
    };
    let language_specs = ghidra_language_specs_for_artifact(&artifact);

    Ok(BinaryLoadReport {
        artifact,
        sections,
        symbols,
        strings,
        relocations,
        entrypoints,
        language_specs,
    })
}

pub fn write_binary_facts_in_store<S: GraphStore>(
    store: &mut S,
    report: &BinaryLoadReport,
) -> GraphStoreResult<()> {
    let program = program_graph_for_load_report(report);
    store.upsert_node(artifact_node(&report.artifact))?;
    write_program_graph_in_store(store, &program, &report.artifact.artifact_id)?;
    for language_spec in &report.language_specs {
        write_language_spec_in_store(store, &program, language_spec)?;
    }
    for section in &report.sections {
        store.upsert_node(section_node(section))?;
        link_artifact(
            store,
            &report.artifact.artifact_id,
            &section.section_id,
            ARTIFACT_HAS_SECTION,
        )?;
        link_manager_fact(
            store,
            &program,
            ProgramManagerKind::Memory,
            &section.section_id,
        )?;
    }
    for symbol in &report.symbols {
        store.upsert_node(symbol_node(symbol))?;
        link_artifact(
            store,
            &report.artifact.artifact_id,
            &symbol.symbol_id,
            ARTIFACT_HAS_SYMBOL,
        )?;
        link_manager_fact(
            store,
            &program,
            ProgramManagerKind::Symbol,
            &symbol.symbol_id,
        )?;
    }
    for string in &report.strings {
        store.upsert_node(string_node(string))?;
        link_artifact(
            store,
            &report.artifact.artifact_id,
            &string.string_id,
            ARTIFACT_HAS_STRING,
        )?;
        link_manager_fact(
            store,
            &program,
            ProgramManagerKind::String,
            &string.string_id,
        )?;
    }
    for relocation in &report.relocations {
        store.upsert_node(relocation_node(relocation))?;
        link_artifact(
            store,
            &report.artifact.artifact_id,
            &relocation.relocation_id,
            ARTIFACT_HAS_RELOCATION,
        )?;
        link_manager_fact(
            store,
            &program,
            ProgramManagerKind::Relocation,
            &relocation.relocation_id,
        )?;
    }
    for entrypoint in &report.entrypoints {
        store.upsert_node(entrypoint_node(entrypoint))?;
        link_artifact(
            store,
            &report.artifact.artifact_id,
            &entrypoint.entrypoint_id,
            ARTIFACT_HAS_ENTRYPOINT,
        )?;
        link_manager_fact(
            store,
            &program,
            ProgramManagerKind::Entrypoint,
            &entrypoint.entrypoint_id,
        )?;
    }
    Ok(())
}

pub fn program_graph_for_load_report(report: &BinaryLoadReport) -> ProgramGraph {
    let program_id = program_id_for_artifact(&report.artifact.artifact_id);
    let managers = ProgramManagerKind::all_program_managers()
        .into_iter()
        .map(|kind| ProgramManager {
            manager_id: program_manager_id(&program_id, kind),
            program_id: program_id.clone(),
            kind,
            display_name: kind.display_name().to_string(),
            ghidra_manager_order: kind.ghidra_manager_order(),
            fact_count: manager_fact_count(report, kind),
        })
        .collect();
    ProgramGraph {
        program_id,
        artifact_id: report.artifact.artifact_id.clone(),
        name: report.artifact.name.clone(),
        executable_format: report.artifact.format.clone(),
        architecture: report.artifact.arch.clone(),
        endian: report.artifact.endian.clone(),
        entrypoint: report.artifact.entrypoint,
        byte_len: report.artifact.byte_len,
        managers,
    }
}

pub fn ghidra_language_specs_for_artifact(artifact: &BinaryArtifact) -> Vec<ProgramLanguageSpec> {
    let arch = artifact.arch.trim().to_ascii_lowercase();
    let endian = artifact.endian.trim().to_ascii_lowercase();
    if matches!(arch.as_str(), "x86_64" | "x86-64" | "amd64") && endian == "little" {
        return vec![ghidra_x86_64_default_language_spec(&artifact.artifact_id)];
    }
    Vec::new()
}

pub fn ghidra_language_specs_from_pack(
    artifact_id: &str,
    ldefs_xml: &str,
    processor_specs: &BTreeMap<String, String>,
    compiler_specs: &BTreeMap<String, String>,
) -> GraphStoreResult<Vec<ProgramLanguageSpec>> {
    let definitions = parse_ghidra_language_definitions(ldefs_xml)?;
    definitions
        .into_iter()
        .map(|definition| {
            let language_spec_id =
                language_spec_id_for_artifact(artifact_id, &definition.language_id);
            let processor_spec = parse_ghidra_processor_spec(
                &language_spec_id,
                &definition.processor_spec_file,
                processor_specs
                    .get(&definition.processor_spec_file)
                    .map(String::as_str),
            )?;
            let mut compiler_specs_out = Vec::new();
            for compiler in definition.compilers {
                let mut compiler_spec = language_compiler_spec(
                    &language_spec_id,
                    &compiler.compiler_id,
                    &compiler.name,
                    &compiler.spec_file,
                );
                if let Some(cspec_xml) = compiler_specs.get(&compiler.spec_file) {
                    apply_ghidra_compiler_spec(&mut compiler_spec, cspec_xml)?;
                }
                compiler_specs_out.push(compiler_spec);
            }
            Ok(ProgramLanguageSpec {
                language_spec_id: language_spec_id.clone(),
                artifact_id: artifact_id.to_string(),
                language_id: definition.language_id,
                processor: definition.processor,
                endian: definition.endian,
                size_bits: definition.size_bits,
                variant: definition.variant,
                version: definition.version,
                sla_file: definition.sla_file,
                processor_spec_file: definition.processor_spec_file,
                manual_index_file: definition.manual_index_file,
                description: definition.description,
                external_names: definition.external_names,
                compiler_specs: compiler_specs_out,
                processor_spec,
            })
        })
        .collect()
}

pub fn ghidra_language_specs_from_directory(
    artifact_id: &str,
    languages_dir: impl AsRef<Path>,
) -> GraphStoreResult<Vec<ProgramLanguageSpec>> {
    let languages_dir = languages_dir.as_ref();
    let mut ldefs_sources = Vec::<String>::new();
    let mut processor_specs = BTreeMap::<String, String>::new();
    let mut compiler_specs = BTreeMap::<String, String>::new();

    for entry in fs::read_dir(languages_dir).map_err(|error| {
        xml_parse_error(format!(
            "failed to read Ghidra language directory {}: {error}",
            languages_dir.display()
        ))
    })? {
        let entry = entry.map_err(xml_parse_error)?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        match extension {
            "ldefs" => ldefs_sources.push(read_language_pack_file(&path)?),
            "pspec" => {
                if let Some(file_name) = file_name_string(&path) {
                    processor_specs.insert(file_name, read_language_pack_file(&path)?);
                }
            }
            "cspec" => {
                if let Some(file_name) = file_name_string(&path) {
                    compiler_specs.insert(file_name, read_language_pack_file(&path)?);
                }
            }
            _ => {}
        }
    }

    ldefs_sources.sort();
    let mut language_specs = Vec::new();
    for ldefs_xml in ldefs_sources {
        language_specs.extend(ghidra_language_specs_from_pack(
            artifact_id,
            &ldefs_xml,
            &processor_specs,
            &compiler_specs,
        )?);
    }
    language_specs.sort_by(|left, right| left.language_id.cmp(&right.language_id));
    Ok(language_specs)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ParsedLanguageDefinition {
    processor: String,
    endian: String,
    size_bits: u16,
    variant: String,
    version: String,
    sla_file: String,
    processor_spec_file: String,
    manual_index_file: Option<String>,
    language_id: String,
    description: String,
    compilers: Vec<ParsedCompilerDefinition>,
    external_names: Vec<ProgramLanguageExternalName>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedCompilerDefinition {
    compiler_id: String,
    name: String,
    spec_file: String,
}

fn parse_ghidra_language_definitions(
    ldefs_xml: &str,
) -> GraphStoreResult<Vec<ParsedLanguageDefinition>> {
    let mut reader = Reader::from_str(ldefs_xml);
    reader.config_mut().trim_text(true);
    let mut languages = Vec::new();
    let mut current = None::<ParsedLanguageDefinition>;
    let mut inside_description = false;

    loop {
        match reader.read_event().map_err(xml_parse_error)? {
            Event::Start(start) if is_element(&start, b"language") => {
                current = Some(ParsedLanguageDefinition {
                    processor: required_attr(&start, b"processor")?,
                    endian: required_attr(&start, b"endian")?,
                    size_bits: parse_u16_attr(&start, b"size")?,
                    variant: required_attr(&start, b"variant")?,
                    version: required_attr(&start, b"version")?,
                    sla_file: required_attr(&start, b"slafile")?,
                    processor_spec_file: required_attr(&start, b"processorspec")?,
                    manual_index_file: optional_attr(&start, b"manualindexfile")?,
                    language_id: required_attr(&start, b"id")?,
                    ..ParsedLanguageDefinition::default()
                });
            }
            Event::Start(start) if is_element(&start, b"description") => {
                inside_description = true;
            }
            Event::Empty(empty) if is_element(&empty, b"compiler") => {
                if let Some(language) = current.as_mut() {
                    language.compilers.push(ParsedCompilerDefinition {
                        compiler_id: required_attr(&empty, b"id")?,
                        name: required_attr(&empty, b"name")?,
                        spec_file: required_attr(&empty, b"spec")?,
                    });
                }
            }
            Event::Empty(empty) if is_element(&empty, b"external_name") => {
                if let Some(language) = current.as_mut() {
                    language.external_names.push(ProgramLanguageExternalName {
                        tool: required_attr(&empty, b"tool")?,
                        name: required_attr(&empty, b"name")?,
                    });
                }
            }
            Event::Text(text) if inside_description => {
                if let Some(language) = current.as_mut() {
                    language
                        .description
                        .push_str(text.xml10_content().map_err(xml_parse_error)?.trim());
                }
            }
            Event::End(end) if end.name().as_ref() == b"description" => {
                inside_description = false;
            }
            Event::End(end) if end.name().as_ref() == b"language" => {
                if let Some(language) = current.take() {
                    languages.push(language);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(languages)
}

fn parse_ghidra_processor_spec(
    language_spec_id: &str,
    spec_file: &str,
    pspec_xml: Option<&str>,
) -> GraphStoreResult<ProgramProcessorSpec> {
    let mut spec = ProgramProcessorSpec {
        processor_spec_id: processor_spec_id(language_spec_id, spec_file),
        language_spec_id: language_spec_id.to_string(),
        spec_file: spec_file.to_string(),
        program_counter_register: String::new(),
        properties: Vec::new(),
        context_values: Vec::new(),
        tracked_values: Vec::new(),
        register_groups: Vec::new(),
    };
    let Some(pspec_xml) = pspec_xml else {
        return Ok(spec);
    };

    let mut reader = Reader::from_str(pspec_xml);
    reader.config_mut().trim_text(true);
    let mut context_space = None::<(bool, String)>;
    let mut register_groups = BTreeMap::<String, Vec<String>>::new();

    loop {
        match reader.read_event().map_err(xml_parse_error)? {
            Event::Empty(empty) if is_element(&empty, b"property") => {
                spec.properties.push(ProgramProcessorProperty {
                    key: required_attr(&empty, b"key")?,
                    value: required_attr(&empty, b"value")?,
                });
            }
            Event::Empty(empty) if is_element(&empty, b"programcounter") => {
                spec.program_counter_register = required_attr(&empty, b"register")?;
            }
            Event::Start(start) if is_element(&start, b"context_set") => {
                context_space = Some((false, required_attr(&start, b"space")?));
            }
            Event::Start(start) if is_element(&start, b"tracked_set") => {
                context_space = Some((true, required_attr(&start, b"space")?));
            }
            Event::Empty(empty) if is_element(&empty, b"set") => {
                if let Some((tracked, space)) = context_space.as_ref() {
                    let value = ProgramProcessorContextValue {
                        space: space.clone(),
                        name: required_attr(&empty, b"name")?,
                        value: required_attr(&empty, b"val")?,
                    };
                    if *tracked {
                        spec.tracked_values.push(value);
                    } else {
                        spec.context_values.push(value);
                    }
                }
            }
            Event::Empty(empty) if is_element(&empty, b"register") => {
                if let (Some(group), Some(name)) = (
                    optional_attr(&empty, b"group")?,
                    optional_attr(&empty, b"name")?,
                ) {
                    register_groups.entry(group).or_default().push(name);
                }
            }
            Event::End(end)
                if end.name().as_ref() == b"context_set"
                    || end.name().as_ref() == b"tracked_set" =>
            {
                context_space = None;
            }
            Event::Eof => break,
            _ => {}
        }
    }

    spec.register_groups = register_groups
        .into_iter()
        .map(
            |(group, exemplar_registers)| ProgramProcessorRegisterGroup {
                group,
                exemplar_registers,
            },
        )
        .collect();
    Ok(spec)
}

fn apply_ghidra_compiler_spec(
    compiler_spec: &mut ProgramCompilerSpec,
    cspec_xml: &str,
) -> GraphStoreResult<()> {
    let mut reader = Reader::from_str(cspec_xml);
    reader.config_mut().trim_text(true);
    let mut data_organization = BTreeMap::<String, u8>::new();
    let mut inside_data_organization = false;
    let mut inside_default_proto = false;
    let mut current_callfixup = None::<ProgramCompilerCallFixup>;
    let mut current_callotherfixup = None::<ProgramCompilerCallOtherFixup>;
    let mut capture_body = false;

    loop {
        match reader.read_event().map_err(xml_parse_error)? {
            Event::Start(start) if is_element(&start, b"data_organization") => {
                inside_data_organization = true;
            }
            Event::Empty(empty) if inside_data_organization => {
                if let Some(key) = data_organization_key(empty.name()) {
                    if let Some(value) = optional_attr(&empty, b"value")? {
                        data_organization.insert(
                            key.to_string(),
                            value.parse::<u8>().map_err(|error| {
                                xml_parse_error(format!(
                                    "invalid data organization value {value}: {error}"
                                ))
                            })?,
                        );
                    }
                }
            }
            Event::Empty(empty) if is_element(&empty, b"stackpointer") => {
                compiler_spec.stack_pointer_register = optional_attr(&empty, b"register")?;
                compiler_spec.stack_pointer_space = optional_attr(&empty, b"space")?;
            }
            Event::Start(start) if is_element(&start, b"default_proto") => {
                inside_default_proto = true;
            }
            Event::Start(start) if is_element(&start, b"prototype") => {
                record_compiler_prototype(compiler_spec, &start, inside_default_proto)?;
            }
            Event::Empty(empty) if is_element(&empty, b"prototype") => {
                record_compiler_prototype(compiler_spec, &empty, inside_default_proto)?;
            }
            Event::Start(start) if is_element(&start, b"callfixup") => {
                current_callfixup = Some(ProgramCompilerCallFixup {
                    name: required_attr(&start, b"name")?,
                    targets: Vec::new(),
                    param_shift: 0,
                    dynamic: false,
                    incidental_copy: false,
                    input_parameters: Vec::new(),
                    output_parameters: Vec::new(),
                    pcode_body: None,
                    pcode_body_hash: None,
                });
            }
            Event::Start(start) if is_element(&start, b"callotherfixup") => {
                current_callotherfixup = Some(ProgramCompilerCallOtherFixup {
                    targetop: required_attr(&start, b"targetop")?,
                    param_shift: 0,
                    dynamic: false,
                    incidental_copy: false,
                    input_parameters: Vec::new(),
                    output_parameters: Vec::new(),
                    pcode_body: None,
                    pcode_body_hash: None,
                });
            }
            Event::Start(start) if is_element(&start, b"pcode") => {
                if let Some(callfixup) = current_callfixup.as_mut() {
                    apply_pcode_inject_attributes(callfixup, &start)?;
                }
                if let Some(callotherfixup) = current_callotherfixup.as_mut() {
                    apply_callother_pcode_inject_attributes(callotherfixup, &start)?;
                }
            }
            Event::Empty(empty) if is_element(&empty, b"input") => {
                if let Some(callfixup) = current_callfixup.as_mut() {
                    callfixup
                        .input_parameters
                        .push(parse_pcode_inject_parameter(&empty)?);
                }
                if let Some(callotherfixup) = current_callotherfixup.as_mut() {
                    callotherfixup
                        .input_parameters
                        .push(parse_pcode_inject_parameter(&empty)?);
                }
            }
            Event::Empty(empty) if is_element(&empty, b"output") => {
                if let Some(callfixup) = current_callfixup.as_mut() {
                    callfixup
                        .output_parameters
                        .push(parse_pcode_inject_parameter(&empty)?);
                }
                if let Some(callotherfixup) = current_callotherfixup.as_mut() {
                    callotherfixup
                        .output_parameters
                        .push(parse_pcode_inject_parameter(&empty)?);
                }
            }
            Event::Empty(empty) if is_element(&empty, b"target") => {
                if let Some(callfixup) = current_callfixup.as_mut() {
                    if let Some(name) = optional_attr(&empty, b"name")? {
                        push_unique(&mut callfixup.targets, name);
                    }
                }
            }
            Event::Start(start) if is_element(&start, b"body") => {
                capture_body = true;
            }
            Event::Text(text) if capture_body => {
                if let Some(callfixup) = current_callfixup.as_mut() {
                    let body = text.xml10_content().map_err(xml_parse_error)?;
                    append_pcode_body(callfixup, &body);
                }
                if let Some(callotherfixup) = current_callotherfixup.as_mut() {
                    let body = text.xml10_content().map_err(xml_parse_error)?;
                    append_callother_pcode_body(callotherfixup, &body);
                }
            }
            Event::CData(cdata) if capture_body => {
                if let Some(callfixup) = current_callfixup.as_mut() {
                    let body = String::from_utf8_lossy(cdata.as_ref());
                    append_pcode_body(callfixup, &body);
                }
                if let Some(callotherfixup) = current_callotherfixup.as_mut() {
                    let body = String::from_utf8_lossy(cdata.as_ref());
                    append_callother_pcode_body(callotherfixup, &body);
                }
            }
            Event::End(end) if end.name().as_ref() == b"body" => {
                capture_body = false;
            }
            Event::End(end) if end.name().as_ref() == b"callfixup" => {
                if let Some(mut callfixup) = current_callfixup.take() {
                    if let Some(body) = callfixup.pcode_body.take() {
                        let normalized = normalize_pcode_body(&body);
                        callfixup.pcode_body_hash = Some(sha256_hex(normalized.as_bytes()));
                        callfixup.pcode_body = Some(normalized);
                    }
                    compiler_spec.callfixups.push(callfixup);
                }
            }
            Event::End(end) if end.name().as_ref() == b"callotherfixup" => {
                if let Some(mut callotherfixup) = current_callotherfixup.take() {
                    if let Some(body) = callotherfixup.pcode_body.take() {
                        let normalized = normalize_pcode_body(&body);
                        callotherfixup.pcode_body_hash = Some(sha256_hex(normalized.as_bytes()));
                        callotherfixup.pcode_body = Some(normalized);
                    }
                    compiler_spec.callotherfixups.push(callotherfixup);
                }
            }
            Event::End(end) if end.name().as_ref() == b"default_proto" => {
                inside_default_proto = false;
            }
            Event::End(end) if end.name().as_ref() == b"data_organization" => {
                inside_data_organization = false;
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if !data_organization.is_empty() {
        compiler_spec.data_organization = Some(ProgramCompilerDataOrganization {
            pointer_size: data_organization
                .get("pointer_size")
                .copied()
                .unwrap_or_default(),
            default_pointer_alignment: data_organization
                .get("default_pointer_alignment")
                .copied()
                .unwrap_or_default(),
            machine_alignment: data_organization
                .get("machine_alignment")
                .copied()
                .unwrap_or_default(),
            default_alignment: data_organization
                .get("default_alignment")
                .copied()
                .unwrap_or_default(),
            integer_size: data_organization
                .get("integer_size")
                .copied()
                .unwrap_or_default(),
            long_size: data_organization
                .get("long_size")
                .copied()
                .unwrap_or_default(),
        });
    }

    Ok(())
}

fn is_element(start: &BytesStart<'_>, name: &[u8]) -> bool {
    start.name().as_ref() == name
}

fn data_organization_key(name: QName<'_>) -> Option<&'static str> {
    match name.as_ref() {
        b"pointer_size" => Some("pointer_size"),
        b"default_pointer_alignment" => Some("default_pointer_alignment"),
        b"machine_alignment" => Some("machine_alignment"),
        b"default_alignment" => Some("default_alignment"),
        b"integer_size" => Some("integer_size"),
        b"long_size" => Some("long_size"),
        _ => None,
    }
}

fn record_compiler_prototype(
    compiler_spec: &mut ProgramCompilerSpec,
    start: &BytesStart<'_>,
    inside_default_proto: bool,
) -> GraphStoreResult<()> {
    if let Some(name) = optional_attr(start, b"name")? {
        push_unique(&mut compiler_spec.prototype_names, name.clone());
        if inside_default_proto && compiler_spec.default_proto.is_none() {
            compiler_spec.default_proto = Some(name);
        }
    }
    Ok(())
}

fn optional_attr(start: &BytesStart<'_>, name: &[u8]) -> GraphStoreResult<Option<String>> {
    for attr in start.attributes() {
        let attr = attr.map_err(xml_parse_error)?;
        if attr.key == QName(name) {
            return Ok(Some(
                attr.unescape_value().map_err(xml_parse_error)?.into_owned(),
            ));
        }
    }
    Ok(None)
}

fn required_attr(start: &BytesStart<'_>, name: &[u8]) -> GraphStoreResult<String> {
    optional_attr(start, name)?.ok_or_else(|| {
        xml_parse_error(format!(
            "missing required attribute {} on <{}>",
            String::from_utf8_lossy(name),
            String::from_utf8_lossy(start.name().as_ref())
        ))
    })
}

fn parse_u16_attr(start: &BytesStart<'_>, name: &[u8]) -> GraphStoreResult<u16> {
    let value = required_attr(start, name)?;
    value.parse::<u16>().map_err(|error| {
        xml_parse_error(format!(
            "invalid u16 attribute {}={value}: {error}",
            String::from_utf8_lossy(name)
        ))
    })
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn append_pcode_body(callfixup: &mut ProgramCompilerCallFixup, body: &str) {
    let slot = callfixup.pcode_body.get_or_insert_with(String::new);
    slot.push_str(body);
}

fn append_callother_pcode_body(callotherfixup: &mut ProgramCompilerCallOtherFixup, body: &str) {
    let slot = callotherfixup.pcode_body.get_or_insert_with(String::new);
    slot.push_str(body);
}

fn apply_pcode_inject_attributes(
    callfixup: &mut ProgramCompilerCallFixup,
    start: &BytesStart<'_>,
) -> GraphStoreResult<()> {
    if let Some(param_shift) = optional_attr(start, b"paramshift")? {
        callfixup.param_shift = param_shift.parse::<i32>().map_err(|error| {
            xml_parse_error(format!(
                "invalid p-code inject paramshift {param_shift}: {error}"
            ))
        })?;
    }
    if let Some(dynamic) = optional_attr(start, b"dynamic")? {
        callfixup.dynamic = parse_xml_bool(&dynamic)?;
    }
    if let Some(incidental_copy) = optional_attr(start, b"incidentalcopy")? {
        callfixup.incidental_copy = parse_xml_bool(&incidental_copy)?;
    }
    Ok(())
}

fn apply_callother_pcode_inject_attributes(
    callotherfixup: &mut ProgramCompilerCallOtherFixup,
    start: &BytesStart<'_>,
) -> GraphStoreResult<()> {
    if let Some(param_shift) = optional_attr(start, b"paramshift")? {
        callotherfixup.param_shift = param_shift.parse::<i32>().map_err(|error| {
            xml_parse_error(format!(
                "invalid p-code inject paramshift {param_shift}: {error}"
            ))
        })?;
    }
    if let Some(dynamic) = optional_attr(start, b"dynamic")? {
        callotherfixup.dynamic = parse_xml_bool(&dynamic)?;
    }
    if let Some(incidental_copy) = optional_attr(start, b"incidentalcopy")? {
        callotherfixup.incidental_copy = parse_xml_bool(&incidental_copy)?;
    }
    Ok(())
}

fn parse_pcode_inject_parameter(
    start: &BytesStart<'_>,
) -> GraphStoreResult<ProgramCompilerInjectParameter> {
    let byte_len = optional_attr(start, b"size")?
        .unwrap_or_else(|| "0".to_string())
        .parse::<u32>()
        .map_err(|error| xml_parse_error(format!("invalid inject parameter size: {error}")))?;
    Ok(ProgramCompilerInjectParameter {
        name: required_attr(start, b"name")?,
        byte_len,
    })
}

fn parse_xml_bool(value: &str) -> GraphStoreResult<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(xml_parse_error(format!("invalid boolean value {value}"))),
    }
}

fn normalize_pcode_body(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_language_pack_file(path: &Path) -> GraphStoreResult<String> {
    fs::read_to_string(path).map_err(|error| {
        xml_parse_error(format!(
            "failed to read Ghidra language pack file {}: {error}",
            path.display()
        ))
    })
}

fn file_name_string(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::to_string)
}

fn xml_parse_error(error: impl std::fmt::Display) -> GraphStoreError {
    GraphStoreError::new(
        "ghidra_language_pack_parse_failed",
        format!("failed to parse Ghidra language pack XML: {error}"),
    )
}

fn ghidra_x86_64_default_language_spec(artifact_id: &str) -> ProgramLanguageSpec {
    let language_id = "x86:LE:64:default";
    let language_spec_id = language_spec_id_for_artifact(artifact_id, language_id);
    let processor_spec = ProgramProcessorSpec {
        processor_spec_id: processor_spec_id(&language_spec_id, "x86-64.pspec"),
        language_spec_id: language_spec_id.clone(),
        spec_file: "x86-64.pspec".to_string(),
        program_counter_register: "RIP".to_string(),
        properties: vec![
            ProgramProcessorProperty {
                key: "useOperandReferenceAnalyzerSwitchTables".to_string(),
                value: "true".to_string(),
            },
            ProgramProcessorProperty {
                key: "assemblyRating:x86:LE:64:default".to_string(),
                value: "GOLD".to_string(),
            },
            ProgramProcessorProperty {
                key: "useropLibs".to_string(),
                value: "x86".to_string(),
            },
        ],
        context_values: vec![
            ProgramProcessorContextValue {
                space: "ram".to_string(),
                name: "addrsize".to_string(),
                value: "2".to_string(),
            },
            ProgramProcessorContextValue {
                space: "ram".to_string(),
                name: "opsize".to_string(),
                value: "1".to_string(),
            },
            ProgramProcessorContextValue {
                space: "ram".to_string(),
                name: "rexprefix".to_string(),
                value: "0".to_string(),
            },
            ProgramProcessorContextValue {
                space: "ram".to_string(),
                name: "longMode".to_string(),
                value: "1".to_string(),
            },
        ],
        tracked_values: vec![ProgramProcessorContextValue {
            space: "ram".to_string(),
            name: "DF".to_string(),
            value: "0".to_string(),
        }],
        register_groups: vec![
            ProgramProcessorRegisterGroup {
                group: "DEBUG".to_string(),
                exemplar_registers: vec!["DR0".to_string(), "DR15".to_string()],
            },
            ProgramProcessorRegisterGroup {
                group: "CONTROL".to_string(),
                exemplar_registers: vec!["CR0".to_string(), "CR15".to_string()],
            },
            ProgramProcessorRegisterGroup {
                group: "AVX".to_string(),
                exemplar_registers: vec![
                    "XMM0".to_string(),
                    "YMM0".to_string(),
                    "ZMM0".to_string(),
                ],
            },
            ProgramProcessorRegisterGroup {
                group: "FLAGS".to_string(),
                exemplar_registers: vec!["CF".to_string(), "PF".to_string()],
            },
        ],
    };
    ProgramLanguageSpec {
        language_spec_id: language_spec_id.clone(),
        artifact_id: artifact_id.to_string(),
        language_id: language_id.to_string(),
        processor: "x86".to_string(),
        endian: "little".to_string(),
        size_bits: 64,
        variant: "default".to_string(),
        version: "4.7".to_string(),
        sla_file: "x86-64.sla".to_string(),
        processor_spec_file: "x86-64.pspec".to_string(),
        manual_index_file: Some("../manuals/x86.idx".to_string()),
        description: "Intel/AMD 64-bit x86".to_string(),
        external_names: vec![
            ProgramLanguageExternalName {
                tool: "gnu".to_string(),
                name: "i386:x86-64:intel".to_string(),
            },
            ProgramLanguageExternalName {
                tool: "gnu".to_string(),
                name: "i386:x86-64".to_string(),
            },
            ProgramLanguageExternalName {
                tool: "IDA-PRO".to_string(),
                name: "metapc".to_string(),
            },
            ProgramLanguageExternalName {
                tool: "DWARF.register.mapping.file".to_string(),
                name: "x86-64.dwarf".to_string(),
            },
            ProgramLanguageExternalName {
                tool: "Golang.register.info.file".to_string(),
                name: "x86-64-golang.register.info".to_string(),
            },
            ProgramLanguageExternalName {
                tool: "qemu".to_string(),
                name: "qemu-x86_64".to_string(),
            },
            ProgramLanguageExternalName {
                tool: "qemu_system".to_string(),
                name: "qemu-system-x86_64".to_string(),
            },
        ],
        compiler_specs: vec![
            language_compiler_spec(
                &language_spec_id,
                "windows",
                "Visual Studio",
                "x86-64-win.cspec",
            ),
            language_compiler_spec(
                &language_spec_id,
                "clangwindows",
                "clang",
                "x86-64-win.cspec",
            ),
            gcc_x86_64_compiler_spec(&language_spec_id),
            language_compiler_spec(&language_spec_id, "golang", "golang", "x86-64-golang.cspec"),
        ],
        processor_spec,
    }
}

fn language_compiler_spec(
    language_spec_id: &str,
    compiler_id: &str,
    name: &str,
    spec_file: &str,
) -> ProgramCompilerSpec {
    ProgramCompilerSpec {
        compiler_spec_id: compiler_spec_id(language_spec_id, compiler_id, spec_file),
        language_spec_id: language_spec_id.to_string(),
        compiler_id: compiler_id.to_string(),
        name: name.to_string(),
        spec_file: spec_file.to_string(),
        stack_pointer_register: None,
        stack_pointer_space: None,
        default_proto: None,
        prototype_names: Vec::new(),
        data_organization: None,
        callfixups: Vec::new(),
        callotherfixups: Vec::new(),
    }
}

fn gcc_x86_64_compiler_spec(language_spec_id: &str) -> ProgramCompilerSpec {
    let mut spec = language_compiler_spec(language_spec_id, "gcc", "gcc", "x86-64-gcc.cspec");
    spec.stack_pointer_register = Some("RSP".to_string());
    spec.stack_pointer_space = Some("ram".to_string());
    spec.default_proto = Some("__stdcall".to_string());
    spec.prototype_names = vec![
        "__stdcall".to_string(),
        "MSABI".to_string(),
        "syscall".to_string(),
        "processEntry".to_string(),
    ];
    spec.data_organization = Some(ProgramCompilerDataOrganization {
        pointer_size: 8,
        default_pointer_alignment: 8,
        machine_alignment: 2,
        default_alignment: 1,
        integer_size: 4,
        long_size: 8,
    });
    spec
}

pub fn extract_ascii_strings(bytes: &[u8], min_len: usize) -> Vec<(usize, String)> {
    let mut strings = Vec::new();
    let mut start = None;
    for (index, byte) in bytes.iter().copied().enumerate() {
        let printable = byte.is_ascii_graphic() || byte == b' ';
        match (start, printable) {
            (None, true) => start = Some(index),
            (Some(begin), false) => {
                if index - begin >= min_len {
                    strings.push((
                        begin,
                        String::from_utf8_lossy(&bytes[begin..index]).to_string(),
                    ));
                }
                start = None;
            }
            _ => {}
        }
    }
    if let Some(begin) = start {
        if bytes.len() - begin >= min_len {
            strings.push((begin, String::from_utf8_lossy(&bytes[begin..]).to_string()));
        }
    }
    strings
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn collect_relocations<'data, O: Object<'data>>(
    parsed: &O,
    artifact_id: &str,
    artifact_sha256: &str,
) -> Vec<BinaryRelocation> {
    let mut relocations = Vec::new();
    for section in parsed.sections() {
        let section_index = section.index().0;
        for (offset, relocation) in section.relocations() {
            relocations.push(binary_relocation(
                artifact_id,
                artifact_sha256,
                section_index,
                offset,
                &relocation,
                parsed,
                false,
            ));
        }
    }
    if let Some(dynamic_relocations) = parsed.dynamic_relocations() {
        for (address, relocation) in dynamic_relocations {
            let section_index = section_index_for_address(parsed, address).unwrap_or(usize::MAX);
            relocations.push(binary_relocation(
                artifact_id,
                artifact_sha256,
                section_index,
                address,
                &relocation,
                parsed,
                true,
            ));
        }
    }
    relocations
}

fn binary_relocation<'data, O: Object<'data>>(
    artifact_id: &str,
    artifact_sha256: &str,
    section_index: usize,
    offset: u64,
    relocation: &Relocation,
    parsed: &O,
    dynamic: bool,
) -> BinaryRelocation {
    let kind = if dynamic {
        format!("Dynamic:{:?}", relocation.kind())
    } else {
        format!("{:?}", relocation.kind())
    };
    let target = relocation_target_name(parsed, relocation.target());
    BinaryRelocation {
        relocation_id: format!(
            "bin:relocation:{}",
            stable_hash(json!([
                artifact_sha256,
                section_index,
                offset,
                &kind,
                &target,
                relocation.addend()
            ]))
        ),
        artifact_id: artifact_id.to_string(),
        section_index,
        offset,
        kind,
        target,
    }
}

fn section_index_for_address<'data, O: Object<'data>>(parsed: &O, address: u64) -> Option<usize> {
    parsed.sections().find_map(|section| {
        let start = section.address();
        let end = start.saturating_add(section.size());
        (address >= start && address < end).then_some(section.index().0)
    })
}

fn relocation_target_name<'data, O: Object<'data>>(parsed: &O, target: RelocationTarget) -> String {
    match target {
        RelocationTarget::Symbol(index) => parsed
            .symbol_by_index(index)
            .ok()
            .and_then(|symbol| symbol.name().ok().map(str::to_string))
            .filter(|name| !name.is_empty())
            .map(|name| format!("symbol:{name}"))
            .unwrap_or_else(|| format!("symbol:{}", index.0)),
        RelocationTarget::Section(index) => parsed
            .section_by_index(index)
            .ok()
            .and_then(|section| section.name().ok().map(str::to_string))
            .filter(|name| !name.is_empty())
            .map(|name| format!("section:{name}"))
            .unwrap_or_else(|| format!("section:{}", index.0)),
        RelocationTarget::Absolute => "absolute".to_string(),
        _ => format!("{target:?}"),
    }
}

fn manager_fact_count(report: &BinaryLoadReport, kind: ProgramManagerKind) -> usize {
    match kind {
        ProgramManagerKind::Memory => report.sections.len(),
        ProgramManagerKind::Symbol => report.symbols.len(),
        ProgramManagerKind::Relocation => report.relocations.len(),
        ProgramManagerKind::String => report.strings.len(),
        ProgramManagerKind::Entrypoint => report.entrypoints.len(),
        ProgramManagerKind::External => report
            .symbols
            .iter()
            .filter(|symbol| !symbol.is_definition)
            .count(),
        ProgramManagerKind::Reference => report.relocations.len(),
        ProgramManagerKind::Code
        | ProgramManagerKind::Namespace
        | ProgramManagerKind::Function
        | ProgramManagerKind::DataType
        | ProgramManagerKind::Equate
        | ProgramManagerKind::Bookmark
        | ProgramManagerKind::Context
        | ProgramManagerKind::Property
        | ProgramManagerKind::Tree
        | ProgramManagerKind::SourceFile => 0,
    }
}

fn program_id_for_artifact(artifact_id: &str) -> String {
    format!(
        "program:{}",
        stable_hash(json!([PROGRAM_GRAPH_VERSION, artifact_id]))
    )
}

fn program_manager_id(program_id: &str, kind: ProgramManagerKind) -> String {
    format!("{program_id}:manager:{}", kind.stable_name())
}

fn language_spec_id_for_artifact(artifact_id: &str, language_id: &str) -> String {
    format!(
        "program:language:{}",
        stable_hash(json!([
            PROGRAM_LANGUAGE_PACK_VERSION,
            artifact_id,
            language_id
        ]))
    )
}

fn compiler_spec_id(language_spec_id: &str, compiler_id: &str, spec_file: &str) -> String {
    format!(
        "{language_spec_id}:compiler:{}",
        stable_hash(json!([compiler_id, spec_file]))
    )
}

fn processor_spec_id(language_spec_id: &str, spec_file: &str) -> String {
    format!(
        "{language_spec_id}:processor:{}",
        stable_hash(json!([spec_file]))
    )
}

fn artifact_node(artifact: &BinaryArtifact) -> NodeRecord {
    NodeRecord::new(
        &artifact.artifact_id,
        [BINARY_ARTIFACT_LABEL],
        json!({
            "sha256": artifact.sha256,
            "name": artifact.name,
            "format": artifact.format,
            "arch": artifact.arch,
            "endian": artifact.endian,
            "entrypoint": artifact.entrypoint,
            "byte_len": artifact.byte_len,
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
        }),
    )
}

fn program_node(program: &ProgramGraph) -> NodeRecord {
    NodeRecord::new(
        &program.program_id,
        [PROGRAM_GRAPH_LABEL],
        json!({
            "artifact_id": program.artifact_id,
            "name": program.name,
            "executable_format": program.executable_format,
            "architecture": program.architecture,
            "endian": program.endian,
            "entrypoint": program.entrypoint,
            "byte_len": program.byte_len,
            "manager_count": program.managers.len(),
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
            "program_graph_version": PROGRAM_GRAPH_VERSION,
        }),
    )
}

fn program_manager_node(manager: &ProgramManager) -> NodeRecord {
    NodeRecord::new(
        &manager.manager_id,
        [PROGRAM_MANAGER_LABEL],
        json!({
            "program_id": manager.program_id,
            "kind": manager.kind,
            "stable_name": manager.kind.stable_name(),
            "display_name": manager.display_name,
            "ghidra_manager_order": manager.ghidra_manager_order,
            "fact_count": manager.fact_count,
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
            "program_graph_version": PROGRAM_GRAPH_VERSION,
        }),
    )
}

fn language_spec_node(language_spec: &ProgramLanguageSpec) -> NodeRecord {
    NodeRecord::new(
        &language_spec.language_spec_id,
        [PROGRAM_LANGUAGE_SPEC_LABEL],
        json!({
            "artifact_id": language_spec.artifact_id,
            "language_id": language_spec.language_id,
            "processor": language_spec.processor,
            "endian": language_spec.endian,
            "size_bits": language_spec.size_bits,
            "variant": language_spec.variant,
            "language_version": language_spec.version,
            "sla_file": language_spec.sla_file,
            "processor_spec_file": language_spec.processor_spec_file,
            "manual_index_file": language_spec.manual_index_file,
            "description": language_spec.description,
            "external_names": language_spec.external_names,
            "compiler_spec_count": language_spec.compiler_specs.len(),
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
            "language_pack_version": PROGRAM_LANGUAGE_PACK_VERSION,
        }),
    )
}

fn compiler_spec_node(compiler_spec: &ProgramCompilerSpec) -> NodeRecord {
    NodeRecord::new(
        &compiler_spec.compiler_spec_id,
        [PROGRAM_COMPILER_SPEC_LABEL],
        json!({
            "language_spec_id": compiler_spec.language_spec_id,
            "compiler_id": compiler_spec.compiler_id,
            "name": compiler_spec.name,
            "spec_file": compiler_spec.spec_file,
            "stack_pointer_register": compiler_spec.stack_pointer_register,
            "stack_pointer_space": compiler_spec.stack_pointer_space,
            "default_proto": compiler_spec.default_proto,
            "prototype_names": compiler_spec.prototype_names,
            "data_organization": compiler_spec.data_organization,
            "callfixups": compiler_spec.callfixups,
            "callfixup_count": compiler_spec.callfixups.len(),
            "callotherfixups": compiler_spec.callotherfixups,
            "callotherfixup_count": compiler_spec.callotherfixups.len(),
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
            "language_pack_version": PROGRAM_LANGUAGE_PACK_VERSION,
        }),
    )
}

fn processor_spec_node(processor_spec: &ProgramProcessorSpec) -> NodeRecord {
    NodeRecord::new(
        &processor_spec.processor_spec_id,
        [PROGRAM_PROCESSOR_SPEC_LABEL],
        json!({
            "language_spec_id": processor_spec.language_spec_id,
            "spec_file": processor_spec.spec_file,
            "program_counter_register": processor_spec.program_counter_register,
            "properties": processor_spec.properties,
            "context_values": processor_spec.context_values,
            "tracked_values": processor_spec.tracked_values,
            "register_groups": processor_spec.register_groups,
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
            "language_pack_version": PROGRAM_LANGUAGE_PACK_VERSION,
        }),
    )
}

fn section_node(section: &BinarySection) -> NodeRecord {
    NodeRecord::new(
        &section.section_id,
        [BINARY_SECTION_LABEL],
        json!({
            "artifact_id": section.artifact_id,
            "index": section.index,
            "name": section.name,
            "address": section.address,
            "size": section.size,
            "kind": section.kind,
            "executable": section.executable,
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
        }),
    )
}

fn symbol_node(symbol: &BinarySymbol) -> NodeRecord {
    NodeRecord::new(
        &symbol.symbol_id,
        [BINARY_SYMBOL_LABEL],
        json!({
            "artifact_id": symbol.artifact_id,
            "index": symbol.index,
            "name": symbol.name,
            "address": symbol.address,
            "size": symbol.size,
            "kind": symbol.kind,
            "scope": symbol.scope,
            "is_definition": symbol.is_definition,
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
        }),
    )
}

fn string_node(string: &BinaryString) -> NodeRecord {
    NodeRecord::new(
        &string.string_id,
        [BINARY_STRING_LABEL],
        json!({
            "artifact_id": string.artifact_id,
            "offset": string.offset,
            "value": string.value,
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
        }),
    )
}

fn relocation_node(relocation: &BinaryRelocation) -> NodeRecord {
    NodeRecord::new(
        &relocation.relocation_id,
        [BINARY_RELOCATION_LABEL],
        json!({
            "artifact_id": relocation.artifact_id,
            "section_index": relocation.section_index,
            "offset": relocation.offset,
            "kind": relocation.kind,
            "target": relocation.target,
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
        }),
    )
}

fn entrypoint_node(entrypoint: &BinaryEntrypoint) -> NodeRecord {
    NodeRecord::new(
        &entrypoint.entrypoint_id,
        [BINARY_ENTRYPOINT_LABEL],
        json!({
            "artifact_id": entrypoint.artifact_id,
            "address": entrypoint.address,
            "kind": entrypoint.kind,
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
        }),
    )
}

fn write_program_graph_in_store<S: GraphStore>(
    store: &mut S,
    program: &ProgramGraph,
    artifact_id: &str,
) -> GraphStoreResult<()> {
    store.upsert_node(program_node(program))?;
    store.upsert_edge(EdgeRecord::new(
        format!(
            "program:edge:{}",
            stable_hash(json!([
                &program.program_id,
                PROGRAM_FOR_ARTIFACT,
                artifact_id
            ]))
        ),
        &program.program_id,
        PROGRAM_FOR_ARTIFACT,
        artifact_id,
        json!({
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
            "program_graph_version": PROGRAM_GRAPH_VERSION,
        }),
    ))?;
    for manager in &program.managers {
        store.upsert_node(program_manager_node(manager))?;
        store.upsert_edge(EdgeRecord::new(
            format!(
                "program:edge:{}",
                stable_hash(json!([
                    &program.program_id,
                    PROGRAM_HAS_MANAGER,
                    &manager.manager_id
                ]))
            ),
            &program.program_id,
            PROGRAM_HAS_MANAGER,
            &manager.manager_id,
            json!({
                "authority": "observed_fact",
                "source": BINFORMAT_SOURCE,
                "version": BINFORMAT_VERSION,
                "program_graph_version": PROGRAM_GRAPH_VERSION,
            }),
        ))?;
    }
    Ok(())
}

fn write_language_spec_in_store<S: GraphStore>(
    store: &mut S,
    program: &ProgramGraph,
    language_spec: &ProgramLanguageSpec,
) -> GraphStoreResult<()> {
    store.upsert_node(language_spec_node(language_spec))?;
    store.upsert_edge(EdgeRecord::new(
        format!(
            "program:edge:{}",
            stable_hash(json!([
                &program.program_id,
                PROGRAM_HAS_LANGUAGE_SPEC,
                &language_spec.language_spec_id
            ]))
        ),
        &program.program_id,
        PROGRAM_HAS_LANGUAGE_SPEC,
        &language_spec.language_spec_id,
        json!({
            "language_id": &language_spec.language_id,
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
            "program_graph_version": PROGRAM_GRAPH_VERSION,
            "language_pack_version": PROGRAM_LANGUAGE_PACK_VERSION,
        }),
    ))?;
    store.upsert_node(processor_spec_node(&language_spec.processor_spec))?;
    store.upsert_edge(EdgeRecord::new(
        format!(
            "program:edge:{}",
            stable_hash(json!([
                &language_spec.language_spec_id,
                LANGUAGE_HAS_PROCESSOR_SPEC,
                &language_spec.processor_spec.processor_spec_id
            ]))
        ),
        &language_spec.language_spec_id,
        LANGUAGE_HAS_PROCESSOR_SPEC,
        &language_spec.processor_spec.processor_spec_id,
        json!({
            "spec_file": &language_spec.processor_spec.spec_file,
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
            "language_pack_version": PROGRAM_LANGUAGE_PACK_VERSION,
        }),
    ))?;
    for compiler_spec in &language_spec.compiler_specs {
        store.upsert_node(compiler_spec_node(compiler_spec))?;
        store.upsert_edge(EdgeRecord::new(
            format!(
                "program:edge:{}",
                stable_hash(json!([
                    &language_spec.language_spec_id,
                    LANGUAGE_HAS_COMPILER_SPEC,
                    &compiler_spec.compiler_spec_id
                ]))
            ),
            &language_spec.language_spec_id,
            LANGUAGE_HAS_COMPILER_SPEC,
            &compiler_spec.compiler_spec_id,
            json!({
                "compiler_id": &compiler_spec.compiler_id,
                "spec_file": &compiler_spec.spec_file,
                "authority": "observed_fact",
                "source": BINFORMAT_SOURCE,
                "version": BINFORMAT_VERSION,
                "language_pack_version": PROGRAM_LANGUAGE_PACK_VERSION,
            }),
        ))?;
    }
    Ok(())
}

fn link_artifact<S: GraphStore>(
    store: &mut S,
    artifact_id: &str,
    target_id: &str,
    edge_type: &str,
) -> GraphStoreResult<()> {
    store.upsert_edge(EdgeRecord::new(
        format!(
            "bin:edge:{}",
            stable_hash(json!([artifact_id, edge_type, target_id]))
        ),
        artifact_id,
        edge_type,
        target_id,
        json!({
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
        }),
    ))?;
    Ok(())
}

fn link_manager_fact<S: GraphStore>(
    store: &mut S,
    program: &ProgramGraph,
    kind: ProgramManagerKind,
    target_id: &str,
) -> GraphStoreResult<()> {
    let manager_id = program_manager_id(&program.program_id, kind);
    store.upsert_edge(EdgeRecord::new(
        format!(
            "program:edge:{}",
            stable_hash(json!([manager_id, MANAGER_HAS_FACT, target_id]))
        ),
        &manager_id,
        MANAGER_HAS_FACT,
        target_id,
        json!({
            "manager_kind": kind,
            "authority": "observed_fact",
            "source": BINFORMAT_SOURCE,
            "version": BINFORMAT_VERSION,
            "program_graph_version": PROGRAM_GRAPH_VERSION,
        }),
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::{InMemoryGraphStore, NodeQuery};

    #[test]
    fn string_extraction_keeps_offsets() {
        let strings = extract_ascii_strings(b"\0/api/login\0abc\0sqlite3_prepare_v2\0", 4);
        assert_eq!(strings[0], (1, "/api/login".to_string()));
        assert_eq!(strings[1], (16, "sqlite3_prepare_v2".to_string()));
    }

    #[test]
    fn graph_write_records_observed_artifact_facts() {
        let report = BinaryLoadReport {
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
                section_id: "section:1".to_string(),
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
            relocations: vec![BinaryRelocation {
                relocation_id: "relocation:1".to_string(),
                artifact_id: "sha256:test".to_string(),
                section_index: 0,
                offset: 1,
                kind: "Relative".to_string(),
                target: "symbol:target".to_string(),
            }],
            entrypoints: Vec::new(),
            language_specs: ghidra_language_specs_for_artifact(&BinaryArtifact {
                artifact_id: "sha256:test".to_string(),
                sha256: "test".to_string(),
                name: "fixture".to_string(),
                format: "Elf".to_string(),
                arch: "X86_64".to_string(),
                endian: "Little".to_string(),
                entrypoint: 0x1000,
                byte_len: 3,
            }),
        };
        let mut store = InMemoryGraphStore::new();
        write_binary_facts_in_store(&mut store, &report).unwrap();
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(BINARY_ARTIFACT_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(BINARY_SECTION_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(BINARY_RELOCATION_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(PROGRAM_GRAPH_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(PROGRAM_MANAGER_LABEL))
                .len(),
            ProgramManagerKind::all_program_managers().len()
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(PROGRAM_LANGUAGE_SPEC_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(PROGRAM_COMPILER_SPEC_LABEL))
                .len(),
            4
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(PROGRAM_PROCESSOR_SPEC_LABEL))
                .len(),
            1
        );
        let memory_managers = store.query_nodes(
            NodeQuery::label(PROGRAM_MANAGER_LABEL)
                .with_property("kind", json!(ProgramManagerKind::Memory)),
        );
        assert_eq!(memory_managers.len(), 1);
        assert_eq!(
            memory_managers[0].properties["ghidra_manager_order"],
            json!(0)
        );
        assert_eq!(memory_managers[0].properties["fact_count"], json!(1));
        let relocation_managers = store.query_nodes(
            NodeQuery::label(PROGRAM_MANAGER_LABEL)
                .with_property("kind", json!(ProgramManagerKind::Relocation)),
        );
        assert_eq!(relocation_managers.len(), 1);
        assert_eq!(
            relocation_managers[0].properties["ghidra_manager_order"],
            json!(13)
        );
        assert_eq!(relocation_managers[0].properties["fact_count"], json!(1));
    }

    #[test]
    fn program_graph_models_ghidra_manager_order() {
        let report = BinaryLoadReport {
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
            sections: Vec::new(),
            symbols: Vec::new(),
            strings: Vec::new(),
            relocations: Vec::new(),
            entrypoints: Vec::new(),
            language_specs: Vec::new(),
        };

        let program = program_graph_for_load_report(&report);
        let ordered = program
            .managers
            .iter()
            .filter_map(|manager| {
                manager
                    .ghidra_manager_order
                    .map(|order| (order, manager.kind))
            })
            .collect::<Vec<_>>();

        assert_eq!(ordered.len(), 15);
        assert_eq!(ordered[0], (0, ProgramManagerKind::Memory));
        assert_eq!(ordered[1], (1, ProgramManagerKind::Code));
        assert_eq!(ordered[2], (2, ProgramManagerKind::Symbol));
        assert_eq!(ordered[4], (4, ProgramManagerKind::Function));
        assert_eq!(ordered[13], (13, ProgramManagerKind::Relocation));
        assert_eq!(ordered[14], (14, ProgramManagerKind::SourceFile));
        assert!(program
            .managers
            .iter()
            .any(|manager| manager.kind == ProgramManagerKind::String
                && manager.ghidra_manager_order.is_none()));
    }

    #[test]
    fn x86_64_language_pack_matches_ghidra_reference_files() {
        let specs = ghidra_language_specs_for_artifact(&BinaryArtifact {
            artifact_id: "sha256:test".to_string(),
            sha256: "test".to_string(),
            name: "fixture".to_string(),
            format: "Elf".to_string(),
            arch: "X86_64".to_string(),
            endian: "Little".to_string(),
            entrypoint: 0x1000,
            byte_len: 3,
        });

        assert_eq!(specs.len(), 1);
        let language = &specs[0];
        assert_eq!(language.language_id, "x86:LE:64:default");
        assert_eq!(language.sla_file, "x86-64.sla");
        assert_eq!(language.processor_spec_file, "x86-64.pspec");
        assert!(language
            .external_names
            .iter()
            .any(|external| { external.tool == "qemu" && external.name == "qemu-x86_64" }));
        assert!(language
            .external_names
            .iter()
            .any(|external| { external.tool == "IDA-PRO" && external.name == "metapc" }));
        assert_eq!(language.processor_spec.program_counter_register, "RIP");
        assert!(language
            .processor_spec
            .context_values
            .iter()
            .any(|context| context.name == "longMode" && context.value == "1"));
        let gcc = language
            .compiler_specs
            .iter()
            .find(|compiler| compiler.compiler_id == "gcc")
            .expect("gcc compiler spec");
        assert_eq!(gcc.spec_file, "x86-64-gcc.cspec");
        assert_eq!(gcc.stack_pointer_register.as_deref(), Some("RSP"));
        assert_eq!(gcc.stack_pointer_space.as_deref(), Some("ram"));
        assert_eq!(gcc.default_proto.as_deref(), Some("__stdcall"));
        assert!(gcc.prototype_names.iter().any(|name| name == "MSABI"));
        assert!(gcc.prototype_names.iter().any(|name| name == "syscall"));
        assert_eq!(
            gcc.data_organization.as_ref().map(|data| data.pointer_size),
            Some(8)
        );
    }

    #[test]
    fn parses_ghidra_language_pack_xml_into_program_facts() {
        let ldefs = r#"
            <language_definitions>
              <language processor="x86"
                        endian="little"
                        size="64"
                        variant="default"
                        version="4.7"
                        slafile="x86-64.sla"
                        processorspec="x86-64.pspec"
                        manualindexfile="../manuals/x86.idx"
                        id="x86:LE:64:default">
                <description>Intel/AMD 64-bit x86</description>
                <compiler name="gcc" spec="x86-64-gcc.cspec" id="gcc"/>
                <external_name tool="IDA-PRO" name="metapc"/>
                <external_name tool="qemu" name="qemu-x86_64"/>
              </language>
            </language_definitions>
        "#;
        let pspec = r#"
            <processor_spec>
              <properties>
                <property key="useOperandReferenceAnalyzerSwitchTables" value="true"/>
                <property key="useropLibs" value="x86"/>
              </properties>
              <programcounter register="RIP"/>
              <context_data>
                <context_set space="ram">
                  <set name="longMode" val="1"/>
                  <set name="opsize" val="1"/>
                </context_set>
                <tracked_set space="ram">
                  <set name="DF" val="0"/>
                </tracked_set>
              </context_data>
              <register_data>
                <register name="RAX" group="GENERAL"/>
                <register name="RSP" group="GENERAL"/>
                <register name="XMM0" group="AVX"/>
              </register_data>
            </processor_spec>
        "#;
        let cspec = r#"
            <compiler_spec>
              <data_organization>
                <machine_alignment value="2" />
                <default_alignment value="1" />
                <default_pointer_alignment value="8" />
                <pointer_size value="8" />
                <integer_size value="4" />
                <long_size value="8" />
              </data_organization>
              <stackpointer register="RSP" space="ram"/>
              <default_proto>
                <prototype name="__stdcall" extrapop="8" stackshift="8"/>
              </default_proto>
              <prototype name="syscall" extrapop="8" stackshift="8"/>
              <callfixup name="x86_return_thunk">
                <target name="__x86_return_thunk"/>
                <pcode paramshift="1" incidentalcopy="true" dynamic="false">
                  <input name="call_target" size="8"/>
                  <output name="next_pc" size="8"/>
                  <body><![CDATA[
                    RIP = *:8 RSP;
                    RSP = RSP + 8;
                    return [RIP];
                  ]]></body>
                </pcode>
              </callfixup>
              <callotherfixup targetop="cpuid">
                <pcode incidentalcopy="true">
                  <input name="leaf" size="4"/>
                  <output name="eax_out" size="4"/>
                  <body><![CDATA[
                    EAX = leaf;
                    CALLOTHER cpuid_semantics;
                  ]]></body>
                </pcode>
              </callotherfixup>
            </compiler_spec>
        "#;
        let mut processor_specs = BTreeMap::new();
        processor_specs.insert("x86-64.pspec".to_string(), pspec.to_string());
        let mut compiler_specs = BTreeMap::new();
        compiler_specs.insert("x86-64-gcc.cspec".to_string(), cspec.to_string());

        let specs = ghidra_language_specs_from_pack(
            "sha256:test",
            ldefs,
            &processor_specs,
            &compiler_specs,
        )
        .unwrap();

        assert_eq!(specs.len(), 1);
        let language = &specs[0];
        assert_eq!(language.language_id, "x86:LE:64:default");
        assert_eq!(language.sla_file, "x86-64.sla");
        assert_eq!(language.processor_spec.program_counter_register, "RIP");
        assert!(language
            .processor_spec
            .properties
            .iter()
            .any(|property| property.key == "useropLibs" && property.value == "x86"));
        assert!(language
            .processor_spec
            .context_values
            .iter()
            .any(|context| context.name == "longMode" && context.value == "1"));
        assert!(language
            .processor_spec
            .tracked_values
            .iter()
            .any(|context| context.name == "DF" && context.value == "0"));
        assert!(language
            .processor_spec
            .register_groups
            .iter()
            .any(|group| group.group == "AVX"
                && group.exemplar_registers == vec!["XMM0".to_string()]));
        let gcc = language
            .compiler_specs
            .iter()
            .find(|compiler| compiler.compiler_id == "gcc")
            .expect("gcc compiler spec");
        assert_eq!(gcc.stack_pointer_register.as_deref(), Some("RSP"));
        assert_eq!(gcc.stack_pointer_space.as_deref(), Some("ram"));
        assert_eq!(gcc.default_proto.as_deref(), Some("__stdcall"));
        assert!(gcc.prototype_names.iter().any(|name| name == "syscall"));
        assert_eq!(
            gcc.data_organization.as_ref().map(|data| data.pointer_size),
            Some(8)
        );
        assert_eq!(gcc.callfixups.len(), 1);
        assert_eq!(gcc.callfixups[0].name, "x86_return_thunk");
        assert_eq!(gcc.callfixups[0].targets, vec!["__x86_return_thunk"]);
        assert_eq!(gcc.callfixups[0].param_shift, 1);
        assert!(!gcc.callfixups[0].dynamic);
        assert!(gcc.callfixups[0].incidental_copy);
        assert_eq!(
            gcc.callfixups[0].input_parameters,
            vec![ProgramCompilerInjectParameter {
                name: "call_target".to_string(),
                byte_len: 8,
            }]
        );
        assert_eq!(
            gcc.callfixups[0].output_parameters,
            vec![ProgramCompilerInjectParameter {
                name: "next_pc".to_string(),
                byte_len: 8,
            }]
        );
        assert_eq!(
            gcc.callfixups[0].pcode_body.as_deref(),
            Some("RIP = *:8 RSP;\nRSP = RSP + 8;\nreturn [RIP];")
        );
        assert!(gcc.callfixups[0].pcode_body_hash.is_some());
        assert_eq!(gcc.callotherfixups.len(), 1);
        assert_eq!(gcc.callotherfixups[0].targetop, "cpuid");
        assert_eq!(gcc.callotherfixups[0].param_shift, 0);
        assert!(!gcc.callotherfixups[0].dynamic);
        assert!(gcc.callotherfixups[0].incidental_copy);
        assert_eq!(
            gcc.callotherfixups[0].input_parameters,
            vec![ProgramCompilerInjectParameter {
                name: "leaf".to_string(),
                byte_len: 4,
            }]
        );
        assert_eq!(
            gcc.callotherfixups[0].output_parameters,
            vec![ProgramCompilerInjectParameter {
                name: "eax_out".to_string(),
                byte_len: 4,
            }]
        );
        assert_eq!(
            gcc.callotherfixups[0].pcode_body.as_deref(),
            Some("EAX = leaf;\nCALLOTHER cpuid_semantics;")
        );
        assert!(gcc.callotherfixups[0].pcode_body_hash.is_some());
    }

    #[test]
    fn reads_ghidra_language_pack_directory() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "theorem-ghidra-language-pack-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("toy.ldefs"),
            r#"
                <language_definitions>
                  <language processor="toy"
                            endian="big"
                            size="32"
                            variant="default"
                            version="1"
                            slafile="toy.sla"
                            processorspec="toy.pspec"
                            id="toy:BE:32:default">
                    <description>Toy processor</description>
                    <compiler name="default" spec="toy.cspec" id="default"/>
                  </language>
                </language_definitions>
            "#,
        )
        .unwrap();
        std::fs::write(
            dir.join("toy.pspec"),
            r#"
                <processor_spec>
                  <programcounter register="PC"/>
                  <context_data>
                    <context_set space="ram">
                      <set name="mode" val="0"/>
                    </context_set>
                  </context_data>
                </processor_spec>
            "#,
        )
        .unwrap();
        std::fs::write(
            dir.join("toy.cspec"),
            r#"
                <compiler_spec>
                  <stackpointer register="SP" space="ram"/>
                  <default_proto>
                    <prototype name="toycall"/>
                  </default_proto>
                </compiler_spec>
            "#,
        )
        .unwrap();

        let specs = ghidra_language_specs_from_directory("sha256:test", &dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(specs.len(), 1);
        let language = &specs[0];
        assert_eq!(language.language_id, "toy:BE:32:default");
        assert_eq!(language.processor_spec.program_counter_register, "PC");
        let compiler = &language.compiler_specs[0];
        assert_eq!(compiler.stack_pointer_register.as_deref(), Some("SP"));
        assert_eq!(compiler.default_proto.as_deref(), Some("toycall"));
    }
}
