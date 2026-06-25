# theorem-browser-agent

The Rust-native browser-use perceive/govern/afford kernel: it resolves a browsing command into a permission/risk policy, perceives candidates against a `GraphStore`, and emits a gated action rail plus a browsing-run receipt. Servo-free and graph-adjacent; low-level web I/O routes through `rustyred-web`.

## Pipeline and key API

- Resolve: `resolve_context_command(ContextCommandRequest) -> ContextCommandState` (carries `can_write_hot_graph`, `can_execute_web_action`). Policies `PermissionPolicy` (`read_only()`, `web_consume()`), `RetrievalPolicy`, `TracePolicy`; enums `BrowserSurface { BrowseWithMe, BrowseForMe, WebConsume }`, `RiskMode`, `GraphLayer`, `ToolName`.
- Perceive: `perceive_with_graph<S: GraphStore>(store, &ContextCommandState, PerceptionInput) -> PerceptionBundle`. `PerceptionCandidate`, `CoverageDiagnosis`, `PageObservation`, `ObservedElement`, plus a visual-perceiver bridge.
- Afford: `build_action_rail(&ContextCommandState, &PerceptionBundle) -> ActionRailBundle`, `gate_action`. `ActionCandidate` with `ActionType`/`ActionCategory`/`ActionRisk`/`ExecutionRoute`.
- Receipt: `default_browser_playbooks()`, `browsing_run_receipt(...) -> BrowsingRunReceipt`. Consts `OPEN_WEB_UNVERIFIED_LAYER`, `DEFAULT_CONFIDENCE_CEILING = 0.35`.

Path dep: `rustyred-thg-core`. Single source file (`lib.rs`). A Python-fixture round-trip test implies a parity contract with a Python counterpart.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p theorem-browser-agent
```

Tests are inline in `lib.rs`. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
