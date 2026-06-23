//! Module 2 - draft into Gmail.
//!
//! For each queued lead: assemble the same context the 0.1 draft module uses
//! (role text, matched skills, contact, proof points), render it through a
//! template chosen by lead type, and create it as a Gmail draft. jobintel never
//! sends; the operator clicks send.
//!
//! Templates live in `templates/outreach/*.txt` and are compiled in via
//! `include_str!` so a draft renders from any working directory. The first
//! `Subject:` line is the subject template; the rest is the body. Rendering is
//! deterministic substitution of the role's own language plus the fixed proof
//! block - jobintel has no model in the loop, so the draft is a complete,
//! editable first message, not a slot-filled stub (README divergence note).

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::client::RustyRedClient;
use crate::error::{JobIntelError, Result};
use crate::graph::skills_of;
use crate::model::{props, Role, ScoredLead};
use crate::profile::ResolvedProfile;

use super::gmail::{DraftRef, GmailClient};
use super::state::{apply_role_updates, log_event};
use super::{fmt_timestamp, lead_type, Clock, EventKind, LeadType, OutreachStatus};

const HN_FOUNDER_TPL: &str = include_str!("../../templates/outreach/hn_founder.txt");
const ATS_ROLE_TPL: &str = include_str!("../../templates/outreach/ats_role.txt");
const CONTRACT_TPL: &str = include_str!("../../templates/outreach/contract_explicit.txt");
const FOLLOWUP_TPL: &str = include_str!("../../templates/outreach/followup.txt");

/// Max characters of the role's own language quoted into the email.
const SNIPPET_CHARS: usize = 200;

/// The draft-creation seam. `GmailDraftSink` is the default impl; tests inject a
/// fake so the draft flow is exercised without network.
pub trait DraftSink {
    fn create_draft(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        thread_id: Option<&str>,
    ) -> Result<DraftRef>;
}

/// Default `DraftSink`: the real Gmail create-draft call.
pub struct GmailDraftSink {
    gmail: GmailClient,
}

impl GmailDraftSink {
    pub fn new(gmail: GmailClient) -> Self {
        Self { gmail }
    }
}

impl DraftSink for GmailDraftSink {
    fn create_draft(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        thread_id: Option<&str>,
    ) -> Result<DraftRef> {
        self.gmail.create_draft(to, subject, body, thread_id)
    }
}

/// A rendered email: subject + plaintext body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedEmail {
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Default, Clone)]
pub struct DraftRunStats {
    pub drafted: usize,
    /// Selected leads skipped because they had no resolvable contact.
    pub skipped_no_contact: usize,
    /// (company, draft_id) for the report.
    pub created: Vec<(String, String)>,
}

/// Draft the initial outreach for each selected lead (already filtered to
/// draftable + capped by `cadence::select_for_draft`). Per lead: render -> create
/// Gmail draft -> flip the Role to `drafted` (read-modify-write) -> log the event.
/// Idempotent at the selection boundary: a lead already `drafted` or later is not
/// in `selected`, so re-running never double-drafts.
pub fn draft_top(
    client: &RustyRedClient,
    sink: &dyn DraftSink,
    clock: &dyn Clock,
    profile: &ResolvedProfile,
    selected: &[ScoredLead],
) -> Result<DraftRunStats> {
    let mut stats = DraftRunStats::default();
    for lead in selected {
        if lead.contact.is_none() {
            stats.skipped_no_contact += 1;
            continue;
        }
        let draft = draft_lead(client, sink, clock, profile, lead)?;
        stats.drafted += 1;
        stats.created.push((lead.role.company.clone(), draft.draft_id));
    }
    Ok(stats)
}

/// Draft one lead: render the lead-type email, create the Gmail draft (new
/// thread), store an auditable pack, then read-modify-write the Role to `drafted`
/// with the gmail ids + template and log the event. Returns the DraftRef. Takes
/// the resolved [`ScoredLead`] (not a bare role_id) because the email needs the
/// ranked + contact-resolved context the selection step already produced.
pub fn draft_lead(
    client: &RustyRedClient,
    sink: &dyn DraftSink,
    clock: &dyn Clock,
    profile: &ResolvedProfile,
    lead: &ScoredLead,
) -> Result<DraftRef> {
    let to = lead.contact.clone().ok_or_else(|| {
        JobIntelError::Outreach(format!("lead {} has no contact to draft to", lead.role.id))
    })?;
    let lt = lead_type(&lead.role);
    let email = render_initial(profile, lead, lt);
    let draft = sink.create_draft(&to, &email.subject, &email.body, None)?;
    // Dual-use: keep an auditable server-side artifact of what was drafted.
    record_pack(client, lead, &email);

    apply_role_updates(
        client,
        &lead.role.id,
        &[
            (props::OUTREACH_STATUS, json!(OutreachStatus::Drafted.as_str())),
            (props::OUTREACH_TEMPLATE, json!(lt.as_str())),
            (props::OUTREACH_TO, json!(to)),
            (props::GMAIL_DRAFT_ID, json!(draft.draft_id)),
            (props::GMAIL_THREAD_ID, json!(draft.thread_id)),
            (props::LAST_TOUCH_AT, json!(fmt_timestamp(clock.now()))),
            (props::TOUCH_COUNT, json!(0)),
        ],
    )?;
    log_event(
        client,
        clock,
        &lead.role.id,
        EventKind::Drafted,
        &format!("template={} to={}", lt.as_str(), to),
    )?;
    Ok(draft)
}

/// Render the initial email for a lead via its lead-type template.
pub fn render_initial(profile: &ResolvedProfile, lead: &ScoredLead, lt: LeadType) -> RenderedEmail {
    let template = match lt {
        LeadType::HnFounder => HN_FOUNDER_TPL,
        LeadType::AtsRole => ATS_ROLE_TPL,
        LeadType::ContractExplicit => CONTRACT_TPL,
    };
    render(template, &substitutions(profile, &lead.role, &lead.matched_skills))
}

/// Render the short follow-up nudge (same proof block, in-thread subject). The
/// follow-up template only needs title/company/proof, so a bare `Role` suffices.
pub fn render_followup(profile: &ResolvedProfile, role: &Role) -> RenderedEmail {
    render(FOLLOWUP_TPL, &substitutions(profile, role, &[]))
}

/// The placeholder map shared by both renderers.
fn substitutions(
    profile: &ResolvedProfile,
    role: &Role,
    matched_skills: &[String],
) -> Vec<(&'static str, String)> {
    let matched = if matched_skills.is_empty() {
        "a Rust-native graph and retrieval stack".to_string()
    } else {
        matched_skills.join(", ")
    };
    vec![
        ("company", role.company.clone()),
        ("title", role.title.clone()),
        ("role_language", role_language(&role.body, matched_skills)),
        ("matched_skills", matched),
        ("proof_repo", profile.proof.repo.clone()),
        ("proof_metal_to_model", profile.proof.metal_to_model.clone()),
        ("proof_benchmarks", profile.proof.benchmarks.clone()),
        ("sender", profile.handle.clone()),
    ]
}

/// Build a standalone sentence injecting the role's own language. Prefers the
/// first sentence of the post that mentions a matched skill; falls back to a
/// neutral fit line so the template never reads with a dangling clause.
fn role_language(body: &str, matched_skills: &[String]) -> String {
    match role_language_snippet(body, matched_skills) {
        Some(snippet) => format!("One line that stood out: \"{snippet}\"."),
        None => "The work maps directly onto a Rust-native graph and retrieval stack.".to_string(),
    }
}

/// Extract a quotable clause from the post: the first ~200-char sentence that
/// carries a matched skill, else the first substantive sentence.
fn role_language_snippet(body: &str, matched_skills: &[String]) -> Option<String> {
    let matched: HashSet<&String> = matched_skills.iter().collect();
    let sentences: Vec<&str> = body
        .split(['.', '!', '?', '\n'])
        .map(str::trim)
        .filter(|s| s.len() >= 20)
        .collect();
    sentences
        .iter()
        .find(|s| skills_of(s).iter().any(|sk| matched.contains(sk)))
        .or_else(|| sentences.first())
        .map(|s| clean_clause(s))
}

/// Collapse whitespace, strip wrapping quotes, and cap to `SNIPPET_CHARS` at a
/// word boundary.
fn clean_clause(s: &str) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let unquoted = collapsed.trim_matches(['"', '\'']).trim();
    if unquoted.chars().count() <= SNIPPET_CHARS {
        return unquoted.to_string();
    }
    let mut out = String::new();
    for word in unquoted.split(' ') {
        if out.chars().count() + word.chars().count() + 1 > SNIPPET_CHARS {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out.push_str("...");
    out
}

/// Parse `Subject:`-first template and substitute `{{key}}` placeholders.
fn render(template: &str, subs: &[(&'static str, String)]) -> RenderedEmail {
    let (subject_tpl, body_tpl) = split_template(template);
    RenderedEmail {
        subject: apply_subs(&subject_tpl, subs),
        body: apply_subs(&body_tpl, subs),
    }
}

/// Split a template into (subject, body). The first line of the form
/// `Subject: ...` is the subject; everything after the following blank line is
/// the body. Missing subject => a sensible default.
fn split_template(template: &str) -> (String, String) {
    let normalized = template.replace("\r\n", "\n");
    let mut lines = normalized.lines();
    let first = lines.next().unwrap_or_default();
    if let Some(subject) = first.strip_prefix("Subject:") {
        // Body is the remainder after the (blank) separator line.
        let rest: String = lines.collect::<Vec<_>>().join("\n");
        (subject.trim().to_string(), rest.trim_start_matches('\n').trim().to_string())
    } else {
        ("Following up".to_string(), normalized.trim().to_string())
    }
}

fn apply_subs(template: &str, subs: &[(&'static str, String)]) -> String {
    let mut out = template.to_string();
    for (key, value) in subs {
        out = out.replace(&format!("{{{{{key}}}}}"), value);
    }
    out
}

/// Store the rendered draft (and its lead context) server-side as a context pack,
/// best-effort. Dual-use: exercises the MCP context-pack route the way 0.1 does,
/// and keeps an auditable artifact of what was drafted. Failures are non-fatal.
pub fn record_pack(client: &RustyRedClient, lead: &ScoredLead, email: &RenderedEmail) {
    let artifact_id = format!("outreach:{}", lead.role.id);
    let pack: Value = json!({
        "kind": "outreach_draft",
        "role_id": lead.role.id,
        "company": lead.role.company,
        "subject": email.subject,
        "body": email.body,
    });
    if let Err(err) = client.context_pack(&artifact_id, json!([pack]), json!({})) {
        eprintln!("  context/pack store failed for {artifact_id}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Role, Source};
    use crate::profile::{ProofPoints, ResolvedProfile};

    fn profile() -> ResolvedProfile {
        ResolvedProfile {
            id: "profile:travis".into(),
            handle: "travis".into(),
            text: "rust graph vector".into(),
            skills: vec!["rust".into(), "graph".into(), "vector".into()],
            embedding: vec![0.1],
            proof: ProofPoints::default(),
        }
    }

    fn lead(source: Source, contract: bool, founder: bool, body: &str) -> ScoredLead {
        ScoredLead {
            role: Role {
                id: "role:hn:1".into(),
                company: "Qdrant".into(),
                company_id: "company:qdrant".into(),
                title: "Senior Rust Engineer".into(),
                location: "Remote".into(),
                url: "https://x".into(),
                body: body.into(),
                source: source.as_str().into(),
                remote: true,
                contract,
                founder_posted: founder,
                email_present: true,
                emails: vec!["hire@qdrant.tech".into()],
                comp: None,
                company_domain: Some("qdrant.tech".into()),
            },
            score: 0.8,
            semantic: 0.7,
            graph: 0.6,
            flags: 1.0,
            matched_skills: vec!["rust".into(), "graph".into()],
            contact: Some("hire@qdrant.tech".into()),
            needs_contact: false,
        }
    }

    #[test]
    fn split_template_separates_subject_and_body() {
        let (subject, body) = split_template("Subject: Hi {{company}}\n\nLine one\nLine two\n");
        assert_eq!(subject, "Hi {{company}}");
        assert_eq!(body, "Line one\nLine two");
    }

    #[test]
    fn render_initial_injects_role_language_and_proof() {
        let p = profile();
        let l = lead(
            Source::Hn,
            false,
            true,
            "We need someone to build a Rust graph engine with vector search. Remote, equity.",
        );
        let email = render_initial(&p, &l, LeadType::HnFounder);
        assert!(email.subject.contains("Senior Rust Engineer"));
        assert!(email.subject.contains("Qdrant"));
        // role's own language quoted:
        assert!(email.body.contains("Rust graph engine"));
        // fixed proof block present:
        assert!(email.body.contains(&p.proof.repo));
        // matched skills present:
        assert!(email.body.contains("rust, graph"));
        // no unfilled placeholders:
        assert!(!email.body.contains("{{"));
        assert!(!email.subject.contains("{{"));
    }

    #[test]
    fn contract_lead_uses_contract_template() {
        let p = profile();
        let l = lead(Source::Greenhouse, true, false, "Contract Rust work building a graph store.");
        let email = render_initial(&p, &l, LeadType::ContractExplicit);
        assert!(email.subject.to_lowercase().contains("contract"));
        assert!(!email.body.contains("{{"));
    }

    #[test]
    fn role_language_falls_back_when_no_sentence() {
        let snippet = role_language_snippet("short", &["rust".into()]);
        assert!(snippet.is_none());
        let line = role_language("short", &["rust".into()]);
        assert!(line.contains("Rust-native graph"));
    }

    #[test]
    fn clean_clause_caps_and_collapses() {
        let long = "rust ".repeat(100);
        let cleaned = clean_clause(&long);
        assert!(cleaned.chars().count() <= SNIPPET_CHARS + 3);
        assert!(cleaned.ends_with("..."));
        assert_eq!(clean_clause("  a   b\n c "), "a b c");
    }

    #[test]
    fn followup_is_short_and_threaded_subject() {
        let p = profile();
        let l = lead(Source::Hn, false, true, "Build a Rust graph engine.");
        let email = render_followup(&p, &l.role);
        assert!(email.subject.starts_with("Re:"));
        assert!(email.body.contains(&p.proof.repo));
        assert!(!email.body.contains("{{"));
    }
}
