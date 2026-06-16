//! Pure text primitives shared by the HN and ATS ingest paths: HTML stripping,
//! email extraction (with light de-obfuscation), and the remote/contract/
//! founder keyword classifiers. All pure and unit-tested - the network layer
//! (hn.rs / ats.rs) only fetches and delegates here.

use std::sync::OnceLock;

use regex::Regex;

/// Strip HTML to readable text: `<p>` becomes a blank line, other tags are
/// removed, and the common HTML entities HN emits are decoded.
pub fn strip_html(raw: &str) -> String {
    // Paragraph tags carry the line structure of HN comments.
    let with_breaks = raw.replace("<p>", "\n\n").replace("</p>", "\n");
    let tag = tag_re();
    let no_tags = tag.replace_all(&with_breaks, " ");
    decode_entities(&no_tags)
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Decode the common HTML entities HN/Greenhouse emit. Public because
/// Greenhouse `content` is entity-encoded HTML (`&lt;p&gt;...`) and must be
/// decoded *before* tag stripping.
pub fn decode_entities(s: &str) -> String {
    s.replace("&#x2F;", "/")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// Extract email addresses. Runs the standard pattern first; if that finds
/// nothing, normalizes the common `name [at] domain [dot] com` obfuscation and
/// retries. De-duplicated, lowercased, order-preserving.
pub fn extract_emails(text: &str) -> Vec<String> {
    let mut found = scan_emails(text);
    if found.is_empty() {
        let deobfuscated = deobfuscate(text);
        found = scan_emails(&deobfuscated);
    }
    let mut seen = Vec::new();
    for e in found {
        let lower = e.to_lowercase();
        if !seen.contains(&lower) {
            seen.push(lower);
        }
    }
    seen
}

fn scan_emails(text: &str) -> Vec<String> {
    email_re()
        .find_iter(text)
        .map(|m| m.as_str().to_string())
        .collect()
}

/// Normalize ` at `/`[at]`/`(at)` -> `@` and ` dot `/`[dot]`/`(dot)` -> `.`.
fn deobfuscate(text: &str) -> String {
    let mut s = text.to_string();
    for pat in [" [at] ", " (at) ", "[at]", "(at)", " at ", " AT "] {
        s = s.replace(pat, "@");
    }
    for pat in [" [dot] ", " (dot) ", "[dot]", "(dot)", " dot ", " DOT "] {
        s = s.replace(pat, ".");
    }
    s
}

/// Remote signal: a "remote/wfh/distributed" cue that is not explicitly negated.
pub fn detect_remote(text: &str) -> bool {
    let lower = text.to_lowercase();
    let negated = [
        "no remote",
        "not remote",
        "non-remote",
        "onsite only",
        "on-site only",
        "in office only",
        "in-office only",
        "no wfh",
    ]
    .iter()
    .any(|n| lower.contains(n));
    if negated {
        return false;
    }
    [
        "remote",
        "wfh",
        "work from home",
        "work from anywhere",
        "fully distributed",
        "distributed team",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

/// Contract signal: contract/freelance/part-time style engagement cues.
pub fn detect_contract(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "contract",
        "contractor",
        "freelance",
        "freelancer",
        "consulting",
        "consultant",
        "part-time",
        "part time",
        "c2c",
        "1099",
        "fractional",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

/// Founder/exec signal (HN posts are often written by a founder or C-level).
pub fn detect_founder(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "founder",
        "co-founder",
        "cofounder",
        "founding team",
        " ceo",
        "ceo ",
        " cto",
        "cto ",
        " ceo/",
        "i'm the ceo",
        "i am the founder",
        "we're the founders",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

/// Best-effort compensation snippet: a `$NNk` / `$NNN,NNN` / `NNk-NNk` figure.
pub fn find_comp(text: &str) -> Option<String> {
    comp_re().find(text).map(|m| m.as_str().trim().to_string())
}

// ---- cached regexes --------------------------------------------------------

fn tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<[^>]+>").unwrap())
}

fn email_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap())
}

fn comp_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // $120k, $120,000, $120k-$160k, 120k-160k
        Regex::new(r"\$?\d{2,3}[kK](?:\s*[-\u{2013}to]+\s*\$?\d{2,3}[kK])?|\$\d{2,3},\d{3}(?:\s*[-\u{2013}to]+\s*\$\d{2,3},\d{3})?").unwrap()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags_and_decodes_entities() {
        let html = "<p>We use Rust &amp; Python.</p><p>Apply: jobs&#x2F;careers</p>";
        let text = strip_html(html);
        assert!(text.contains("We use Rust & Python."));
        assert!(text.contains("jobs/careers"));
        assert!(!text.contains('<'));
    }

    #[test]
    fn extracts_plain_emails_deduped() {
        let text = "Reach hiring@qdrant.tech or HIRING@qdrant.tech for the role.";
        assert_eq!(extract_emails(text), vec!["hiring@qdrant.tech"]);
    }

    #[test]
    fn extracts_obfuscated_email_as_fallback() {
        let text = "Email me: travis [at] example [dot] com to apply.";
        assert_eq!(extract_emails(text), vec!["travis@example.com"]);
    }

    #[test]
    fn remote_detection_respects_negation() {
        assert!(detect_remote("This is a fully remote role."));
        assert!(detect_remote("WFH friendly, distributed team."));
        assert!(!detect_remote("No remote, onsite only in SF."));
        assert!(!detect_remote("Backend engineer in New York."));
    }

    #[test]
    fn contract_detection() {
        assert!(detect_contract("6-month contract, C2C ok."));
        assert!(detect_contract("Looking for a freelance Rust dev."));
        assert!(!detect_contract("Full-time senior engineer."));
    }

    #[test]
    fn founder_detection() {
        assert!(detect_founder("I'm the founder and we're hiring."));
        assert!(detect_founder("Our CTO is leading this team."));
        assert!(!detect_founder("Apply via our careers page."));
    }

    #[test]
    fn comp_extraction() {
        assert_eq!(
            find_comp("Salary $120k-$160k plus equity").as_deref(),
            Some("$120k-$160k")
        );
        assert_eq!(find_comp("Pays $145,000 base").as_deref(), Some("$145,000"));
        assert_eq!(find_comp("Great culture, no numbers here"), None);
    }
}
