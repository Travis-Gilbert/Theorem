use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::railway_client::McpClient;
use crate::status::StatusHandle;
use crate::{stable_hash, Result, SyncError};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoundReceipt {
    pub local_hash: String,
    pub remote_hash: String,
    pub merged_hash: String,
    pub applied_local: bool,
    pub applied_remote: bool,
}

pub async fn run_round(
    local: &McpClient,
    remote: &McpClient,
    status: &StatusHandle,
) -> Result<RoundReceipt> {
    let local_snapshot = compile_snapshot(local).await?;
    let remote_snapshot = compile_snapshot(remote).await?;
    let local_hash = stable_hash(&local_snapshot);
    let remote_hash = stable_hash(&remote_snapshot);

    let merge = local
        .call_tool(
            "rustyred_thg_graph_version_merge",
            json!({
                "base": remote_snapshot.clone(),
                "ours": local_snapshot.clone(),
                "theirs": remote_snapshot.clone(),
                "strategy": "auto_confidence",
                "include_payloads": true
            }),
        )
        .await?;
    let merged_snapshot = merge
        .get("merge")
        .and_then(|merge| merge.get("merged_snapshot"))
        .cloned()
        .ok_or_else(|| SyncError::Mcp("merge response missing merged_snapshot".to_string()))?;
    let merged_hash = stable_hash(&merged_snapshot);

    apply_snapshot(local, &merged_snapshot).await?;
    apply_snapshot(remote, &merged_snapshot).await?;

    let receipt = RoundReceipt {
        local_hash,
        remote_hash,
        merged_hash: merged_hash.clone(),
        applied_local: true,
        applied_remote: true,
    };
    status
        .update(|status| {
            status.last_round = Some(format!("round:{merged_hash}"));
        })
        .await;
    Ok(receipt)
}

pub async fn compile_snapshot(client: &McpClient) -> Result<Value> {
    let response = client
        .call_tool(
            "rustyred_thg_graph_version_compile",
            json!({ "include_payloads": true }),
        )
        .await?;
    // `pack.manifest` is required (counts + version); `pack.objects` is NOT.
    // The server's CompiledGraphPack uses `#[serde(skip_serializing_if =
    // Vec::is_empty)]` on `objects`, so an empty graph (or any payload-less
    // pack) omits the field entirely on the wire. Treat missing as empty.
    response
        .get("pack")
        .and_then(|pack| pack.get("manifest"))
        .ok_or_else(|| SyncError::Mcp("compile response missing pack.manifest".to_string()))?;
    pack_snapshot(&response)
}

pub async fn apply_snapshot(client: &McpClient, snapshot: &Value) -> Result<()> {
    let nodes = snapshot
        .get("nodes")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let edges = snapshot
        .get("edges")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));

    client
        .call_tool(
            "graphql_mutate",
            json!({
                "query": "mutation($n:JSON!){ bulkNodes(nodes:$n){ ok inserted failed } }",
                "variables": { "n": nodes }
            }),
        )
        .await?;
    client
        .call_tool(
            "graphql_mutate",
            json!({
                "query": "mutation($e:JSON!){ bulkEdges(edges:$e){ ok inserted failed } }",
                "variables": { "e": edges }
            }),
        )
        .await?;
    Ok(())
}

fn pack_snapshot(response: &Value) -> Result<Value> {
    // Manifest is the only required field; `objects` is optional (see
    // compile_snapshot comment). The iteration below already handles missing
    // arrays via `as_array().into_iter().flatten()`.
    let checkout_like = response
        .get("pack")
        .and_then(|pack| pack.get("manifest"))
        .ok_or_else(|| SyncError::Mcp("compile response missing manifest".to_string()))?;
    let nodes_total = checkout_like
        .get("nodes_total")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let edges_total = checkout_like
        .get("edges_total")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for object in response["pack"]["objects"].as_array().into_iter().flatten() {
        let kind = object
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(payload) = object.get("payload").cloned() else {
            continue;
        };
        match kind {
            "node" => nodes.push(payload),
            "edge" => edges.push(payload),
            _ => {}
        }
    }
    if nodes.len() as u64 != nodes_total || edges.len() as u64 != edges_total {
        return Err(SyncError::Mcp(format!(
            "snapshot object totals mismatch: nodes {}/{nodes_total}, edges {}/{edges_total}",
            nodes.len(),
            edges.len()
        )));
    }
    Ok(json!({
        "version": response["pack"]["manifest"]["graph_version"].clone(),
        "nodes": nodes,
        "edges": edges
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_snapshot_extracts_node_and_edge_payloads() {
        let response = json!({
            "pack": {
                "manifest": { "graph_version": 7, "nodes_total": 1, "edges_total": 1 },
                "objects": [
                    { "kind": "node", "payload": { "id": "n:1", "labels": ["MemoryDocument"], "properties": {} } },
                    { "kind": "edge", "payload": { "id": "e:1", "from_id": "n:1", "to_id": "n:1", "type": "RELATES", "properties": {} } }
                ]
            }
        });

        let snapshot = pack_snapshot(&response).expect("snapshot");

        assert_eq!(snapshot["version"], 7);
        assert_eq!(snapshot["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(snapshot["edges"].as_array().unwrap().len(), 1);
    }
}
