use serde::{Deserialize, Serialize};
use theorem_copresence::SubstratePeer;

use crate::delta::FederationFrame;
use crate::trust::TrustPolicy;
use crate::Result;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InboundFrameKind {
    StructuredDelta,
    TextUpdate,
    Presence,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InboundApplyReceipt {
    pub endpoint_id: String,
    pub scope: String,
    pub kind: InboundFrameKind,
    pub trust_score: f64,
    pub applied_structured: usize,
    pub applied_text: usize,
    pub presence_seen: usize,
}

pub fn apply_inbound_frame(
    peer: &mut SubstratePeer,
    trust: &TrustPolicy,
    remote_endpoint_id: &str,
    frame: FederationFrame,
) -> Result<InboundApplyReceipt> {
    trust.require_inbound(remote_endpoint_id)?;
    let trust_score = trust.score(remote_endpoint_id);

    match frame {
        FederationFrame::StructuredDelta { scope, batch } => {
            let report = peer.merge_remote_delta(batch)?;
            Ok(InboundApplyReceipt {
                endpoint_id: remote_endpoint_id.to_string(),
                scope,
                kind: InboundFrameKind::StructuredDelta,
                trust_score,
                applied_structured: report.applied,
                applied_text: 0,
                presence_seen: 0,
            })
        }
        FederationFrame::TextUpdate {
            scope,
            region_id,
            update_v1,
            ..
        } => {
            peer.apply_text_update(&region_id, &update_v1)?;
            Ok(InboundApplyReceipt {
                endpoint_id: remote_endpoint_id.to_string(),
                scope,
                kind: InboundFrameKind::TextUpdate,
                trust_score,
                applied_structured: 0,
                applied_text: 1,
                presence_seen: 0,
            })
        }
        FederationFrame::Presence { scope, .. } => Ok(InboundApplyReceipt {
            endpoint_id: remote_endpoint_id.to_string(),
            scope,
            kind: InboundFrameKind::Presence,
            trust_score,
            applied_structured: 0,
            applied_text: 0,
            presence_seen: 1,
        }),
    }
}
