//! MCP access over the consumer store (plan unit AG3).
//!
//! Any model reaches the store over MCP, so the workspace is not tied to one
//! frontend model. This is the protocol core: a JSON-RPC 2.0 dispatcher over the
//! [`Commonplace`] store. The MCP stdio transport is newline-delimited JSON-RPC,
//! so the `commonplace-mcp` binary just pipes lines through [`handle_request`].
//!
//! Tools: `put_note` + `ingest` (write), `get_item` + `list_items` + `search`
//! (read). Tool execution failures come back as `isError: true` content (the
//! model sees them); unknown methods are JSON-RPC errors.

use commonplace::{
    BlobStore, Commonplace, EmbeddingGraphStore, IngestInput, IngestPipeline, Item, ItemBody,
    ItemKind,
};
use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2024-11-05";

/// Dispatch one JSON-RPC request against the store. Returns the JSON-RPC
/// response, or `None` for notifications (requests without an `id`).
pub fn handle_request<S, B>(cp: &mut Commonplace<S, B>, request: &Value) -> Option<Value>
where
    S: EmbeddingGraphStore,
    B: BlobStore,
{
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");

    match method {
        "initialize" => Some(ok(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "commonplace-mcp", "version": env!("CARGO_PKG_VERSION") },
            }),
        )),
        "notifications/initialized" => None,
        "ping" => Some(ok(id, json!({}))),
        "tools/list" => Some(ok(id, json!({ "tools": tool_specs() }))),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let result = match call_tool(cp, name, &arguments) {
                Ok(value) => json!({
                    "content": [ { "type": "text", "text": value.to_string() } ],
                    "isError": false,
                }),
                Err(message) => json!({
                    "content": [ { "type": "text", "text": message } ],
                    "isError": true,
                }),
            };
            Some(ok(id, result))
        }
        _ => {
            if id.is_some() {
                Some(err(id, -32601, &format!("method not found: {method}")))
            } else {
                None
            }
        }
    }
}

fn ok(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result })
}

fn err(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "error": { "code": code, "message": message } })
}

fn tool_specs() -> Value {
    json!([
        {
            "name": "put_note",
            "description": "Create a note item in the store.",
            "inputSchema": { "type": "object", "properties": {
                "title": { "type": "string" }, "text": { "type": "string" },
                "tags": { "type": "array", "items": { "type": "string" } }
            }, "required": ["title", "text"] }
        },
        {
            "name": "ingest",
            "description": "Auto-structure a document into the store: embed, classify into a collection, file, and link to similar items.",
            "inputSchema": { "type": "object", "properties": {
                "title": { "type": "string" }, "text": { "type": "string" }, "kind": { "type": "string" }
            }, "required": ["title", "text"] }
        },
        {
            "name": "get_item",
            "description": "Fetch one item by id.",
            "inputSchema": { "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] }
        },
        {
            "name": "list_items",
            "description": "List items, optionally filtered to a kind.",
            "inputSchema": { "type": "object", "properties": { "kind": { "type": "string" } } }
        },
        {
            "name": "search",
            "description": "Similarity search over items.",
            "inputSchema": { "type": "object", "properties": {
                "query": { "type": "string" }, "k": { "type": "integer" }
            }, "required": ["query"] }
        }
    ])
}

fn call_tool<S, B>(cp: &mut Commonplace<S, B>, name: &str, args: &Value) -> Result<Value, String>
where
    S: EmbeddingGraphStore,
    B: BlobStore,
{
    match name {
        "put_note" => {
            let title = str_arg(args, "title")?;
            let text = str_arg(args, "text")?;
            let mut item = Item::note(title, text);
            if let Some(tags) = args.get("tags").and_then(Value::as_array) {
                item = item.with_tags(
                    tags.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>(),
                );
            }
            let stored = cp.put_item(item).map_err(store_err)?;
            Ok(lean_item(&stored))
        }
        "ingest" => {
            let title = str_arg(args, "title")?;
            let text = str_arg(args, "text")?;
            let kind = args
                .get("kind")
                .and_then(Value::as_str)
                .map(|kind| ItemKind::from(kind.to_string()))
                .unwrap_or(ItemKind::Doc);
            let receipt = IngestPipeline::default()
                .ingest(cp, IngestInput::text(title, text, kind))
                .map_err(store_err)?;
            Ok(lean_item(&receipt.item))
        }
        "get_item" => {
            let id = str_arg(args, "id")?;
            match cp.get_item(&id).map_err(store_err)? {
                Some(item) => Ok(lean_item(&item)),
                None => Ok(Value::Null),
            }
        }
        "list_items" => {
            let items = match args.get("kind").and_then(Value::as_str) {
                Some(kind) => cp
                    .items_by_kind(&ItemKind::from(kind.to_string()))
                    .map_err(store_err)?,
                None => cp.all_items().map_err(store_err)?,
            };
            Ok(Value::Array(items.iter().map(lean_item).collect()))
        }
        "search" => {
            let query = str_arg(args, "query")?;
            let k = args.get("k").and_then(Value::as_u64).unwrap_or(10).max(1) as usize;
            let hits = IngestPipeline::default()
                .search(cp, &query, k)
                .map_err(store_err)?;
            let mut results = Vec::with_capacity(hits.len());
            for (id, distance) in hits {
                if let Some(item) = cp.get_item(&id).map_err(store_err)? {
                    let mut value = lean_item(&item);
                    if let Some(object) = value.as_object_mut() {
                        object.insert("score".to_string(), json!(1.0 - distance as f64));
                    }
                    results.push(value);
                }
            }
            Ok(Value::Array(results))
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

fn str_arg(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing required string argument: {key}"))
}

fn store_err(error: rustyred_thg_core::GraphStoreError) -> String {
    format!("{error:?}")
}

fn lean_item(item: &Item) -> Value {
    let body = match &item.body {
        ItemBody::Inline { text } => Some(text.clone()),
        ItemBody::Blob { content_hash, .. } => Some(format!("[blob {content_hash}]")),
        ItemBody::Empty => None,
    };
    json!({
        "id": item.id,
        "kind": item.kind.as_str(),
        "title": item.title,
        "body": body,
        "residency": item.residency.as_str(),
        "tags": item.tags,
        "collections": item.collections,
        "classification": item.classification,
        "created_at_ms": item.created_at_ms,
        "updated_at_ms": item.updated_at_ms,
    })
}
