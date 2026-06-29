use std::sync::{Arc, Mutex};

use redis::Commands;

use crate::Result;

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SyncCursors {
    pub stream_cursor: u64,
    pub last_round: Option<String>,
    pub last_head: Option<String>,
}

pub trait CursorStore: Send + Sync {
    fn load(&self, tenant: &str) -> Result<SyncCursors>;
    fn save(&self, tenant: &str, cursors: &SyncCursors) -> Result<()>;
}

#[derive(Clone, Default)]
pub struct InMemoryCursorStore {
    cursors: Arc<Mutex<SyncCursors>>,
}

impl CursorStore for InMemoryCursorStore {
    fn load(&self, _tenant: &str) -> Result<SyncCursors> {
        Ok(self
            .cursors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone())
    }

    fn save(&self, _tenant: &str, cursors: &SyncCursors) -> Result<()> {
        *self
            .cursors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = cursors.clone();
        Ok(())
    }
}

pub struct ValkeyCursorStore {
    client: redis::Client,
}

impl ValkeyCursorStore {
    pub fn new(url: &str) -> Result<Self> {
        Ok(Self {
            client: redis::Client::open(url)?,
        })
    }
}

impl CursorStore for ValkeyCursorStore {
    fn load(&self, tenant: &str) -> Result<SyncCursors> {
        let mut conn = self.client.get_connection()?;
        let stream_cursor: Option<u64> = conn.get(cursor_key(tenant))?;
        let last_round: Option<String> = conn.get(last_round_key(tenant))?;
        let last_head: Option<String> = conn.get(last_head_key(tenant))?;
        Ok(SyncCursors {
            stream_cursor: stream_cursor.unwrap_or(0),
            last_round,
            last_head,
        })
    }

    fn save(&self, tenant: &str, cursors: &SyncCursors) -> Result<()> {
        let mut conn = self.client.get_connection()?;
        let _: () = conn.set(cursor_key(tenant), cursors.stream_cursor)?;
        if let Some(last_round) = &cursors.last_round {
            let _: () = conn.set(last_round_key(tenant), last_round)?;
        }
        if let Some(last_head) = &cursors.last_head {
            let _: () = conn.set(last_head_key(tenant), last_head)?;
        }
        Ok(())
    }
}

pub fn cursor_key(tenant: &str) -> String {
    format!("sync:cursor:{}", normalize_tenant(tenant))
}

pub fn last_round_key(tenant: &str) -> String {
    format!("sync:last_round:{}", normalize_tenant(tenant))
}

pub fn last_head_key(tenant: &str) -> String {
    format!("sync:last_head:{}", normalize_tenant(tenant))
}

fn normalize_tenant(tenant: &str) -> String {
    tenant.trim().to_ascii_lowercase()
}
