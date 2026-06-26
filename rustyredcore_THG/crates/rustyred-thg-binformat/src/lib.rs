//! Binary artifact loading for the Theorem reconstruction pipeline.
//!
//! This crate owns observed binary facts only: artifact identity, file format,
//! architecture, sections, symbols, relocations when available, entrypoints,
//! and printable strings. Downstream crates decode, lift, infer, and compile
//! reconstruction obligations from these facts.

use object::{Object, ObjectSection, ObjectSymbol};
use rustyred_thg_core::{
    stable_hash, EdgeRecord, GraphStore, GraphStoreError, GraphStoreResult, NodeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

pub const BINARY_ARTIFACT_LABEL: &str = "BinaryArtifact";
pub const BINARY_SECTION_LABEL: &str = "BinarySection";
pub const BINARY_SYMBOL_LABEL: &str = "BinarySymbol";
pub const BINARY_STRING_LABEL: &str = "BinaryString";
pub const BINARY_RELOCATION_LABEL: &str = "BinaryRelocation";
pub const BINARY_ENTRYPOINT_LABEL: &str = "BinaryEntrypoint";

pub const ARTIFACT_HAS_SECTION: &str = "ARTIFACT_HAS_SECTION";
pub const ARTIFACT_HAS_SYMBOL: &str = "ARTIFACT_HAS_SYMBOL";
pub const ARTIFACT_HAS_STRING: &str = "ARTIFACT_HAS_STRING";
pub const ARTIFACT_HAS_RELOCATION: &str = "ARTIFACT_HAS_RELOCATION";
pub const ARTIFACT_HAS_ENTRYPOINT: &str = "ARTIFACT_HAS_ENTRYPOINT";

pub const BINFORMAT_SOURCE: &str = "rustyred-thg-binformat";
pub const BINFORMAT_VERSION: &str = "rustyred-thg-binformat-v0";

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BinaryLoadReport {
    pub artifact: BinaryArtifact,
    pub sections: Vec<BinarySection>,
    pub symbols: Vec<BinarySymbol>,
    pub strings: Vec<BinaryString>,
    pub relocations: Vec<BinaryRelocation>,
    pub entrypoints: Vec<BinaryEntrypoint>,
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

    Ok(BinaryLoadReport {
        artifact,
        sections,
        symbols,
        strings,
        relocations: Vec::new(),
        entrypoints,
    })
}

pub fn write_binary_facts_in_store<S: GraphStore>(
    store: &mut S,
    report: &BinaryLoadReport,
) -> GraphStoreResult<()> {
    store.upsert_node(artifact_node(&report.artifact))?;
    for section in &report.sections {
        store.upsert_node(section_node(section))?;
        link_artifact(
            store,
            &report.artifact.artifact_id,
            &section.section_id,
            ARTIFACT_HAS_SECTION,
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
    }
    for string in &report.strings {
        store.upsert_node(string_node(string))?;
        link_artifact(
            store,
            &report.artifact.artifact_id,
            &string.string_id,
            ARTIFACT_HAS_STRING,
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
    }
    for entrypoint in &report.entrypoints {
        store.upsert_node(entrypoint_node(entrypoint))?;
        link_artifact(
            store,
            &report.artifact.artifact_id,
            &entrypoint.entrypoint_id,
            ARTIFACT_HAS_ENTRYPOINT,
        )?;
    }
    Ok(())
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
            relocations: Vec::new(),
            entrypoints: Vec::new(),
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
    }
}
