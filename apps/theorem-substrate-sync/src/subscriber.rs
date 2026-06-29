use serde_json::{json, Value};

use crate::cursor::{CursorStore, SyncCursors};
use crate::railway_client::McpClient;
use crate::round::apply_snapshot;
use crate::status::{StatusHandle, StreamState};
use crate::{Result, SyncError};

pub async fn subscribe(remote: &McpClient, tenant: &str) -> Result<()> {
    remote
        .call_tool(
            "stream_subscribe",
            json!({
                "actor": "theorem-substrate-sync",
                "stream": format!("tenant:{tenant}")
            }),
        )
        .await?;
    Ok(())
}

pub async fn read_and_apply_once(
    local: &McpClient,
    remote: &McpClient,
    cursors: &dyn CursorStore,
    tenant: &str,
    status: &StatusHandle,
) -> Result<usize> {
    let read = remote
        .call_tool(
            "stream_read",
            json!({
                "actor": "theorem-substrate-sync",
                "stream": format!("tenant:{tenant}"),
                "advance": true,
                "limit": 100
            }),
        )
        .await;
    let read = match read {
        Ok(read) => read,
        Err(error) => {
            status
                .update(|status| {
                    status.stream = StreamState::Disconnected;
                    status.warnings.push(format!("stream read failed: {error}"));
                })
                .await;
            return Err(error);
        }
    };

    let events = read
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for event in &events {
        apply_stream_event(local, event).await?;
    }
    let stream_key = format!("tenant:{tenant}");
    let cursor = read
        .get("new_cursors")
        .and_then(|cursors| cursors.get(&stream_key))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut saved = cursors.load(tenant)?;
    saved.stream_cursor = saved.stream_cursor.max(cursor);
    cursors.save(tenant, &saved)?;
    status
        .update(|status| {
            status.stream = StreamState::Connected;
            if let Some(last) = events
                .last()
                .and_then(|event| event.get("id"))
                .and_then(Value::as_str)
            {
                status.last_event = Some(last.to_string());
            }
        })
        .await;
    Ok(events.len())
}

pub async fn apply_stream_event(local: &McpClient, event: &Value) -> Result<bool> {
    let Some(snapshot) = snapshot_from_stream_event(event) else {
        return Ok(false);
    };
    apply_snapshot(local, &snapshot).await?;
    Ok(true)
}

pub fn snapshot_from_stream_event(event: &Value) -> Option<Value> {
    let payload = event.get("payload")?;
    if let Some(snapshot) = payload.get("snapshot") {
        return Some(snapshot.clone());
    }

    let delta = payload.get("property_delta")?;
    let record = delta.get("record")?.clone();
    match delta.get("record_kind").and_then(Value::as_str) {
        Some("node") => Some(json!({ "nodes": [record], "edges": [] })),
        Some("edge") => Some(json!({ "nodes": [], "edges": [record] })),
        _ => None,
    }
}

pub fn stream_retention_gap(cursors: &SyncCursors, first_available: u64) -> Option<SyncError> {
    (cursors.stream_cursor > 0 && cursors.stream_cursor < first_available).then(|| {
        SyncError::Mcp(format!(
            "saved stream cursor {} is older than first available {first_available}",
            cursors.stream_cursor
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_node_mutation_event_becomes_applyable_snapshot() {
        let event = json!({
            "payload": {
                "tenant": "Travis-Gilbert",
                "op_kind": "node_upserted",
                "property_delta": {
                    "record_kind": "node",
                    "record": {
                        "id": "mem:a",
                        "labels": ["MemoryDocument"],
                        "properties": { "status": "active" },
                        "version": 1,
                        "tombstone": false
                    }
                }
            }
        });

        let snapshot = snapshot_from_stream_event(&event).expect("snapshot");

        assert_eq!(snapshot["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(snapshot["nodes"][0]["id"], "mem:a");
        assert_eq!(snapshot["edges"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn snapshot_payload_still_passes_through() {
        let event = json!({
            "payload": {
                "snapshot": {
                    "nodes": [{ "id": "mem:a", "labels": ["MemoryDocument"], "properties": {} }],
                    "edges": []
                }
            }
        });

        let snapshot = snapshot_from_stream_event(&event).expect("snapshot");

        assert_eq!(snapshot["nodes"][0]["id"], "mem:a");
    }
}
