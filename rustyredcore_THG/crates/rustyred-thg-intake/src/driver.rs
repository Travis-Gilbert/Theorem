//! The sync driver (Layer A1/A3): run one spoke against one tenant's store.

use commonplace::{
    BlobStore, Commonplace, Embedder, EmbeddingGraphStore, IngestPipeline, IngestReceipt, SourceRef,
};
use rustyred_thg_core::GraphStoreError;

use crate::spoke::{SourceCursor, SourceError, SourceResult, SourceScope, SourceSpoke};

/// The outcome of one sync: how many records were seen, how many were new vs
/// updated-in-place vs skipped, the advanced cursor to persist, and the receipts.
#[derive(Clone, Debug)]
pub struct SyncReport {
    pub source_id: String,
    pub fetched: usize,
    pub ingested: usize,
    pub updated: usize,
    pub skipped: usize,
    pub next_cursor: SourceCursor,
    pub receipts: Vec<IngestReceipt>,
}

/// Run one spoke against one tenant's store: fetch the scoped delta page by page,
/// map each record to [`IngestInput`](commonplace::IngestInput), ingest the page
/// through the batch path (A4), stamp each item with its source ref so a re-run
/// updates in place (A3), and advance the cursor. Records that fail mapping are
/// skipped, not fatal, so one bad record does not abort the sync.
pub fn sync_source<S, B, E>(
    commonplace: &mut Commonplace<S, B>,
    spoke: &dyn SourceSpoke,
    scope: &SourceScope,
    cursor: SourceCursor,
    pipeline: &IngestPipeline<E>,
) -> SourceResult<SyncReport>
where
    S: EmbeddingGraphStore,
    B: BlobStore,
    E: Embedder,
{
    let source_id = spoke.source_id().to_string();
    let mut report = SyncReport {
        source_id: source_id.clone(),
        fetched: 0,
        ingested: 0,
        updated: 0,
        skipped: 0,
        next_cursor: cursor.clone(),
        receipts: Vec::new(),
    };

    let mut current = cursor;
    loop {
        let page = spoke.fetch(scope, &current)?;
        report.fetched += page.records.len();

        let mut inputs = Vec::with_capacity(page.records.len());
        for record in &page.records {
            let mut input = match spoke.to_ingest_input(record) {
                Ok(input) => input,
                Err(SourceError::Mapping(_)) => {
                    report.skipped += 1;
                    continue;
                }
                Err(other) => return Err(other),
            };
            // Canonical source ref (A3): drives both idempotency and the
            // `Item.source` the rest of the system reads.
            input.source_ref = Some(SourceRef::new(&source_id, &record.external_id));

            if commonplace
                .item_by_source_ref(&source_id, &record.external_id)
                .map_err(store_err)?
                .is_some()
            {
                report.updated += 1;
            } else {
                report.ingested += 1;
            }
            inputs.push(input);
        }

        let receipts = pipeline.ingest_batch(commonplace, inputs).map_err(store_err)?;
        report.receipts.extend(receipts);

        report.next_cursor = page.next.clone();
        if page.exhausted {
            break;
        }
        current = page.next;
    }

    Ok(report)
}

fn store_err(error: GraphStoreError) -> SourceError {
    SourceError::Transport(format!("store: {error:?}"))
}
