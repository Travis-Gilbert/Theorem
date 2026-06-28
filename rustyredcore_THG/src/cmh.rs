//! Continuous Agent Memory Harness (CMH) native helpers.
//!
//! Exposes PyO3 hash helpers that mirror the historical Theseus Python
//! callers in ``apps.orchestrate.runtime.memory_canonical`` and
//! ``apps.orchestrate.runtime.handoff_compiler``. The corresponding
//! Python implementations remain in place as graceful fallbacks per the
//! `push_ppr` pattern (see `apps/notebook/sparse_ppr.py`). Byte-parity
//! is enforced by `rustyredcore_THG/tests/test_cmh_parity.py`.
//!
//! This module is not the Theorems Harness V2 memory storage or recall
//! path. Native harness memory is written and read through
//! `theorem-harness-runtime::memory` and the `rustyred-thg-mcp`
//! `McpMemoryStore` adapter over the caller's `GraphStore`
//! (`RedCoreGraphStore` for durable local stores). These CMH helpers only
//! pin the atom-id and handoff-hash algorithms so older Theseus CMH
//! artifacts and Rust-native peers can agree on identifiers.
//! The pure Rust owner is `theorem-harness-core::cmh`; this module is
//! only the Python ABI wrapper.
//!
//! Why Rust here, given Python's hashlib is already C-backed?
//!
//! Honest answer based on a 50k-iteration microbenchmark on the build
//! machine: at single-call granularity, **Python is faster than Rust**
//! for these hashers (Py 1.85us vs Rust 3.60us per call). The PyO3
//! boundary crossing costs more than the work saved because each
//! function does only two SHA256 rounds + a string concat. As a
//! result, the Python paths in
//! `apps/orchestrate/runtime/memory_canonical.py::_atom_id` and
//! `apps/orchestrate/runtime/handoff_compiler.py::_state_hash` do NOT
//! route through this module in production — they remain pure Python.
//!
//! This module is retained for three real-but-narrow reasons:
//!
//!   1. **Federation contract**: the CMH spec (§9, §11.4) expects
//!      cross-language consumers to compute the same atom_id and
//!      handoff state_hash from raw inputs. A canonical Rust impl that
//!      Python is byte-parity-tested against prevents algorithm drift
//!      between hosts even if no single host calls both.
//!
//!   2. **Future batch interface**: a hypothetical
//!      `cmh_atom_id_v1_batch(rows: Vec<(String, String, String)>)`
//!      that hashes ~hundreds of atoms in one PyO3 call would amortize
//!      the boundary overhead and become a net win for nightly
//!      bulk-canonicalize. Adding it later does not require changing
//!      the Python contract.
//!
//!   3. **Second source of truth for parity tests**:
//!      `theseus_native/tests/test_cmh_parity.py` cross-checks the
//!      Python implementation against this Rust reference so that any
//!      future refactor of the Python impl cannot silently change the
//!      output digest.
//!
//! Algorithm v1 contract (must stay byte-stable across host languages):
//!
//!   body_hash(text)
//!     = sha256_hex( normalize(text) )
//!     where normalize(text) = " ".join(text.to_lowercase().split_whitespace())
//!
//!   atom_id_v1(workstream_id, kind, body)
//!     = "atom:" + sha256_hex(workstream_id || "\0" || kind || "\0" || body_hash(body))[..32]
//!
//!   handoff_state_hash_v1(canonical_json, target_tokens, hard_cap)
//!     = "sha256:" + sha256_hex( canonical_json )
//!     where canonical_json is produced by the caller with
//!     json.dumps(payload, sort_keys=True, default=str). Target tokens
//!     and hard_cap participate in `payload` per the existing Python
//!     impl; this function does not re-canonicalize.

use pyo3::prelude::*;
use theorem_harness_core::{
    cmh_atom_id_v1 as core_cmh_atom_id_v1, cmh_body_hash as core_cmh_body_hash,
    cmh_handoff_state_hash_v1 as core_cmh_handoff_state_hash_v1,
};

/// SHA256 hex of the normalized body text.
#[pyfunction]
pub fn cmh_body_hash(text: &str) -> String {
    core_cmh_body_hash(text)
}

/// Deterministic atom id v1 used by ``memory_canonical._atom_id``.
///
/// Encoding mirrors the Python implementation exactly so the
/// canonicalize pipeline can emit byte-identical ids regardless of
/// which language ran it.
#[pyfunction]
pub fn cmh_atom_id_v1(workstream_id: &str, kind: &str, body: &str) -> String {
    core_cmh_atom_id_v1(workstream_id, kind, body)
}

/// HandoffArtifact state hash v1. The caller passes the canonical JSON
/// string (sorted keys, str fallback) so this function is purely a
/// digest step. Keeping serialization on the Python side avoids
/// recomputing the dict layout in Rust and matches the established
/// Python contract byte-for-byte.
#[pyfunction]
pub fn cmh_handoff_state_hash_v1(canonical_json: &str) -> String {
    core_cmh_handoff_state_hash_v1(canonical_json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_hash_normalizes_whitespace_and_case() {
        let a = cmh_body_hash("Use   GraphStore  for STORAGE");
        let b = cmh_body_hash(" use graphstore for storage ");
        assert_eq!(a, b);
    }

    #[test]
    fn atom_id_is_deterministic() {
        let a = cmh_atom_id_v1("workstream:1", "decision", "Pin luma.gl 9.2.6");
        let b = cmh_atom_id_v1("workstream:1", "decision", "Pin luma.gl 9.2.6");
        assert_eq!(a, b);
        assert!(a.starts_with("atom:"));
        assert_eq!(a.len(), "atom:".len() + 32);
    }

    #[test]
    fn atom_id_separates_workstream_from_kind() {
        // The null-byte separator must prevent ws||kind concatenation
        // collisions (e.g., ws="a", kind="bx" vs ws="ab", kind="x").
        let a = cmh_atom_id_v1("a", "bx", "body");
        let b = cmh_atom_id_v1("ab", "x", "body");
        assert_ne!(a, b);
    }

    #[test]
    fn handoff_state_hash_prefixes_sha256() {
        let h = cmh_handoff_state_hash_v1("{\"a\":1}");
        assert!(h.starts_with("sha256:"));
        assert_eq!(h.len(), "sha256:".len() + 64);
    }
}
