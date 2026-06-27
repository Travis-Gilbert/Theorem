use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use object::{
    Object, ObjectSection, ObjectSymbol, RelocationTarget, SectionFlags, SectionKind, SymbolKind,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::program_analysis::{
    AnalyzerPassReceipt, BinaryArtifact, BinaryImport, BinaryRelocation, BinarySection,
    BinaryString, BinarySymbol, LoaderFact, ProgramAnalysisStatus, BINARY_ARTIFACT_LABEL,
    LOADER_FACT_LABEL,
};
use rustyred_thg_core::stable_hash;

pub const NATIVE_LOADER_ANALYZER_ID: &str = "rustyred-thg-code:native-loader-object-v0";

const AUTHORITY_OBSERVED_FACT: &str = "observed_fact";
const MIN_ASCII_STRING_LEN: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeLoaderOutput {
    pub artifact: BinaryArtifact,
    pub loader_fact: LoaderFact,
    pub analyzer_receipt: AnalyzerPassReceipt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeLoaderError {
    EmptyInput,
    Parse(String),
    Section(String),
    Symbol(String),
    Import(String),
}

impl fmt::Display for NativeLoaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyInput => write!(formatter, "native loader input is empty"),
            Self::Parse(message) => write!(formatter, "failed to parse object file: {message}"),
            Self::Section(message) => write!(formatter, "failed to read object section: {message}"),
            Self::Symbol(message) => write!(formatter, "failed to read object symbol: {message}"),
            Self::Import(message) => write!(formatter, "failed to read object import: {message}"),
        }
    }
}

impl Error for NativeLoaderError {}

pub fn load_native_binary(
    bytes: &[u8],
    evidence_ids: Vec<String>,
) -> Result<NativeLoaderOutput, NativeLoaderError> {
    if bytes.is_empty() {
        return Err(NativeLoaderError::EmptyInput);
    }

    let file =
        object::File::parse(bytes).map_err(|error| NativeLoaderError::Parse(error.to_string()))?;
    let evidence_ids = normalize_evidence_ids(evidence_ids);
    let sha256 = sha256_hex(bytes);
    let artifact_id = format!("artifact:sha256:{sha256}");
    let entrypoints = match file.entry() {
        0 => Vec::new(),
        entrypoint => vec![hex_addr(entrypoint)],
    };
    let load_base = match file.relative_address_base() {
        0 => None,
        base => Some(hex_addr(base)),
    };

    let artifact = BinaryArtifact {
        artifact_id: artifact_id.clone(),
        sha256: sha256.clone(),
        format: format!("{:?}", file.format()),
        arch: format!("{:?}", file.architecture()),
        endian: if file.is_little_endian() {
            "little".to_string()
        } else {
            "big".to_string()
        },
        entrypoints,
        load_base,
        evidence_ids: evidence_ids.clone(),
    };

    let loader_fact = LoaderFact {
        fact_id: format!(
            "program-analysis:loader:{}",
            stable_hash(json!({
                "analyzer_id": NATIVE_LOADER_ANALYZER_ID,
                "artifact_sha256": &sha256,
                "format": &artifact.format,
                "arch": &artifact.arch,
            }))
        ),
        sections: collect_sections(&file)?,
        symbols: collect_symbols(&file)?,
        relocations: collect_relocations(&file)?,
        imports: collect_imports(&file)?,
        strings: collect_ascii_strings(bytes),
        evidence_ids: evidence_ids.clone(),
    };

    let analyzer_receipt = AnalyzerPassReceipt {
        receipt_id: format!(
            "program-analysis:receipt:{}",
            stable_hash(json!({
                "analyzer_id": NATIVE_LOADER_ANALYZER_ID,
                "artifact_sha256": &sha256,
                "byte_len": bytes.len(),
            }))
        ),
        analyzer_id: NATIVE_LOADER_ANALYZER_ID.to_string(),
        input_labels: vec![BINARY_ARTIFACT_LABEL.to_string()],
        output_labels: vec![LOADER_FACT_LABEL.to_string()],
        authority_layer: AUTHORITY_OBSERVED_FACT.to_string(),
        input_hash: stable_hash(json!({
            "artifact_sha256": &sha256,
            "byte_len": bytes.len(),
        })),
        status: ProgramAnalysisStatus::Complete,
        evidence_ids,
    };

    Ok(NativeLoaderOutput {
        artifact,
        loader_fact,
        analyzer_receipt,
    })
}

fn collect_sections(file: &object::File<'_>) -> Result<Vec<BinarySection>, NativeLoaderError> {
    let mut sections = Vec::new();
    for section in file.sections() {
        let name = section
            .name()
            .map_err(|error| NativeLoaderError::Section(error.to_string()))?;
        if name.is_empty() {
            continue;
        }
        sections.push(BinarySection {
            name: name.to_string(),
            address: hex_addr(section.address()),
            size: section.size(),
            permissions: section_permissions(&section),
        });
    }
    sections.sort_by(|left, right| {
        left.address
            .cmp(&right.address)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(sections)
}

fn collect_symbols(file: &object::File<'_>) -> Result<Vec<BinarySymbol>, NativeLoaderError> {
    let mut symbols = Vec::new();
    for symbol in file.symbols() {
        let name = symbol
            .name()
            .map_err(|error| NativeLoaderError::Symbol(error.to_string()))?;
        if name.is_empty() {
            continue;
        }
        symbols.push(BinarySymbol {
            name: name.to_string(),
            address: hex_addr(symbol.address()),
            kind: symbol_kind(symbol.kind(), symbol.is_undefined()),
        });
    }
    symbols.sort_by(|left, right| {
        left.address
            .cmp(&right.address)
            .then_with(|| left.name.cmp(&right.name))
    });
    symbols.dedup_by(|left, right| {
        left.name == right.name && left.address == right.address && left.kind == right.kind
    });
    Ok(symbols)
}

fn collect_relocations(
    file: &object::File<'_>,
) -> Result<Vec<BinaryRelocation>, NativeLoaderError> {
    let mut relocations = Vec::new();
    for section in file.sections() {
        let section_address = section.address();
        for (offset, relocation) in section.relocations() {
            relocations.push(BinaryRelocation {
                address: hex_addr(section_address.saturating_add(offset)),
                target: relocation_target_name(file, relocation.target()),
                kind: format!("{:?}", relocation.kind()),
            });
        }
    }
    relocations.sort_by(|left, right| {
        left.address
            .cmp(&right.address)
            .then_with(|| left.target.cmp(&right.target))
    });
    relocations.dedup();
    Ok(relocations)
}

fn collect_imports(file: &object::File<'_>) -> Result<Vec<BinaryImport>, NativeLoaderError> {
    let mut imports = BTreeSet::new();

    let dynamic_imports = file
        .imports()
        .map_err(|error| NativeLoaderError::Import(error.to_string()))?;
    for import in dynamic_imports {
        let name = String::from_utf8_lossy(import.name()).trim().to_string();
        if name.is_empty() {
            continue;
        }
        let library = String::from_utf8_lossy(import.library()).trim().to_string();
        imports.insert((
            if library.is_empty() {
                None
            } else {
                Some(library)
            },
            name,
            None,
        ));
    }

    for symbol in file.symbols() {
        if !symbol.is_undefined() {
            continue;
        }
        let name = symbol
            .name()
            .map_err(|error| NativeLoaderError::Symbol(error.to_string()))?
            .trim()
            .to_string();
        if !name.is_empty() {
            imports.insert((None, name, None));
        }
    }

    Ok(imports
        .into_iter()
        .map(|(library, name, address)| BinaryImport {
            library,
            name,
            address,
        })
        .collect())
}

fn collect_ascii_strings(bytes: &[u8]) -> Vec<BinaryString> {
    let mut strings = Vec::new();
    let mut start = None;

    for (index, byte) in bytes.iter().enumerate() {
        if byte.is_ascii_graphic() || *byte == b' ' {
            start.get_or_insert(index);
            continue;
        }
        flush_ascii_string(bytes, &mut strings, start.take(), index);
    }
    flush_ascii_string(bytes, &mut strings, start.take(), bytes.len());

    strings.sort_by(|left, right| {
        left.address
            .cmp(&right.address)
            .then_with(|| left.value.cmp(&right.value))
    });
    strings
}

fn flush_ascii_string(
    bytes: &[u8],
    strings: &mut Vec<BinaryString>,
    start: Option<usize>,
    end: usize,
) {
    let Some(start) = start else {
        return;
    };
    if end.saturating_sub(start) < MIN_ASCII_STRING_LEN {
        return;
    }
    let value = String::from_utf8_lossy(&bytes[start..end]).to_string();
    strings.push(BinaryString {
        address: Some(format!("file+{}", hex_addr(start as u64))),
        value,
        encoding: "ascii".to_string(),
    });
}

fn section_permissions<'data, S: ObjectSection<'data>>(section: &S) -> Vec<String> {
    let mut permissions = BTreeSet::new();

    match section.flags() {
        SectionFlags::Elf { sh_flags } => {
            if sh_flags & object::elf::SHF_ALLOC as u64 != 0 {
                permissions.insert("alloc".to_string());
                permissions.insert("read".to_string());
            }
            if sh_flags & object::elf::SHF_WRITE as u64 != 0 {
                permissions.insert("write".to_string());
            }
            if sh_flags & object::elf::SHF_EXECINSTR as u64 != 0 {
                permissions.insert("execute".to_string());
            }
        }
        SectionFlags::None => {}
        other => {
            permissions.insert(format!("{other:?}"));
        }
    }

    match section.kind() {
        SectionKind::Text => {
            permissions.insert("execute".to_string());
        }
        SectionKind::Data
        | SectionKind::UninitializedData
        | SectionKind::Tls
        | SectionKind::UninitializedTls
        | SectionKind::TlsVariables => {
            permissions.insert("write".to_string());
        }
        SectionKind::ReadOnlyData | SectionKind::ReadOnlyString | SectionKind::OtherString => {
            permissions.insert("read".to_string());
        }
        _ => {}
    }

    if permissions.is_empty() {
        permissions.insert("metadata".to_string());
    }
    permissions.into_iter().collect()
}

fn relocation_target_name(file: &object::File<'_>, target: RelocationTarget) -> String {
    match target {
        RelocationTarget::Symbol(index) => file
            .symbol_by_index(index)
            .ok()
            .and_then(|symbol| symbol.name().ok().map(str::to_string))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("symbol:{index}")),
        RelocationTarget::Section(index) => file
            .section_by_index(index)
            .ok()
            .and_then(|section| section.name().ok().map(str::to_string))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("section:{index}")),
        RelocationTarget::Absolute => "absolute".to_string(),
        _ => format!("{target:?}"),
    }
}

fn symbol_kind(kind: SymbolKind, is_undefined: bool) -> String {
    if is_undefined {
        "Import".to_string()
    } else {
        format!("{kind:?}")
    }
}

fn normalize_evidence_ids(mut evidence_ids: Vec<String>) -> Vec<String> {
    evidence_ids = evidence_ids
        .into_iter()
        .map(|evidence_id| evidence_id.trim().to_string())
        .filter(|evidence_id| !evidence_id.is_empty())
        .collect();
    if evidence_ids.is_empty() {
        evidence_ids.push("native-loader:input".to_string());
    }
    evidence_ids.sort();
    evidence_ids.dedup();
    evidence_ids
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hex_addr(value: u64) -> String {
    format!("0x{value:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_loader_rejects_empty_input() {
        let error = load_native_binary(&[], Vec::new()).unwrap_err();
        assert_eq!(error, NativeLoaderError::EmptyInput);
    }
}
