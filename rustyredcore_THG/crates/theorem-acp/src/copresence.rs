use std::path::Path;

use rustyred_thg_core::ActorId;
use serde::{Deserialize, Serialize};
use theorem_copresence::{CoResult, Presence, SubstratePeer};

pub const DEFAULT_COPRESENCE_SCOPE_PREFIX: &str = "commonplace.workspace";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CopresenceSession {
    pub session_id: String,
    pub agent_id: String,
    pub workspace: String,
    pub scope: String,
    pub label: String,
}

impl CopresenceSession {
    pub fn new(
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
        workspace: impl AsRef<Path>,
    ) -> Self {
        let session_id = session_id.into();
        let agent_id = agent_id.into();
        let workspace = workspace.as_ref().to_string_lossy().into_owned();
        let scope = format!("{DEFAULT_COPRESENCE_SCOPE_PREFIX}:{workspace}");
        let label = format!("{agent_id} ACP");
        Self {
            session_id,
            agent_id,
            workspace,
            scope,
            label,
        }
    }

    pub fn actor(&self) -> ActorId {
        ActorId::from_label(&format!("acp:{}:{}", self.agent_id, self.session_id))
    }
}

pub fn agent_presence(session: &CopresenceSession) -> Presence {
    Presence::agent(
        session.actor(),
        session.scope.clone(),
        session.label.clone(),
        Some(session.workspace.clone()),
    )
}

pub fn announce_agent_session(
    peer: &mut SubstratePeer,
    session: &CopresenceSession,
) -> CoResult<Presence> {
    let presence = agent_presence(session);
    peer.announce(presence.clone())?;
    Ok(presence)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acp_sessions_have_agent_presence_in_workspace_scope() {
        let session = CopresenceSession::new("s1", "claude", "/work/theorem");
        let presence = agent_presence(&session);
        assert_eq!(presence.kind, theorem_copresence::PresenceKind::Agent);
        assert_eq!(presence.scope, "commonplace.workspace:/work/theorem");
        assert_eq!(presence.label, "claude ACP");
        assert_eq!(presence.focus_region.as_deref(), Some("/work/theorem"));
    }
}
