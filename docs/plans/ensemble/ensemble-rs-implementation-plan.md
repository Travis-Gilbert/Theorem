# ensemble.rs implementation plan (grounded)

Date: 2026-06-05
Owner: Claude Code (per docs/plans/ensemble/README.md ownership: "destination architecture and the ensemble.rs core")
Coordination: room `repo:theorem:branch:main`, tenant `default`. Codex is live on the
rustyred-thg-mcp / theorem-harness-runtime / server / web working set; ensemble is built
as a NEW crate to stay additive and non-colliding.

## The gap (grounded against the code, not the prose)

Three layers already exist; ensemble fills the one between them:

| Layer | Where | Granularity | What it does |
|---|---|---|---|
| Affordance registry + selection + charter | `crates/rustyred-thg-affordances` (`registry.rs`, `selection.rs`, `charter.rs`, `types.rs`) | tool / connector | registers tools+engines as `Affordance` nodes; `select_affordances` / `select_affordances_by_embedding` (PPR + cosine); compiles per-binding charters (`BindingCharter`, CHARTER.COMPILED / CAPABILITIES.SELECTED) |
| Skill-pack serving | `crates/theorem-harness-runtime/src/skill_pack.rs` (Codex) | one pack kind | `SkillPackGraphStore` blanket trait over `GraphStore`; content-addressed `pack_content_hash`; publish/list/get/apply for `kind == skill_pack` only |
| **ensemble (this crate)** | **`crates/ensemble` (new)** | **pack** | **registry for ALL pack kinds + budgeted selector emitting a replayable Ensemble decision + trust ladder** |

`CapabilityPackSpec` is a **JSON contract** (a `kind` discriminator: skill / agent / tool /
validator / renderer / compute / policy / domain / context), not a Rust struct -- `skill_pack.rs`
only checks `kind == skill_pack`. ensemble stores the spec as a content-addressed JSON node
with that `kind`, generalizing the skill_pack storage pattern to every kind.

## New crate: `rustyredcore_THG/crates/ensemble`

- Dependencies: `rustyred-thg-core` (the `GraphStore` trait + `NodeRecord`/`EdgeRecord`/`NodeQuery`),
  `theorem-harness-core` (`stable_value_hash` for content addressing; binding/charter contracts),
  `serde` / `serde_json`. Optionally `rustyred-thg-affordances` (reuse `select_affordances` for the
  tool-level layer beneath the pack selector) -- add only when slice 2 needs it.
- Additive: the only shared file is `rustyredcore_THG/Cargo.toml` `[workspace] members` (Codex has
  NOT claimed it) plus a `Cargo.lock` regen. Build scoped: `cargo build -p ensemble`.
- Do NOT put ensemble inside `theorem-harness-core`, `theorem-harness-runtime`, or `rustyred-thg-mcp`
  -- all three are in Codex's active claim.

## Module structure

- `src/lib.rs` -- crate root, re-exports.
- `src/registry.rs` -- `CapabilityPack` (kind, content_hash, spec JSON, trust, exposure, source/artifact
  hashes), `PackKind`, `TrustTier`, and an `EnsembleGraphStore` blanket trait over `GraphStore`
  (mirror `SkillPackGraphStore`): `pack_upsert_node` / `pack_upsert_edge` / `pack_get_node` /
  `pack_query_nodes`. Content-addressed via `stable_value_hash`. Edges: `PACK_SOURCE`, `PACK_ARTIFACT`.
  `register_pack` / `get_pack` / `list_packs(kind?)`.
- `src/selector.rs` -- `EnsembleSelectRequest { task, budget, priors }` -> `EnsembleDecision`
  (selected packs/agents/tools, rejected candidates + reasons, risk summary, priors used). Pure,
  replayable, deterministic. Budget-bounded. This is the heart.
- `src/trust.rs` -- `TrustTier` ladder (`unverified` -> `first_party`) + passport id; gating in
  selection.
- `src/decision.rs` -- the `EnsembleDecision` artifact type (the former OrchestrateDecision), serde,
  content-addressable for replay/audit/training.

## Slices (each independently buildable + testable, `-p ensemble`)

- S1 registry [DONE]: `CapabilityPack` + `PackKind` + `TrustTier` + `EnsembleGraphStore` trait +
  `register_pack` / `get_pack` / `list_packs`. Content addressing + source/artifact edges. Unit tests
  over `InMemoryGraphStore`. (Mirrors skill_pack; the foundational slice.) `list_packs` +
  `pack_query_nodes` (mirroring Codex's `skill_pack_query_nodes`) landed with S2.
- S2 budgeted selector + `EnsembleDecision` [DONE -- `src/selector.rs`]: pure deterministic `select`
  over an explicit candidate set (score DESC, content-hash ASC tie-break; learned prior with a lexical
  cold-start fallback; budget-bounded greedy fill that keeps walking so a cheaper lower-score pack can
  still fit the remainder), rejected-with-reasons, plus a store-backed `select_from_store` wrapper.
  Scores rounded to 6dp so the decision `content_address` is reproducible. 8 pure-function tests.
- S3 trust ladder [DONE -- `src/trust.rs`]: `trust_rank` / `trust_score` / `parse_trust_floor` /
  `meets_floor` / `passport_id`; the selector applies the `min_trust` floor (priors) as an intrinsic
  gate and folds `trust_score` into relevance as a bounded bonus. 4 tests.
- S4 (COORDINATE WITH CODEX, later): MCP exposure (`ensemble_register` / `ensemble_select` /
  `ensemble_decision`) in `rustyred-thg-mcp/src/lib.rs` -- Codex's hot file. Claim + coordinate before
  touching. Until then ensemble is a pure library exercised by tests.

Validation (2026-06-05): `cargo test -p ensemble` = 18 passed; `cargo clippy -p ensemble
--all-targets --no-deps -- -D warnings` = clean. S2/S3 complete the crate as a pure library; only
S4 (the Codex-coordinated MCP surface) remains.

## Stays out of scope (named, not cut)

- The offline evolution / learning workbench (MAP-Elites, PBT, CMA-ES, bandits) stays Python; it
  writes the priors the native selector reads (content-addressed publish seam).
- The skill compile/encode pipeline stays Python.
- Hermes / OpenClaw / Perplexity participants are later capability packs surfaced through the selector,
  not part of the core crate.

## Validation per slice

`cargo test -p ensemble` and `cargo clippy -p ensemble --all-targets --no-deps -- -D warnings`,
scoped so it does not pull Codex's in-flight crates. Note: the local tree is currently `behind 5` from
origin and holds Codex's uncommitted work; a clean `cargo build -p ensemble` may require the tree to
settle (Codex's burst landing + a reconcile) -- the crate code is written to be correct against the
grounded `GraphStore` API regardless.
