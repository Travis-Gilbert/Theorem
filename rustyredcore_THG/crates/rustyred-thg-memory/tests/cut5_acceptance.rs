//! Independent acceptance suite for Handoff 5 (Permeable Project Scope).
//!
//! Written by the verifier head (claude-code) against the PUBLIC api of
//! `rustyred-thg-memory`, with zero edits to the crate source. It maps 1:1 to the
//! handoff's acceptance criteria so it doubles as the recall-side regression guard
//! for the implementation head (codex). Fixtures are built directly in an
//! `InMemoryGraphStore` using the crate's own `project_anchor_node_id` /
//! `project_membership_edge_id` / `MEMORY_IN_PROJECT`, so the test agrees with the
//! recall contract by construction.

use rustyred_thg_core::{EdgeRecord, InMemoryGraphStore, NodeRecord};
use rustyred_thg_memory::{
    project_anchor_node_id, project_membership_edge_id, recall, MemoryRecallInput,
    HARNESS_MEMORY_LABEL, MEMORY_DOCUMENT_LABEL, MEMORY_IN_PROJECT, MEMORY_PLUGIN_SOURCE,
    MEMORY_PROJECT_LABEL,
};
use serde_json::json;

const TENANT: &str = "theorem";

fn doc(store: &mut InMemoryGraphStore, id: &str, tenant: &str, text: &str, project: Option<&str>) {
    let mut props = json!({
        "tenant_id": tenant,
        "doc_id": id,
        "title": text,
        "content": text,
        "summary": text,
    });
    if let Some(project) = project {
        props["project_slug"] = json!(project);
    }
    store
        .upsert_node(NodeRecord::new(
            id,
            [HARNESS_MEMORY_LABEL, MEMORY_DOCUMENT_LABEL],
            props,
        ))
        .unwrap();
}

fn anchor(store: &mut InMemoryGraphStore, tenant: &str, project: &str) {
    store
        .upsert_node(NodeRecord::new(
            project_anchor_node_id(tenant, project),
            [HARNESS_MEMORY_LABEL, MEMORY_PROJECT_LABEL],
            json!({
                "tenant_id": tenant,
                "tenant_slug": tenant,
                "project_slug": project,
                "source": MEMORY_PLUGIN_SOURCE,
            }),
        ))
        .unwrap();
}

fn join_project(store: &mut InMemoryGraphStore, tenant: &str, member: &str, project: &str) {
    store
        .upsert_edge(EdgeRecord::new(
            project_membership_edge_id(tenant, member, project),
            member,
            MEMORY_IN_PROJECT,
            &project_anchor_node_id(tenant, project),
            json!({ "tenant_id": tenant, "project_slug": project }),
        ))
        .unwrap();
}

fn recall_input(tenant: &str, query: &str, project: &str, permeability: f64) -> MemoryRecallInput {
    MemoryRecallInput {
        tenant_id: tenant.to_string(),
        query: query.to_string(),
        project_slug: project.to_string(),
        project_permeability: permeability,
        top_k: 10,
        budget_tokens: 100_000,
        bump_activation: false,
        ..MemoryRecallInput::default()
    }
}

fn rank_of(result: &rustyred_thg_memory::RankedMemories, graph_id: &str) -> Option<usize> {
    result.memories.iter().position(|m| m.graph_id == graph_id)
}

/// Criterion 3: "Seeding the project anchor lifts the project's member cluster:
/// at equal lexical score, in-project members rank above unrelated tenant memories."
///
/// This is the discriminating test, built to exercise the clobber-prone ordering on
/// purpose. Both docs use realistic `mem:doc:*` ids, which sort BEFORE the anchor id
/// (`mem:project:*`), so each member's reverse edge is pushed into the anchor's
/// adjacency BEFORE the anchor is visited in the sorted id loop. If `memory_adjacency`
/// ever REPLACES (rather than extends) the anchor's adjacency on its own iteration,
/// the reverse edges are clobbered, the anchor cannot reach its members, and the bias
/// vanishes. The unrelated doc (`...a-out`) also sorts before the in-project member
/// (`...b-in`), so the score tie-break (graph_id ascending) favors the outsider; only
/// a working anchor seed through a bidirectional MEMORY_IN_PROJECT edge can flip it.
#[test]
fn c3_anchor_seed_lifts_member_at_equal_lexical() {
    let mut store = InMemoryGraphStore::new();
    let text = "shared planning marker";
    anchor(&mut store, TENANT, "alpha");
    doc(
        &mut store,
        "mem:doc:theorem:b-in",
        TENANT,
        text,
        Some("alpha"),
    );
    join_project(&mut store, TENANT, "mem:doc:theorem:b-in", "alpha");
    // Unrelated tenant memory, equal lexical, id sorts BEFORE the in-project member.
    doc(&mut store, "mem:doc:theorem:a-out", TENANT, text, None);

    let result = recall(&mut store, recall_input(TENANT, text, "alpha", 1.0)).unwrap();

    let zin = rank_of(&result, "mem:doc:theorem:b-in");
    let aout = rank_of(&result, "mem:doc:theorem:a-out");
    assert_eq!(
        zin,
        Some(0),
        "in-project member must rank first under high permeability; got order {:?}",
        result
            .memories
            .iter()
            .map(|m| (m.graph_id.as_str(), m.score))
            .collect::<Vec<_>>()
    );
    assert!(
        aout.is_some() && zin < aout,
        "anchor seed must lift the in-project member above the equal-lexical outsider"
    );
}

/// Criterion 1: high permeability ranks the project first, but a strongly-connected
/// sibling-project memory still surfaces (a bias, not a filter).
#[test]
fn c1_high_permeability_ranks_project_first_sibling_still_surfaces() {
    let mut store = InMemoryGraphStore::new();
    anchor(&mut store, TENANT, "alpha");
    anchor(&mut store, TENANT, "beta");
    doc(
        &mut store,
        "mem:alpha-1",
        TENANT,
        "shared graph planning",
        Some("alpha"),
    );
    doc(
        &mut store,
        "mem:alpha-2",
        TENANT,
        "shared graph build",
        Some("alpha"),
    );
    doc(
        &mut store,
        "mem:beta-1",
        TENANT,
        "shared graph sibling",
        Some("beta"),
    );
    join_project(&mut store, TENANT, "mem:alpha-1", "alpha");
    join_project(&mut store, TENANT, "mem:alpha-2", "alpha");
    join_project(&mut store, TENANT, "mem:beta-1", "beta");
    // Strong cross-project link: an alpha memory supports the beta memory.
    store
        .upsert_edge(EdgeRecord::new(
            "edge:alpha-beta",
            "mem:alpha-1",
            "supports",
            "mem:beta-1",
            json!({ "tenant_id": TENANT }),
        ))
        .unwrap();

    let result = recall(
        &mut store,
        recall_input(TENANT, "shared graph", "alpha", 1.0),
    )
    .unwrap();

    let first = &result.memories[0].graph_id;
    assert!(
        first == "mem:alpha-1" || first == "mem:alpha-2",
        "an alpha (in-project) memory must rank first, got {first}"
    );
    assert!(
        rank_of(&result, "mem:beta-1").is_some(),
        "the strongly-connected sibling-project memory must still surface (bias, not filter)"
    );
}

/// Criterion 1 (sweep): permeability is a dial. At max it concentrates on the
/// project; at zero the project gets no extra pull and recall is tenant-wide. We
/// assert the in-project member's rank is no worse under high permeability than
/// under zero (the dial only ever helps the project), and strictly better when the
/// outsider would otherwise win the tie.
#[test]
fn c1_permeability_is_a_dial() {
    let build = || {
        let mut store = InMemoryGraphStore::new();
        let text = "dial marker text";
        anchor(&mut store, TENANT, "alpha");
        doc(
            &mut store,
            "mem:doc:theorem:b-member",
            TENANT,
            text,
            Some("alpha"),
        );
        join_project(&mut store, TENANT, "mem:doc:theorem:b-member", "alpha");
        doc(&mut store, "mem:doc:theorem:a-other", TENANT, text, None);
        store
    };

    let mut high = build();
    let high_result = recall(
        &mut high,
        recall_input(TENANT, "dial marker text", "alpha", 1.0),
    )
    .unwrap();
    let mut zero = build();
    let zero_result = recall(
        &mut zero,
        recall_input(TENANT, "dial marker text", "alpha", 0.0),
    )
    .unwrap();

    let high_rank = rank_of(&high_result, "mem:doc:theorem:b-member").unwrap();
    let zero_rank = rank_of(&zero_result, "mem:doc:theorem:b-member").unwrap();
    assert!(
        high_rank <= zero_rank,
        "raising permeability must not demote the in-project member (high {high_rank} <= zero {zero_rank})"
    );
    assert_eq!(
        high_rank, 0,
        "at max permeability the in-project member should lead the equal-lexical outsider"
    );
}

/// Criterion 2: the hard tenant wall is unchanged. A memory from another tenant
/// never appears regardless of project_slug or permeability.
#[test]
fn c2_hard_tenant_wall_is_unchanged() {
    let mut store = InMemoryGraphStore::new();
    anchor(&mut store, TENANT, "alpha");
    anchor(&mut store, "other-tenant", "alpha");
    doc(
        &mut store,
        "mem:mine",
        TENANT,
        "shared project secret",
        Some("alpha"),
    );
    join_project(&mut store, TENANT, "mem:mine", "alpha");
    // Same project slug, DIFFERENT tenant. Even joined to its own anchor it must
    // never cross the tenant wall.
    doc(
        &mut store,
        "mem:theirs",
        "other-tenant",
        "shared project secret",
        Some("alpha"),
    );
    join_project(&mut store, "other-tenant", "mem:theirs", "alpha");

    for permeability in [0.0, 0.5, 1.0] {
        let result = recall(
            &mut store,
            recall_input(TENANT, "shared project", "alpha", permeability),
        )
        .unwrap();
        assert!(
            rank_of(&result, "mem:theirs").is_none(),
            "cross-tenant memory must never appear (permeability {permeability})"
        );
        assert!(
            rank_of(&result, "mem:mine").is_some(),
            "same-tenant in-project memory must appear (permeability {permeability})"
        );
    }
}
