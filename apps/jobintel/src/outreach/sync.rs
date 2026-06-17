//! Module 4 - reply detection (and drafted -> sent advancement).
//!
//! `outreach sync` reconciles each tracked lead against Gmail so follow-ups stop
//! on a reply and outcomes are recorded. Two transitions, scoped to threads
//! jobintel created (a thread id is on file):
//!
//! - reply: a message in the thread whose sender matches the lead's stored
//!   addresses -> `replied` (clear the follow-up, record the outcome).
//! - sent: a `drafted` lead whose Gmail draft has left the drafts list while the
//!   thread still holds a message -> `sent` (the operator clicked send; schedule
//!   the first follow-up). This is what advances the loop without a manual verb,
//!   since jobintel never sends. The send date is taken as the sync date.
//!
//! Reply detection is the spec's acceptance; sent advancement is the complement
//! that gives the cadence `sent` leads to act on (README divergence note).

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::client::RustyRedClient;
use crate::error::Result;
use crate::model::props;

use super::cadence::next_followup_date;
use super::gmail::GmailClient;
use super::outcomes::record_outcome;
use super::state::{apply_role_updates, log_event, read_leads, LeadState};
use super::{fmt_date, fmt_timestamp, Clock, EventKind, OutreachStatus};

/// The reply/sent-detection seam: the two Gmail reads `sync` needs. Tests inject
/// a fake; `GmailReplySource` is the live impl.
pub trait ReplySource {
    /// Lowercased sender addresses seen in `thread_id`.
    fn thread_senders(&self, thread_id: &str) -> Result<Vec<String>>;
    /// Ids of the operator's currently-live drafts.
    fn live_draft_ids(&self) -> Result<Vec<String>>;
}

/// Default `ReplySource` over the real Gmail client.
pub struct GmailReplySource {
    gmail: GmailClient,
}

impl GmailReplySource {
    pub fn new(gmail: GmailClient) -> Self {
        Self { gmail }
    }
}

impl ReplySource for GmailReplySource {
    fn thread_senders(&self, thread_id: &str) -> Result<Vec<String>> {
        self.gmail.thread_senders(thread_id)
    }
    fn live_draft_ids(&self) -> Result<Vec<String>> {
        self.gmail.list_draft_ids()
    }
}

#[derive(Debug, Default, Clone)]
pub struct SyncRunStats {
    /// Leads with a thread id that were reconciled.
    pub checked: usize,
    /// Leads flipped to `replied` this run.
    pub replied: usize,
    /// Leads advanced drafted -> sent this run.
    pub advanced_sent: usize,
}

/// Reconcile every tracked lead against Gmail (spec Module 4). Fetches the live
/// draft ids once, then per lead detects a reply or a send.
pub fn run_sync(
    client: &RustyRedClient,
    source: &dyn ReplySource,
    clock: &dyn Clock,
    followup_days: &[u32],
) -> Result<SyncRunStats> {
    let leads = read_leads(client)?;
    let live_drafts: HashSet<String> = source.live_draft_ids()?.into_iter().collect();

    let mut stats = SyncRunStats::default();
    for lead in &leads {
        // Only reconcile drafted/sent leads with a thread on file.
        if !matches!(lead.status, OutreachStatus::Drafted | OutreachStatus::Sent) {
            continue;
        }
        let Some(thread_id) = lead.thread_id.clone() else {
            continue;
        };
        stats.checked += 1;

        let senders = source.thread_senders(&thread_id)?;
        if reply_detected(lead, &senders) {
            mark_replied(client, clock, lead)?;
            stats.replied += 1;
            continue;
        }

        if lead.status == OutreachStatus::Drafted
            && looks_sent(lead.draft_id.as_deref(), &live_drafts, &senders)
        {
            advance_to_sent(client, clock, lead, followup_days)?;
            stats.advanced_sent += 1;
        }
    }
    Ok(stats)
}

/// True when any thread sender matches one of the lead's stored addresses.
fn reply_detected(lead: &LeadState, senders: &[String]) -> bool {
    let lead_addrs = lead_addresses(lead);
    senders.iter().any(|s| lead_addrs.contains(&s.to_lowercase()))
}

/// The lead's known addresses (the drafted-to contact plus any ingested emails),
/// lowercased. A reply from any of these is the lead replying.
fn lead_addresses(lead: &LeadState) -> HashSet<String> {
    let mut set: HashSet<String> = lead
        .role
        .emails
        .iter()
        .map(|e| e.trim().to_lowercase())
        .collect();
    if let Some(to) = &lead.to {
        set.insert(to.trim().to_lowercase());
    }
    set
}

/// A drafted lead's draft has left the drafts list while the thread still holds a
/// message => the operator sent it (vs deleting it, which empties the thread).
fn looks_sent(draft_id: Option<&str>, live_drafts: &HashSet<String>, senders: &[String]) -> bool {
    match draft_id {
        Some(id) => !live_drafts.contains(id) && !senders.is_empty(),
        None => false,
    }
}

fn mark_replied(client: &RustyRedClient, clock: &dyn Clock, lead: &LeadState) -> Result<()> {
    apply_role_updates(
        client,
        &lead.role.id,
        &[
            (props::OUTREACH_STATUS, json!(OutreachStatus::Replied.as_str())),
            (props::NEXT_FOLLOWUP_AT, Value::Null),
            (props::LAST_TOUCH_AT, json!(fmt_timestamp(clock.now()))),
        ],
    )?;
    log_event(
        client,
        clock,
        &lead.role.id,
        EventKind::ReplyDetected,
        "reply from lead address",
    )?;
    record_outcome(client, clock, lead, OutreachStatus::Replied)
}

fn advance_to_sent(
    client: &RustyRedClient,
    clock: &dyn Clock,
    lead: &LeadState,
    followup_days: &[u32],
) -> Result<()> {
    let today = clock.today();
    // Initial send counts as touch 1; first nudge is scheduled at sent + days[0].
    let next = next_followup_date(1, today, followup_days);
    apply_role_updates(
        client,
        &lead.role.id,
        &[
            (props::OUTREACH_STATUS, json!(OutreachStatus::Sent.as_str())),
            (props::OUTREACH_SENT_AT, json!(fmt_date(today))),
            (props::TOUCH_COUNT, json!(1u32)),
            (props::LAST_TOUCH_AT, json!(fmt_timestamp(clock.now()))),
            (
                props::NEXT_FOLLOWUP_AT,
                next.map(|d| json!(fmt_date(d))).unwrap_or(Value::Null),
            ),
        ],
    )?;
    log_event(
        client,
        clock,
        &lead.role.id,
        EventKind::Sent,
        "draft sent by operator (detected via Gmail)",
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Role, Source};
    use crate::outreach::OutreachStatus;

    fn lead_with(
        status: OutreachStatus,
        draft_id: Option<&str>,
        thread_id: Option<&str>,
        emails: &[&str],
        to: Option<&str>,
    ) -> LeadState {
        LeadState {
            role: Role {
                id: "role:hn:1".into(),
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
                email_present: !emails.is_empty(),
                emails: emails.iter().map(|e| e.to_string()).collect(),
                comp: None,
                company_domain: None,
            },
            status,
            next_followup_at: None,
            touch_count: 0,
            sent_at: None,
            thread_id: thread_id.map(String::from),
            draft_id: draft_id.map(String::from),
            template: Some("hn_founder".into()),
            to: to.map(String::from),
        }
    }

    #[test]
    fn reply_detected_matches_lead_address_case_insensitively() {
        let lead = lead_with(
            OutreachStatus::Sent,
            None,
            Some("t1"),
            &["lead@company.com"],
            Some("Lead@Company.com"),
        );
        assert!(reply_detected(&lead, &["LEAD@company.com".into()]));
        assert!(!reply_detected(&lead, &["me@self.com".into()]));
        assert!(!reply_detected(&lead, &[]));
    }

    #[test]
    fn looks_sent_only_when_draft_gone_and_thread_nonempty() {
        let live: HashSet<String> = ["d-live".to_string()].into_iter().collect();
        // draft still live -> not sent
        assert!(!looks_sent(Some("d-live"), &live, &["me@self.com".into()]));
        // draft gone + thread has a message -> sent
        assert!(looks_sent(Some("d-gone"), &live, &["me@self.com".into()]));
        // draft gone but empty thread -> deleted, not sent
        assert!(!looks_sent(Some("d-gone"), &live, &[]));
        // no draft id -> not sent
        assert!(!looks_sent(None, &live, &["me@self.com".into()]));
    }
}
