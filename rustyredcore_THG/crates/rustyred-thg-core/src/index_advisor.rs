use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::index_manifest::IndexManifest;
use crate::index_proposal::IndexProposal;
use crate::query_receipt::QueryReceipt;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IndexAdvisorConfig {
    pub min_repeated_receipts: usize,
    pub min_total_full_scans: usize,
}

impl Default for IndexAdvisorConfig {
    fn default() -> Self {
        Self {
            min_repeated_receipts: 2,
            min_total_full_scans: 2,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IndexPainSignal {
    pub query_signature: String,
    pub supporting_receipts: Vec<String>,
    pub total_full_scans: usize,
    pub total_results: usize,
    pub reason: String,
}

#[derive(Clone, Debug, Default)]
pub struct IndexAdvisor {
    config: IndexAdvisorConfig,
}

impl IndexAdvisor {
    pub fn new(config: IndexAdvisorConfig) -> Self {
        Self { config }
    }

    pub fn detect_full_scan_pain<'a>(
        &self,
        receipts: impl IntoIterator<Item = &'a QueryReceipt>,
    ) -> Vec<IndexPainSignal> {
        #[derive(Default)]
        struct Accumulator {
            receipt_ids: Vec<String>,
            full_scans: usize,
            results: usize,
        }

        let mut by_signature: BTreeMap<String, Accumulator> = BTreeMap::new();
        for receipt in receipts {
            if receipt.full_scan_count == 0 {
                continue;
            }
            let entry = by_signature
                .entry(receipt.query_signature.clone())
                .or_default();
            entry.receipt_ids.push(receipt.id.clone());
            entry.full_scans = entry.full_scans.saturating_add(receipt.full_scan_count);
            entry.results = entry.results.saturating_add(receipt.result_count);
        }

        by_signature
            .into_iter()
            .filter_map(|(query_signature, accumulator)| {
                if accumulator.receipt_ids.len() < self.config.min_repeated_receipts
                    || accumulator.full_scans < self.config.min_total_full_scans
                {
                    return None;
                }
                let receipt_count = accumulator.receipt_ids.len();
                Some(IndexPainSignal {
                    query_signature,
                    supporting_receipts: accumulator.receipt_ids,
                    total_full_scans: accumulator.full_scans,
                    total_results: accumulator.results,
                    reason: format!(
                        "query family performed {} full scans across {} receipts",
                        accumulator.full_scans, receipt_count
                    ),
                })
            })
            .collect()
    }

    pub fn proposal_for_pain(
        &self,
        id: impl Into<String>,
        manifest_draft: IndexManifest,
        pain: &IndexPainSignal,
    ) -> IndexProposal {
        IndexProposal::new(id, manifest_draft, pain.reason.clone())
            .with_supporting_receipts(pain.supporting_receipts.clone())
            .with_estimates(
                pain.total_full_scans as f64,
                pain.total_full_scans as f64,
                0,
                0,
                0.0,
            )
            .with_shadow_validation_plan("replay supporting receipts against shadow index")
    }
}
