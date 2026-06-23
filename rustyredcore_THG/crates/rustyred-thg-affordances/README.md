# rustyred-thg-affordances

Connector-as-substrate learning registry: MCP tools as first-class Affordance graph nodes with learned, scoped selection over RustyRedCore-THG.

## What it is

Connector-as-substrate learning registry.

MCP connector tools become first-class `Affordance` graph nodes. The
substrate learns which affordances to reach for from accumulated outcomes
(PPR over `SERVED_TASK`/`PRODUCED_OUTCOME` edges + fitness), scoped per
agent by a `CapabilityScope` (the capability-scope plane of the
AgentBinding). This is not a passthrough aggregator: selection compounds
with use because it rides the same graph the rest of the substrate learns on.

Layering: this crate sits above `rustyred-thg-core` (graph store, PPR) and
reuses `theorem-harness-core` (the affordance vocabulary + the pairformer
A/B validation gate). It is the structural sibling of the LoRA adapter
catalog (`rustyred-thg-adapters`) and the dependency-shape sibling of
`theorem-harness-runtime`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-affordances
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
