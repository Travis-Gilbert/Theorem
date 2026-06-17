//! jobintel 0.2 - the outreach engine.
//!
//! 0.1 turns open job sources into a ranked role graph. 0.2 turns that ranked
//! backlog into a tracked outreach loop: triage to a small daily queue, draft
//! into Gmail (never send), follow up on a cadence, learn from replies.
//!
//! Module map (one file per spec module):
//! - [`state`] (Module 1): outreach state machine on the Role graph + the append-only `OutreachEvent` trail.
//! - [`gmail`]: the Gmail create-draft / thread-read transport (a second HTTP seam, base-URL-swappable for the mock-server test).
//! - [`draft`] (Module 2): render a per-lead email from a template set and create it as a Gmail draft.
//! - [`cadence`] (Module 3): the daily cap and the day-4/day-9 follow-up schedule.
//! - [`sync`] (Module 4): reply detection (and drafted -> sent advancement).
//! - [`outcomes`] (Module 5): the per-template / per-lead-type learning signal.
//!
//! Shared invariant: jobintel writes drafts, never sends. The daily cap and the
//! two-nudge ceiling are the safety rails, kept by design (spec "Guardrail").

pub mod cadence;
pub mod draft;
pub mod gmail;
pub mod outcomes;
pub mod state;
pub mod sync;

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};

use crate::model::Role;

/// The outreach lifecycle of a `(Role, contact)` lead (spec Module 1). A Role
/// with no `outreach_status` property reads as `New`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutreachStatus {
    New,
    Queued,
    Drafted,
    Sent,
    Replied,
    Dead,
}

impl OutreachStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            OutreachStatus::New => "new",
            OutreachStatus::Queued => "queued",
            OutreachStatus::Drafted => "drafted",
            OutreachStatus::Sent => "sent",
            OutreachStatus::Replied => "replied",
            OutreachStatus::Dead => "dead",
        }
    }

    /// Parse a stored status string. Unknown / absent values default to `New`,
    /// so a freshly ingested Role is draftable without a migration step.
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_lowercase().as_str() {
            "queued" => OutreachStatus::Queued,
            "drafted" => OutreachStatus::Drafted,
            "sent" => OutreachStatus::Sent,
            "replied" => OutreachStatus::Replied,
            "dead" => OutreachStatus::Dead,
            _ => OutreachStatus::New,
        }
    }

    /// New + Queued are both draftable (spec Module 3: "skip anything not
    /// new/queued"). The automatic pipeline goes new -> drafted; `queued` is
    /// honored if an operator/external process sets it.
    pub fn is_draftable(self) -> bool {
        matches!(self, OutreachStatus::New | OutreachStatus::Queued)
    }

    /// Terminal states record an outcome and never re-enter the queue.
    pub fn is_terminal(self) -> bool {
        matches!(self, OutreachStatus::Replied | OutreachStatus::Dead)
    }
}

/// The append-only event kinds on the `OutreachEvent` trail (spec Module 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Drafted,
    Sent,
    ReplyDetected,
    FollowupDrafted,
    MarkedDead,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Drafted => "drafted",
            EventKind::Sent => "sent",
            EventKind::ReplyDetected => "reply_detected",
            EventKind::FollowupDrafted => "followup_drafted",
            EventKind::MarkedDead => "marked_dead",
        }
    }
}

/// Lead type drives both the template choice (Module 2) and the stats grouping
/// (Module 5), so it lives here as the single source of truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeadType {
    /// An explicit contract / freelance posting. Highest precedence: it is the
    /// strongest signal for an operator seeking contract work, whatever the
    /// source. (Divergence noted in the README: the spec lists the three types
    /// without precedence; contract_explicit winning is a named choice.)
    ContractExplicit,
    /// An HN "Who is Hiring" lead written by the founder/poster you email direct.
    HnFounder,
    /// An ATS board role (Greenhouse / Lever / Ashby): more formal, company-addressed.
    AtsRole,
}

impl LeadType {
    /// Template-file key, also the stored `lead_type` on the OutcomeRecord.
    pub fn as_str(self) -> &'static str {
        match self {
            LeadType::ContractExplicit => "contract_explicit",
            LeadType::HnFounder => "hn_founder",
            LeadType::AtsRole => "ats_role",
        }
    }
}

/// Classify a Role into a [`LeadType`]. Total by construction: every role is
/// contract or not; if not contract, it is an HN lead or an ATS lead.
pub fn lead_type(role: &Role) -> LeadType {
    if role.contract {
        LeadType::ContractExplicit
    } else if role.source.eq_ignore_ascii_case("hn") {
        LeadType::HnFounder
    } else {
        LeadType::AtsRole
    }
}

/// Time seam. The CLI uses [`SystemClock`]; tests inject a fixed instant so the
/// event timestamps and "today" are deterministic. Pure date math lives in
/// `cadence` and takes dates directly, so only timestamp-stamping needs a clock.
pub trait Clock {
    fn now(&self) -> DateTime<Utc>;
    fn today(&self) -> NaiveDate {
        self.now().date_naive()
    }
}

/// Wall-clock UTC.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Format an instant as the stored `at` / `last_touch_at` timestamp (RFC3339,
/// second precision - the cadence math only needs day granularity).
pub fn fmt_timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Format a date as the stored `next_followup_at` / `outreach_sent_at` (ISO date).
pub fn fmt_date(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Parse a stored ISO date back to a `NaiveDate`.
pub fn parse_date(raw: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d").ok()
}

#[cfg(test)]
pub(crate) mod testclock {
    use super::*;

    /// A clock pinned to a fixed instant for deterministic tests.
    pub struct FixedClock(pub DateTime<Utc>);

    impl FixedClock {
        /// Build from an ISO date at 12:00:00Z.
        pub fn on(date: &str) -> Self {
            let d = parse_date(date).expect("valid test date");
            FixedClock(
                d.and_hms_opt(12, 0, 0)
                    .expect("valid time")
                    .and_utc(),
            )
        }
    }

    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Role, Source};

    fn role(source: Source, contract: bool, founder: bool) -> Role {
        Role {
            id: "role:x".into(),
            company: "Acme".into(),
            company_id: "company:acme".into(),
            title: "Engineer".into(),
            location: "Remote".into(),
            url: "https://x".into(),
            body: "rust".into(),
            source: source.as_str().into(),
            remote: true,
            contract,
            founder_posted: founder,
            email_present: true,
            emails: vec!["a@b.com".into()],
            comp: None,
            company_domain: None,
        }
    }

    #[test]
    fn status_round_trips_and_defaults_to_new() {
        for s in [
            OutreachStatus::New,
            OutreachStatus::Queued,
            OutreachStatus::Drafted,
            OutreachStatus::Sent,
            OutreachStatus::Replied,
            OutreachStatus::Dead,
        ] {
            assert_eq!(OutreachStatus::parse(s.as_str()), s);
        }
        assert_eq!(OutreachStatus::parse(""), OutreachStatus::New);
        assert_eq!(OutreachStatus::parse("bogus"), OutreachStatus::New);
    }

    #[test]
    fn draftable_and_terminal_partition_the_lifecycle() {
        assert!(OutreachStatus::New.is_draftable());
        assert!(OutreachStatus::Queued.is_draftable());
        assert!(!OutreachStatus::Drafted.is_draftable());
        assert!(OutreachStatus::Replied.is_terminal());
        assert!(OutreachStatus::Dead.is_terminal());
        assert!(!OutreachStatus::Sent.is_terminal());
    }

    #[test]
    fn lead_type_is_total_with_contract_taking_precedence() {
        // contract wins even on an HN founder post.
        assert_eq!(
            lead_type(&role(Source::Hn, true, true)),
            LeadType::ContractExplicit
        );
        assert_eq!(
            lead_type(&role(Source::Hn, false, true)),
            LeadType::HnFounder
        );
        // HN but not founder is still an HN lead (not ATS).
        assert_eq!(
            lead_type(&role(Source::Hn, false, false)),
            LeadType::HnFounder
        );
        assert_eq!(
            lead_type(&role(Source::Greenhouse, false, false)),
            LeadType::AtsRole
        );
        assert_eq!(
            lead_type(&role(Source::Lever, false, false)),
            LeadType::AtsRole
        );
    }

    #[test]
    fn date_round_trips() {
        let d = parse_date("2026-06-16").unwrap();
        assert_eq!(fmt_date(d), "2026-06-16");
        assert_eq!(parse_date("not-a-date"), None);
    }

    #[test]
    fn fixed_clock_reports_its_pinned_date() {
        let clock = testclock::FixedClock::on("2026-06-16");
        assert_eq!(clock.today(), parse_date("2026-06-16").unwrap());
    }
}
