use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::model::ModelUsage;
use crate::{AgentdError, AgentdResult};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub timestamp_unix_ms: u128,
    pub turn_id: String,
    pub prompt: String,
    pub mode: String,
    pub tool_name: Option<String>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

impl LedgerEntry {
    pub fn new(
        turn_id: String,
        prompt: String,
        mode: String,
        tool_name: Option<String>,
        usage: ModelUsage,
    ) -> Self {
        Self {
            timestamp_unix_ms: now_unix_ms(),
            turn_id,
            prompt,
            mode,
            tool_name,
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        }
    }
}

pub fn append_ledger_entry(path: &Path, entry: &LedgerEntry) -> AgentdResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(AgentdError::from)?;
    serde_json::to_writer(&mut file, entry)?;
    file.write_all(b"\n")?;
    Ok(())
}

pub fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_jsonl_ledger() {
        let path = std::env::temp_dir().join(format!(
            "theorem-agentd-ledger-{}.jsonl",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        append_ledger_entry(
            &path,
            &LedgerEntry::new(
                "turn-1".to_string(),
                "prompt".to_string(),
                "once".to_string(),
                Some("coordination_context".to_string()),
                ModelUsage {
                    prompt_tokens: Some(1),
                    completion_tokens: Some(2),
                    total_tokens: Some(3),
                },
            ),
        )
        .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"turn_id\":\"turn-1\""));
        let _ = std::fs::remove_file(path);
    }
}
