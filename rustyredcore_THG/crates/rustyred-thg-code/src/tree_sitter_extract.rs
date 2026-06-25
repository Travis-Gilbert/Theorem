use std::collections::{BTreeMap, BTreeSet};

use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};

use super::{community_id, identifier_tokens, symbol_node_id, IndexedSymbol, DEFAULT_TRUST_TIER};

struct LanguageSpec {
    language: Language,
    query: &'static str,
}

struct DefinitionCapture<'tree> {
    kind: String,
    node: Node<'tree>,
    name_node: Node<'tree>,
}

struct ReferenceCapture {
    kind: String,
    start_byte: usize,
    name: String,
}

pub(super) fn extract_symbols(
    repo_id: &str,
    file_id: &str,
    file_path: &str,
    language: &str,
    text: &str,
) -> Option<Vec<IndexedSymbol>> {
    let spec = language_spec(language, file_path)?;
    let mut parser = Parser::new();
    parser.set_language(&spec.language).ok()?;
    let tree = parser.parse(text, None)?;
    if tree.root_node().has_error() {
        return None;
    }

    let query = Query::new(&spec.language, spec.query).ok()?;
    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), text.as_bytes());
    let mut definitions = Vec::new();
    let mut references = Vec::new();

    while let Some(query_match) = matches.next() {
        let mut name_node = None;
        let mut definition = None;
        let mut reference_capture = None;

        for capture in query_match.captures {
            let capture_name = capture_names
                .get(capture.index as usize)
                .copied()
                .unwrap_or_default();
            if capture_name == "name" {
                name_node = Some(capture.node);
            } else if let Some(kind) = capture_name.strip_prefix("definition.") {
                definition = Some((kind.to_string(), capture.node));
            } else if let Some(kind) = capture_name.strip_prefix("reference.") {
                reference_capture = Some((kind.to_string(), capture.node.start_byte()));
            }
        }

        if let (Some((kind, node)), Some(name_node)) = (definition, name_node) {
            definitions.push(DefinitionCapture {
                kind: normalize_definition_kind(language, &kind, node),
                node,
                name_node,
            });
            continue;
        }

        if let (Some((kind, start_byte)), Some(name_node)) = (reference_capture, name_node) {
            if let Ok(name) = name_node.utf8_text(text.as_bytes()) {
                let name = clean_name(name);
                if !name.is_empty() {
                    references.push(ReferenceCapture {
                        kind,
                        start_byte,
                        name,
                    });
                }
            }
        }
    }

    if definitions.is_empty() {
        return None;
    }

    definitions.sort_by_key(|definition| {
        (
            definition.node.start_byte(),
            definition.name_node.start_byte(),
            definition.kind.clone(),
        )
    });
    definitions.dedup_by(|a, b| {
        a.kind == b.kind
            && a.name_node.start_byte() == b.name_node.start_byte()
            && a.node.start_byte() == b.node.start_byte()
    });

    let mut call_references_by_definition_start = BTreeMap::<usize, BTreeSet<String>>::new();
    let mut dependency_references_by_definition_start = BTreeMap::<usize, BTreeSet<String>>::new();
    for reference in &references {
        if let Some(definition) = definitions
            .iter()
            .rev()
            .find(|definition| definition.node.start_byte() <= reference.start_byte)
        {
            let target = if reference.kind == "call" {
                &mut call_references_by_definition_start
            } else {
                &mut dependency_references_by_definition_start
            };
            target
                .entry(definition.node.start_byte())
                .or_default()
                .insert(reference.name.clone());
        }
    }

    let lines = text.lines().collect::<Vec<_>>();
    let mut symbols = Vec::with_capacity(definitions.len());
    for (idx, definition) in definitions.iter().enumerate() {
        let Ok(name) = definition.name_node.utf8_text(text.as_bytes()) else {
            continue;
        };
        let name = clean_name(name);
        if name.is_empty() {
            continue;
        }

        let next_start = definitions
            .get(idx + 1)
            .map(|next| next.node.start_byte())
            .unwrap_or(text.len());
        let start_byte = definition.node.start_byte();
        let node_end = definition.node.end_byte();
        let end_byte = if node_end > start_byte {
            node_end
        } else {
            next_start
        }
        .min(text.len());
        let body = text
            .get(start_byte..end_byte)
            .unwrap_or_default()
            .trim_end()
            .to_string();
        let line = definition.node.start_position().row as u64 + 1;
        let signature = signature_at(&lines, line, &body);
        let mut call_names = call_references_by_definition_start
            .remove(&definition.node.start_byte())
            .unwrap_or_else(|| inferred_body_names(&body, &name));
        call_names.remove(&name);
        let mut dependency_names = dependency_references_by_definition_start
            .remove(&definition.node.start_byte())
            .unwrap_or_default();
        dependency_names.remove(&name);

        symbols.push(IndexedSymbol {
            symbol_id: symbol_node_id(repo_id, file_path, &definition.kind, &name, line),
            file_id: file_id.to_string(),
            file_path: file_path.to_string(),
            kind: definition.kind.clone(),
            name: name.clone(),
            language: language.to_string(),
            line,
            signature: signature.clone(),
            snippet: signature,
            body,
            trust_tier: DEFAULT_TRUST_TIER.to_string(),
            community_id: community_id(repo_id, language, &definition.kind),
            call_names,
            dependency_names,
            parser_backed: true,
        });
    }

    if symbols.is_empty() {
        None
    } else {
        Some(symbols)
    }
}

fn language_spec(language: &str, file_path: &str) -> Option<LanguageSpec> {
    match language {
        "javascript" => Some(LanguageSpec {
            language: tree_sitter_javascript::LANGUAGE.into(),
            query: tree_sitter_javascript::TAGS_QUERY,
        }),
        "python" => Some(LanguageSpec {
            language: tree_sitter_python::LANGUAGE.into(),
            query: tree_sitter_python::TAGS_QUERY,
        }),
        "rust" => Some(LanguageSpec {
            language: tree_sitter_rust::LANGUAGE.into(),
            query: tree_sitter_rust::TAGS_QUERY,
        }),
        "typescript" => Some(LanguageSpec {
            language: if file_path.ends_with(".tsx") {
                tree_sitter_typescript::LANGUAGE_TSX.into()
            } else {
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
            },
            query: TYPESCRIPT_TAGS_QUERY,
        }),
        _ => None,
    }
}

fn signature_at(lines: &[&str], line: u64, body: &str) -> String {
    lines
        .get(line.saturating_sub(1) as usize)
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .or_else(|| body.lines().next().map(|line| line.trim().to_string()))
        .unwrap_or_default()
}

fn clean_name(name: &str) -> String {
    name.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_matches('`')
        .to_string()
}

fn normalize_definition_kind(language: &str, raw_kind: &str, node: Node<'_>) -> String {
    if language != "rust" {
        return raw_kind.to_string();
    }
    match node.kind() {
        "enum_item" => "enum".to_string(),
        "struct_item" => "struct".to_string(),
        "trait_item" => "trait".to_string(),
        "type_item" => "type".to_string(),
        "union_item" => "union".to_string(),
        _ if raw_kind == "interface" => "trait".to_string(),
        _ => raw_kind.to_string(),
    }
}

fn inferred_body_names(body: &str, symbol_name: &str) -> BTreeSet<String> {
    identifier_tokens(body)
        .into_iter()
        .filter(|token| *token != symbol_name)
        .filter(|token| !is_low_signal_token(token))
        .map(str::to_string)
        .collect()
}

fn is_low_signal_token(token: &str) -> bool {
    matches!(
        token,
        "async"
            | "await"
            | "class"
            | "const"
            | "def"
            | "else"
            | "export"
            | "false"
            | "fn"
            | "for"
            | "function"
            | "if"
            | "impl"
            | "interface"
            | "let"
            | "new"
            | "None"
            | "null"
            | "pub"
            | "return"
            | "Self"
            | "self"
            | "struct"
            | "this"
            | "trait"
            | "true"
            | "type"
            | "var"
            | "void"
    )
}

const TYPESCRIPT_TAGS_QUERY: &str = r#"
(function_signature
  name: (identifier) @name) @definition.function

(function_declaration
  name: (identifier) @name) @definition.function

(method_signature
  name: (property_identifier) @name) @definition.method

(method_definition
  name: (property_identifier) @name) @definition.method

(abstract_method_signature
  name: (property_identifier) @name) @definition.method

(class_declaration
  name: (type_identifier) @name) @definition.class

(abstract_class_declaration
  name: (type_identifier) @name) @definition.class

(module
  name: (identifier) @name) @definition.module

(interface_declaration
  name: (type_identifier) @name) @definition.interface

(type_alias_declaration
  name: (type_identifier) @name) @definition.type

(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function_expression)])) @definition.function

(call_expression
  function: [
    (identifier) @name
    (member_expression
      property: (property_identifier) @name)
  ]) @reference.call

(new_expression
  constructor: (identifier) @name) @reference.class
"#;
