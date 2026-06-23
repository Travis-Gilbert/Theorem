use std::fmt;

use serde::{Deserialize, Serialize};
use url::Url;

pub const LABEL_URL: &str = "url";
pub const LABEL_DOMAIN: &str = "domain";
pub const EDGE_LINKS_TO: &str = "links_to";
pub const EDGE_ON_DOMAIN: &str = "on_domain";

pub const STATE_FRONTIER: &str = "frontier";
pub const STATE_IN_FLIGHT: &str = "in_flight";
pub const STATE_FETCHED: &str = "fetched";
pub const STATE_ERROR: &str = "error";
pub const STATE_SKIPPED: &str = "skipped";

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct UrlFingerprint(pub [u8; 32]);

impl UrlFingerprint {
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push(hex_char(byte >> 4));
            out.push(hex_char(byte & 0x0f));
        }
        out
    }

    pub fn from_hex(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (index, chunk) in raw.as_bytes().chunks_exact(2).enumerate() {
            let hi = hex_value(chunk[0])?;
            let lo = hex_value(chunk[1])?;
            bytes[index] = (hi << 4) | lo;
        }
        Some(Self(bytes))
    }
}

impl fmt::Debug for UrlFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("UrlFingerprint")
            .field(&self.to_hex())
            .finish()
    }
}

impl fmt::Display for UrlFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

pub fn fingerprint(method: &str, url_canonical: &str, body: &[u8]) -> UrlFingerprint {
    let mut hasher = blake3::Hasher::new();
    hasher.update(url_canonical.as_bytes());
    hasher.update(b"\n");
    hasher.update(method.to_ascii_uppercase().as_bytes());
    hasher.update(b"\n");
    hasher.update(hex_body(body).as_bytes());
    UrlFingerprint(*hasher.finalize().as_bytes())
}

pub fn canonicalize_url(raw: &str, base: Option<&Url>) -> Option<String> {
    let mut url = match base {
        Some(base) => base.join(raw).ok()?,
        None => Url::parse(raw).ok()?,
    };
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_fragment(None);
    let _ = url.set_username("");
    let _ = url.set_password(None);
    if let Some(host) = url.host_str().map(str::to_ascii_lowercase) {
        url.set_host(Some(&host)).ok()?;
    }
    if (url.scheme() == "http" && url.port() == Some(80))
        || (url.scheme() == "https" && url.port() == Some(443))
    {
        url.set_port(None).ok()?;
    }

    let mut query_pairs = url.query_pairs().into_owned().collect::<Vec<_>>();
    query_pairs.sort();
    url.set_query(None);
    if !query_pairs.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query_pairs {
            pairs.append_pair(&key, &value);
        }
    }

    Some(url.to_string())
}

pub fn domain_for_url(canonical_url: &str) -> Option<String> {
    Url::parse(canonical_url)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredLink {
    pub url_raw: String,
    pub anchor_text: String,
    pub rel: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FetchTask {
    pub fp: UrlFingerprint,
    pub url: String,
    pub domain: String,
    pub depth: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum FetchOutcome {
    Ok {
        final_url: String,
        status: u16,
        content_hash: [u8; 32],
        etag: Option<String>,
        links: Vec<DiscoveredLink>,
    },
    Error {
        status: Option<u16>,
        retryable: bool,
    },
    Skipped {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct UrlNodeView {
    pub fp: UrlFingerprint,
    pub url: String,
    pub domain: String,
    pub depth: u32,
    pub state: String,
    pub priority: f64,
    pub retry_count: u32,
}

fn hex_body(body: &[u8]) -> String {
    let mut out = String::with_capacity(body.len() * 2);
    for byte in body {
        out.push(hex_char(byte >> 4));
        out.push(hex_char(byte & 0x0f));
    }
    out
}

fn hex_char(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'a' + (nibble - 10)) as char,
        _ => '0',
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalization_sorts_query_and_strips_fragment() {
        assert_eq!(
            canonicalize_url("HTTPS://Example.com:443/a/../b?z=2&a=1#frag", None).unwrap(),
            "https://example.com/b?a=1&z=2"
        );
    }

    #[test]
    fn fingerprint_round_trips_as_hex() {
        let fp = fingerprint("GET", "https://example.com/", b"");
        assert_eq!(UrlFingerprint::from_hex(&fp.to_hex()), Some(fp));
    }
}
