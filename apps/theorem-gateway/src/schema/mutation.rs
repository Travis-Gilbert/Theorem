//! Build mutations: ingest and reindex code graphs through theorem-grpc.
//!
//! Both are rate-limited per IP. `ingestCodebase` is guarded by the public
//! ingest allowlist (AC5): a non-allowlisted URL is refused WITHOUT dialing
//! gRPC. Ingest runs as a server-side job, so the resolver submits then polls
//! the job to a terminal state within a bounded window (so a following
//! `searchCode` observes the built graph) before returning the receipt.

use std::time::{Duration, Instant};

use async_graphql::{Context, Object, Result};
use tonic::transport::Channel;
use tonic::Request;

use crate::pb::{code, CodeCrawlerServiceClient};
use crate::schema::types::IngestReceipt;
use crate::schema::{enforce_rate_limit, gateway_ctx, map_status};

const GATEWAY_ACTOR: &str = "theorem-gateway";

pub struct Mutation;

#[Object]
impl Mutation {
    /// Ingest a remote repository into a code graph. Only repo URLs matching
    /// `PUBLIC_INGEST_ALLOWLIST` are accepted; anything else is refused with no
    /// gRPC call. -> CodeCrawlerService.IngestCodebase (+ poll to completion)
    async fn ingest_codebase(&self, ctx: &Context<'_>, repo_url: String) -> Result<IngestReceipt> {
        enforce_rate_limit(ctx).await?;
        let gw = gateway_ctx(ctx)?;

        // AC5: the allowlist gate is a pure check that precedes any dial.
        if !gw.config.ingest_url_allowed(&repo_url) {
            return Err(async_graphql::Error::new(format!(
                "ingest refused: '{repo_url}' is not in the public ingest allowlist"
            )));
        }

        let tenant_id = gw.config.tenant_id.clone();
        let repo_id = derive_repo_id(&repo_url);
        let mut client = gw.code.clone();
        let req = code::IngestCodebaseRequest {
            tenant_id: tenant_id.clone(),
            repo_id,
            repo_url,
            actor: GATEWAY_ACTOR.to_string(),
            ..Default::default()
        };
        let ack = client
            .ingest_codebase(Request::new(req))
            .await
            .map_err(map_status)?
            .into_inner();
        let final_resp = finalize_ingest(client, tenant_id, ack, gw.config.ingest_wait).await?;
        Ok(IngestReceipt::from(final_resp))
    }

    /// Reindex an already-ingested repo (incremental: unchanged files are
    /// carried forward). -> CodeCrawlerService.ReindexCodebase (+ poll)
    async fn reindex_codebase(&self, ctx: &Context<'_>, repo_id: String) -> Result<IngestReceipt> {
        enforce_rate_limit(ctx).await?;
        let gw = gateway_ctx(ctx)?;
        let tenant_id = gw.config.tenant_id.clone();
        let mut client = gw.code.clone();
        let req = code::ReindexCodebaseRequest {
            tenant_id: tenant_id.clone(),
            repo_id,
            actor: GATEWAY_ACTOR.to_string(),
            ..Default::default()
        };
        let ack = client
            .reindex_codebase(Request::new(req))
            .await
            .map_err(map_status)?
            .into_inner();
        let final_resp = finalize_ingest(client, tenant_id, ack, gw.config.ingest_wait).await?;
        Ok(IngestReceipt::from(final_resp))
    }
}

/// Poll a submitted ingest/reindex job to a terminal state within `wait`. On
/// completion returns the job's output (with real counts); on timeout returns
/// the submit ack marked "running" so the caller can poll with the `jobId`.
async fn finalize_ingest(
    mut client: CodeCrawlerServiceClient<Channel>,
    tenant_id: String,
    ack: code::IngestCodebaseResponse,
    wait: Duration,
) -> Result<code::IngestCodebaseResponse> {
    let job_id = ack.job_id.clone();
    // No job id (synchronous server) or zero wait: return the ack as-is.
    if wait.is_zero() || job_id.is_empty() {
        return Ok(ack);
    }

    let deadline = Instant::now() + wait;
    loop {
        let status = client
            .get_ingest_status(Request::new(code::GetIngestStatusRequest {
                tenant_id: tenant_id.clone(),
                job_id: job_id.clone(),
            }))
            .await
            .map_err(map_status)?
            .into_inner();

        match status.state.as_str() {
            "finished" => {
                let mut out = status.output.unwrap_or_else(|| ack.clone());
                if out.status.is_empty() {
                    out.status = "ok".to_string();
                }
                return Ok(out);
            }
            "failed" => {
                let mut out = status.output.unwrap_or_else(|| ack.clone());
                out.status = "failed".to_string();
                out.message = if status.error_message.is_empty() {
                    status.error_code
                } else {
                    status.error_message
                };
                return Ok(out);
            }
            "budget_exceeded" => {
                let mut out = status.output.unwrap_or_else(|| ack.clone());
                out.status = "budget_exceeded".to_string();
                return Ok(out);
            }
            _ => {
                if Instant::now() >= deadline {
                    let mut out = ack.clone();
                    out.status = if status.state.is_empty() {
                        "running".to_string()
                    } else {
                        status.state
                    };
                    return Ok(out);
                }
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        }
    }
}

/// Derive a stable `owner/repo` id from a clone URL (the server re-derives its
/// own; the response's repo_id is authoritative and used in the receipt). This
/// is only a hint sent on the request.
fn derive_repo_id(repo_url: &str) -> String {
    let trimmed = repo_url.trim().trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    match segments.len() {
        0 => trimmed.to_string(),
        1 => segments[0].to_string(),
        n => format!("{}/{}", segments[n - 2], segments[n - 1]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_repo_id_extracts_owner_repo() {
        assert_eq!(
            derive_repo_id("https://github.com/Travis-Gilbert/RustyRed-Graph-Database"),
            "Travis-Gilbert/RustyRed-Graph-Database"
        );
        assert_eq!(
            derive_repo_id("https://github.com/Travis-Gilbert/RustyRed-Graph-Database.git"),
            "Travis-Gilbert/RustyRed-Graph-Database"
        );
        assert_eq!(
            derive_repo_id("https://github.com/owner/repo/"),
            "owner/repo"
        );
    }
}
