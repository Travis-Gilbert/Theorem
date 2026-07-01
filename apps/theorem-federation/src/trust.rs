use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{FederationError, Result};

pub const DEFAULT_TRUST_FLOOR: f64 = 0.35;
pub const DEFAULT_TRUST_ALPHA: f64 = 0.2;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PeerTrust {
    pub endpoint_id: String,
    pub score: f64,
    pub observations: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TrustPolicy {
    pub floor: f64,
    pub alpha: f64,
    pub peers: BTreeMap<String, PeerTrust>,
}

impl Default for TrustPolicy {
    fn default() -> Self {
        Self {
            floor: DEFAULT_TRUST_FLOOR,
            alpha: DEFAULT_TRUST_ALPHA,
            peers: BTreeMap::new(),
        }
    }
}

impl TrustPolicy {
    pub fn with_floor(floor: f64) -> Self {
        Self {
            floor,
            ..Self::default()
        }
    }

    pub fn score(&self, endpoint_id: &str) -> f64 {
        self.peers
            .get(endpoint_id)
            .map(|peer| peer.score)
            .unwrap_or(1.0)
    }

    pub fn record_outcome(&mut self, endpoint_id: impl Into<String>, success: bool) -> f64 {
        let endpoint_id = endpoint_id.into();
        let target = if success { 1.0 } else { 0.0 };
        let entry = self.peers.entry(endpoint_id.clone()).or_insert(PeerTrust {
            endpoint_id,
            score: 1.0,
            observations: 0,
        });
        entry.score = (self.alpha * target) + ((1.0 - self.alpha) * entry.score);
        entry.observations = entry.observations.saturating_add(1);
        entry.score
    }

    pub fn allow_inbound(&self, endpoint_id: &str) -> bool {
        self.score(endpoint_id) >= self.floor
    }

    pub fn require_inbound(&self, endpoint_id: &str) -> Result<()> {
        let score = self.score(endpoint_id);
        if score >= self.floor {
            return Ok(());
        }
        Err(FederationError::TrustRejected {
            endpoint_id: endpoint_id.to_string(),
            score,
            floor: self.floor,
        })
    }

    pub fn set_floor(&mut self, floor: f64) {
        self.floor = floor;
    }
}
