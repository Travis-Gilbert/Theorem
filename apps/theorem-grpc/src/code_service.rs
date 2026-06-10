//! theorem_code.v1.CodeCrawlerService implementation.
//!
//! Ingest and reindex are job submissions: the unary call returns a `job_id`
//! immediately and the heavy path (clone, walk, parse, commit) runs on the
//! code-index worker with no client deadline. `WatchIngest` streams the job's
//! ordered event log; `GetIngestStatus` is the poll variant.

use std::pin::Pin;
use std::time::Duration;

use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use crate::code_index::{
    CodeContextInput, CodeContextOutput, CodeGraphEdgeRecord, CodeHitRecord, CodeIndexError,
    CodeIndexRuntime, CodeSymbolRecord, ExplainCodeInput, ExplainCodeOutput, ExploreCodeInput,
    ExploreCodeOutput, IngestCodebaseInput, IngestCodebaseOutput, IngestJobEvent,
    IngestJobEventKind, IngestJobRequest, IngestJobStatus, IngestStageTimings, RecognizeCodeInput,
    RecognizeCodeOutput, RecordUseReceiptInput, RecordUseReceiptOutput, RepoFetchCaps,
    SearchCodeInput, SearchCodeOutput,
};
use crate::pb;

const WATCH_POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone)]
pub struct TheoremCodeCrawlerService {
    runtime: CodeIndexRuntime,
}

impl TheoremCodeCrawlerService {
    pub fn new(runtime: CodeIndexRuntime) -> Self {
        Self { runtime }
    }

    fn submit(&self, input: IngestCodebaseInput, repo_url: String, operation: &str) -> IngestJobStatus {
        let caps = RepoFetchCaps::from_requested(input.max_total_bytes);
        self.runtime.submit_ingest_job(IngestJobRequest {
            input,
            operation: operation.to_string(),
            repo_url,
            caps,
            parse_budget_ms: None,
            ..Default::default()
        })
    }
}

#[tonic::async_trait]
impl pb::CodeCrawlerService for TheoremCodeCrawlerService {
    async fn ingest_codebase(
        &self,
        request: Request<pb::IngestCodebaseRequest>,
    ) -> Result<Response<pb::IngestCodebaseResponse>, Status> {
        let req = request.into_inner();
        let repo_url = req.repo_url.clone();
        let submitted = self.submit(input_from_ingest(req), repo_url, "ingest");
        Ok(Response::new(submission_ack(submitted)))
    }

    async fn reindex_codebase(
        &self,
        request: Request<pb::ReindexCodebaseRequest>,
    ) -> Result<Response<pb::IngestCodebaseResponse>, Status> {
        let req = request.into_inner();
        let repo_url = req.repo_url.clone();
        let submitted = self.submit(input_from_reindex(req), repo_url, "reindex");
        Ok(Response::new(submission_ack(submitted)))
    }

    type WatchIngestStream =
        Pin<Box<dyn Stream<Item = Result<pb::IngestEvent, Status>> + Send + 'static>>;

    async fn watch_ingest(
        &self,
        request: Request<pb::code::WatchIngestRequest>,
    ) -> Result<Response<Self::WatchIngestStream>, Status> {
        let req = request.into_inner();
        let job_id = req.job_id.trim().to_string();
        let registry = self.runtime.ingest_jobs();
        let Some(status) = registry.status(&job_id) else {
            return Err(Status::not_found(format!(
                "ingest job {job_id} was not found"
            )));
        };
        guard_job_tenant(&req.tenant_id, &status)?;

        let (sender, receiver) = tokio::sync::mpsc::channel::<Result<pb::IngestEvent, Status>>(32);
        tokio::task::spawn_blocking(move || {
            let mut after_sequence = 0u64;
            loop {
                match registry.wait_events(&job_id, after_sequence, WATCH_POLL_INTERVAL) {
                    None => {
                        let _ = sender.blocking_send(Err(Status::not_found(
                            "ingest job was evicted from the registry",
                        )));
                        return;
                    }
                    Some((events, terminal)) => {
                        for event in events {
                            after_sequence = event.sequence;
                            if sender
                                .blocking_send(Ok(ingest_event_to_pb(&job_id, &event)))
                                .is_err()
                            {
                                return;
                            }
                        }
                        if terminal {
                            return;
                        }
                    }
                }
            }
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(receiver))))
    }

    async fn get_ingest_status(
        &self,
        request: Request<pb::code::GetIngestStatusRequest>,
    ) -> Result<Response<pb::code::IngestStatus>, Status> {
        let req = request.into_inner();
        let job_id = req.job_id.trim();
        let status = self
            .runtime
            .ingest_job_status(job_id)
            .ok_or_else(|| Status::not_found(format!("ingest job {job_id} was not found")))?;
        guard_job_tenant(&req.tenant_id, &status)?;
        Ok(Response::new(job_status_to_pb(status)))
    }

    async fn search_code(
        &self,
        request: Request<pb::SearchCodeRequest>,
    ) -> Result<Response<pb::SearchCodeResponse>, Status> {
        let output = self
            .runtime
            .search_code(SearchCodeInput {
                tenant_id: request.get_ref().tenant_id.clone(),
                query: request.get_ref().query.clone(),
                repo_id: request.get_ref().repo_id.clone(),
                path_prefix: request.get_ref().path_prefix.clone(),
                kinds: request.get_ref().kinds.clone(),
                limit: request.get_ref().limit,
            })
            .map_err(status_from_code_error)?;
        Ok(Response::new(search_to_pb(output)))
    }

    async fn code_context(
        &self,
        request: Request<pb::CodeContextRequest>,
    ) -> Result<Response<pb::CodeContextResponse>, Status> {
        let req = request.into_inner();
        let output = self
            .runtime
            .code_context(CodeContextInput {
                tenant_id: req.tenant_id,
                node_id: req.node_id,
                repo_id: req.repo_id,
                file_path: req.file_path,
                before_lines: req.before_lines,
                after_lines: req.after_lines,
                max_chars: req.max_chars,
            })
            .map_err(status_from_code_error)?;
        Ok(Response::new(context_to_pb(output)))
    }

    async fn recognize_code(
        &self,
        request: Request<pb::RecognizeCodeRequest>,
    ) -> Result<Response<pb::RecognizeCodeResponse>, Status> {
        let req = request.into_inner();
        let output = self
            .runtime
            .recognize_code(RecognizeCodeInput {
                tenant_id: req.tenant_id,
                repo_id: req.repo_id,
                file_path: req.file_path,
                text: req.text,
                limit: req.limit,
            })
            .map_err(status_from_code_error)?;
        Ok(Response::new(recognize_to_pb(output)))
    }

    async fn explore_code(
        &self,
        request: Request<pb::ExploreCodeRequest>,
    ) -> Result<Response<pb::ExploreCodeResponse>, Status> {
        let req = request.into_inner();
        let output = self
            .runtime
            .explore_code(ExploreCodeInput {
                tenant_id: req.tenant_id,
                node_id: req.node_id,
                query: req.query,
                repo_id: req.repo_id,
                max_depth: req.max_depth,
                limit: req.limit,
            })
            .map_err(status_from_code_error)?;
        Ok(Response::new(explore_to_pb(output)))
    }

    async fn explain_code(
        &self,
        request: Request<pb::ExplainCodeRequest>,
    ) -> Result<Response<pb::ExplainCodeResponse>, Status> {
        let req = request.into_inner();
        let output = self
            .runtime
            .explain_code(ExplainCodeInput {
                tenant_id: req.tenant_id,
                node_id: req.node_id,
                query: req.query,
                repo_id: req.repo_id,
                max_chars: req.max_chars,
            })
            .map_err(status_from_code_error)?;
        Ok(Response::new(explain_to_pb(output)))
    }

    async fn record_use_receipt(
        &self,
        request: Request<pb::RecordUseReceiptRequest>,
    ) -> Result<Response<pb::RecordUseReceiptResponse>, Status> {
        let req = request.into_inner();
        let output = self
            .runtime
            .record_use_receipt(RecordUseReceiptInput {
                tenant_id: req.tenant_id,
                node_id: req.node_id,
                repo_id: req.repo_id,
                query: req.query,
                action: req.action,
                outcome: req.outcome,
                actor: req.actor,
                use_json: req.use_json,
            })
            .map_err(status_from_code_error)?;
        Ok(Response::new(record_use_to_pb(output)))
    }
}

fn input_from_ingest(req: pb::IngestCodebaseRequest) -> IngestCodebaseInput {
    IngestCodebaseInput {
        tenant_id: req.tenant_id,
        repo_path: req.repo_path,
        repo_id: req.repo_id,
        include_extensions: req.include_extensions,
        exclude_dirs: req.exclude_dirs,
        max_files: req.max_files,
        max_file_bytes: req.max_file_bytes,
        max_total_bytes: req.max_total_bytes,
        actor: req.actor,
    }
}

fn input_from_reindex(req: pb::ReindexCodebaseRequest) -> IngestCodebaseInput {
    IngestCodebaseInput {
        tenant_id: req.tenant_id,
        repo_path: req.repo_path,
        repo_id: req.repo_id,
        include_extensions: req.include_extensions,
        exclude_dirs: req.exclude_dirs,
        max_files: req.max_files,
        max_file_bytes: req.max_file_bytes,
        max_total_bytes: req.max_total_bytes,
        actor: req.actor,
    }
}

/// A submission acknowledgement rides the same response message: `status` is
/// "submitted", `job_id` identifies the background job, counts stay zero.
fn submission_ack(status: IngestJobStatus) -> pb::IngestCodebaseResponse {
    pb::IngestCodebaseResponse {
        tenant_id: status.tenant_id,
        repo_id: status.repo_id,
        status: "submitted".to_string(),
        message:
            "ingest job submitted; stream WatchIngest(job_id) or poll GetIngestStatus(job_id)"
                .to_string(),
        job_id: status.job_id,
        ..Default::default()
    }
}

/// Reject a watch/status read when the caller names a different tenant than
/// the one the job was submitted for. An empty tenant skips the check.
fn guard_job_tenant(requested_tenant: &str, status: &IngestJobStatus) -> Result<(), Status> {
    let requested = requested_tenant.trim();
    if requested.is_empty() || requested == status.tenant_id {
        Ok(())
    } else {
        Err(Status::not_found(format!(
            "ingest job {} was not found",
            status.job_id
        )))
    }
}

fn ingest_to_pb(output: IngestCodebaseOutput, job_id: &str) -> pb::IngestCodebaseResponse {
    pb::IngestCodebaseResponse {
        tenant_id: output.tenant_id,
        repo_id: output.repo_id,
        repo_root: output.repo_root,
        generation: output.generation,
        files_indexed: output.files_indexed,
        symbols_indexed: output.symbols_indexed,
        files_skipped: output.files_skipped,
        graph_version: output.graph_version,
        receipt_hash: output.receipt_hash,
        receipt_json: output.receipt_json,
        status: output.status,
        message: output.message,
        job_id: job_id.to_string(),
        files_parsed: output.files_parsed,
        files_carried: output.files_carried,
    }
}

fn stage_timings_to_pb(timings: &IngestStageTimings) -> pb::code::IngestStageTimings {
    pb::code::IngestStageTimings {
        clone_ms: timings.clone_ms,
        resolve_ms: timings.resolve_ms,
        walk_ms: timings.walk_ms,
        parse_ms: timings.parse_ms,
        mutation_ms: timings.mutation_ms,
        write_ms: timings.write_ms,
        total_ms: timings.total_ms,
    }
}

fn job_status_to_pb(status: IngestJobStatus) -> pb::code::IngestStatus {
    pb::code::IngestStatus {
        job_id: status.job_id.clone(),
        state: status.state.as_str().to_string(),
        stage: status.stage.clone(),
        files_total: status.files_total,
        files_done: status.files_done,
        submitted_at_ms: status.submitted_at_ms,
        updated_at_ms: status.updated_at_ms,
        output: status
            .output
            .as_ref()
            .map(|output| ingest_to_pb(output.clone(), &status.job_id)),
        error_code: status.error_code,
        error_message: status.error_message,
    }
}

fn ingest_event_to_pb(job_id: &str, event: &IngestJobEvent) -> pb::IngestEvent {
    use pb::code::ingest_event::Event;
    let payload = match &event.kind {
        IngestJobEventKind::CloneDone { ms } => {
            Event::CloneDone(pb::code::IngestCloneDone { ms: *ms })
        }
        IngestJobEventKind::WalkDone { files_found } => Event::WalkDone(pb::code::IngestWalkDone {
            files_found: *files_found,
        }),
        IngestJobEventKind::ParseProgress { done, total } => {
            Event::ParseProgress(pb::code::IngestParseProgress {
                done: *done,
                total: *total,
            })
        }
        IngestJobEventKind::CommitDone { graph_version } => {
            Event::CommitDone(pb::code::IngestCommitDone {
                graph_version: *graph_version,
            })
        }
        IngestJobEventKind::Finished { output } => Event::Finished(pb::code::IngestFinished {
            stage_timings: Some(stage_timings_to_pb(&output.stage_timings)),
            output: Some(ingest_to_pb(output.clone(), job_id)),
        }),
        IngestJobEventKind::Failed { code, message } => Event::Failed(pb::code::IngestFailed {
            code: code.clone(),
            message: message.clone(),
        }),
    };
    pb::IngestEvent {
        job_id: job_id.to_string(),
        sequence: event.sequence,
        recorded_at_ms: event.recorded_at_ms,
        event: Some(payload),
    }
}

fn search_to_pb(output: SearchCodeOutput) -> pb::SearchCodeResponse {
    pb::SearchCodeResponse {
        tenant_id: output.tenant_id,
        query: output.query,
        hits: output.hits.into_iter().map(hit_to_pb).collect(),
        total_admitted: output.total_admitted,
        total_returned: output.total_returned,
        latency_ms: output.latency_ms,
        receipt_hash: output.receipt_hash,
        receipt_json: output.receipt_json,
    }
}

fn context_to_pb(output: CodeContextOutput) -> pb::CodeContextResponse {
    pb::CodeContextResponse {
        tenant_id: output.tenant_id,
        repo_id: output.repo_id,
        file_id: output.file_id,
        symbol_id: output.symbol_id,
        file_path: output.file_path,
        start_line: output.start_line,
        end_line: output.end_line,
        context: output.context,
        symbols: output.symbols.into_iter().map(symbol_to_pb).collect(),
        receipt_hash: output.receipt_hash,
        receipt_json: output.receipt_json,
    }
}

fn recognize_to_pb(output: RecognizeCodeOutput) -> pb::RecognizeCodeResponse {
    pb::RecognizeCodeResponse {
        tenant_id: output.tenant_id,
        repo_id: output.repo_id,
        file_path: output.file_path,
        symbols: output.symbols.into_iter().map(symbol_to_pb).collect(),
        receipt_hash: output.receipt_hash,
        receipt_json: output.receipt_json,
    }
}

fn explore_to_pb(output: ExploreCodeOutput) -> pb::ExploreCodeResponse {
    pb::ExploreCodeResponse {
        tenant_id: output.tenant_id,
        focus: output.focus.map(symbol_to_pb),
        related_symbols: output
            .related_symbols
            .into_iter()
            .map(symbol_to_pb)
            .collect(),
        edges: output.edges.into_iter().map(edge_to_pb).collect(),
        receipt_hash: output.receipt_hash,
        receipt_json: output.receipt_json,
    }
}

fn explain_to_pb(output: ExplainCodeOutput) -> pb::ExplainCodeResponse {
    pb::ExplainCodeResponse {
        tenant_id: output.tenant_id,
        symbol: output.symbol.map(symbol_to_pb),
        summary: output.summary,
        context: output.context,
        edges: output.edges.into_iter().map(edge_to_pb).collect(),
        receipt_hash: output.receipt_hash,
        receipt_json: output.receipt_json,
    }
}

fn record_use_to_pb(output: RecordUseReceiptOutput) -> pb::RecordUseReceiptResponse {
    pb::RecordUseReceiptResponse {
        tenant_id: output.tenant_id,
        node_id: output.node_id,
        repo_id: output.repo_id,
        receipt_hash: output.receipt_hash,
        receipt_json: output.receipt_json,
        status: output.status,
        message: output.message,
    }
}

fn hit_to_pb(hit: CodeHitRecord) -> pb::CodeHit {
    pb::CodeHit {
        node_id: hit.node_id,
        repo_id: hit.repo_id,
        file_id: hit.file_id,
        file_path: hit.file_path,
        kind: hit.kind,
        name: hit.name,
        language: hit.language,
        line: hit.line,
        snippet: hit.snippet,
        score: hit.score,
        trust_tier: hit.trust_tier,
        community_id: hit.community_id,
    }
}

fn symbol_to_pb(symbol: CodeSymbolRecord) -> pb::CodeSymbol {
    pb::CodeSymbol {
        node_id: symbol.node_id,
        repo_id: symbol.repo_id,
        file_id: symbol.file_id,
        file_path: symbol.file_path,
        kind: symbol.kind,
        name: symbol.name,
        language: symbol.language,
        line: symbol.line,
        signature: symbol.signature,
        snippet: symbol.snippet,
        trust_tier: symbol.trust_tier,
        community_id: symbol.community_id,
        callers: symbol.callers,
        callees: symbol.callees,
        dependencies: symbol.dependencies,
        dependents: symbol.dependents,
    }
}

fn edge_to_pb(edge: CodeGraphEdgeRecord) -> pb::CodeGraphEdge {
    pb::CodeGraphEdge {
        from_node_id: edge.from_node_id,
        to_node_id: edge.to_node_id,
        edge_type: edge.edge_type,
        from_name: edge.from_name,
        to_name: edge.to_name,
        evidence: edge.evidence,
    }
}

fn status_from_code_error(error: CodeIndexError) -> Status {
    match error.code.as_str() {
        "invalid_code_index_request" => Status::invalid_argument(error.message),
        "code_index_io_error" | "code_index_cwd_error" => {
            Status::failed_precondition(error.message)
        }
        _ => Status::internal(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rustyred_thg_core::{RedCoreDurability, RedCoreOptions};
    use tokio_stream::StreamExt;
    use tonic::Request;

    use super::*;
    use crate::pb::CodeCrawlerService as _;

    fn test_options() -> RedCoreOptions {
        RedCoreOptions {
            durability: RedCoreDurability::AofAlways,
            snapshot_interval_writes: 100,
            strict_acid: true,
        }
    }

    fn unique_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "theorem-code-service-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    /// D1 + D6: ingest is a job submission that returns immediately;
    /// WatchIngest streams ordered events ending in `finished`;
    /// GetIngestStatus agrees; and the SAME service's search reads the store
    /// the ingest wrote (one `CodeIndexRuntime`, exactly how main.rs wires
    /// the crawler and app-affordance services together).
    #[tokio::test]
    async fn ingest_submits_streams_and_search_reads_the_store_ingest_wrote() {
        let repo_dir = unique_dir("repo");
        std::fs::create_dir_all(repo_dir.join("src")).unwrap();
        std::fs::write(
            repo_dir.join("src/lib.rs"),
            "pub fn native_fixture_helper() -> usize {\n    1\n}\n\npub fn native_fixture_fn() -> usize {\n    native_fixture_helper()\n}\n",
        )
        .unwrap();
        let store_dir = unique_dir("store");
        let runtime = CodeIndexRuntime::try_new_at(&store_dir, test_options()).unwrap();
        let service = TheoremCodeCrawlerService::new(runtime.clone());

        let submitted = service
            .ingest_codebase(Request::new(pb::IngestCodebaseRequest {
                tenant_id: "theorem".to_string(),
                repo_path: repo_dir.display().to_string(),
                repo_id: "repo:code-service-fixture".to_string(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(submitted.status, "submitted");
        assert!(!submitted.job_id.is_empty());
        assert_eq!(submitted.files_indexed, 0, "the ack carries no counts");

        let mut stream = service
            .watch_ingest(Request::new(pb::code::WatchIngestRequest {
                tenant_id: "theorem".to_string(),
                job_id: submitted.job_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        let mut labels = Vec::new();
        while let Some(event) = stream.next().await {
            let event = event.unwrap();
            assert_eq!(event.job_id, submitted.job_id);
            let label = match event.event.expect("event payload") {
                pb::code::ingest_event::Event::CloneDone(_) => "clone_done",
                pb::code::ingest_event::Event::WalkDone(_) => "walk_done",
                pb::code::ingest_event::Event::ParseProgress(_) => "parse_progress",
                pb::code::ingest_event::Event::CommitDone(_) => "commit_done",
                pb::code::ingest_event::Event::Finished(finished) => {
                    let output = finished.output.expect("finished output");
                    assert_eq!(output.status, "ok");
                    assert_eq!(output.files_indexed, 1);
                    assert_eq!(output.files_parsed, 1);
                    assert!(finished.stage_timings.is_some(), "stage timings ride the final event");
                    "finished"
                }
                pb::code::ingest_event::Event::Failed(failed) => {
                    panic!("ingest failed: {} {}", failed.code, failed.message)
                }
            };
            labels.push(label);
        }
        let position = |label: &str| labels.iter().position(|item| *item == label);
        assert!(position("walk_done") < position("parse_progress"), "{labels:?}");
        assert!(position("parse_progress") < position("commit_done"), "{labels:?}");
        assert_eq!(labels.last(), Some(&"finished"), "{labels:?}");

        let status = service
            .get_ingest_status(Request::new(pb::code::GetIngestStatusRequest {
                tenant_id: "theorem".to_string(),
                job_id: submitted.job_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(status.state, "finished");
        let output = status.output.expect("finished status output");
        assert_eq!(output.job_id, submitted.job_id);
        assert_eq!(output.symbols_indexed, 2);

        // D6: search on the same tenant returns hits from the store the
        // ingest job wrote. One runtime, one store.
        let search = service
            .search_code(Request::new(pb::SearchCodeRequest {
                tenant_id: "theorem".to_string(),
                query: "native_fixture_fn".to_string(),
                repo_id: "repo:code-service-fixture".to_string(),
                limit: 5,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(search.hits[0].name, "native_fixture_fn");

        let missing = service
            .get_ingest_status(Request::new(pb::code::GetIngestStatusRequest {
                tenant_id: "theorem".to_string(),
                job_id: "code:ingest-job:unknown".to_string(),
            }))
            .await;
        assert_eq!(missing.unwrap_err().code(), tonic::Code::NotFound);

        drop(service);
        drop(runtime);
        std::fs::remove_dir_all(repo_dir).ok();
        std::fs::remove_dir_all(store_dir).ok();
    }
}
