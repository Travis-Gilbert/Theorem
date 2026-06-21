# North Star Skill-Corpus + Completeness Addendum

Implementation of `theorem-harness-north-star-addendum-skill-corpus.md` (extends the North
Star execution loop). Reconciled against the codebase first; most of the hard machinery was
already built, so the addendum is mostly bring-up + small reliability fixes + design passes.

## Tier 1 (queueable) -- BUILT + GREEN

### T7: Skill-corpus bring-up -- DONE
The skill corpus is the same machine as the tool corpus. Reconciliation found the Compound
spine (T5) already treats skill_apply receipts as first-class compounding artifacts:
`skill_apply` -> `SkillPackUseReceipt` -> `collect_used_items` -> `apply_compound_standing`
is wired and tested (`compound_engineering.rs`). So T7 reduced to (a) publishing the three
packs and (b) closing a real integration gap so a compounded pack ranks in `ensemble_select`.

- Gap found + fixed: skill-published packs were invisible to `ensemble_select`. `CapabilityPack`
  requires a `spec` field `SkillPackState` lacked, and `effective_pack_fitness_from_node` read a
  top-level scalar `fitness`, not the nested `metadata.fitness.compound` the close hook writes.
  Bridge (additive, no new ranking): `SkillPackState` gains `spec` (publish sets `spec = pack.spec
  || pack`); `effective_pack_fitness_from_node` reads compound standing as a Laplace-smoothed
  success rate fallback. One dual-labeled `["CapabilityPack","SkillPack"]` node now serves both
  the skill loop and the ensemble registry.
- Packs: `crates/theorem-harness-runtime/src/engineering_packs.rs` authors rust-engineering from
  the plugin prose (canonical hash `sha256:325ba9c...`) and reuses `prose_check` /`design_check`
  canonical payloads for writing/design. `publish_engineering_packs` seeds all three.
- Acceptance: `crates/rustyred-thg-mcp/tests/skill_corpus_acceptance.rs` proves the full loop end
  to end (publish -> `skill_list` non-zero -> apply records a receipt -> a repeatedly-applied pack
  ranks FIRST with a strictly higher score; unused packs do not overtake it). Green.
- Live: `skill_list` for tenant `Travis-Gilbert` was zero; rust-engineering and writing-engineering
  are now published live (advisory), reachable by both heads. The design pack (49KB, artifact-heavy)
  seeds via the same proven `publish_engineering_packs` function rather than a 49KB inline MCP call.

### T8: Search-reach threshold -- DONE
`crates/rustyred-web/src/trigger_gate.rs`: collapsed the web-call reach decision onto one named
configurable value, `web_reach_threshold` (env `RUSTYRED_WEB_REACH_THRESHOLD` via
`with_env_overrides`), raised the default (0.2 vs the prior 0.01 mean-score floor) so similarity
search reaches the web more readily. Kept distinct from the rerank admission gate
(SPEC-SEARCH-RERANK-GATE-1.0). Tests: a previously-local moderate-evidence query now crawls;
lowering the single dial keeps it local. Green.

### T9: Dispatch-mirror fix -- DONE
`crates/rustyred-thg-server/src/state.rs`: the board write (`job_submit_to_store`) is canonical
and already committed, but a `?` propagated a Postgres dispatch-mirror failure and failed the whole
submit (the reliability bug). Fix: the mirror is now non-fatal (records `dispatch_mirrored:false` +
`dispatch_mirror_error`, still returns Ok), and an idempotent `migrate()` runs before submit so a
reachable-but-unmigrated database still receives the row. Regression test:
`job_submit_survives_a_failing_dispatch_mirror`. Green.

## Tier 2 (named, design pass) -- see `tier2-design-passes.md`
egglog (broad), Theorem programmability, training-data export, per-language packs +
codebase-architecture skill. Each is reconciled against what is built and given an approach,
acceptance, and the spec's open-design decision.

## Plugin-surface item -- see `plugin-command-structure.md`
Two commands plus ambient coordination, not three. Reconciled against the current plugin set.

## Validation
- `cargo test -p rustyred-thg-server --lib job_submit_survives_a_failing_dispatch_mirror` (T9)
- `cargo test -p rustyred-web --lib trigger_gate` (T8)
- `cargo test -p theorem-harness-runtime -p ensemble` (T7 bridge + packs)
- `cargo test -p rustyred-thg-mcp --test skill_corpus_acceptance` (T7 end-to-end)

Builds reuse the main checkout's `CARGO_TARGET_DIR` (disk-constrained host; see session report).
