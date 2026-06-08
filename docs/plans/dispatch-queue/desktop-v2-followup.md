# Dispatch v2 follow-up: desktop queue panel

Date: 2026-06-08.
Parent: docs/plans/dispatch-queue/dispatch-v2.md.

## Goal

Update the Theorem Desktop queue panel and Tauri commands from the retired strict queue surface to the Dispatch v2 board.

## Scope

- Replace `queue_status` command and API usage with `job_list`.
- Keep `job_submit`, but update the payload shape: no `kind`, no branch, no notes; accept `spec_ref` or `spec_inline`, `priority`, `target_head`, and `not_before`.
- Add desktop affordances for `job_note` and `job_archive` if the queue panel needs receipts or close-thread actions.
- Display derived state (`pending`, `started`, `archived`) from the MCP payload instead of legacy status.
- Preserve existing desktop bootstrap and runtime patterns. Do not touch unrelated browser or Tauri code.

## Known Stale Call Sites

- `apps/desktop/src/lib/commands.ts`
- `apps/desktop/src-tauri/src/lib.rs`

## Acceptance

- No desktop or Tauri code calls `queue_status`, `job_claim`, `job_complete`, `job_cancel`, or `job_promote`.
- Desktop can list Dispatch v2 jobs through `job_list`.
- Desktop submit uses the v2 job payload shape.
- Targeted desktop or Tauri tests run, or a compile check runs if no narrower test exists.
- `git diff --check` is clean.
