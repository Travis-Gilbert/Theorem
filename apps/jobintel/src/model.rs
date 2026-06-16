//! Shared domain model: the ingested `JobRecord`, the graph-readback `Role`
//! view, the `ScoredLead`, and the deterministic id scheme.
//!
//! Idempotency: every node/edge id is a pure function of stable inputs (the
//! source, the source's own id, the company slug, the skill name). Re-running
//! `ingest` upserts the same ids, so the graph converges instead of
//! duplicating. This is why dedup "beyond (company, title)" is out of scope in
//! the spec: the id scheme already collapses exact re-ingests.

use serde::{Deserialize, Serialize};

/// Origin of a job posting. Serialized lowercase to match graph property values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Hn,
    Greenhouse,
    Lever,
    Ashby,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Hn => "hn",
            Source::Greenhouse => "greenhouse",
            Source::Lever => "lever",
            Source::Ashby => "ashby",
        }
    }

    /// Node id for the `Source` graph node this posting arrived via.
    pub fn node_id(self) -> String {
        format!("source:{}", self.as_str())
    }

    /// Human label for the `Source` node.
    pub fn label(self) -> &'static str {
        match self {
            Source::Hn => "Hacker News (Who is Hiring)",
            Source::Greenhouse => "Greenhouse",
            Source::Lever => "Lever",
            Source::Ashby => "Ashby",
        }
    }
}

/// A single ingested job posting. Field set is the spec's Module 1 deliverable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    /// Stable per-source id, e.g. `role:hn:<comment_id>` or `role:greenhouse:<job_id>`.
    pub id: String,
    pub source: Source,
    pub company: String,
    /// Company web domain when known (ATS companies, from slugs.toml). Drives
    /// Hunter.io contact lookup. HN companies leave this None (they carry emails).
    pub company_domain: Option<String>,
    pub title: String,
    pub location: String,
    pub remote: bool,
    pub comp: Option<String>,
    pub url: String,
    pub body: String,
    pub posted_at: Option<String>,
    pub emails: Vec<String>,
    pub contract: bool,
    /// True when the post reads as written by a founder/CEO (HN signal).
    pub founder_posted: bool,
}

impl JobRecord {
    /// Slugified company name, used as the `Company` node id stem.
    pub fn company_slug(&self) -> String {
        slugify(&self.company)
    }

    pub fn company_id(&self) -> String {
        format!("company:{}", self.company_slug())
    }

    /// Did the post carry at least one email address?
    pub fn email_present(&self) -> bool {
        !self.emails.is_empty()
    }
}

/// A `Role` as read back from the graph (Module 3/5 operate over persisted
/// nodes, not in-memory `JobRecord`s, because `rank` and `draft` are separate
/// CLI invocations from `ingest`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub id: String,
    pub company: String,
    pub company_id: String,
    pub title: String,
    pub location: String,
    pub url: String,
    pub body: String,
    pub source: String,
    pub remote: bool,
    pub contract: bool,
    pub founder_posted: bool,
    pub email_present: bool,
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(default)]
    pub comp: Option<String>,
    #[serde(default)]
    pub company_domain: Option<String>,
}

/// A ranked lead: a `Role` plus its blended score, the per-signal breakdown
/// (for transparency in the printed shortlist), matched skills, and contact.
#[derive(Debug, Clone, Serialize)]
pub struct ScoredLead {
    pub role: Role,
    pub score: f32,
    pub semantic: f32,
    pub graph: f32,
    pub flags: f32,
    pub matched_skills: Vec<String>,
    pub contact: Option<String>,
    pub needs_contact: bool,
}

// ---- id + slug helpers -----------------------------------------------------

/// Normalize an arbitrary company string into a url/id-safe slug.
/// "Qdrant, Inc." -> "qdrant-inc"; "LiveKit" -> "livekit".
pub fn slugify(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_dash = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.extend(ch.to_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

pub fn skill_id(name: &str) -> String {
    format!("skill:{}", slugify(name))
}

pub fn profile_id(handle: &str) -> String {
    if handle.starts_with("profile:") {
        handle.to_string()
    } else {
        format!("profile:{}", slugify(handle))
    }
}

/// Deterministic, collision-resistant edge id. Re-ingesting the same triple
/// upserts the same edge id, so edges dedupe like nodes.
pub fn edge_id(from: &str, edge_type: &str, to: &str) -> String {
    format!("edge:{from}|{edge_type}|{to}")
}

// ---- graph vocabulary (single source of truth for keys/labels) -------------

pub mod labels {
    pub const COMPANY: &str = "Company";
    pub const ROLE: &str = "Role";
    pub const PERSON: &str = "Person";
    pub const SOURCE: &str = "Source";
    pub const SKILL: &str = "Skill";
    pub const PROFILE: &str = "Profile";
}

pub mod edges {
    // The four spec-named types (Module 2 acceptance: "all four edge types").
    pub const POSTS: &str = "posts"; // Company -> Role
    pub const REQUIRES: &str = "requires"; // Role -> Skill  (and Profile -> Skill)
    pub const HIRING_FOR: &str = "hiring_for"; // Person -> Role
    pub const VIA: &str = "via"; // Role -> Source

    // Reverse-traversal edges. RustyRed PPR/PageRank adjacency is strictly
    // from->to, so these orient mass the way the spec's algorithm INTENT needs:
    // Profile -> Skill -> Role -> Company. Without them, PPR seeded on skill
    // sinks would propagate nothing. See rank::profile_seeds.
    pub const REQUIRED_BY: &str = "required_by"; // Skill -> Role
    pub const POSTED_BY: &str = "posted_by"; // Role -> Company
}

pub mod props {
    pub const EMBEDDING: &str = "embedding";
    pub const TITLE: &str = "title";
    pub const COMPANY: &str = "company";
    pub const COMPANY_ID: &str = "company_id";
    pub const LOCATION: &str = "location";
    pub const URL: &str = "url";
    pub const BODY: &str = "body";
    pub const SOURCE: &str = "source";
    pub const REMOTE: &str = "remote";
    pub const CONTRACT: &str = "contract";
    pub const FOUNDER_POSTED: &str = "founder_posted";
    pub const EMAIL_PRESENT: &str = "email_present";
    pub const EMAILS: &str = "emails";
    pub const COMP: &str = "comp";
    pub const POSTED_AT: &str = "posted_at";
    pub const NAME: &str = "name";
    pub const DOMAIN: &str = "domain";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_normalizes_company_names() {
        assert_eq!(slugify("Qdrant, Inc."), "qdrant-inc");
        assert_eq!(slugify("LiveKit"), "livekit");
        assert_eq!(slugify("  Fireworks AI  "), "fireworks-ai");
        assert_eq!(slugify("weaviate"), "weaviate");
    }

    #[test]
    fn profile_id_is_idempotent_on_prefixed_input() {
        assert_eq!(profile_id("travis"), "profile:travis");
        assert_eq!(profile_id("profile:travis"), "profile:travis");
    }

    #[test]
    fn edge_id_is_stable_for_same_triple() {
        let a = edge_id("company:qdrant", edges::POSTS, "role:hn:1");
        let b = edge_id("company:qdrant", edges::POSTS, "role:hn:1");
        assert_eq!(a, b);
    }

    #[test]
    fn source_round_trips_through_serde() {
        let json = serde_json::to_string(&Source::Greenhouse).unwrap();
        assert_eq!(json, "\"greenhouse\"");
        let back: Source = serde_json::from_str("\"hn\"").unwrap();
        assert_eq!(back, Source::Hn);
    }
}
