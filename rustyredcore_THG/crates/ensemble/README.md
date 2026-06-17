# ensemble

Ensemble: the capability-pack registry, budgeted selector, and trust ladder over RustyRedCore-THG. The pack-level layer above rustyred-thg-affordances (tools) and theorem-harness-runtime::skill_pack (one kind).

## What it is

Ensemble: the capability layer over RustyRedCore-THG.

Ensemble is the pack-level registry + budgeted selector + trust ladder that sits
ABOVE the tool-level `rustyred-thg-affordances` crate (which selects individual
tools/connectors) and the single-kind `theorem-harness-runtime::skill_pack` (which
serves only `kind == skill_pack`). It registers `CapabilityPack`s of every kind
(skill, agent, tool, validator, renderer, compute, policy, domain, context) as
content-addressed nodes in the same GraphStore skill packs use, and -- in later
slices -- selects which packs/agents/tools to bring in per task under a budget,
emitting a replayable `EnsembleDecision`.

Status: slices S1 (registry), S2 (budgeted selector + replayable `EnsembleDecision`), and S3
(trust-ladder gating in selection) are implemented as a pure library. MCP exposure (S4 --
`ensemble_register` / `ensemble_select` in Codex's hot `rustyred-thg-mcp`) stays a coordinated
follow-up. Tracked in `docs/plans/ensemble/ensemble-rs-implementation-plan.md`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p ensemble
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
