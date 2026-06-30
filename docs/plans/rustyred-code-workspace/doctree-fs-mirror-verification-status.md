# DocTree FS Mirror Verification Status

Date: 2026-06-30

This manifest tracks the implementation status for the downloaded DocTree
filesystem mirror specs:

- `SPEC-DOCTREE-FS-MIRROR-DOWNSTREAM-GRAPH`
- `SPEC-DOCTREE-FS-MIRROR-VERIFICATION`

Legend: `[x]` verified, `[~]` implemented partially or blocked by a concrete
repo/platform seam.

## Downstream Graph Spec

| # | Requirement | Status | Evidence |
| --- | --- | --- | --- |
| 1 | Workspace root is the canonical code mirror | [x] | `import_checkout_mirror`, real-path DocTree entries, byte parity tests |
| 2 | Import lands real files | [x] | `mirror_import_indexes_real_files_and_oracle_is_clean` |
| 3 | Graph is downstream index and rebuildable | [x] | `drop_and_rebuild_code_index_preserves_non_file_graph_content` |
| 4 | notify keeps index in agreement | [x] | watcher create/modify/delete/rename/subtree tests |
| 5 | Agent works in directory directly | [x] | `agent_session_plan_points_at_real_mirror_root`; direct edit loop is covered by watcher tests |
| 6 | CommonPlace connect is the front door | [x] | `commonplace-api` exposes `connectRepository`, backed by an injected Engine repository connector; `repo_connect_acceptance` verifies local checkout, public URL, and credentialed private URL paths produce real files and File nodes |

## Verification Extension

| # | Requirement | Status | Evidence |
| --- | --- | --- | --- |
| 1 | Consistency oracle | [x] | `audit_consistency`, clean/manual-change/stray/fresh-read tests |
| 2 | Runtime reconcile and audit | [x] | `reconcile`, startup catch-up, forced repair, Prometheus renderer, `WorkspaceMirrorAuditMonitor`, and CommonPlace `/metrics`; `periodic_audit_monitor_reports_drift_and_recovery_metrics` plus `public_router_exposes_repository_mirror_metrics_when_configured` |
| 3 | Atomic save | [x] | `watcher_converges_atomic_save_bursts_and_subtree_moves` |
| 4 | Rapid burst | [x] | `watcher_converges_atomic_save_bursts_and_subtree_moves` |
| 5 | Delete | [x] | `watcher_reconciles_create_modify_delete_and_rename` |
| 6 | Rename | [x] | `watcher_reconciles_create_modify_delete_and_rename` |
| 7 | Subtree delete or move | [x] | `watcher_converges_atomic_save_bursts_and_subtree_moves` |
| 8 | Excluded region | [x] | import filter tests and fuzz excluded writes |
| 9 | Watcher gap | [x] | `watcher_start_reconciled_repairs_changes_missed_while_stopped` |
| 10 | Event overflow | [x] | `watcher_debouncer_error_repairs_event_overflow_gap` injects the debouncer error batch shape used for rescan signals, and `watcher_force_rescan_repairs_event_overflow_style_gap` verifies the repair path |
| 11 | Binary file | [x] | `mirror_handles_binary_file_without_crashing` |
| 12 | Large file | [x] | `mirror_options_skip_oversized_file_and_remove_stale_projection` |
| 13 | Symlink | [x] | `mirror_symlink_cannot_escape_workspace_root` |
| 14 | Unreadable file | [x] | `unreadable_file_is_skipped_and_rest_of_tree_stays_consistent`; gated to Unix permissions |
| 15 | Crash mid-write | [x] | `startup_reconcile_repairs_crash_gaps_on_both_sides` |
| 16 | Concurrent edit during index | [x] | `concurrent_edit_during_index_converges_to_final_bytes` |
| 17 | Property and fuzz harness with falsification | [x] | `randomized_mirror_property_harness_covers_full_operation_space` runs the configured minimum operation count across create/modify/delete/rename/subtree/atomic/burst/large/binary/symlink/excluded operations; `oracle_catches_injected_rename_modify_defect` proves falsification |
| 18 | First-class isolation hardened | [x] | rebuild isolation and fuzz isolation tests |
| 19 | Anti-skip governance | [x] | this manifest tracks every row as verified, consistency tests use the filesystem oracle, `DEFAULT_CONVERGENCE_TIMEOUT` bounds convergence, and CommonPlace `/metrics` exposes runtime audit |

## Remaining Non-Local Gaps

None recorded.
