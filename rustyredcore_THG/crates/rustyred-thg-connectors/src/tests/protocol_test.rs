use serde_json::json;

use rustyred_thg_affordances::CONNECTOR_FAMILY;

use crate::protocol::{
    connector_manifest, parse_initialize, parse_tool_call_result, parse_tools_list,
    tool_manifest_from_descriptor, ToolDescriptor, CONTENT_EXTRACTION_FAMILY,
};

#[test]
fn parses_tools_list_into_descriptors() {
    let result = json!({
        "tools": [
            {
                "name": "search",
                "description": "Search the web",
                "inputSchema": { "type": "object", "properties": { "q": { "type": "string" } } }
            },
            { "name": "fetch", "description": "Fetch a URL" },
            { "description": "nameless tool is skipped" }
        ]
    });
    let descriptors = parse_tools_list(&result).expect("parse");
    assert_eq!(descriptors.len(), 2);
    assert_eq!(descriptors[0].name, "search");
    assert_eq!(descriptors[0].description, "Search the web");
    assert_eq!(
        descriptors[0].input_schema["properties"]["q"]["type"],
        json!("string")
    );
    assert_eq!(descriptors[1].name, "fetch");
    assert_eq!(descriptors[1].input_schema, json!({}));
}

#[test]
fn missing_tools_array_is_a_protocol_error() {
    let result = json!({ "not_tools": [] });
    assert!(parse_tools_list(&result).is_err());
}

#[test]
fn descriptor_maps_to_tool_manifest_with_no_embedding() {
    let descriptor = ToolDescriptor {
        name: "search".into(),
        description: "Search".into(),
        input_schema: json!({ "type": "object" }),
        ..Default::default()
    };
    let manifest = tool_manifest_from_descriptor(&descriptor);
    assert_eq!(manifest.name, "search");
    assert_eq!(manifest.label, "search");
    assert_eq!(manifest.description, "Search");
    assert_eq!(manifest.family, CONNECTOR_FAMILY);
    assert_eq!(manifest.input_schema, json!({ "type": "object" }));
    assert!(manifest.description_embedding.is_none());
    assert!(manifest.permissions.is_empty());
}

#[test]
fn annotations_map_to_writeback_policy() {
    let result = json!({
        "tools": [
            { "name": "read_file", "annotations": { "readOnlyHint": true } },
            { "name": "delete_file", "annotations": { "destructiveHint": true } },
            { "name": "write_file", "annotations": { "readOnlyHint": false } },
            { "name": "mystery" }
        ]
    });
    let d = parse_tools_list(&result).expect("parse");
    assert_eq!(d.len(), 4);
    assert_eq!(
        tool_manifest_from_descriptor(&d[0]).writeback_policy,
        "read-only"
    );
    assert_eq!(
        tool_manifest_from_descriptor(&d[1]).writeback_policy,
        "destructive"
    );
    assert_eq!(
        tool_manifest_from_descriptor(&d[2]).writeback_policy,
        "write"
    );
    // No annotation maps to "unknown", NOT "read-only": the firing gate must not
    // assume a tool with no declared side-effect profile is safe to auto-fire.
    assert_eq!(
        tool_manifest_from_descriptor(&d[3]).writeback_policy,
        "unknown"
    );
}

#[test]
fn connector_manifest_assembles_full_catalog() {
    let descriptors = vec![
        ToolDescriptor {
            name: "a".into(),
            description: String::new(),
            input_schema: json!({}),
            ..Default::default()
        },
        ToolDescriptor {
            name: "b".into(),
            description: String::new(),
            input_schema: json!({}),
            ..Default::default()
        },
    ];
    let manifest = connector_manifest("acme", "websearch", "Web Search", &descriptors);
    assert_eq!(manifest.tenant_id, "acme");
    assert_eq!(manifest.server_id, "websearch");
    assert_eq!(manifest.label, "Web Search");
    assert_eq!(manifest.tools.len(), 2);
}

#[test]
fn content_core_manifest_surfaces_content_extraction_family_and_guidance() {
    let descriptors = vec![
        ToolDescriptor {
            name: "extract_content".into(),
            description: "Extract content from a URL or file.".into(),
            input_schema: json!({ "type": "object" }),
            ..Default::default()
        },
        ToolDescriptor {
            name: "summarize_content".into(),
            description: "Summarize content.".into(),
            input_schema: json!({ "type": "object" }),
            ..Default::default()
        },
    ];
    let manifest = connector_manifest("acme", "content-core", "Content Core", &descriptors);
    assert_eq!(manifest.tools.len(), 2);
    assert!(manifest
        .tools
        .iter()
        .all(|tool| tool.writeback_policy == "read-only"));
    assert!(manifest
        .tools
        .iter()
        .all(|tool| tool.family == CONTENT_EXTRACTION_FAMILY));
    let extract = manifest
        .tools
        .iter()
        .find(|tool| tool.name == "extract_content")
        .expect("extract tool");
    assert_eq!(extract.family, CONTENT_EXTRACTION_FAMILY);
    assert!(!extract
        .tags
        .contains(&CONTENT_EXTRACTION_FAMILY.to_string()));
    assert!(extract.tags.contains(&"document".to_string()));
    assert!(extract.tags.contains(&"media".to_string()));
    assert!(extract
        .description
        .contains("Images and screenshots stay on the vision spine"));
}

#[test]
fn parses_initialize_and_tool_call_result() {
    let init = json!({
        "protocolVersion": "2025-06-18",
        "serverInfo": { "name": "everything", "version": "1.2.0" }
    });
    let info = parse_initialize(&init);
    assert_eq!(info.server_name, "everything");
    assert_eq!(info.server_version, "1.2.0");
    assert_eq!(info.protocol_version, "2025-06-18");

    let call = json!({
        "content": [
            { "type": "text", "text": "hello" },
            { "type": "text", "text": "world" }
        ],
        "isError": false
    });
    let outcome = parse_tool_call_result(&call);
    assert!(!outcome.is_error);
    assert_eq!(outcome.text, "hello\nworld");
}
