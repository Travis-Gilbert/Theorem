//! theorem_code.v1.CodeCrawlerService implementation.

use tonic::{Request, Response, Status};

use crate::code_index::{
    CodeContextInput, CodeContextOutput, CodeGraphEdgeRecord, CodeHitRecord, CodeIndexError,
    CodeIndexRuntime, CodeSymbolRecord, ExplainCodeInput, ExplainCodeOutput, ExploreCodeInput,
    ExploreCodeOutput, IngestCodebaseInput, IngestCodebaseOutput, RecognizeCodeInput,
    RecognizeCodeOutput, RecordUseReceiptInput, RecordUseReceiptOutput, SearchCodeInput,
    SearchCodeOutput,
};
use crate::pb;

#[derive(Clone)]
pub struct TheoremCodeCrawlerService {
    runtime: CodeIndexRuntime,
}

impl TheoremCodeCrawlerService {
    pub fn new(runtime: CodeIndexRuntime) -> Self {
        Self { runtime }
    }
}

#[tonic::async_trait]
impl pb::CodeCrawlerService for TheoremCodeCrawlerService {
    async fn ingest_codebase(
        &self,
        request: Request<pb::IngestCodebaseRequest>,
    ) -> Result<Response<pb::IngestCodebaseResponse>, Status> {
        let output = self
            .runtime
            .ingest_codebase(input_from_ingest(request.into_inner()))
            .map_err(status_from_code_error)?;
        Ok(Response::new(ingest_to_pb(output)))
    }

    async fn reindex_codebase(
        &self,
        request: Request<pb::ReindexCodebaseRequest>,
    ) -> Result<Response<pb::IngestCodebaseResponse>, Status> {
        let output = self
            .runtime
            .reindex_codebase(input_from_reindex(request.into_inner()))
            .map_err(status_from_code_error)?;
        Ok(Response::new(ingest_to_pb(output)))
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
        max_total_bytes: 0,
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
        max_total_bytes: 0,
        actor: req.actor,
    }
}

fn ingest_to_pb(output: IngestCodebaseOutput) -> pb::IngestCodebaseResponse {
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
