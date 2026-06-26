use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::graph_store::{GraphStoreError, GraphStoreResult};
use crate::index_manifest::IndexManifest;
use crate::index_proposal::IndexProposal;
use crate::query_receipt::QueryReceipt;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct IndexRegistry {
    manifests: BTreeMap<String, IndexManifest>,
    receipts: BTreeMap<String, QueryReceipt>,
    proposals: BTreeMap<String, IndexProposal>,
}

impl IndexRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_manifest(&mut self, manifest: IndexManifest) -> GraphStoreResult<()> {
        manifest.validate()?;
        if self.manifests.contains_key(&manifest.id) {
            return Err(GraphStoreError::new(
                "index_manifest_exists",
                format!("index manifest {} is already registered", manifest.id),
            ));
        }
        self.manifests.insert(manifest.id.clone(), manifest);
        Ok(())
    }

    pub fn upsert_manifest(&mut self, mut manifest: IndexManifest) -> GraphStoreResult<()> {
        manifest.validate()?;
        manifest.refresh_hashes();
        self.manifests.insert(manifest.id.clone(), manifest);
        Ok(())
    }

    pub fn get_manifest(&self, id: &str) -> Option<&IndexManifest> {
        self.manifests.get(id)
    }

    pub fn list_manifests(&self) -> Vec<&IndexManifest> {
        self.manifests.values().collect()
    }

    pub fn retire_manifest(&mut self, id: &str, reason: impl Into<String>) -> GraphStoreResult<()> {
        let Some(manifest) = self.manifests.get_mut(id) else {
            return Err(GraphStoreError::new(
                "index_manifest_not_found",
                format!("index manifest {id} is not registered"),
            ));
        };
        manifest.retire(reason);
        Ok(())
    }

    pub fn record_receipt(&mut self, receipt: QueryReceipt) -> GraphStoreResult<()> {
        if receipt.id.trim().is_empty() {
            return Err(GraphStoreError::new(
                "invalid_query_receipt",
                "query receipt id is required",
            ));
        }
        if self.receipts.contains_key(&receipt.id) {
            return Err(GraphStoreError::new(
                "query_receipt_exists",
                format!("query receipt {} is already recorded", receipt.id),
            ));
        }
        self.receipts.insert(receipt.id.clone(), receipt);
        Ok(())
    }

    pub fn get_receipt(&self, id: &str) -> Option<&QueryReceipt> {
        self.receipts.get(id)
    }

    pub fn receipts_for_signature(&self, query_signature: &str) -> Vec<&QueryReceipt> {
        self.receipts
            .values()
            .filter(|receipt| receipt.query_signature == query_signature)
            .collect()
    }

    pub fn register_proposal(&mut self, proposal: IndexProposal) -> GraphStoreResult<()> {
        if proposal.id.trim().is_empty() {
            return Err(GraphStoreError::new(
                "invalid_index_proposal",
                "index proposal id is required",
            ));
        }
        if self.proposals.contains_key(&proposal.id) {
            return Err(GraphStoreError::new(
                "index_proposal_exists",
                format!("index proposal {} is already registered", proposal.id),
            ));
        }
        self.proposals.insert(proposal.id.clone(), proposal);
        Ok(())
    }

    pub fn get_proposal(&self, id: &str) -> Option<&IndexProposal> {
        self.proposals.get(id)
    }

    pub fn list_proposals(&self) -> Vec<&IndexProposal> {
        self.proposals.values().collect()
    }
}
