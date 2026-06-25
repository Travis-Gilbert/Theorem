use std::collections::{BTreeMap, BTreeSet};

use rustyred_thg_core::{
    now_ms, stable_hash, EdgeRecord, GraphStore, GraphStoreResult, NeighborQuery, NodeQuery,
    NodeRecord,
};
use serde_json::{json, Value};

use super::ir::{
    CodeDependencySnapshot, CodeFileSnapshot, CodeSpecCompileInput, CodeSpecCompileOutput,
    CodeSymbolSnapshot, CODE_COMPILER_FEATURE_VERSION, CODE_COMPILER_VERSION, CODE_SPEC_LABEL,
    SPECIFIES_CODE,
};
use crate::{
    property_bool, property_string, property_string_set, property_u64, CALLS_SYMBOL,
    CODE_FILE_LABEL, CODE_SYMBOL_LABEL, DEPENDS_ON_SYMBOL, SOURCE,
};

pub fn compile_code_spec_snapshot<S: GraphStore>(
    store: &S,
    input: &CodeSpecCompileInput,
) -> GraphStoreResult<CodeSpecCompileOutput> {
    let files = collect_code_files(store, input);
    let symbols = collect_code_symbols(
        store,
        &input.tenant_id,
        &input.repo_id,
        input.symbol_limit(),
    );
    let dependency_edges = collect_dependency_edges(store, &symbols);
    let repo_label = input
        .repo_label
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&input.repo_id);
    let title = input
        .spec_title
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("Compiled code spec for {repo_label}"));
    let spec_id = input.spec_id.clone().unwrap_or_else(|| {
        format!(
            "code:spec:{}",
            stable_hash(json!([&input.tenant_id, &input.repo_id, &title]))
        )
    });
    let structure_count = symbols
        .iter()
        .filter(|symbol| is_structure_kind(&symbol.kind))
        .count();
    let member_count = symbols.len().saturating_sub(structure_count);
    let spec_body = render_spec_body(&title, &input.repo_id, &files, &symbols, &dependency_edges);
    let artifact_hash = stable_hash(json!({
        "compiler_version": CODE_COMPILER_VERSION,
        "feature_version": CODE_COMPILER_FEATURE_VERSION,
        "tenant_id": &input.tenant_id,
        "repo_id": &input.repo_id,
        "files": &files,
        "symbols": &symbols,
        "dependency_edges": &dependency_edges,
        "spec_body": &spec_body,
    }));
    let spec_node = NodeRecord::new(
        &spec_id,
        [CODE_SPEC_LABEL],
        json!({
            "tenant_id": &input.tenant_id,
            "repo_id": &input.repo_id,
            "repo_label": repo_label,
            "title": &title,
            "body": &spec_body,
            "compiler_version": CODE_COMPILER_VERSION,
            "feature_version": CODE_COMPILER_FEATURE_VERSION,
            "artifact_hash": &artifact_hash,
            "file_count": files.len(),
            "symbol_count": symbols.len(),
            "structure_count": structure_count,
            "member_count": member_count,
            "dependency_edge_count": dependency_edges.len(),
            "compiled_files": &files,
            "compiled_symbols": &symbols,
            "compiled_dependency_edges": &dependency_edges,
            "compiled_at_ms": now_ms(),
            "source": SOURCE,
        }),
    );
    let spec_edges = symbols
        .iter()
        .map(|symbol| {
            EdgeRecord::new(
                spec_symbol_edge_id(&spec_id, &symbol.symbol_id),
                &spec_id,
                SPECIFIES_CODE,
                &symbol.symbol_id,
                json!({
                    "tenant_id": &input.tenant_id,
                    "repo_id": &input.repo_id,
                    "compiler_version": CODE_COMPILER_VERSION,
                    "feature_version": CODE_COMPILER_FEATURE_VERSION,
                    "symbol_key": symbol.symbol_key(),
                    "source": SOURCE,
                }),
            )
        })
        .collect::<Vec<_>>();

    Ok(CodeSpecCompileOutput {
        spec_node,
        spec_edges,
        file_count: files.len(),
        symbol_count: symbols.len(),
        structure_count,
        member_count,
        dependency_edge_count: dependency_edges.len(),
        artifact_hash,
        spec_body,
        files,
        symbols,
        dependency_edges,
    })
}

pub fn compile_code_spec_in_store<S: GraphStore>(
    store: &mut S,
    input: CodeSpecCompileInput,
) -> GraphStoreResult<CodeSpecCompileOutput> {
    let output = compile_code_spec_snapshot(store, &input)?;
    store.upsert_node(output.spec_node.clone())?;
    for edge in &output.spec_edges {
        store.upsert_edge(edge.clone())?;
    }
    Ok(output)
}

pub(super) fn collect_code_symbols<S: GraphStore>(
    store: &S,
    tenant_id: &str,
    repo_id: &str,
    limit: usize,
) -> Vec<CodeSymbolSnapshot> {
    let mut symbols = store
        .query_nodes(
            NodeQuery::label(CODE_SYMBOL_LABEL)
                .with_property("tenant_id", json!(tenant_id))
                .with_property("repo_id", json!(repo_id))
                .with_limit(limit),
        )
        .into_iter()
        .filter(|node| !node.tombstone)
        .filter_map(|node| symbol_snapshot(&node))
        .collect::<Vec<_>>();
    symbols.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.symbol_id.cmp(&right.symbol_id))
    });
    symbols
}

fn collect_code_files<S: GraphStore>(
    store: &S,
    input: &CodeSpecCompileInput,
) -> Vec<CodeFileSnapshot> {
    let mut files = store
        .query_nodes(
            NodeQuery::label(CODE_FILE_LABEL)
                .with_property("tenant_id", json!(&input.tenant_id))
                .with_property("repo_id", json!(&input.repo_id))
                .with_limit(100_000),
        )
        .into_iter()
        .filter(|node| !node.tombstone)
        .filter_map(|node| file_snapshot(&node))
        .collect::<Vec<_>>();
    files.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.file_id.cmp(&right.file_id))
    });
    files
}

fn collect_dependency_edges<S: GraphStore>(
    store: &S,
    symbols: &[CodeSymbolSnapshot],
) -> Vec<CodeDependencySnapshot> {
    let symbol_ids = symbols
        .iter()
        .map(|symbol| symbol.symbol_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut dependencies = Vec::new();
    for symbol in symbols {
        for edge_type in [CALLS_SYMBOL, DEPENDS_ON_SYMBOL] {
            for hit in store
                .neighbors(NeighborQuery::out(&symbol.symbol_id).with_edge_type(edge_type))
                .into_iter()
                .filter(|hit| symbol_ids.contains(hit.node_id.as_str()))
            {
                let key = (
                    symbol.symbol_id.clone(),
                    hit.node_id.clone(),
                    hit.edge_type.clone(),
                );
                if !seen.insert(key.clone()) {
                    continue;
                }
                dependencies.push(CodeDependencySnapshot {
                    from_symbol_id: key.0,
                    to_symbol_id: key.1,
                    edge_type: key.2,
                });
            }
        }
    }
    dependencies.sort_by(|left, right| {
        left.from_symbol_id
            .cmp(&right.from_symbol_id)
            .then_with(|| left.edge_type.cmp(&right.edge_type))
            .then_with(|| left.to_symbol_id.cmp(&right.to_symbol_id))
    });
    dependencies
}

fn file_snapshot(node: &NodeRecord) -> Option<CodeFileSnapshot> {
    Some(CodeFileSnapshot {
        file_id: property_string(&node.properties, "file_id").unwrap_or_else(|| node.id.clone()),
        path: property_string(&node.properties, "path")?,
        language: property_string(&node.properties, "language").unwrap_or_default(),
        content_hash: non_empty_property(&node.properties, "content_hash"),
    })
}

fn symbol_snapshot(node: &NodeRecord) -> Option<CodeSymbolSnapshot> {
    let mut call_names = property_string_set(&node.properties, "call_names")
        .into_iter()
        .collect::<Vec<_>>();
    let mut dependency_names = property_string_set(&node.properties, "dependency_names")
        .into_iter()
        .collect::<Vec<_>>();
    call_names.sort();
    dependency_names.sort();

    Some(CodeSymbolSnapshot {
        symbol_id: property_string(&node.properties, "symbol_id")
            .unwrap_or_else(|| node.id.clone()),
        file_id: non_empty_property(&node.properties, "file_id"),
        file_path: property_string(&node.properties, "file_path")?,
        kind: property_string(&node.properties, "kind").unwrap_or_else(|| "symbol".to_string()),
        name: property_string(&node.properties, "name").unwrap_or_else(|| node.id.clone()),
        language: property_string(&node.properties, "language").unwrap_or_default(),
        line: property_u64(&node.properties, "line"),
        signature: non_empty_property(&node.properties, "signature"),
        call_names,
        dependency_names,
        parser_backed: property_bool(&node.properties, "parser_backed"),
    })
}

fn render_spec_body(
    title: &str,
    repo_id: &str,
    files: &[CodeFileSnapshot],
    symbols: &[CodeSymbolSnapshot],
    dependency_edges: &[CodeDependencySnapshot],
) -> String {
    let mut symbols_by_file: BTreeMap<&str, Vec<&CodeSymbolSnapshot>> = BTreeMap::new();
    for symbol in symbols {
        symbols_by_file
            .entry(symbol.file_path.as_str())
            .or_default()
            .push(symbol);
    }

    let mut lines = vec![
        format!("# {title}"),
        String::new(),
        format!("Repository: `{repo_id}`"),
        format!("Compiler: `{CODE_COMPILER_VERSION}`"),
        String::new(),
        "## Module Inventory".to_string(),
    ];
    if files.is_empty() {
        lines.push("- No files were present in the code graph.".to_string());
    } else {
        for file in files {
            let symbol_count = symbols_by_file
                .get(file.path.as_str())
                .map(|items| items.len())
                .unwrap_or(0);
            lines.push(format!(
                "- `{}` ({}) - {} symbols",
                file.path, file.language, symbol_count
            ));
        }
    }

    lines.push(String::new());
    lines.push("## Structure Catalog".to_string());
    let structures = symbols
        .iter()
        .filter(|symbol| is_structure_kind(&symbol.kind))
        .collect::<Vec<_>>();
    if structures.is_empty() {
        lines.push("- No structure symbols were present in the code graph.".to_string());
    } else {
        for symbol in structures {
            lines.push(format_symbol_line(symbol));
        }
    }

    lines.push(String::new());
    lines.push("## Symbol Catalog".to_string());
    if symbols.is_empty() {
        lines.push("- No symbols were present in the code graph.".to_string());
    } else {
        for symbol in symbols {
            lines.push(format_symbol_line(symbol));
        }
    }

    lines.push(String::new());
    lines.push("## Symbol Dependency Graph".to_string());
    if dependency_edges.is_empty() {
        lines.push("- No call or dependency edges were present in the code graph.".to_string());
    } else {
        for edge in dependency_edges {
            lines.push(format!(
                "- `{}` {} `{}`",
                edge.from_symbol_id, edge.edge_type, edge.to_symbol_id
            ));
        }
    }

    lines.push(String::new());
    lines.join("\n")
}

pub(super) fn is_structure_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class" | "enum" | "impl" | "interface" | "mod" | "module" | "struct" | "trait" | "type"
    )
}

fn format_symbol_line(symbol: &CodeSymbolSnapshot) -> String {
    let line = symbol
        .line
        .map(|line| format!(":{}", line))
        .unwrap_or_default();
    let signature = symbol
        .signature
        .as_ref()
        .map(|signature| format!(" - `{signature}`"))
        .unwrap_or_default();
    format!(
        "- `{}` `{}` in `{}`{}{}",
        symbol.kind, symbol.name, symbol.file_path, line, signature
    )
}

fn spec_symbol_edge_id(spec_id: &str, symbol_id: &str) -> String {
    format!(
        "code:edge:spec_symbol:{}",
        stable_hash(json!([spec_id, symbol_id]))
    )
}

fn non_empty_property(properties: &Value, key: &str) -> Option<String> {
    property_string(properties, key).filter(|value| !value.trim().is_empty())
}
