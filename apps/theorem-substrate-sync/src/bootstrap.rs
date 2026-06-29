use serde_json::{json, Value};

use crate::railway_client::McpClient;
use crate::round::{apply_snapshot, compile_snapshot};
use crate::status::StatusHandle;
use crate::{Result, SyncError};

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BootstrapReceipt {
    pub remote_nodes_total: usize,
    pub remote_edges_total: usize,
    pub applied: bool,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct MemoryDocumentsBootstrapReceipt {
    pub remote_nodes_total: usize,
    pub pages: usize,
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

pub async fn bootstrap_memory_documents_from_remote(
    local: &McpClient,
    remote: &McpClient,
    status: &StatusHandle,
) -> Result<MemoryDocumentsBootstrapReceipt> {
    let mut before = String::new();
    let mut remote_nodes_total = 0usize;
    let mut pages = 0usize;

    loop {
        let mut args = json!({
            "limit": 500,
            "include_inactive": true
        });
        if !before.is_empty() {
            args["before"] = Value::String(before.clone());
        }
        let page = remote.call_tool("memory_documents_dump", args).await?;
        let nodes = page
            .get("nodes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if nodes.is_empty() {
            break;
        }
        let node_count = nodes.len();
        local
            .call_tool(
                "graphql_mutate",
                json!({
                    "query": "mutation($n:JSON!){ bulkNodes(nodes:$n){ ok inserted failed } }",
                    "variables": { "n": nodes }
                }),
            )
            .await?;
        pages += 1;
        remote_nodes_total += page
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or(node_count as u64) as usize;
        if !page
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            break;
        }
        before = page
            .get("next_before")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if before.is_empty() {
            return Err(SyncError::Mcp(
                "memory_documents_dump response was truncated without next_before".to_string(),
            ));
        }
    }

    status
        .update(|status| {
            status.last_round = Some(format!(
                "bootstrap:memory_docs:nodes={remote_nodes_total}:pages={pages}"
            ));
        })
        .await;
    Ok(MemoryDocumentsBootstrapReceipt {
        remote_nodes_total,
        pages,
        applied: true,
    })
}
