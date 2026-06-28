//! Continuous Agent Memory Harness hash contracts.
//!
//! This module owns the pure native CMH identifier and handoff-hash
//! algorithms. PyO3 exports in `rustyredcore_THG/src/cmh.rs` wrap these
//! functions for historical `theseus_native.cmh_*` compatibility.

use sha2::{Digest, Sha256};

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn normalize_body(text: &str) -> String {
    text.split_whitespace()
        .map(|chunk| chunk.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

/// SHA256 hex of the normalized body text.
pub fn cmh_body_hash(text: &str) -> String {
    sha256_hex(normalize_body(text).as_bytes())
}

/// Deterministic atom id v1 used by the historical CMH canonicalization path.
pub fn cmh_atom_id_v1(workstream_id: &str, kind: &str, body: &str) -> String {
    let mut keyed = String::with_capacity(workstream_id.len() + kind.len() + 66);
    keyed.push_str(workstream_id);
    keyed.push('\0');
    keyed.push_str(kind);
    keyed.push('\0');
    keyed.push_str(&cmh_body_hash(body));
    let digest = sha256_hex(keyed.as_bytes());
    let prefix: String = digest.chars().take(32).collect();
    format!("atom:{prefix}")
}

/// HandoffArtifact state hash v1.
///
/// The caller passes canonical JSON with sorted keys and the same string
/// fallback as the historical Python caller. This function only performs the
/// digest step.
pub fn cmh_handoff_state_hash_v1(canonical_json: &str) -> String {
    format!("sha256:{}", sha256_hex(canonical_json.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_hash_normalizes_whitespace_and_case() {
        let a = cmh_body_hash("Use   GraphStore  for STORAGE");
        let b = cmh_body_hash(" use graphstore for storage ");
        assert_eq!(a, b);
        assert_eq!(
            a,
            "26c1cfd82b8259188a4f2d4a8b1ffe512b229353c94d40be871bf9dd2b8a9502"
        );
    }

    #[test]
    fn atom_id_is_deterministic() {
        let a = cmh_atom_id_v1("workstream:1", "decision", "Pin luma.gl 9.2.6");
        let b = cmh_atom_id_v1("workstream:1", "decision", "Pin luma.gl 9.2.6");
        assert_eq!(a, b);
        assert_eq!(a, "atom:84dc8c8d6022d7c054d91b2a97ffdac4");
        assert!(a.starts_with("atom:"));
        assert_eq!(a.len(), "atom:".len() + 32);
    }

    #[test]
    fn atom_id_separates_workstream_from_kind() {
        let a = cmh_atom_id_v1("a", "bx", "body");
        let b = cmh_atom_id_v1("ab", "x", "body");
        assert_ne!(a, b);
    }

    #[test]
    fn handoff_state_hash_prefixes_sha256() {
        let h = cmh_handoff_state_hash_v1("{\"a\":1}");
        assert_eq!(
            h,
            "sha256:015abd7f5cc57a2dd94b7590f04ad8084273905ee33ec5cebeae62276a97f862"
        );
        assert!(h.starts_with("sha256:"));
        assert_eq!(h.len(), "sha256:".len() + 64);
    }
}
