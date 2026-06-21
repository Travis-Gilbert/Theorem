# Tier 2 design passes

The addendum names four Tier-2 items as "needs a design pass." Each is reconciled against what
is already built, then given an approach, the spec's open-design decision, and acceptance.

---

## 1. egglog -- the expose-the-reasoning half (broad egraph)

**Want:** port Theorem's Python Datalog symbolic-reasoning layer (under `apps/notebook/`) to
egglog/egg in Rust, into the substrate as a first-class harness tool. Modernization +
consolidation, not greenfield. Aligns with the `03-datalog-symbolic-derivation` and
`05-egraph-equivalence-theorems` plans.

**Already built (reconciled):**
- Cut 9 added `epistemic_egraph_dedup` in `rustyred-thg-core/src/epistemic.rs` using `egg = "0.10"`
  (an e-graph already in the workspace). The cut-9 dedup is one congruence relation that falls out
  of this broader item.
- The symbolic surface is partly exposed: `rustyred_thg_symbolic_datalog_derive`,
  `..._probabilistic_source_reliability`, `..._probabilistic_expected_value` MCP tools, backed by
  `rustyred-thg-core/src/symbolic.rs`.
- The byte-parity Datalog gate (`derive_datalog_receipt`) is parity-tested against the Python
  reference corpus in `apps/notebook/benchmarks/`.

**Approach:** introduce egglog as a *backend* behind the existing symbolic surface, not a
replacement. Keep `derive_datalog_receipt` and its Python-parity gate untouched (an egglog rewrite
would break byte-parity). Add an egglog-derived path as a parallel, non-parity capability:
`symbolic_egraph_derive(rules, facts)` compiles the ruleset into egglog rules, saturates, and reads
back equalities/derivations. The cut-9 `epistemic_egraph_dedup` is the reference for the egg/egglog
plumbing (node-limit lifting, `find()`-driven grouping). Expose it as a new MCP/GraphQL field under
the existing symbolic domain.

**Open design (spec):** the mapping from the Python ruleset (`apps/notebook/.../*.py` Datalog) to
egglog rules. Sub-questions: which rules are pure rewrites (egglog-native) vs side-effecting; how to
represent provenance edges so derived facts stay attributable; whether to vendor `egglog` or keep
`egg` + hand-written saturation.

**Acceptance:** a planted ruleset derives the same closure under both the Python Datalog gate and
the egglog backend on a shared fixture (semantic, not byte, equality); the egg dedup remains green;
the byte-parity gate is unchanged.

---

## 2. Theorem programmability -- the external-developer surface

**Want:** one unified surface for outside developers, named as one spec instead of three
fragments: the SDK as the orchestration brain (cut 14), ensemble pack publishing as
content-addressed `CapabilityPack`s a third party publishes (the T7 mechanism opened up), connector
authoring (publish-your-own-MCP), and the browser-use-style domain-skill contribution model.

**Already built (reconciled):**
- T7 (this addendum) made `skill_publish` / `ensemble_register` the pack-authoring surface and
  unified skill packs into the ensemble registry (the `spec` bridge). This IS the pack-publishing
  half, now needing a third-party trust boundary.
- `ensemble` carries a `TrustTier` (`unverified` / `first_party`) and `PackExposure` -- the trust
  ladder already exists.
- `rustyred-thg-connectors` is the live MCP connector transport (connect -> `tools/list` ->
  register as learnable `Affordance`); `rustyred-thg-affordances` learns selection. This is the
  connector runtime; authoring is the missing half.
- `theorem-harness` (SDK v2) is the source surface for generated Python/Node/Swift/WASM bindings.

**Approach:** the spec is "give outside authors the publish side of surfaces whose runtime already
exists." Four seams: (a) pack publishing already works (T7) -- add a third-party path that forces
`TrustTier::unverified` + a review/exposure gate before a third-party pack can be `ensemble_select`ed
into another tenant's runs; (b) connector authoring = a thin authoring CLI/route over
`rustyred-thg-connectors::register_connector` that content-addresses a `ConnectorManifest` and
publishes it as an `Affordance` pack; (c) SDK orchestration = document + stabilize the
`theorem-harness` run-handle surface as the public contract; (d) domain-skill contribution = the
per-language pack pattern (item 4) opened to PRs.

**Open design (spec):** the authoring/publishing surface shape and the trust boundary for
third-party packs. Sub-questions: per-tenant allowlist vs a global review queue; whether unverified
packs are advisory-only until a benchmark gate; signing/provenance for third-party authors.

**Acceptance:** a third-party-authored pack publishes at `TrustTier::unverified`, is selectable only
in its author's tenant (or an opted-in tenant) until promoted, and a connector authored via the new
seam round-trips `register -> tools/list -> select`. No third-party pack can reach a non-opted-in
tenant's selection.

---

## 3. Training-data export -- users own their training data

**Want:** a user-owned export of the masked-edge graph dataset, the inferred-candidate admission
outcomes as preference data, and the tool-use trajectories. Adjacent to cut 13.

**Already built (reconciled):**
- `rustyred-thg-affordances::export_affordance_training_view` already exports the affordance
  ranking view (the tool-selection preference signal).
- The Compound spine writes compound ledgers on skill packs + memory docs (`metadata.fitness.compound`,
  the T5/T7 receipts) -- a ready preference signal.
- Reflexive RustyRed (`rustyred-thg-adapters`) produces the masked-edge dataset (the graph is its
  own label source) and inferred-candidate admission outcomes (quarantined advisory nodes).

**Approach:** a tenant-scoped `training_export` tool that bundles the existing signals into one
user-owned archive: (a) `export_affordance_training_view` (tool selection), (b) skill-pack +
memory compound ledgers (the apply/outcome trajectories), (c) the masked-edge dataset from the
reflexive layer, (d) membrane admission outcomes as preference pairs. Format: JSONL per record type
with a manifest (record counts, schema version, tenant, generated-at). Strictly read-only.

**Open design (spec):** the export format and the tenant boundary. Sub-questions: JSONL vs parquet;
whether to include raw content or only structural/preference signals (privacy); how to scope
shared-tenant coordination data so one head's export does not leak another actor's private memory.

**Acceptance:** `training_export(tenant)` returns only that tenant's records across all four signal
types; a separate tenant's export shares no records; the archive round-trips (re-importable schema);
no cross-tenant leakage.

---

## 4. Per-language engineering packs + codebase-architecture skill

**Want:** JS, Python, C, C++, and Go packs on the browser-use domain-skill pattern, plus a
codebase-architecture / anti-sprawl skill (agents scatter code in a way a human architect does not).

**Caution (carried verbatim from the spec):** read the Bencium and Plock skill libraries first and
encode from what they actually contain. Do not fabricate the contents.

**Already built (reconciled):**
- The T7 rust-engineering pack is the template: `rust_engineering_pack_payload` in
  `engineering_packs.rs`, authored faithfully from plugin prose, content-addressed.
- `prose-check` / `design-check` are the in-repo "skill-pack payload crate" pattern (a checker +
  `*_pack_payload()` + `pack_hash()`), reused by T7.
- The plugin prose pattern (`skills/<name>/SKILL.md`) is the authoring source for rust-engineering.

**Approach:** each language pack follows one of two shapes already in the tree -- (a) plugin prose
(`skills/<lang>-engineering/SKILL.md`) + a `*_engineering_pack_payload()` authored from it (the
rust-engineering shape, lightest), or (b) a payload crate with a real checker (the prose-check /
design-check shape, when the language warrants deterministic receipts). Add each to
`engineering_capability_packs()` so `publish_engineering_packs` seeds the whole corpus. The
codebase-architecture / anti-sprawl skill is a non-language pack in the same shape, encoding
placement/structure heuristics (where new code belongs, when to extract, sprawl smells).

**Open design (spec) + gate:** sourcing. This item is explicitly gated on reading the **Bencium**
and **Plock** skill libraries, which are external and not in this repo. The design pass names the
gate: do not author these packs from imagination; first ingest the two libraries (locate or be
given them), extract their actual capabilities/checks/anti-patterns, and encode from that. Until
those libraries are in hand, only the *shape* (above) is settled, not the *content*.

**Acceptance (when unblocked):** each language pack's capabilities/validators trace to a concrete
item in the Bencium/Plock source (no fabricated content); `publish_engineering_packs` seeds them;
each is `skill_apply`-able and compounds like rust-engineering; the codebase-architecture skill
flags a planted sprawl case in a fixture.
