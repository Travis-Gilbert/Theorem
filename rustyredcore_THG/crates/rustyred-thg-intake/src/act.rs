//! The volume-absorber seam to delegation (Layer C2).
//!
//! The tier-two residue is where the agent acts: intake floods, the engine sorts
//! (tier one), delegation absorbs, the surface bounds. A `NeedsYou` item can
//! carry an [`AgentSuggestion`]; accepting it fires a federated-MCP affordance
//! through the existing connector path and the result is written back onto the
//! object. This is the *seam*, not a redesign of delegation - the act side is
//! already built (`rustyred-thg-connectors`); this wires the residue to it.

use commonplace::{BlobStore, Commonplace, Item};
use rustyred_thg_core::GraphStore;
use serde_json::{json, Value};

use crate::spoke::{SourceError, SourceResult};

/// What an agent does with a `NeedsYou` item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionAction {
    Draft,
    Delegate,
    Develop,
}

impl SuggestionAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            SuggestionAction::Draft => "draft",
            SuggestionAction::Delegate => "delegate",
            SuggestionAction::Develop => "develop",
        }
    }
}

/// A suggested act attached to a `NeedsYou` item. Firing it invokes
/// `affordance_id` with `arguments` through the gateway path.
#[derive(Clone, Debug)]
pub struct AgentSuggestion {
    pub action: SuggestionAction,
    /// The task type recorded with the invocation (for affordance learning).
    pub task_type: String,
    /// Which federated-MCP affordance fires when this is accepted.
    pub affordance_id: String,
    pub arguments: Value,
    /// When true, the batch-absorb path may fire this without a human; when
    /// false the item stays in the needs-you set for a person.
    pub auto_absorb: bool,
}

/// The act seam: fire a suggestion's affordance and return its result for
/// writeback onto the object. The production impl ([`ConnectorActSeam`]) wraps
/// `invoke_affordance` with `InvokePolicy::FireAllowlist`; tests use a recording
/// mock.
pub trait ActSeam {
    fn act(&mut self, suggestion: &AgentSuggestion) -> SourceResult<Value>;
}

/// Accept a suggestion on an item: fire its affordance and land the result on the
/// object so it is readable afterward (the drafted reply attaches to the email,
/// the delegated run's output to the task). Returns the updated item.
pub fn accept_suggestion<S, B>(
    commonplace: &mut Commonplace<S, B>,
    item_id: &str,
    suggestion: &AgentSuggestion,
    seam: &mut dyn ActSeam,
) -> SourceResult<Item>
where
    S: GraphStore,
    B: BlobStore,
{
    // Validate the target item BEFORE firing the affordance, so a bad item id
    // does not trigger an external side effect that then fails on writeback.
    let mut item = commonplace
        .get_item(item_id)
        .map_err(store_err)?
        .ok_or_else(|| SourceError::Mapping(format!("item {item_id} not found")))?;
    let result = seam.act(suggestion)?;
    item.extra
        .insert("agent_action".into(), json!(suggestion.action.as_str()));
    item.extra.insert("agent_result".into(), result);
    commonplace.put_item(item).map_err(store_err)
}

/// The outcome of a batch absorb: which residue items the agent cleared and
/// which still genuinely need a human.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AbsorbReport {
    pub absorbed: Vec<String>,
    pub needs_human: Vec<String>,
}

/// Hand a whole connected stream's `NeedsYou` residue to the agent: fire the
/// auto-absorbable suggestions and leave only the human-needed items (those with
/// no suggestion, or a non-auto one) in the needs-you set. This clears the
/// auto-decidable remainder of a busy first-connect down to the bounded set that
/// genuinely needs a person.
pub fn absorb_residue<S, B>(
    commonplace: &mut Commonplace<S, B>,
    residue: Vec<(String, Option<AgentSuggestion>)>,
    seam: &mut dyn ActSeam,
) -> SourceResult<AbsorbReport>
where
    S: GraphStore,
    B: BlobStore,
{
    let mut report = AbsorbReport::default();
    for (item_id, suggestion) in residue {
        match suggestion {
            Some(suggestion) if suggestion.auto_absorb => {
                accept_suggestion(commonplace, &item_id, &suggestion, seam)?;
                report.absorbed.push(item_id);
            }
            _ => report.needs_human.push(item_id),
        }
    }
    Ok(report)
}

fn store_err(error: rustyred_thg_core::GraphStoreError) -> SourceError {
    SourceError::Transport(format!("store: {error:?}"))
}

// ---- Production binding ------------------------------------------------------

/// The production act seam: fire the suggested affordance through the existing
/// connector path (`invoke_affordance` with `InvokePolicy::FireAllowlist`, the
/// human-in-the-loop gate), returning a small JSON receipt for writeback. The
/// affordance's own `writeback_policy` governs the server-side landing; this
/// consumer-side receipt is what [`accept_suggestion`] writes onto the object.
pub struct ConnectorActSeam<'a, S>
where
    S: rustyred_thg_affordances::AffordanceGraphStore,
{
    pub store: &'a mut S,
    pub tenant_id: String,
    pub actor: Option<String>,
}

impl<S> ActSeam for ConnectorActSeam<'_, S>
where
    S: rustyred_thg_affordances::AffordanceGraphStore,
{
    fn act(&mut self, suggestion: &AgentSuggestion) -> SourceResult<Value> {
        use rustyred_thg_connectors::{invoke_affordance, InvokePolicy, InvokeRequest};

        let report = invoke_affordance(
            self.store,
            InvokeRequest {
                tenant_id: self.tenant_id.clone(),
                task_type: suggestion.task_type.clone(),
                affordance_id: suggestion.affordance_id.clone(),
                arguments: suggestion.arguments.clone(),
                candidate_affordance_ids: vec![suggestion.affordance_id.clone()],
            },
            // Fire only this affordance, nothing else.
            &InvokePolicy::FireAllowlist(vec![suggestion.affordance_id.clone()]),
            self.actor.as_deref(),
        )
        .map_err(|e| SourceError::Transport(e.to_string()))?;

        Ok(json!({
            "affordance_id": suggestion.affordance_id,
            "fired": report.fired,
            "is_error": report.outcome.as_ref().map(|o| o.is_error),
            "dry_run_reason": report.dry_run_reason,
        }))
    }
}
