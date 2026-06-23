//! Cross-crate parity guard for the project anchor id (cut 5, Finding 2).
//!
//! The write path (`theorem-harness-runtime::memory::project_anchor_node_id`) and the
//! recall-side helper (`rustyred_thg_memory::project_anchor_node_id`) must resolve to
//! the SAME anchor node id for every input, or a membership edge written by one is
//! invisible to recall through the other and the project bias silently vanishes. The
//! two live in separate crates with separate impls; this test fails the moment they
//! drift, which is the practical guarantee behind "one shared anchor id".

use rustyred_thg_memory::project_anchor_node_id as memory_anchor;
use theorem_harness_runtime::memory::project_anchor_node_id as runtime_anchor;

#[test]
fn project_anchor_ids_match_across_write_and_recall_crates() {
    // Deliberately includes the non-trivial slugs that exposed the divergence:
    // capitals, spaces, punctuation, mixed tenant casing, and the empty edge cases.
    let cases = [
        ("theorem", "alpha"),
        ("theorem", "jobintel"),
        ("Theorem", "Alpha"),
        ("theorem", "My Project"),
        ("theorem", "Edge_Case 42!"),
        ("  Mixed-Case  ", "  Spaced Slug  "),
        ("TENANT", "a/b\\c:d"),
        ("t", "ALL CAPS PROJECT NAME"),
        ("theorem", ""),
        ("", "alpha"),
        ("", ""),
        ("theorem", "!!!"),
    ];
    for (tenant, project) in cases {
        assert_eq!(
            runtime_anchor(tenant, project),
            memory_anchor(tenant, project),
            "anchor id drift between write path and recall for tenant={tenant:?} project={project:?}"
        );
    }
}
