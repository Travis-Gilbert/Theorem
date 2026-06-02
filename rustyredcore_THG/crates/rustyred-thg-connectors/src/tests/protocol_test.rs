use serde_json::json;

use crate::protocol::{
    connector_manifest, parse_initialize, parse_tool_call_result, parse_tools_list,
    tool_manifest_from_descriptor, ToolDescriptor,
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
    };
    let manifest = tool_manifest_from_descriptor(&descriptor);
    assert_eq!(manifest.name, "search");
    assert_eq!(manifest.label, "search");
    assert_eq!(manifest.description, "Search");
    assert_eq!(manifest.input_schema, json!({ "type": "object" }));
    assert!(manifest.description_embedding.is_none());
    assert!(manifest.permissions.is_empty());
}

#[test]
fn connector_manifest_assembles_full_catalog() {
    let descriptors = vec![
        ToolDescriptor {
            name: "a".into(),
            description: String::new(),
            input_schema: json!({}),
        },
        ToolDescriptor {
            name: "b".into(),
            description: String::new(),
            input_schema: json!({}),
        },
    ];
    let manifest = connector_manifest("acme", "websearch", "Web Search", &descriptors);
    assert_eq!(manifest.tenant_id, "acme");
    assert_eq!(manifest.server_id, "websearch");
    assert_eq!(manifest.label, "Web Search");
    assert_eq!(manifest.tools.len(), 2);
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
