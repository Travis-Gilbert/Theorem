//! Sessions: continuity handles over AgentBinding scopes.

use rustyred_thg_core::GraphStore;
use theorem_harness_core::{BindingMemoryScope, PublishedScope};
use theorem_harness_runtime::{
    recall_memory, remember_memory, MemoryRecallItem, MemoryWriteInput, RecallMemoryInput,
    RememberMemoryReceipt,
};

use crate::run::{SdkError, SdkResult};

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
    tenant: String,
}

impl Session {
    /// Open a session bound to an agent's scopes, in the default tenant.
    pub fn open(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            tenant: "default".to_string(),
        }
    }

    /// Scope this session to a specific tenant.
    pub fn with_tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = tenant.into();
        self
    }

    /// The agent this session is bound to.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// The tenant this session is scoped to.
    pub fn tenant(&self) -> &str {
        &self.tenant
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

    /// Save a durable memory in this session's scope, attributed to the session's
    /// agent and tenant. `kind` selects the memory shape (for example `belief`,
    /// `feedback`, `solution`, `postmortem`).
    pub fn remember<S: GraphStore>(
        &self,
        store: &mut S,
        kind: impl Into<String>,
        title: impl Into<String>,
        content: impl Into<String>,
    ) -> SdkResult<RememberMemoryReceipt> {
        let input = MemoryWriteInput {
            tenant_slug: self.tenant.clone(),
            actor_id: self.agent_id.clone(),
            kind: kind.into(),
            title: title.into(),
            content: content.into(),
            ..MemoryWriteInput::default()
        };
        remember_memory(store, input).map_err(SdkError::from)
    }

    /// Recall memories matching `query` in this session's tenant, most relevant
    /// first, up to `limit`.
    pub fn recall<S: GraphStore>(
        &self,
        store: &mut S,
        query: impl Into<String>,
        limit: usize,
    ) -> SdkResult<Vec<MemoryRecallItem>> {
        let input = RecallMemoryInput {
            tenant_slug: self.tenant.clone(),
            actor: self.agent_id.clone(),
            query: query.into(),
            limit,
            ..RecallMemoryInput::default()
        };
        recall_memory(store, input).map_err(SdkError::from)
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

    #[test]
    fn remember_then_recall_round_trips() {
        use rustyred_thg_core::InMemoryGraphStore;
        let mut store = InMemoryGraphStore::default();
        let session = Session::open("claude-code");
        session
            .remember(
                &mut store,
                "belief",
                "RedCore is durable",
                "The Node binding persists harness state to RedCore via the AOF.",
            )
            .expect("remember");
        let hits = session.recall(&mut store, "RedCore", 10).expect("recall");
        assert!(hits
            .iter()
            .any(|item| item.content.contains("RedCore") || item.title.contains("RedCore")));
    }
}
