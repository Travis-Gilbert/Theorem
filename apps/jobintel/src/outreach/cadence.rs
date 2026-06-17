//! Module 3 - cadence and follow-up.
//!
//! Two rails: a daily draft cap, and a fixed follow-up schedule (day 4 and day 9
//! after send, then stop). The cap drains the backlog over days, never in a
//! blast; the two-nudge ceiling keeps personalization, not volume, doing the work
//! (spec "Guardrail").
//!
//! The date math is pure (`select_for_draft`, `next_followup_action`,
//! `next_followup_date`) and unit-tested with fixed dates. `draft_followup` and
//! `run_followups` are the orchestration that drives Gmail + the graph state.
//!
//! Lifecycle of a sent lead (touch_count = total outbound touches; 1 = initial
//! send): at send, next_followup_at = sent + days[0]. Each `followups` run that
//! finds it due drafts the next nudge and advances the schedule. After the last
//! nudge, the schedule points one day past the final interval; the next run reaps
//! the lead to `dead` if no reply arrived.

use std::collections::HashMap;

use chrono::{Days, NaiveDate};
use serde_json::{json, Value};

use crate::client::RustyRedClient;
use crate::error::Result;
use crate::model::{props, ScoredLead};
use crate::profile::ResolvedProfile;

use super::draft::{render_followup, DraftSink};
use super::outcomes::record_outcome;
use super::state::{
    apply_role_updates, due_for_followup, log_event, read_leads, LeadState,
};
use super::{fmt_date, fmt_timestamp, Clock, EventKind, OutreachStatus};

/// Days after the final follow-up interval at which a non-replying lead is reaped
/// to `dead`. One day, so reaping never collides with drafting the last nudge.
const REAP_AFTER_LAST_DAYS: u32 = 1;

/// Select leads to draft this run, in rank order: skip anything not draftable
/// (status new/queued) or without a resolvable contact, then cap. Pure.
pub fn select_for_draft(
    ranked: &[ScoredLead],
    statuses: &HashMap<String, OutreachStatus>,
    cap: usize,
) -> Vec<String> {
    ranked
        .iter()
        .filter(|l| l.contact.is_some() && !l.needs_contact)
        .filter(|l| {
            statuses
                .get(&l.role.id)
                .copied()
                .unwrap_or(OutreachStatus::New)
                .is_draftable()
        })
        .take(cap)
        .map(|l| l.role.id.clone())
        .collect()
}

/// What `followups` should do with a due lead next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowupAction {
    /// Draft nudge `index` (0-based into the schedule); it is due on `due`.
    Nudge { index: usize, due: NaiveDate },
    /// All nudges sent; mark the lead dead on/after `due`.
    Reap { due: NaiveDate },
}

impl FollowupAction {
    pub fn due(self) -> NaiveDate {
        match self {
            FollowupAction::Nudge { due, .. } | FollowupAction::Reap { due } => due,
        }
    }
}

/// The next cadence action for a sent lead given how many touches it has had.
/// `touch_count` is total outbound touches (1 after the initial send).
pub fn next_followup_action(
    touch_count: u32,
    sent_at: NaiveDate,
    days: &[u32],
) -> Option<FollowupAction> {
    if touch_count == 0 || days.is_empty() {
        return None;
    }
    let nudges_done = (touch_count - 1) as usize;
    if nudges_done < days.len() {
        add_days(sent_at, days[nudges_done]).map(|due| FollowupAction::Nudge {
            index: nudges_done,
            due,
        })
    } else {
        let last = *days.last().expect("non-empty checked above");
        add_days(sent_at, last + REAP_AFTER_LAST_DAYS).map(|due| FollowupAction::Reap { due })
    }
}

/// The date a sent lead next needs attention (spec Module 1 `next_followup_date`),
/// i.e. the `due` of [`next_followup_action`]. None only when there is no anchor.
pub fn next_followup_date(touch_count: u32, sent_at: NaiveDate, days: &[u32]) -> Option<NaiveDate> {
    next_followup_action(touch_count, sent_at, days).map(FollowupAction::due)
}

fn add_days(date: NaiveDate, n: u32) -> Option<NaiveDate> {
    date.checked_add_days(Days::new(n as u64))
}

/// The result of acting on one due lead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowupOutcome {
    Nudged { draft_id: String },
    Reaped,
    Skipped,
}

/// Act on one due lead (spec Module 3 `draft_followup`): either draft the next
/// in-thread nudge (advancing touch_count + schedule) or reap it to `dead`.
pub fn draft_followup(
    client: &RustyRedClient,
    sink: &dyn DraftSink,
    clock: &dyn Clock,
    profile: &ResolvedProfile,
    lead: &LeadState,
    days: &[u32],
) -> Result<FollowupOutcome> {
    let Some(sent_at) = lead.sent_at else {
        return Ok(FollowupOutcome::Skipped);
    };
    match next_followup_action(lead.touch_count, sent_at, days) {
        Some(FollowupAction::Nudge { index, .. }) => {
            let Some(to) = lead.to.clone().or_else(|| lead.role.emails.first().cloned()) else {
                return Ok(FollowupOutcome::Skipped);
            };
            let email = render_followup(profile, &lead.role);
            // Thread the nudge into the original conversation (spec: same Gmail
            // thread, replyToMessageId = gmail_thread_id).
            let draft = sink.create_draft(
                &to,
                &email.subject,
                &email.body,
                lead.thread_id.as_deref(),
            )?;
            let new_touch = lead.touch_count + 1;
            let next = next_followup_date(new_touch, sent_at, days);
            apply_role_updates(
                client,
                &lead.role.id,
                &[
                    (props::TOUCH_COUNT, json!(new_touch)),
                    (props::GMAIL_DRAFT_ID, json!(draft.draft_id)),
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
                EventKind::FollowupDrafted,
                &format!("nudge {} (day {}) to {}", index + 1, days[index], to),
            )?;
            Ok(FollowupOutcome::Nudged {
                draft_id: draft.draft_id,
            })
        }
        Some(FollowupAction::Reap { .. }) => {
            apply_role_updates(
                client,
                &lead.role.id,
                &[
                    (props::OUTREACH_STATUS, json!(OutreachStatus::Dead.as_str())),
                    (props::NEXT_FOLLOWUP_AT, Value::Null),
                ],
            )?;
            log_event(
                client,
                clock,
                &lead.role.id,
                EventKind::MarkedDead,
                "no reply after final follow-up",
            )?;
            record_outcome(client, clock, lead, OutreachStatus::Dead)?;
            Ok(FollowupOutcome::Reaped)
        }
        None => Ok(FollowupOutcome::Skipped),
    }
}

#[derive(Debug, Default, Clone)]
pub struct FollowupRunStats {
    pub nudged: usize,
    pub reaped: usize,
    pub skipped: usize,
    pub created: Vec<(String, String)>,
}

/// Draft the next touch for every lead past its follow-up date, and reap the
/// exhausted ones (spec Module 3). Reads leads, finds the due set, acts on each.
pub fn run_followups(
    client: &RustyRedClient,
    sink: &dyn DraftSink,
    clock: &dyn Clock,
    profile: &ResolvedProfile,
    days: &[u32],
) -> Result<FollowupRunStats> {
    let today = clock.today();
    let leads = read_leads(client)?;
    let due = due_for_followup(&leads, today);
    let by_id: HashMap<&str, &LeadState> =
        leads.iter().map(|l| (l.role.id.as_str(), l)).collect();

    let mut stats = FollowupRunStats::default();
    for role_id in &due {
        let Some(lead) = by_id.get(role_id.as_str()) else {
            continue;
        };
        match draft_followup(client, sink, clock, profile, lead, days)? {
            FollowupOutcome::Nudged { draft_id } => {
                stats.nudged += 1;
                stats.created.push((lead.role.company.clone(), draft_id));
            }
            FollowupOutcome::Reaped => stats.reaped += 1,
            FollowupOutcome::Skipped => stats.skipped += 1,
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Role, Source};
    use crate::outreach::parse_date;

    fn scored(id: &str, contact: Option<&str>) -> ScoredLead {
        ScoredLead {
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
                email_present: contact.is_some(),
                emails: contact.map(|c| vec![c.to_string()]).unwrap_or_default(),
                comp: None,
                company_domain: None,
            },
            score: 0.5,
            semantic: 0.0,
            graph: 0.0,
            flags: 0.0,
            matched_skills: vec![],
            contact: contact.map(String::from),
            needs_contact: contact.is_none(),
        }
    }

    #[test]
    fn select_skips_non_draftable_and_uncontactable_then_caps() {
        let ranked = vec![
            scored("role:a", Some("a@x.com")), // draftable + contact
            scored("role:b", None),            // no contact -> skip
            scored("role:c", Some("c@x.com")), // already sent -> skip
            scored("role:d", Some("d@x.com")), // draftable + contact
            scored("role:e", Some("e@x.com")), // draftable, but over cap
        ];
        let mut statuses = HashMap::new();
        statuses.insert("role:c".to_string(), OutreachStatus::Sent);
        let picked = select_for_draft(&ranked, &statuses, 2);
        assert_eq!(picked, vec!["role:a", "role:d"]); // rank order, b/c skipped, cap 2
    }

    #[test]
    fn cap_bounds_a_large_request() {
        let ranked: Vec<ScoredLead> = (0..50)
            .map(|i| scored(&format!("role:{i}"), Some("x@y.com")))
            .collect();
        let picked = select_for_draft(&ranked, &HashMap::new(), 8);
        assert_eq!(picked.len(), 8, "cap 8 bounds a draft --top 50");
    }

    #[test]
    fn followup_schedule_walks_day4_day9_then_reaps() {
        let sent = parse_date("2026-06-01").unwrap();
        let days = [4, 9];
        // touch 1 (just sent): next nudge at day 4.
        assert_eq!(
            next_followup_action(1, sent, &days),
            Some(FollowupAction::Nudge {
                index: 0,
                due: parse_date("2026-06-05").unwrap()
            })
        );
        // touch 2 (after nudge 1): next nudge at day 9.
        assert_eq!(
            next_followup_action(2, sent, &days),
            Some(FollowupAction::Nudge {
                index: 1,
                due: parse_date("2026-06-10").unwrap()
            })
        );
        // touch 3 (after nudge 2): reap one day after the last interval.
        assert_eq!(
            next_followup_action(3, sent, &days),
            Some(FollowupAction::Reap {
                due: parse_date("2026-06-11").unwrap()
            })
        );
    }

    #[test]
    fn next_followup_date_matches_action_due() {
        let sent = parse_date("2026-06-01").unwrap();
        assert_eq!(next_followup_date(1, sent, &[4, 9]), parse_date("2026-06-05"));
        assert_eq!(next_followup_date(0, sent, &[4, 9]), None);
    }
}
