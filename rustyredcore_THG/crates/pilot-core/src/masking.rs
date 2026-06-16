//! Sensitive-data masking: domain-scoped secrets that are resolved at the engine
//! boundary and masked out of everything logged (trace, model context, receipts).
//!
//! Migrated in place from rustyred-web's `browser_perception.rs`; re-exported
//! there so its consumers and tests are unchanged. Pure (BTreeMap + String); no
//! substrate, no Servo.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Marker substituted into any logged text in place of a secret value. The
/// literal secret never enters trace events, model context, or receipts; the
/// trace shows this marker instead.
fn secret_marker(domain: &str, key: &str) -> String {
    format!("<secret:{domain}/{key}>")
}

/// Domain-scoped secrets. Values are substituted into `Type`/upload at the
/// engine boundary by the executor (via [`SensitiveData::resolve`]), while every
/// string that is logged is first passed through [`SensitiveData::mask`] so the
/// literal value never escapes. A `"*"` domain entry applies to all domains.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensitiveData {
    #[serde(default)]
    domain_scoped: BTreeMap<String, BTreeMap<String, String>>,
}

/// The result of masking a string: the masked text plus which keys were hit.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskedText {
    pub masked: String,
    pub used_keys: Vec<String>,
}

impl SensitiveData {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a secret for a domain (use `"*"` for all domains).
    pub fn set(
        &mut self,
        domain: impl Into<String>,
        key: impl Into<String>,
        value: impl Into<String>,
    ) {
        self.domain_scoped
            .entry(domain.into())
            .or_default()
            .insert(key.into(), value.into());
    }

    /// The real secret value for a domain/key, for the executor to substitute
    /// at the engine boundary. Falls back to a `"*"` (all-domain) entry.
    pub fn resolve(&self, domain: &str, key: &str) -> Option<&str> {
        self.domain_scoped
            .get(domain)
            .and_then(|m| m.get(key))
            .or_else(|| self.domain_scoped.get("*").and_then(|m| m.get(key)))
            .map(String::as_str)
    }

    /// Replace a `{{secret:key}}` placeholder with the real value for the
    /// executor. Unknown keys are left intact. This is the only place a literal
    /// secret is produced, and only for the engine boundary.
    pub fn resolve_placeholders(&self, domain: &str, text: &str) -> String {
        let mut out = text.to_string();
        for (key, value) in self.entries_for(domain) {
            out = out.replace(&format!("{{{{secret:{key}}}}}"), &value);
        }
        out
    }

    /// Mask any occurrence of a known secret value (for `domain`) in arbitrary
    /// text, replacing it with the [`secret_marker`]. Use on everything logged:
    /// trace, model context, receipts.
    ///
    /// Leak-safety, three properties the naive `replace` loop did not have:
    /// 1. Longest value first, so a short secret that is a prefix/substring of a
    ///    longer one cannot mask first and leave the longer one's tail in the text.
    /// 2. The union of all applicable secrets (both the `"*"` entry and the
    ///    domain entry, even when they share a key), so a wildcard secret is never
    ///    shadowed out of masking by a same-key domain secret.
    /// 3. A single forward scan (a tokenizer, not find-and-replace): markers are
    ///    written to a fresh output buffer that is never re-scanned, so a secret
    ///    value that collides with the marker template (e.g. a secret literally
    ///    equal to "secret") can never rewrite an already-emitted marker.
    pub fn mask(&self, domain: &str, text: &str) -> MaskedText {
        let mut secrets = self.applicable_secrets(domain);
        secrets.retain(|secret| !secret.value.is_empty());
        // Longest value first; ties broken by marker for determinism. Longest
        // first means a short secret that is a prefix of a longer one cannot
        // match first and leave the longer one's tail behind.
        secrets.sort_by(|a, b| {
            b.value
                .len()
                .cmp(&a.value.len())
                .then_with(|| a.marker.cmp(&b.marker))
        });

        let mut masked = String::with_capacity(text.len());
        let mut used_keys = Vec::new();
        let mut rest = text;
        'scan: while !rest.is_empty() {
            for secret in &secrets {
                if let Some(stripped) = rest.strip_prefix(secret.value.as_str()) {
                    masked.push_str(&secret.marker);
                    used_keys.push(secret.key.clone());
                    rest = stripped;
                    continue 'scan;
                }
            }
            // No secret matches at the front: copy one char (UTF-8 safe) and advance.
            let mut chars = rest.chars();
            let ch = chars.next().expect("rest is non-empty");
            masked.push(ch);
            rest = chars.as_str();
        }
        used_keys.sort();
        used_keys.dedup();
        MaskedText { masked, used_keys }
    }

    /// Every secret that applies to `domain`: the `"*"` (all-domain) entries and
    /// the domain's own entries, kept distinct (a `"*"` and a domain secret with
    /// the same key are two different secrets and both must be masked).
    fn applicable_secrets(&self, domain: &str) -> Vec<ApplicableSecret> {
        let mut out = Vec::new();
        if let Some(global) = self.domain_scoped.get("*") {
            for (key, value) in global {
                out.push(ApplicableSecret {
                    key: key.clone(),
                    marker: secret_marker("*", key),
                    value: value.clone(),
                });
            }
        }
        if domain != "*" {
            if let Some(scoped) = self.domain_scoped.get(domain) {
                for (key, value) in scoped {
                    out.push(ApplicableSecret {
                        key: key.clone(),
                        marker: secret_marker(domain, key),
                        value: value.clone(),
                    });
                }
            }
        }
        out
    }

    /// All (key, value) pairs that apply to a domain (its own plus `"*"`), with
    /// the domain entry shadowing `"*"` on a shared key. Used by
    /// [`Self::resolve_placeholders`], where keys are uniquely delimited so the
    /// shadow is the intended fallback semantics.
    fn entries_for(&self, domain: &str) -> Vec<(String, String)> {
        let mut pairs: BTreeMap<String, String> = BTreeMap::new();
        if let Some(global) = self.domain_scoped.get("*") {
            for (k, v) in global {
                pairs.insert(k.clone(), v.clone());
            }
        }
        if let Some(scoped) = self.domain_scoped.get(domain) {
            for (k, v) in scoped {
                pairs.insert(k.clone(), v.clone());
            }
        }
        pairs.into_iter().collect()
    }
}

/// One secret resolved for masking: its key (for `used_keys`), its display
/// marker, and the literal value to redact.
struct ApplicableSecret {
    key: String,
    marker: String,
    value: String,
}
