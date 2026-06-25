# ensemble

The pack-level capability layer over RustyRedCore-THG: a content-addressed `CapabilityPack` registry, a pure budgeted selector that emits a replayable `EnsembleDecision`, a trust ladder, and pack-invocation outcome/fitness recording. It sits above the tool-level `rustyred-thg-affordances` and the single-kind `theorem-harness-runtime::skill_pack`.

## Key API

- Registry (`registry.rs`): `register_pack`, `get_pack`, `list_packs`. `CapabilityPack`, `PackKind` (skill/agent/tool/validator/renderer/compute/policy/domain/context), `TrustTier` (`Unverified` / `FirstParty`), `PackExposure`, `EnsembleGraphStore`; consts `PACK_LABEL`, `PACK_EXPOSES_AFFORDANCE`, `PACK_IN_DOMAIN`.
- Selector (`selector.rs`): `select` (pure, deterministic), `select_from_store`, `select_unified_from_store` (one ranked ordering across packs and affordances with domain bias). `EnsembleSelectRequest`, `UnifiedSelectionRequest`.
- Decision (`decision.rs`): `EnsembleDecision` (with `content_address()`), `SelectedCapability`, `RejectedCandidate`.
- Trust (`trust.rs`): `trust_rank`, `trust_score`, `parse_trust_floor`, `meets_floor`.
- Outcomes (`outcomes.rs`): `record_pack_invocation` (receipt plus decaying pack fitness, mirroring the affordance crate), `effective_pack_fitness_from_node`.

Path deps: `rustyred-thg-core`, `rustyred-thg-affordances`, `theorem-harness-core`. The registry, budgeted selector, trust gating, invocation outcomes, and unified pack-plus-affordance selection are implemented; MCP exposure (`ensemble_select`/`ensemble_register`) is served from `rustyred-thg-mcp`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p ensemble
```

`tests/cut4_acceptance.rs` (outcome lifts later selection; pure `select` replayable; trust-floor plus budget gates) plus inline tests. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
