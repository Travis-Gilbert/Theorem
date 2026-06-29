use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::outbox::OutboxStore;
use crate::railway_client::McpClient;
use crate::status::{OutboxState, StatusHandle};
use crate::{Result, SyncError};

pub struct OutboxDrainer {
    tenant: String,
    remote: McpClient,
    outbox: Arc<dyn OutboxStore>,
    status: StatusHandle,
}

impl OutboxDrainer {
    pub fn new(
        tenant: impl Into<String>,
        remote: McpClient,
        outbox: Arc<dyn OutboxStore>,
        status: StatusHandle,
    ) -> Self {
        Self {
            tenant: tenant.into(),
            remote,
            outbox,
            status,
        }
    }

    pub async fn drain_once(&self) -> Result<Option<String>> {
        let Some(event) = self.outbox.peek_event(&self.tenant)? else {
            return Ok(None);
        };
        let publish = self
            .remote
            .call_tool(
                "stream_publish",
                json!({
                    "stream": format!("tenant:{}", self.tenant),
                    "actor": "theorem-substrate-sync",
                    "kind": "substrate_mutation",
                    "urgency": "info",
                    "payload": event.event,
                }),
            )
            .await;
        match publish {
            Ok(_) => {
                self.outbox.pop_if_hash(&self.tenant, &event.content_hash)?;
                self.status
                    .update(|status| {
                        status.outbox = OutboxState::Ready;
                        status.last_event = Some(event.content_hash.clone());
                    })
                    .await;
                Ok(Some(event.content_hash))
            }
            Err(SyncError::Auth(message)) => {
                self.status
                    .update(|status| {
                        status.outbox = OutboxState::Blocked;
                        status
                            .warnings
                            .push(format!("outbox auth blocked: {message}"));
                    })
                    .await;
                Err(SyncError::Auth(message))
            }
            Err(error) => Err(error),
        }
    }
}

pub fn retry_after(failures: u32) -> Duration {
    let secs = 2u64.saturating_pow(failures.min(5)).min(30);
    Duration::from_secs(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_caps_at_thirty_seconds() {
        assert_eq!(retry_after(0), Duration::from_secs(1));
        assert_eq!(retry_after(1), Duration::from_secs(2));
        assert_eq!(retry_after(10), Duration::from_secs(30));
    }
}
