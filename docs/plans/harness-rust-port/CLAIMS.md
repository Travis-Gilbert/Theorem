# Harness Rust Port Claims

Date: 2026-06-01
Coordination mode: hybrid. The live harness room/intent/presence tools are
responding again, but direct THG mirror writes still report tenant resolution
degradation. Keep this file and path-scoped commit messages as the durable
coordination fallback until substrate mirroring is clean.

## Current Claims

| Actor | Status | Files | Notes |
|---|---|---|---|
| Codex | done for Lane E held-out Rust validation gate | `/Users/travisgilbert/Tech Dev Local/Creative/Website/Index-API/apps/notebook/encode/benchmarks.py`; `/Users/travisgilbert/Tech Dev Local/Creative/Website/Index-API/apps/notebook/encode/tests/test_benchmarks.py`; `/Users/travisgilbert/Tech Dev Local/Creative/Website/Index-API/apps/notebook/encode/validation_tasks/rust_refactoring_v1.jsonl`; `docs/plans/skill-encoder-theorem-port/implementation-plan.md`; `docs/plans/theorems-harness-plugin-switchover/implementation-plan.md`; `docs/plans/harness-rust-port/CLAIMS.md` | Added the full 20-task Rust refactoring held-out corpus and the pure `rust_held_out_validation_gate` runner over baseline/treatment `UseReceipt`s. The gate enforces task coverage, `validator_pass_rate >= 0.95`, and `benchmark_treatment_beats_baseline`, then emits `canonical_ready`; actual promotion still waits on real E1 pack/run receipts. Validation: `cd /Users/travisgilbert/Tech Dev Local/Creative/Website/Index-API && .venv/bin/python -m pytest apps/notebook/encode/tests/test_benchmarks.py` passed (15 tests). |
| Codex | done for Lane S3b bounded native validator artifact runner | `rustyredcore_THG/crates/theorem-harness-runtime/src/skill_pack.rs`; `docs/plans/skill-encoder-theorem-port/implementation-plan.md`; `docs/plans/theorems-harness-plugin-switchover/implementation-plan.md`; `docs/plans/harness-rust-port/CLAIMS.md` | Extended `skill_apply` from declaration-only validation to a bounded native artifact-descriptor sandbox. Published pack metadata artifacts with `kind: native_validator_candidate` / Rust validator artifact kinds now run the code-member signature predicate in-process and persist receipts with `validator_execution_mode: native_artifact_sandbox`. The runner does not compile or execute arbitrary uploaded Rust source in the MCP request path; future crate/WASM execution remains a separate isolated runner. Validation: `cargo test -p theorem-harness-runtime`, `cargo test -p rustyred-thg-mcp native_skill_pack_tools_round_trip_through_mcp`, and `cargo clippy -p theorem-harness-runtime -p rustyred-thg-mcp --all-targets --no-deps -- -D warnings` passed. |
| Codex | done for plugin core coordination native-first routing | `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/mcp/server.mjs`; `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/sdk/route-policy.mjs`; `docs/plans/theorems-harness-plugin-switchover/implementation-plan.md`; `docs/plans/harness-rust-port/CLAIMS.md` | Routed the source plugin's core room/agent-space tools through the native MCP route first: `coordination_room`, `presence`, `coordination_intent`, `coordination_reflection`, `coordination_decision`, `coordination_tension`, `coordinate`, and `mentions`. Reflection/decision/tension map to native `coordination_record`; old Theseus endpoints stay as fallback. `mentions_wait`, `subscribe`, `continuity_pack`, and hook context refresh still need a native-specific pass. Validation: `node --check mcp/server.mjs`, `node --check sdk/route-policy.mjs`, and `node sdk/route-policy.test.mjs` passed in the plugin repo. |
| Codex | done for plugin skill-pack route wiring | `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/sdk/route-policy.mjs`; `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/sdk/route-policy.test.mjs`; `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/mcp/server.mjs`; `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/README.md`; `docs/plans/theorems-harness-plugin-switchover/implementation-plan.md`; `docs/plans/skill-encoder-theorem-port/implementation-plan.md` | Added a native MCP JSON-RPC client to the source plugin route policy, classified `skill_list`, `skill_get`, `skill_publish`, and `skill_apply` as native skill capability verbs, advertised those tools from the slim MCP, and forwards calls to the Theorem/RustyRed MCP with route receipts. Validation: `node --check sdk/route-policy.mjs`, `node --check mcp/server.mjs`, and `node sdk/route-policy.test.mjs` passed in the plugin repo. |
| Codex | done for Lane S native skill-pack serving slice | `rustyredcore_THG/crates/theorem-harness-runtime/src/skill_pack.rs`; `rustyredcore_THG/crates/theorem-harness-runtime/src/lib.rs`; `rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs`; `rustyredcore_THG/crates/rustyred-web/Cargo.toml`; `rustyredcore_THG/Cargo.lock`; `docs/plans/harness-rust-port/CLAIMS.md`; `docs/plans/skill-encoder-theorem-port/implementation-plan.md`; `docs/plans/theorems-harness-plugin-switchover/implementation-plan.md` | Added `theorem-harness-runtime::skill_pack`, a GraphStore-backed content-addressed `SkillPack` store keyed by `pack_content_hash`, with source/artifact hash nodes, `SKILL_PACK_SOURCE`, `SKILL_PACK_ARTIFACT`, and `SKILL_PACK_APPLIED` edges, plus durable `SkillPackUseReceipt` nodes. `rustyred-thg-mcp` now exposes `skill_list` and `skill_get` in read-only mode and gates `skill_publish` and `skill_apply` behind write mode. `skill_apply` persists use receipts and runs safe deterministic validator declarations; sandboxed execution of arbitrary Rust validator artifacts remains a follow-up. Validation: `cargo test -p theorem-harness-runtime`, `cargo test -p rustyred-thg-mcp`, and `cargo clippy -p theorem-harness-runtime -p rustyred-thg-mcp --all-targets --no-deps -- -D warnings` passed. Cargo resolution was unblocked by moving the dirty optional `rquest` dependency entry to the maintained `wreq` package alias in `rustyred-web`; the `impersonate-fetch` feature itself was not validated. |
| Codex | done for native memory substrate | `rustyredcore_THG/crates/theorem-harness-runtime/**`; `rustyredcore_THG/crates/rustyred-thg-mcp/**`; `docs/plans/harness-rust-port/CLAIMS.md`; `docs/plans/harness-rust-port/native-mcp-superset.md` | Added `theorem-harness-runtime::memory`, a GraphStore-backed native memory atom store for documents and typed memory nodes, plus MCP exposure for `remember`, `recall`, `relate`, `self_note`, `self_revise`, `self_archive`, `self_recall_archive`, `encode`, `forget`, `handoff`, and `observe`. Read tools remain listed in read-only mode; write tools are listed/callable only when write mode is enabled. Validation: `cargo test -p theorem-harness-core`, `cargo test -p theorem-harness-runtime`, `cargo test -p rustyred-thg-mcp`, both runtime/MCP clippy gates, and `git diff --check` passed. Live deploy/write-mode/auth remain Lane O. |
| Codex | done for native coordination record substrate | `rustyredcore_THG/crates/theorem-harness-runtime/**`; `docs/plans/harness-rust-port/CLAIMS.md`; `docs/plans/harness-rust-port/implementation-plan.md`; `CLAUDE.md` | Added `event`, `decision`, `tension`, and `reflection` records as a shared `CoordinationRecordState` graph contract with `CoordinationRecord` nodes and `COORDINATION_RECORD_OF` room edges. Runtime tests cover writes, filtering, limits, invalid type rejection, graph IDs/edges, and RedCore reopen persistence. |
| Codex | done for MCP exposure slice | `rustyredcore_THG/crates/theorem-harness-core/**`; `rustyredcore_THG/crates/theorem-harness-runtime/**`; `rustyredcore_THG/crates/rustyred-thg-mcp/**`; `rustyredcore_THG/Cargo.lock`; `docs/plans/harness-rust-port/CLAIMS.md`; `docs/plans/harness-rust-port/parity/**`; `docs/plans/harness-rust-port/parity-context/**` | Rust `theorem-harness-core` now ports the pure state machine, replay/fork helpers, toolgraph toolkit selector, context-web bounded pack compiler/policy core, pure affordance registry/receipt contract, Pairformer session metrics, federated structural-signal privacy helpers, the pure MapArtifact compiler, and memory preparation contracts. `theorem-harness-runtime` adds the spec's GraphStore-backed event-log seam and native direct-coordination substrate with room membership, live intents, durable presence, direct messages, mentions, and durable records. `rustyred-thg-mcp` now exposes those native coordination tools plus bundled turn-start coordination context, structured contribution-capture packets, durable-write policy receipts, `harness_append_transition`, and `harness_run` over MCP with read-only/write-mode gating and GraphStore-backed round-trip coverage. |
| Codex | done for HTTP coordination-read exposure | `apps/theorem-harness-server/**`; `docs/plans/harness-rust-port/ios-transport-handoff.md`; `CLAUDE.md` | Extended the standalone Axum transport over `theorem-harness-runtime` with native coordination read endpoints: room status, room presence, room intents, actor mentions, and durable room records. The server keeps write/consume semantics only for mention consumption and reads the same `RedCoreGraphStore` as the run transport. |
| Claude Code | done for HTTP run exposure | `apps/theorem-harness-server/**`; `docs/plans/harness-rust-port/ios-transport-handoff.md` | Added the standalone Axum run transport over `theorem-harness-runtime`, including run listing/detail endpoints for the iOS handoff. |
| Claude Code | done for fixture slices | `docs/plans/harness-rust-port/parity/**`; `docs/plans/harness-rust-port/parity-toolgraph/**`; `docs/plans/harness-rust-port/parity-context/**` | Generated Python reference fixtures from `Index-API/apps/orchestrate/runtime/state_machine.py`, `toolgraph.py`, and `context_web/{contracts,policy}.py`; Codex extended the state-machine corpus to 25 scenarios / 260 steps and consumed the toolgraph/context corpora read-only for the Rust ports. |
| Claude Code | done for plugin coordination+memory SOLE-PATH finish + subagent auth + affordances framing | `codex-plugins/theorems-harness/mcp/server.mjs`; `codex-plugins/theorems-harness/scripts/auto-approve-harness.sh` (new); `codex-plugins/theorems-harness/hooks/hooks.json`; `codex-plugins/theorems-harness/hooks/codex-hooks.json`; `codex-plugins/theorems-harness/plugin.manifest.json`; `codex-plugins/theorems-harness/.codex-plugin/plugin.json`; `docs/plans/theorems-harness-plugin-switchover/tool-contract-matrix.md` | Built ON Codex's native-first routing: removed the Theseus `theoremPost` fallbacks + THG-product mirror from all coordination + memory verbs -> native SOLE path (Travis directive; the `/harness` backend is 500ing so the fallback was dead). `mentions_wait` -> native `mentions` client-side poll; `subscribe` folded; `continuity_pack` -> native `coordination_record(record_type=reflection)`; `relate` (edge upsert) -> native `rustyred_thg_bulk_edges` (NOT native `relate`, which is a neighbor read -- confirm edge shape on live write). Removed dead `legacyRemember`. Shipped `scripts/auto-approve-harness.sh` PreToolUse hook (auto-approves `theorems-harness` + `rustyred-thg` namespaces incl the `mcp__plugin_theorems-harness_*` runtime form; defers others) wired in both hook manifests = subagents authorized by default. Manifest `defaultPrompt` + codex output now state the two-execution-heads-of-one-agent framing (advisory intent + deepest-in-file tiebreak, turn-start `coordination_context`, shared frontier, end-of-slice contribution). v0.4.0 verified across all files. Validation: `node --check mcp/server.mjs`; `node sdk/route-policy.test.mjs`; all plugin JSON parse; zero coordination/memory `theoremPost`; live native smoke returned the active room. Uncommitted; peer-review handoff `msg_a2fc9f33b346ad94`. Reconcile: the `mentions_wait`/`subscribe`/`continuity_pack` items Codex listed as open are DONE plugin-side (no new native verb needed). |

## Git Protocol

- Use path-scoped commits only.
- Do not create a second harness crate. The Phase 1 crate is
  `rustyredcore_THG/crates/theorem-harness-core`.
- If editing a claimed file, update this table first in the same path-scoped
  commit or coordinate in the commit message.
- Keep unrelated dirty state (`CLAUDE.md`, `Product.MD`,
  `rustyredcore_THG/crates/reconstruction-engine/Cargo.lock`) out of this slice
  unless Travis explicitly asks to include it.

## Immediate Acceptance Target

- `cargo test -p theorem-harness-core` passes from `rustyredcore_THG/`.
- `cargo test -p theorem-harness-runtime` passes from `rustyredcore_THG/`.
- `cargo test -p rustyred-thg-mcp` passes from `rustyredcore_THG/`.
- `cargo clippy -p theorem-harness-runtime --all-targets --no-deps -- -D warnings`
  passes from `rustyredcore_THG/`.
- `cargo clippy -p rustyred-thg-mcp --all-targets --no-deps -- -D warnings`
  passes from `rustyredcore_THG/`.
- `cargo test` passes from `apps/theorem-harness-server/`.
- `cargo clippy --all-targets -- -D warnings` passes from
  `apps/theorem-harness-server/`.
- A live local `theorem-harness-server` smoke returns `ok`, an empty run list,
  empty presence/intents, empty mentions, empty records, and default room status
  from an empty `RedCoreGraphStore`.
- `python3 docs/plans/harness-rust-port/parity/generate_fixtures.py --check`
  regenerates byte-identical fixtures.
- `python3 docs/plans/harness-rust-port/parity-toolgraph/generate_toolkit_fixtures.py --check`
  regenerates byte-identical toolkit fixtures.
- `python3 docs/plans/harness-rust-port/parity-context/generate_context_fixtures.py --check`
  regenerates byte-identical context pack fixtures.
- The Rust parity test replays legal and illegal transition sequences through
  the Rust port, comparing `state_hash_before`, `state_hash_after`, status,
  sequence number, and guard codes against the Python reference output.
- The Rust toolgraph parity test compares the catalog plus compiled toolkit
  outputs against the Python reference corpus.
- The Rust context-web parity test compares bounded pack output, generated
  artifact policy, token ledger, source mix, edge/path filtering, validation,
  and evaluation output against the Python reference corpus.
- The Rust affordance registry test covers the eleven Python projection
  affordance IDs and the content-addressed `AffordanceReceipt` envelope. Engine
  execution wrappers remain runtime/native-engine work, not core-crate IO.
- The Rust session-metrics tests cover Pairformer mode normalization, JSONL
  loading, completed-session summaries, and Welch-z mode comparison.
- The Rust federated-signal tests cover recursive raw-content rejection, receive
  normalization, coarse privacy buckets, and structural patch projection without
  importing Django.
- The Rust map-artifact tests cover stable map IDs, scope resolution, CodebaseMap
  baseline entries, applied delta upsert/remove behavior, ToolMap metadata,
  markdown/json export, and artifact summaries.
- The Rust memory-contract tests cover typed hydration handles, recall policy
  normalization, nested recall previews, full contract parsing, Python-style
  immutable/editable truthiness, active status defaults, and serialization shape.
- The Rust runtime coordination tests cover room membership persistence,
  `COORDINATION_MEMBER_OF` graph edges, live intent replacement and filtering,
  durable presence heartbeat/end records, direct message persistence,
  `COORDINATION_MENTIONS` graph edges, consume-on-read mention inbox behavior,
  durable event/decision/tension/reflection records,
  `COORDINATION_RECORD_OF` graph edges, and `RedCoreGraphStore` reopen behavior
  for all coordination primitives.
- The Rust MCP coordination tests cover native tool listing, read-only/write
  gating, room join, presence heartbeat/readback, intent write/readback,
  coordinate receipt shape, pending mention reads, consume-on-read semantics,
  room message reads, durable record writes, typed record reads, and bundled
  coordination context reads, plus contribution capture through the MCP server
  surface. Policy hook tests cover required-scope denial, cost-budget denial,
  and allowed writes persisting `policy_receipt` metadata.
- The Rust MCP run-log tests append a complete lifecycle through
  `harness_append_transition`, then read back the same `{run, events}` contract
  through `harness_run`, including the `CONTEXT.PACKED` token ledger and
  `OUTCOME.RECORDED` validator result.
- The HTTP transport tests cover run list/detail and coordination read
  contracts for room status, presence, intents, actor mentions, and
  consume-on-read mention semantics. Room record reads are covered with
  type-filter assertions.
