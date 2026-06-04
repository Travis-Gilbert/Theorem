//! theorem_code.v1.CodeCrawlerService implementation.

use tonic::{Request, Response, Status};

use crate::code_index::{
    CodeContextInput, CodeContextOutput, CodeHitRecord, CodeIndexError, CodeIndexRuntime,
    CodeSymbolRecord, IngestCodebaseInput, IngestCodebaseOutput, SearchCodeInput, SearchCodeOutput,
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
