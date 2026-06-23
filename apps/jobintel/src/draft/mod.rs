//! Module 5 - draft. For each top-K lead, build a context pack (role text +
//! matched skills + contact + fixed proof points), POST it to RustyRed's
//! `context/pack` route (the MCP-served context-pack half of the dual-use
//! demo), and write `out/<company>.json`. An index `out/queue.md` lists them.
//! jobintel does not send email; each pack is the raw material an LLM turns into
//! one outreach message.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::client::RustyRedClient;
use crate::error::Result;
use crate::model::{slugify, ScoredLead};
use crate::profile::ResolvedProfile;

const ROLE_TEXT_CHARS: usize = 1200;

#[derive(Debug, Default)]
pub struct DraftStats {
    /// Packs written to disk.
    pub written: usize,
    /// Packs the server accepted into context/pack.
    pub packed: usize,
    pub out_dir: String,
}

/// Build the per-lead context pack, store it server-side, write it to
/// `out_dir/<company>.json`, and emit `out_dir/queue.md`.
pub fn draft_queue(
    client: &RustyRedClient,
    profile: &ResolvedProfile,
    leads: &[ScoredLead],
    top_n: usize,
    out_dir: &str,
) -> Result<DraftStats> {
    let selected: Vec<&ScoredLead> = leads.iter().take(top_n).collect();
    std::fs::create_dir_all(out_dir)?;

    let mut stats = DraftStats {
        out_dir: out_dir.to_string(),
        ..Default::default()
    };
    let mut used_names: HashSet<String> = HashSet::new();
    let mut entries: Vec<QueueEntry> = Vec::new();

    for lead in &selected {
        let pack = build_pack(profile, lead);

        // Store server-side (dual-use: exercises the MCP context-pack route).
        let artifact_id = format!("lead:{}", lead.role.id);
        match client.context_pack(&artifact_id, json!([pack.clone()]), json!({})) {
            Ok(_) => stats.packed += 1,
            Err(err) => eprintln!("  context/pack store failed for {artifact_id}: {err}"),
        }

        // Write out/<company>.json (disambiguated on collision).
        let filename = unique_filename(&mut used_names, &lead.role.company);
        let path: PathBuf = Path::new(out_dir).join(&filename);
        std::fs::write(&path, serde_json::to_string_pretty(&pack)?)?;
        stats.written += 1;

        entries.push(QueueEntry::from_lead(lead, &filename));
    }

    let queue_md = render_queue(profile, &entries);
    std::fs::write(Path::new(out_dir).join("queue.md"), queue_md)?;

    Ok(stats)
}

/// Build the context pack object for one lead. Pure - the spec's required
/// contents: role text, matched skills, contact, and the fixed proof points.
pub fn build_pack(profile: &ResolvedProfile, lead: &ScoredLead) -> Value {
    let role = &lead.role;
    json!({
        "kind": "outreach_lead",
        "profile": profile.handle,
        "company": role.company,
        "company_domain": role.company_domain,
        "role": {
            "id": role.id,
            "title": role.title,
            "location": role.location,
            "url": role.url,
            "source": role.source,
            "remote": role.remote,
            "contract": role.contract,
            "founder_posted": role.founder_posted,
            "comp": role.comp,
        },
        "role_text": truncate_chars(&role.body, ROLE_TEXT_CHARS),
        "matched_skills": lead.matched_skills,
        "score": {
            "blended": round3(lead.score),
            "semantic": round3(lead.semantic),
            "graph": round3(lead.graph),
            "flags": round3(lead.flags),
        },
        "contact": lead.contact,
        "needs_contact": lead.needs_contact,
        "proof_points": {
            "repo": profile.proof.repo,
            "metal_to_model": profile.proof.metal_to_model,
            "benchmarks": profile.proof.benchmarks,
        },
    })
}

struct QueueEntry {
    score: f32,
    company: String,
    title: String,
    remote: bool,
    contract: bool,
    founder: bool,
    url: String,
    matched_skills: Vec<String>,
    contact: Option<String>,
    needs_contact: bool,
    filename: String,
}

impl QueueEntry {
    fn from_lead(lead: &ScoredLead, filename: &str) -> Self {
        Self {
            score: lead.score,
            company: lead.role.company.clone(),
            title: lead.role.title.clone(),
            remote: lead.role.remote,
            contract: lead.role.contract,
            founder: lead.role.founder_posted,
            url: lead.role.url.clone(),
            matched_skills: lead.matched_skills.clone(),
            contact: lead.contact.clone(),
            needs_contact: lead.needs_contact,
            filename: filename.to_string(),
        }
    }
}

/// Render the queue.md index. Pure for testing.
fn render_queue(profile: &ResolvedProfile, entries: &[QueueEntry]) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "# jobintel lead queue (profile: {})\n\n",
        profile.handle
    ));
    s.push_str(&format!(
        "{} leads, highest blended score first.\n\n",
        entries.len()
    ));
    s.push_str("| # | Score | Company | Role | Remote | Contract | Founder | Contact | Pack |\n");
    s.push_str("|---|-------|---------|------|--------|----------|---------|---------|------|\n");
    for (i, e) in entries.iter().enumerate() {
        let contact = match (&e.contact, e.needs_contact) {
            (Some(email), _) => email.clone(),
            (None, true) => "needs_contact".to_string(),
            (None, false) => "-".to_string(),
        };
        s.push_str(&format!(
            "| {} | {:.3} | {} | {} | {} | {} | {} | {} | [{}]({}) |\n",
            i + 1,
            e.score,
            md_escape(&e.company),
            md_escape(&e.title),
            yn(e.remote),
            yn(e.contract),
            yn(e.founder),
            md_escape(&contact),
            e.filename,
            e.filename,
        ));
    }
    s.push_str("\n## Leads\n\n");
    for (i, e) in entries.iter().enumerate() {
        s.push_str(&format!("### {}. {} - {}\n\n", i + 1, e.company, e.title));
        s.push_str(&format!("- Post: {}\n", e.url));
        s.push_str(&format!(
            "- Matched skills: {}\n",
            if e.matched_skills.is_empty() {
                "(none)".to_string()
            } else {
                e.matched_skills.join(", ")
            }
        ));
        s.push_str(&format!("- Pack: `{}`\n\n", e.filename));
    }
    s.push_str(
        "Each pack is ready to become one outreach email: feed `<company>.json` to an LLM with the proof points and role text. jobintel does not send.\n",
    );
    s
}

// ---- helpers ---------------------------------------------------------------

fn unique_filename(used: &mut HashSet<String>, company: &str) -> String {
    let stem = {
        let s = slugify(company);
        if s.is_empty() {
            "company".to_string()
        } else {
            s
        }
    };
    let mut name = format!("{stem}.json");
    let mut n = 2;
    while used.contains(&name) {
        name = format!("{stem}-{n}.json");
        n += 1;
    }
    used.insert(name.clone());
    name
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max).collect::<String>())
    }
}

fn round3(x: f32) -> f32 {
    (x * 1000.0).round() / 1000.0
}

fn yn(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

fn md_escape(s: &str) -> String {
    s.replace('|', "\\|")
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
            text: "rust graph".into(),
            skills: vec!["rust".into(), "graph".into()],
            embedding: vec![0.1, 0.2],
            proof: ProofPoints::default(),
        }
    }

    fn lead(company: &str, email: Option<&str>) -> ScoredLead {
        ScoredLead {
            role: Role {
                id: format!("role:hn:{company}"),
                company: company.into(),
                company_id: format!("company:{company}"),
                title: "Senior Rust Engineer".into(),
                location: "Remote".into(),
                url: "https://news.ycombinator.com/item?id=1".into(),
                body: "Build a graph database in Rust with vector search.".into(),
                source: Source::Hn.as_str().into(),
                remote: true,
                contract: true,
                founder_posted: true,
                email_present: email.is_some(),
                emails: email.map(|e| vec![e.to_string()]).unwrap_or_default(),
                comp: Some("$160k".into()),
                company_domain: None,
            },
            score: 0.812,
            semantic: 0.7,
            graph: 0.6,
            flags: 1.0,
            matched_skills: vec!["rust".into(), "graph".into()],
            contact: email.map(String::from),
            needs_contact: email.is_none(),
        }
    }

    #[test]
    fn pack_carries_required_contents() {
        let p = profile();
        let pack = build_pack(&p, &lead("Qdrant", Some("hiring@qdrant.tech")));
        assert_eq!(pack["company"], "Qdrant");
        assert_eq!(pack["role"]["title"], "Senior Rust Engineer");
        assert!(pack["role_text"]
            .as_str()
            .unwrap()
            .contains("graph database"));
        assert_eq!(pack["matched_skills"][0], "rust");
        assert_eq!(pack["contact"], "hiring@qdrant.tech");
        assert_eq!(pack["proof_points"]["repo"], p.proof.repo);
    }

    #[test]
    fn unique_filename_disambiguates_collisions() {
        let mut used = HashSet::new();
        assert_eq!(unique_filename(&mut used, "Qdrant"), "qdrant.json");
        assert_eq!(unique_filename(&mut used, "Qdrant"), "qdrant-2.json");
        assert_eq!(unique_filename(&mut used, "Qdrant"), "qdrant-3.json");
    }

    #[test]
    fn queue_lists_all_entries_with_contact_state() {
        let p = profile();
        let entries = vec![
            QueueEntry::from_lead(&lead("Qdrant", Some("hiring@qdrant.tech")), "qdrant.json"),
            QueueEntry::from_lead(&lead("Modal", None), "modal.json"),
        ];
        let md = render_queue(&p, &entries);
        assert!(md.contains("profile: travis"));
        assert!(md.contains("hiring@qdrant.tech"));
        assert!(
            md.contains("needs_contact"),
            "lead without email shows needs_contact"
        );
        assert!(md.contains("[qdrant.json](qdrant.json)"));
    }
}
