# Reconstruction Engine — Codex Work Review Checklist

Running review of Codex's uncommitted reconstruction-engine work (started 2026-06-27).
Codex is **actively editing** `rustyred-thg-reconstruct/src/lib.rs` and `rustyred-thg-code/src/engineering/**`. Fixes applied directly (per Travis: fix in place), via surgical content-anchored edits in quiet windows; the Edit freshness-guard aborts on collision. Verified each edit survives Codex's later writes.

Companion: Codex keeps its own `build-ledger.md` ("what's built" + proofs). This file is the review/fix log (kept separate).

Method note: AI reviewers have a real false-positive rate here — **every finding is verified against live code before action**. Several "Major" claims were verified WRONG (see Dropped). One fix (#5) was reverted after it broke a test that encodes intended behavior.

Status legend: `[x]` fixed+standing · `[r]` reverted · `[ ]` open/pending · `[-]` not-a-bug/won't-fix · `[~]` needs verification

## Batch 1

### Fixes standing (4)
| # | Sev | Issue | Location | Status |
| --- | --- | --- | --- | --- |
| 1 | Major | Jump-table case wrongly flagged `is_default` when it has neither label nor label_value (`.map(...).unwrap_or(true)`). Sparse Ghidra case (destination only) misclassified as switch default. | `reconstruct/src/lib.rs:10776` | `[x]` → `.unwrap_or(false)` |
| 3/6 | Major | Jump-table `case_id` hash folded in post-sort positional `index` ⇒ non-idempotent ids across re-ingest (duplicate nodes). | `program_analysis.rs` `normalize_oracle_jump_table_case_facts` | `[x]` content-only hash (dropped `index`, de-enumerated loop) |
| 7 | Minor | `WORK_ITEM_USES_PASS` edge emitted to a pass node that may not exist ⇒ dangling edge. | `program_analysis.rs` work-item loop | `[x]` guarded by `analysis_passes.any(analyzer_id==…)` |

### Reverted (1)
| # | Sev | Issue | Location | Status |
| --- | --- | --- | --- | --- |
| 5 | — | Reference-drift keys never match (native `…:{fact_kind}:-2` vs oracle `…:{reference_type}:{operand_index}`). I changed both to address-pair only. | `program_analysis.rs` `native_reference_keys`/`reference_key` | `[r]` REVERTED — broke `ghidra_reference_semantic_roles_are_derived_from_ref_type`, which encodes **intended type-sensitive drift** (a recovery-gap signal). NOT a clear bug. See Design Questions. |

### Dropped — verified NOT bugs (reviewer over-claims)
| # | Claim | Location | Why dropped |
| --- | --- | --- | --- |
| 5* | `data_type_layout_hash` omits pointee ids | `reconstruct:5735` | pointees already hashed via `hard/soft_dependency_type_ids` |
| 6 | `FloatNotequal` wrongly commutative | `lift:550` | symmetric incl. NaN; matches Ghidra `isCommutative()` exactly |
| 7 | `IntNegate` wrongly `IntegerLogical` | `lift:500` | `INT_NEGATE` is bitwise NOT (logical); arith negation `INT_2COMP` already correct |
| 11 | symbol `index` from chained enumerate breaks reloc correlation | `binformat:414` | `relocation_target_name` uses object-crate `SymbolIndex` directly; no code joins the two index spaces |
| 12 | `Assign`→`Copy` with `output:None` invalid | `lift:918` | all stmt kinds set `output:None` — deliberately coarse THIR, valid in-model |
| C1 | `component_stack_frame_boost`/`…call_stack_effect_boost` dead | `reconstruct:12156/12183` | both ARE called (lines 6153, 6432) — not dead |

### Low-priority / cosmetic (logged, not fixing)
- #10 reference `MissingOracleFact` expected/observed inverted vs `push_count_drift` — consistent w/ sibling drift fns; cosmetic.
- #15 `GHIDRA_PCODE_MAX=75` name vs value — style.
- #13 `LABEL`/`CROSSBUILD`→`PTRADD`/`PTRSUB` aliases (`lift:456-457`) — SLEIGH directives, not pointer ops; near-zero impact (don't appear in lifted facts); recommend `None` but it's Codex's deliberate pattern (also BUILD→MULTIEQUAL). Flag, don't churn.
- B4 `collect_imports` undefined-symbol uses `NativeLoaderError::Symbol` not `Import` (`native_loader.rs:238`) — misleading variant; trivial.
- B5 `loader_fact` hash omits `endian` (`native_loader.rs:93`) — theoretical (sha256 already covers bytes).

### Test failures observed in batch-1 run (bb4cedw1v, pre-revert)
- `engineering::program_analysis::tests::ghidra_reference_semantic_roles_are_derived_from_ref_type` — caused by my #5; **fixed by reverting #5** (needs re-confirm).
- `tests::runtime_diagnostics_reports_store_status_and_lock_activity` — NOT mine (CodeCrawler runtime subsystem; doesn't touch reconstruction/engineering). Likely Codex's concurrent edit or timing-sensitive under concurrent build load. To confirm + attribute, not fix.

## Batch 2 — candidates (verification in progress)
| # | Sev | Issue | Location | Status |
| --- | --- | --- | --- | --- |
| C2 | Major | `structure_field_access_contracts` evidence not chained into `instruction_for_component` evidence ⇒ EvidencePresence validator may false pass/fail. | `reconstruct/src/lib.rs:~16502-16562` | `[~]` verify |
| C4 | Major | callotherfixup `userop_index` set to per-spec-local `payload_index`, but site matching compares against global CALLOTHER slot ⇒ possible false-positive index bindings. | `reconstruct/src/lib.rs:2262-2311` + `:7639` | `[~]` verify (name-match path may dominate) |
| C5 | Minor | `normalize_high_variable_contracts`: `isolated = isolated \|\| type_locked` forces isolated when only type is locked. | `reconstruct/src/lib.rs:~7018` | `[~]` verify intent |
| C6 | Minor | `Requirement::DecompilerRuleApplication` drops `action_repeated` (used by confidence boost). | `reconstruct/src/lib.rs:1673,16411` | `[~]` verify (adds enum field; cascades) |
| C3 | Minor | `bind_*` `dedup_by(contract_id)` after sort whose primary key isn't `contract_id` (adjacent-only dedup). | `reconstruct/src/lib.rs:2351-2359,2381-2389` | `[-]` latent smell — fn1 is 1:1 map (no dups), fn2 ids distinct per site; no active dups. Low value; output-order risk. |
| B1 | Major | `collect_imports` library attribution can be lost via BTreeSet `(None,name,None)` vs `(Some(lib),name,None)` depending on loop order. | `native_loader.rs:233-244` | `[~]` verify |
| B2 | Major | trace `EvidenceSource.targets` = self ids (not graph node ids) ⇒ HAS_EVIDENCE_SOURCE edges reroute to compile root. | `trace_to_contract.rs:97-115` | `[~]` verify (may be intentional fallback) |
| B3 | Major | `trace_validator_ref` ref format never equals generated `validator_id` ⇒ validator cross-refs resolve to nothing. | `trace_to_contract.rs:292-297` | `[~]` verify |

## Design questions for Codex/Travis (not unilaterally changing)
- **Reference drift type-sensitivity (#5):** native `fact_kind` vs Ghidra `reference_type` are different vocabularies, so real Ghidra exports always produce reference drift. Intended as a recovery-gap signal (tests encode it), or should drift compare on address pair? Left as-is.

## Notes
- Verification: `cargo test -p rustyred-thg-code -p rustyred-thg-reconstruct --lib`; focused re-verify after #5 revert pending. Runs serialize behind Codex's target-dir lock.
- Coverage: reconstruct (hash/lowering/validator/edge/contract families two passes), program_analysis (drift/scheduler/oracle/write-through), lift+binformat+disasm diffs, trace_to_contract + native_loader, reconstruct-harness. Not yet: Ghidra `.java` exporters; out-of-scope crates (intake/pg-server/acp/web/cmh).
