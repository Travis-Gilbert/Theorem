# rustyred-plugin

Programmable Harness plugin host: declarative skill plugins and Extism-backed
WASM capabilities for RustyRed/THG.

This crate is the runtime boundary for agent-authored capabilities. Declarative
skills compose existing affordances as data. WASM plugins run through Extism over
wasmtime with explicit memory, timeout, fuel, WASI, host, and path limits.

## Key API

- WASM host: `PluginHost`, `WasmPluginSpec`, `WasmPluginSource`,
  `PluginLimits`, `LoadedWasmPlugin`.
- Host grants: `HostFunctionGrant::{GraphRead, FactWrite, AffordanceRegister}`.
  Host functions return structured grant-denial payloads when a plugin was not
  declared with the required grant.
- Declarative skills: `DeclarativeSkillDefinition`,
  `DeclarativeSkillStep`, `invoke_declarative_skill`, and
  `DeclarativeAffordanceInvoker`. `to_skill_publish_request` emits the payload
  shape consumed by the existing `skill_publish` registry.
- Safety gate: `CapabilityGateRequest`, `CapabilityGateReceipt`,
  `evaluate_capability_gate`, and `rollback_capability`. Programmable capability
  exposure uses theorem-harness-core's tier-two action policy and requires
  passing declared tests plus human authorization.
- Corpus flywheel: `BehaviorPatternCandidate`,
  `surface_crystallization_candidate`, and
  `crystallize_pattern_to_declarative_skill`.

Path deps: `rustyred-thg-core` and `theorem-harness-core`.
The `rustyred-thg-affordances` bridge registers plugin exports as ordinary
affordances, and `rustyred-thg-connectors` can invoke persisted `rustyred_plugin`
targets through the normal affordance `invoke` path.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-plugin
```

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md)
for the crate map.
