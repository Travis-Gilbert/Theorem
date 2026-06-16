//! Module 4 - contacts. HN posts already carry emails (free); ATS roles need a
//! lookup. `ContactFinder` is the seam; `HunterFinder` is the default impl over
//! Hunter.io domain-search, gated on `HUNTER_API_KEY`. With no key (or no known
//! domain), the lead is left without a contact and `needs_contact` is set true.

use std::time::Duration;

use reqwest::blocking::Client;
use serde_json::Value;

use crate::error::Result;
use crate::model::ScoredLead;

/// The contact-discovery seam. Implementors resolve a company (by name +
/// optional domain) to a best outreach email, or None if nothing is found.
pub trait ContactFinder {
    fn find(&self, company: &str, domain: Option<&str>) -> Result<Option<String>>;
}

/// Default `ContactFinder`: Hunter.io domain-search. Returns the best
/// engineering/founder email for a domain.
pub struct HunterFinder {
    api_key: String,
    http: Client,
}

impl HunterFinder {
    /// Construct from the optional `HUNTER_API_KEY`. Returns None when no key is
    /// set, so callers can treat "no finder" and "no key" identically.
    pub fn from_key(api_key: Option<&str>) -> Option<Self> {
        let key = api_key?.trim();
        if key.is_empty() {
            return None;
        }
        let http = Client::builder()
            .user_agent("jobintel/0.1")
            .timeout(Duration::from_secs(20))
            .build()
            .ok()?;
        Some(Self {
            api_key: key.to_string(),
            http,
        })
    }
}

impl ContactFinder for HunterFinder {
    fn find(&self, _company: &str, domain: Option<&str>) -> Result<Option<String>> {
        let Some(domain) = domain else {
            return Ok(None);
        };
        let url = format!(
            "https://api.hunter.io/v2/domain-search?domain={}&api_key={}",
            domain, self.api_key
        );
        let resp = self.http.get(url).send()?;
        if !resp.status().is_success() {
            // Quota/invalid-domain etc. are non-fatal: just no contact found.
            return Ok(None);
        }
        let body: Value = resp.json()?;
        Ok(pick_best_email(&body))
    }
}

/// Fill contacts for a slice of leads. HN leads keep their in-post email; ATS
/// leads are resolved via `finder` when a domain is known. Postcondition: every
/// lead has either `contact = Some` or `needs_contact = true`.
pub fn fill_contacts(
    finder: Option<&dyn ContactFinder>,
    leads: &mut [ScoredLead],
) -> Result<usize> {
    let mut resolved = 0;
    for lead in leads.iter_mut() {
        if lead.contact.is_some() {
            lead.needs_contact = false;
            resolved += 1;
            continue;
        }
        let domain = lead.role.company_domain.clone();
        match finder {
            Some(f) => match f.find(&lead.role.company, domain.as_deref())? {
                Some(email) => {
                    lead.contact = Some(email);
                    lead.needs_contact = false;
                    resolved += 1;
                }
                None => lead.needs_contact = true,
            },
            None => lead.needs_contact = true,
        }
    }
    Ok(resolved)
}

/// Pick the best outreach email from a Hunter.io domain-search payload.
/// Prefers founders/execs, then engineering, then raw confidence.
pub fn pick_best_email(body: &Value) -> Option<String> {
    let emails = body
        .get("data")
        .and_then(|d| d.get("emails"))
        .and_then(Value::as_array)?;
    let mut best: Option<(f64, String)> = None;
    for entry in emails {
        let Some(value) = entry.get("value").and_then(Value::as_str) else {
            continue;
        };
        let mut score = entry
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            / 100.0;
        let position = entry
            .get("position")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase();
        let department = entry
            .get("department")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase();
        let seniority = entry
            .get("seniority")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase();

        if ["founder", "ceo", "cto", "co-founder"]
            .iter()
            .any(|t| position.contains(t))
            || seniority == "executive"
        {
            score += 3.0;
        }
        if department == "engineering" {
            score += 2.0;
        }

        if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
            best = Some((score, value.to_string()));
        }
    }
    best.map(|(_, email)| email)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn picks_founder_over_higher_confidence_generic() {
        let body = json!({
            "data": { "emails": [
                { "value": "info@acme.com", "confidence": 99, "department": "support", "position": "Support Rep", "seniority": "junior" },
                { "value": "jane@acme.com", "confidence": 70, "department": "executive", "position": "Co-Founder & CEO", "seniority": "executive" }
            ]}
        });
        assert_eq!(pick_best_email(&body).as_deref(), Some("jane@acme.com"));
    }

    #[test]
    fn picks_engineering_when_no_founder() {
        let body = json!({
            "data": { "emails": [
                { "value": "sales@acme.com", "confidence": 90, "department": "sales" },
                { "value": "eng@acme.com", "confidence": 80, "department": "engineering" }
            ]}
        });
        assert_eq!(pick_best_email(&body).as_deref(), Some("eng@acme.com"));
    }

    #[test]
    fn none_when_no_emails() {
        assert_eq!(pick_best_email(&json!({ "data": { "emails": [] } })), None);
        assert_eq!(pick_best_email(&json!({})), None);
    }

    struct NullFinder;
    impl ContactFinder for NullFinder {
        fn find(&self, _c: &str, _d: Option<&str>) -> Result<Option<String>> {
            Ok(None)
        }
    }

    #[test]
    fn fill_contacts_sets_needs_contact_when_unresolved() {
        use crate::model::{Role, Source};
        let role = Role {
            id: "role:greenhouse:1".into(),
            company: "Acme".into(),
            company_id: "company:acme".into(),
            title: "Engineer".into(),
            location: "Remote".into(),
            url: "https://x".into(),
            body: "rust".into(),
            source: Source::Greenhouse.as_str().into(),
            remote: true,
            contract: false,
            founder_posted: false,
            email_present: false,
            emails: vec![],
            comp: None,
            company_domain: Some("acme.com".into()),
        };
        let mut leads = vec![ScoredLead {
            role,
            score: 1.0,
            semantic: 0.0,
            graph: 0.0,
            flags: 0.0,
            matched_skills: vec![],
            contact: None,
            needs_contact: false,
        }];
        let resolved = fill_contacts(Some(&NullFinder), &mut leads).unwrap();
        assert_eq!(resolved, 0);
        assert!(
            leads[0].needs_contact,
            "unresolved ATS lead must set needs_contact"
        );
        assert!(leads[0].contact.is_none());
    }
}
