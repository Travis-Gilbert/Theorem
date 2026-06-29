use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use redis::Commands;
use rustyred_thg_core::{HookError, SubstrateSyncEvent, SubstrateSyncOutbox};

use crate::{Result, SyncError};

#[derive(Clone, Debug, serde::Deserialize, PartialEq, serde::Serialize)]
pub struct QueuedOutboxEvent {
    pub content_hash: String,
    pub event: SubstrateSyncEvent,
}

pub trait OutboxStore: Send + Sync {
    fn push_event(&self, tenant: &str, event: QueuedOutboxEvent) -> Result<()>;
    fn peek_event(&self, tenant: &str) -> Result<Option<QueuedOutboxEvent>>;
    fn pop_if_hash(&self, tenant: &str, content_hash: &str) -> Result<bool>;
    fn len(&self, tenant: &str) -> Result<usize>;
}

#[derive(Clone, Default)]
pub struct InMemoryOutbox {
    events: Arc<Mutex<VecDeque<QueuedOutboxEvent>>>,
}

impl OutboxStore for InMemoryOutbox {
    fn push_event(&self, _tenant: &str, event: QueuedOutboxEvent) -> Result<()> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(event);
        Ok(())
    }

    fn peek_event(&self, _tenant: &str) -> Result<Option<QueuedOutboxEvent>> {
        Ok(self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .front()
            .cloned())
    }

    fn pop_if_hash(&self, _tenant: &str, content_hash: &str) -> Result<bool> {
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if events
            .front()
            .map(|event| event.content_hash == content_hash)
            .unwrap_or(false)
        {
            events.pop_front();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn len(&self, _tenant: &str) -> Result<usize> {
        Ok(self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len())
    }
}

pub struct ValkeyOutbox {
    client: redis::Client,
}

impl ValkeyOutbox {
    pub fn new(url: &str) -> Result<Self> {
        Ok(Self {
            client: redis::Client::open(url)?,
        })
    }
}

impl OutboxStore for ValkeyOutbox {
    fn push_event(&self, tenant: &str, event: QueuedOutboxEvent) -> Result<()> {
        let mut conn = self.client.get_connection()?;
        let encoded = serde_json::to_string(&event)?;
        let _: usize = conn.rpush(outbox_key(tenant), encoded)?;
        Ok(())
    }

    fn peek_event(&self, tenant: &str) -> Result<Option<QueuedOutboxEvent>> {
        let mut conn = self.client.get_connection()?;
        let raw: Option<String> = conn.lindex(outbox_key(tenant), 0)?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(SyncError::from))
            .transpose()
    }

    fn pop_if_hash(&self, tenant: &str, content_hash: &str) -> Result<bool> {
        let mut conn = self.client.get_connection()?;
        let key = outbox_key(tenant);
        let raw: Option<String> = conn.lindex(&key, 0)?;
        let Some(raw) = raw else {
            return Ok(false);
        };
        let event: QueuedOutboxEvent = serde_json::from_str(&raw)?;
        if event.content_hash != content_hash {
            return Ok(false);
        }
        let _: Option<String> = conn.lpop(&key, None)?;
        Ok(true)
    }

    fn len(&self, tenant: &str) -> Result<usize> {
        let mut conn = self.client.get_connection()?;
        let len: usize = conn.llen(outbox_key(tenant))?;
        Ok(len)
    }
}

impl SubstrateSyncOutbox for ValkeyOutbox {
    fn push(&self, tenant: &str, event: &SubstrateSyncEvent) -> std::result::Result<(), HookError> {
        self.push_event(
            tenant,
            QueuedOutboxEvent {
                content_hash: event.content_hash.clone(),
                event: event.clone(),
            },
        )
        .map_err(|error| HookError::new(error.to_string()))
    }
}

pub fn outbox_key(tenant: &str) -> String {
    format!("sync:outbox:{}", normalize_tenant(tenant))
}

fn normalize_tenant(tenant: &str) -> String {
    tenant.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use rustyred_thg_core::{ActorId, Hlc};

    fn event(hash: &str) -> QueuedOutboxEvent {
        QueuedOutboxEvent {
            content_hash: hash.to_string(),
            event: SubstrateSyncEvent {
                tenant: "Travis-Gilbert".to_string(),
                op_kind: "node_upserted".to_string(),
                id: "mem:a".to_string(),
                labels: vec!["MemoryDocument".to_string()],
                changed_props: vec!["status".to_string()],
                property_delta: json!({"properties": {"status": "active"}}),
                committed_at_ms: 1,
                hlc: Hlc::new(1, 0, ActorId::from_label("test")),
                content_hash: hash.to_string(),
            },
        }
    }

    #[test]
    fn in_memory_outbox_peek_and_pop_are_fifo_and_hash_guarded() {
        let outbox = InMemoryOutbox::default();
        outbox.push_event("Travis-Gilbert", event("a")).unwrap();
        outbox.push_event("Travis-Gilbert", event("b")).unwrap();

        assert_eq!(outbox.len("Travis-Gilbert").unwrap(), 2);
        assert_eq!(
            outbox
                .peek_event("Travis-Gilbert")
                .unwrap()
                .unwrap()
                .content_hash,
            "a"
        );
        assert!(!outbox.pop_if_hash("Travis-Gilbert", "b").unwrap());
        assert!(outbox.pop_if_hash("Travis-Gilbert", "a").unwrap());
        assert_eq!(outbox.len("Travis-Gilbert").unwrap(), 1);
    }
}
