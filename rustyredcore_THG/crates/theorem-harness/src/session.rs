//! Sessions: continuity handles over AgentBinding scopes.

use theorem_harness_core::{BindingMemoryScope, PublishedScope};

/// A continuity handle over an agent's binding scopes.
///
/// SDK v2 sessions map onto the AgentBinding scope model: a session is a handle
/// to a binding's working memory. Runs within a session share the in-flight
/// working-memory scope (the versioned scratchpad); state published across
/// sessions is the committed graph. This is the within-agent versus
/// between-agent boundary from the composition model, expressed at the SDK layer
/// as within-session versus across-session.
///
/// The SDK owns this continuity handle; the application owns its own
/// user-facing data. The two are separate stores.
#[derive(Clone, Debug)]
pub struct Session {
    agent_id: String,
}

impl Session {
    /// Open a session bound to an agent's scopes.
    pub fn open(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
        }
    }

    /// The agent this session is bound to.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// The within-session working-memory scope (the versioned scratchpad). Runs
    /// within this session share it.
    pub fn memory_scope(&self) -> BindingMemoryScope {
        BindingMemoryScope::for_agent(&self.agent_id)
    }

    /// The across-session published scope (the committed graph state that
    /// outlives any single session).
    pub fn published_scope(&self) -> PublishedScope {
        PublishedScope::for_agent(&self.agent_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_scopes_to_agent() {
        let session = Session::open("claude-code");
        assert_eq!(session.agent_id(), "claude-code");
        assert!(session.memory_scope().scope_id.contains("claude-code"));
        assert!(session.published_scope().scope_id.contains("claude-code"));
    }

    #[test]
    fn within_and_across_scopes_differ() {
        let session = Session::open("agent-7");
        assert_ne!(
            session.memory_scope().scope_id,
            session.published_scope().scope_id
        );
    }
}
