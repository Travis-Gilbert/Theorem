use iroh_gossip::TopicId;
use serde::{Deserialize, Serialize};
use theorem_copresence::PeerEvent;

use crate::Result;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AwarenessMessage {
    pub tenant: String,
    pub scope: String,
    pub event: PeerEvent,
}

pub fn awareness_topic_id(tenant: &str, scope: &str) -> TopicId {
    let seed = format!("theorem:federation:awareness:{tenant}:{scope}");
    TopicId::from_bytes(*blake3::hash(seed.as_bytes()).as_bytes())
}

pub fn encode_awareness(message: &AwarenessMessage) -> Result<Vec<u8>> {
    serde_json::to_vec(message).map_err(Into::into)
}

pub fn decode_awareness(bytes: &[u8]) -> Result<AwarenessMessage> {
    serde_json::from_slice(bytes).map_err(Into::into)
}
