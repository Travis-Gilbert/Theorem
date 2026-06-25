# Godel Feature DSL Parity

This directory lands the downloaded execution spec for the safe Feature DSL
slice of the Godel self-modification plan.

## Implementation Status

- Spec: `SPEC-GODEL-FEATURE-DSL-PARITY.md`
- Parity source read: `Index-API/docs/plans/2026-04-07 SPEC-GODEL-BUILD-ORDER- Sequenced Build Plan for Self-Modifying Theseus.md`, Section 7
- Rust crate: `rustyredcore_THG/crates/rustyred-thg-core`
- Modules changed:
  - `feature_dsl.rs`: closed serde AST, budgeted evaluator, graph traversal delegation, deterministic adversarial corpus, and DynamicFeature config payloads
  - `ranking.rs`: default-off feature signal slot in the ranking cascade
  - `lib.rs`: public exports

The DSL is read-only and data-only: no source-string evaluation, no codegen, no
unsafe, no IO, no graph mutation. A feature is not trusted because it parses;
it is serialized as a config value and is intended to pass through the Godel
substrate closing-loop gate before being kept.

## Validation

Focused validation:

```bash
cd rustyredcore_THG
cargo test -p rustyred-thg-core feature_dsl
cargo test -p rustyred-thg-core ranking::cascade_tests::feature_rule_is_default_off_and_scores_when_enabled
cargo clippy -p rustyred-thg-core --all-targets --no-deps -- -D warnings
```
