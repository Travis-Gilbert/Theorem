use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct FrontendEnvelope<T> {
    pub session_id: String,
    pub agent_id: String,
    pub payload: T,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum FrontendInbound {
    StartSession {
        agent_id: String,
        cwd: String,
    },
    Prompt {
        session_id: String,
        text: String,
    },
    ApproveFileWrite {
        session_id: String,
        request_id: String,
    },
    DenyFileWrite {
        session_id: String,
        request_id: String,
    },
    ApproveCommand {
        session_id: String,
        terminal_id: String,
    },
    DenyCommand {
        session_id: String,
        terminal_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum FrontendOutbound {
    SessionStarted {
        session_id: String,
        agent_id: String,
    },
    SessionUpdate {
        session_id: String,
        agent_id: String,
        update: Value,
    },
    FileWriteReview {
        session_id: String,
        agent_id: String,
        review: Value,
    },
    CommandApproval {
        session_id: String,
        agent_id: String,
        approval: Value,
    },
    CommandOutput {
        session_id: String,
        agent_id: String,
        output: Value,
    },
    Error {
        session_id: Option<String>,
        agent_id: Option<String>,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_envelopes_preserve_session_and_agent_identity() {
        let value = serde_json::to_value(FrontendOutbound::SessionUpdate {
            session_id: "s1".to_string(),
            agent_id: "claude".to_string(),
            update: serde_json::json!({"sessionUpdate": "agent_message_chunk"}),
        })
        .unwrap();

        assert_eq!(value["type"], "session_update");
        assert_eq!(value["session_id"], "s1");
        assert_eq!(value["agent_id"], "claude");
    }
}
