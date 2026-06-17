//! Module 1 - outreach state on the graph.
//!
//! A lead is the `(Role, contact)` the operator acts on. Its state lives on the
//! Role node (status + cadence fields) plus an append-only `OutreachEvent` trail
//! (`Role -has_outreach-> OutreachEvent`), so the whole history reconstructs from
//! the graph.
//!
//! The load-bearing constraint: RustyRed's node upsert REPLACES a node wholesale
//! (it does not merge properties, and there is no patch/CAS route). So every
//! status change is a read-modify-write: [`read_node`] the full Role,
//! [`apply_role_updates`] mutates the outreach keys inside its complete property
//! map, and re-upserts it - preserving title/body/embedding so rank + search keep
//! working. jobintel is a sequential CLI, so the read-then-write is race-free.

use chrono::NaiveDate;
use serde_json::{json, Value};

use crate::client::{EdgeSpec, NodeSpec, RustyRedClient};
use crate::error::{JobIntelError, Result};
use crate::graph::role_from_node;
use crate::model::{edge_id, edges, labels, props, Role};

use super::{
    fmt_timestamp, lead_type, parse_date, Clock, EventKind, LeadType, OutreachStatus,
};

/// A Role paired with its parsed outreach state. The pure queue/cadence functions
/// operate over these; the CLI builds them from `read_leads`.
#[derive(Debug, Clone)]
pub struct LeadState {
    pub role: Role,
    pub status: OutreachStatus,
    pub next_followup_at: Option<NaiveDate>,
    pub touch_count: u32,
    pub sent_at: Option<NaiveDate>,
    pub thread_id: Option<String>,
    pub draft_id: Option<String>,
    pub template: Option<String>,
    /// Recipient of the initial draft, reused by follow-ups.
    pub to: Option<String>,
}

impl LeadState {
    /// Parse a Role graph node (`{id, labels, properties}`) into a LeadState.
    /// Returns None for non-Role / malformed nodes.
    pub fn from_node(node: &Value) -> Option<Self> {
        let role = role_from_node(node)?;
        let p = node.get("properties").cloned().unwrap_or(Value::Null);
        let status = OutreachStatus::parse(str_prop(&p, props::OUTREACH_STATUS).unwrap_or_default());
        let next_followup_at = str_prop(&p, props::NEXT_FOLLOWUP_AT).and_then(parse_date);
        let sent_at = str_prop(&p, props::OUTREACH_SENT_AT).and_then(parse_date);
        let touch_count = p
            .get(props::TOUCH_COUNT)
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        Some(Self {
            status,
            next_followup_at,
            touch_count,
            sent_at,
            thread_id: str_prop(&p, props::GMAIL_THREAD_ID).map(str::to_string),
            draft_id: str_prop(&p, props::GMAIL_DRAFT_ID).map(str::to_string),
            template: str_prop(&p, props::OUTREACH_TEMPLATE).map(str::to_string),
            to: str_prop(&p, props::OUTREACH_TO)
                .map(str::to_string)
                .or_else(|| role.emails.first().cloned()),
            role,
        })
    }

    pub fn lead_type(&self) -> LeadType {
        lead_type(&self.role)
    }
}

/// The three bounded lists a `outreach queue` run shows (spec Module 1).
#[derive(Debug, Default, Clone)]
pub struct OutreachQueue {
    /// Draftable leads (status new/queued).
    pub to_draft: Vec<String>,
    /// Drafted but not yet sent (operator's one-click pending).
    pub drafted_unsent: Vec<String>,
    /// Sent leads whose follow-up is due on/before `today`.
    pub followups_due: Vec<String>,
}

/// Classify leads into the queue's three lists. Pure; ordered by role id so the
/// output is deterministic. `to_draft` ranking (priority) is `draft`'s job via
/// cadence; `queue` is a status board.
pub fn queue(leads: &[LeadState], today: NaiveDate) -> OutreachQueue {
    let mut q = OutreachQueue::default();
    for lead in leads {
        match lead.status {
            s if s.is_draftable() => q.to_draft.push(lead.role.id.clone()),
            OutreachStatus::Drafted => q.drafted_unsent.push(lead.role.id.clone()),
            OutreachStatus::Sent if is_due(lead, today) => {
                q.followups_due.push(lead.role.id.clone())
            }
            _ => {}
        }
    }
    q.to_draft.sort();
    q.drafted_unsent.sort();
    q.followups_due.sort();
    q
}

/// Sent leads whose `next_followup_at` is on/before `today` (spec Module 1
/// deliverable). A lead with no scheduled follow-up never surfaces.
pub fn due_for_followup(leads: &[LeadState], today: NaiveDate) -> Vec<String> {
    let mut due: Vec<String> = leads
        .iter()
        .filter(|l| l.status == OutreachStatus::Sent && is_due(l, today))
        .map(|l| l.role.id.clone())
        .collect();
    due.sort();
    due
}

fn is_due(lead: &LeadState, today: NaiveDate) -> bool {
    matches!(lead.next_followup_at, Some(d) if d <= today)
}

// ---- graph reads -----------------------------------------------------------

/// Read every Role node and parse it into a LeadState.
pub fn read_leads(client: &RustyRedClient) -> Result<Vec<LeadState>> {
    let nodes = client.query_nodes(labels::ROLE, None)?;
    Ok(nodes.iter().filter_map(LeadState::from_node).collect())
}

/// Fetch one Role node's full Value (`{id, labels, properties}`), or error if it
/// is gone (a status change targets a role that must exist).
pub fn read_node(client: &RustyRedClient, role_id: &str) -> Result<Value> {
    client.get_node(role_id)?.ok_or_else(|| {
        JobIntelError::Outreach(format!("role node `{role_id}` not found for state update"))
    })
}

// ---- graph writes (read-modify-write) --------------------------------------

/// Apply outreach property updates to a Role, preserving every other property.
/// This is the read-modify-write that the no-merge upsert forces.
pub fn apply_role_updates(
    client: &RustyRedClient,
    role_id: &str,
    updates: &[(&str, Value)],
) -> Result<()> {
    let node = read_node(client, role_id)?;
    apply_updates_to_node(client, &node, updates)
}

/// Same as [`apply_role_updates`] but reuses an already-fetched node Value (saves
/// a round-trip when the caller just read it).
pub fn apply_updates_to_node(
    client: &RustyRedClient,
    node: &Value,
    updates: &[(&str, Value)],
) -> Result<()> {
    let id = node
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| JobIntelError::Outreach("node missing id".into()))?
        .to_string();
    let labels = node
        .get("labels")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| vec![labels::ROLE.to_string()]);
    let mut properties = node
        .get("properties")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let map = properties
        .as_object_mut()
        .ok_or_else(|| JobIntelError::Outreach(format!("node `{id}` has non-object properties")))?;
    for (key, value) in updates {
        map.insert((*key).to_string(), value.clone());
    }
    client.upsert_node(&NodeSpec {
        id,
        labels,
        properties,
    })?;
    Ok(())
}

/// Set a lead's outreach status (spec Module 1 `set_status`). Read-modify-write.
pub fn set_status(client: &RustyRedClient, role_id: &str, status: OutreachStatus) -> Result<()> {
    apply_role_updates(
        client,
        role_id,
        &[(props::OUTREACH_STATUS, json!(status.as_str()))],
    )
}

/// Append an `OutreachEvent` to a lead's trail (spec Module 1 `log_event`).
/// Writes the event node and the `Role -has_outreach-> OutreachEvent` edge. The
/// event is never mutated.
pub fn log_event(
    client: &RustyRedClient,
    clock: &dyn Clock,
    role_id: &str,
    kind: EventKind,
    note: &str,
) -> Result<String> {
    let at = clock.now();
    let event_id = format!(
        "event:{role_id}:{kind}:{ts}",
        kind = kind.as_str(),
        ts = at.timestamp_millis()
    );
    client.upsert_node(&NodeSpec {
        id: event_id.clone(),
        labels: vec![labels::OUTREACH_EVENT.to_string()],
        properties: json!({
            props::EVENT_ID: event_id,
            props::ROLE_ID: role_id,
            props::KIND: kind.as_str(),
            props::AT: fmt_timestamp(at),
            props::NOTE: note,
        }),
    })?;
    client.upsert_edge(&EdgeSpec {
        id: edge_id(role_id, edges::HAS_OUTREACH, &event_id),
        from_id: role_id.to_string(),
        to_id: event_id.clone(),
        edge_type: edges::HAS_OUTREACH.to_string(),
        properties: json!({ props::KIND: kind.as_str(), props::AT: fmt_timestamp(at) }),
    })?;
    Ok(event_id)
}

/// One reconstructed trail entry.
#[derive(Debug, Clone)]
pub struct EventView {
    pub kind: String,
    pub at: String,
    pub note: String,
}

/// Reconstruct a lead's event trail from the graph, oldest first (spec Module 1
/// acceptance: "the trail reconstructs from the graph").
pub fn events_for_role(client: &RustyRedClient, role_id: &str) -> Result<Vec<EventView>> {
    let nodes = client.query_nodes(labels::OUTREACH_EVENT, None)?;
    let mut events: Vec<EventView> = nodes
        .iter()
        .filter_map(|n| {
            let p = n.get("properties")?;
            if str_prop(p, props::ROLE_ID)? != role_id {
                return None;
            }
            Some(EventView {
                kind: str_prop(p, props::KIND).unwrap_or_default().to_string(),
                at: str_prop(p, props::AT).unwrap_or_default().to_string(),
                note: str_prop(p, props::NOTE).unwrap_or_default().to_string(),
            })
        })
        .collect();
    events.sort_by(|a, b| a.at.cmp(&b.at));
    Ok(events)
}

// ---- helpers ---------------------------------------------------------------

fn str_prop<'a>(properties: &'a Value, key: &str) -> Option<&'a str> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;

    fn lead(id: &str, status: OutreachStatus, next: Option<&str>) -> LeadState {
        LeadState {
            role: Role {
                id: id.into(),
                company: "Acme".into(),
                company_id: "company:acme".into(),
                title: "Engineer".into(),
                location: "Remote".into(),
                url: "https://x".into(),
                body: "rust".into(),
                source: Source::Hn.as_str().into(),
                remote: true,
                contract: false,
                founder_posted: false,
                email_present: true,
                emails: vec!["a@b.com".into()],
                comp: None,
                company_domain: None,
            },
            status,
            next_followup_at: next.and_then(parse_date),
            touch_count: 0,
            sent_at: None,
            thread_id: None,
            draft_id: None,
            template: None,
            to: Some("a@b.com".into()),
        }
    }

    #[test]
    fn from_node_reads_role_plus_outreach_state() {
        let node = json!({
            "id": "role:hn:7",
            "labels": ["Role"],
            "properties": {
                "title": "Rust Engineer",
                "company": "Qdrant",
                "company_id": "company:qdrant",
                "source": "hn",
                "body": "rust graph",
                "outreach_status": "sent",
                "touch_count": 1,
                "next_followup_at": "2026-06-20",
                "outreach_sent_at": "2026-06-16",
                "gmail_thread_id": "t-9",
                "gmail_draft_id": "d-9",
                "outreach_template": "hn_founder"
            }
        });
        let lead = LeadState::from_node(&node).unwrap();
        assert_eq!(lead.status, OutreachStatus::Sent);
        assert_eq!(lead.touch_count, 1);
        assert_eq!(lead.next_followup_at, parse_date("2026-06-20"));
        assert_eq!(lead.sent_at, parse_date("2026-06-16"));
        assert_eq!(lead.thread_id.as_deref(), Some("t-9"));
        assert_eq!(lead.draft_id.as_deref(), Some("d-9"));
        assert_eq!(lead.template.as_deref(), Some("hn_founder"));
    }

    #[test]
    fn from_node_defaults_unseen_role_to_new() {
        let node = json!({
            "id": "role:hn:1", "labels": ["Role"],
            "properties": { "title": "x", "company": "y", "company_id": "company:y", "source": "hn", "body": "rust" }
        });
        let lead = LeadState::from_node(&node).unwrap();
        assert_eq!(lead.status, OutreachStatus::New);
        assert_eq!(lead.touch_count, 0);
        assert!(lead.next_followup_at.is_none());
    }

    #[test]
    fn queue_partitions_into_three_bounded_lists() {
        let today = parse_date("2026-06-20").unwrap();
        let leads = vec![
            lead("role:new", OutreachStatus::New, None),
            lead("role:queued", OutreachStatus::Queued, None),
            lead("role:drafted", OutreachStatus::Drafted, None),
            lead("role:sent_due", OutreachStatus::Sent, Some("2026-06-18")),
            lead("role:sent_future", OutreachStatus::Sent, Some("2026-06-25")),
            lead("role:replied", OutreachStatus::Replied, Some("2026-06-18")),
            lead("role:dead", OutreachStatus::Dead, None),
        ];
        let q = queue(&leads, today);
        assert_eq!(q.to_draft, vec!["role:new", "role:queued"]);
        assert_eq!(q.drafted_unsent, vec!["role:drafted"]);
        // Only the past-due sent lead; replied/dead/future excluded.
        assert_eq!(q.followups_due, vec!["role:sent_due"]);
    }

    #[test]
    fn due_for_followup_is_sent_and_on_or_before_today() {
        let today = parse_date("2026-06-20").unwrap();
        let leads = vec![
            lead("role:today", OutreachStatus::Sent, Some("2026-06-20")), // boundary: due
            lead("role:past", OutreachStatus::Sent, Some("2026-06-10")),
            lead("role:future", OutreachStatus::Sent, Some("2026-06-21")),
            lead("role:drafted", OutreachStatus::Drafted, Some("2026-06-10")), // not sent
        ];
        let due = due_for_followup(&leads, today);
        assert_eq!(due, vec!["role:past", "role:today"]);
    }
}
