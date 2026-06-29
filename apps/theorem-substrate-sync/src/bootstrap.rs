use serde_json::{json, Value};

use crate::railway_client::McpClient;
use crate::round::{apply_snapshot, compile_snapshot};
use crate::status::StatusHandle;
use crate::Result;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BootstrapReceipt {
    pub remote_nodes_total: usize,
    pub remote_edges_total: usize,
    pub applied: bool,
}

pub async fn bootstrap_from_remote(
    local: &McpClient,
    remote: &McpClient,
    status: &StatusHandle,
) -> Result<BootstrapReceipt> {
    let remote_snapshot = compile_snapshot(remote).await?;
    let nodes_total = remote_snapshot
        .get("nodes")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let edges_total = remote_snapshot
        .get("edges")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    apply_snapshot(local, &remote_snapshot).await?;
    status
        .update(|status| {
            status.last_round = Some(format!("bootstrap:nodes={nodes_total}:edges={edges_total}"));
        })
        .await;
    Ok(BootstrapReceipt {
        remote_nodes_total: nodes_total,
        remote_edges_total: edges_total,
        applied: true,
    })
}

pub async fn remote_head(remote: &McpClient) -> Result<Value> {
    remote
        .call_tool(
            "rustyred_thg_graph_version_ref",
            json!({
                "branch": "main",
                "include_payloads": true
            }),
        )
        .await
}
