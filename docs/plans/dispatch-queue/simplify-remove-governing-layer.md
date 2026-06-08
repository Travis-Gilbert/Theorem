# Dispatch queue simplification: remove the governing layer

Date: 2026-06-08. Supersedes the claim, capacity, and lifecycle-guard sections of HANDOFF.md (3effa4a2). Everything else in that handoff stands (receiver shape, auth model, key stripping, worktree mapping).

## Why

The claim/capacity layer gridlocked three live sessions on 2026-06-07. job-001 and job-002 sat Claimed after their work was delivered in 1c65be2a, and a third session refused buildable job-007 because it read prose sequence as a gate. The coordination layer (rooms, intents, presence, held-not-clobbered) had already proven lock-free shared-worktree work the same night. The queue was enforcing ownership on top of a layer built to make ownership fluid. Authorship of the locks: claude-ai, in the dispatch spec, RQ worker semantics extended past their valid scope.

## Doctrine

The queue is a board, not a governor. Jobs are durable intent: what Travis wants built, where the spec lives, a priority hint. Live coordination is the coordination layer's job: coordination_intent with claimed_files (advisory), presence, room messages. No locks anywhere.

Agent doctrine line, verbatim, for CLAUDE.md and AGENTS.md: "If you can see the work and the files are free per intents, build. Never wait on a status."

## Deliverables

1. `theorem-harness-core/src/job.rs`: collapse JobStatus to `Open | Verifying | Done | Dropped`. Migration mapping for existing nodes: Queued becomes Open; Claimed, Running, PrOpen are removed (live state lives in coordination_intent); Verifying stays; Failed and Cancelled become Dropped. Deserialization of old status strings maps through this table rather than erroring. `claimed_by` and `claimed_at` fields remain readable as historical provenance, are never written by new code, and gate nothing.

2. `theorem-harness-runtime/src/job_queue.rs`: delete the CAS claim path, capacity policy, repo locks, and lane gating. Delete the lifecycle transition guards; any status can be written by any actor with a note. Surviving verbs and their semantics:
   - `job_submit`: unchanged (durable intent, idempotency_key dedupe).
   - `queue_status`: unchanged board view, sorted priority then submitted_at. Optional, not required: join open coordination_intents whose claimed_files overlap the job repo, displayed as "active intents" on the board.
   - `job_promote`: unchanged (priority hint).
   - `job_cancel`: sets Dropped with note.
   - `job_complete` renamed `job_done`: sets Done plus receipts. Non-gating: partial or imperfect work stays Verifying with notes; nothing requires acceptance criteria green to move other work.
   - `job_claim`: removed from the MCP surface entirely.

3. `theorem-receiver`: replace the claim loop. New loop: pick the highest-priority Open job whose DEPENDS_ON edges (if any) point at Done jobs, write a set-once `spawned_by` marker (compare-and-set on that single field, the only surviving exactly-once in the system), spawn the child. No repo capacity governance in graph state; the receiver's own process management decides local concurrency (named choice: default one child per configured worktree, a receiver-local setting, invisible to other actors). Child exits without calling job_done: receiver appends an exit receipt note (exit code, branch tip if any) and sets Verifying. Key stripping and auth model unchanged from HANDOFF.md.

4. `rustyred-thg-mcp` verb surface: reflect deliverable 2. Remove job_claim registration, rename job_complete to job_done, keep the rest.

5. Sequencing: DEPENDS_ON edges between Job nodes only where a real build dependency exists. Prose sequence in coordinate messages or job notes is a priority hint and never grounds for refusing work. The receiver honors edges; interactive sessions treat them as advice.

6. Tests: rewrite to the simplified model. Delete the CAS contention tests for claims. Add: double-spawn prevention via the spawned_by marker (two receivers, one Open job, exactly one child); status migration table; job_done writes receipts without gating.

## Acceptance

- Two interactive sessions can both work the same Open job's repo on disjoint files, announced via coordination_intent, and no verb refuses or warns either of them.
- queue_status returns the board with the four-status model; pre-existing nodes with old statuses appear correctly mapped.
- A receiver pointed at two Open jobs in one worktree spawns them per its local concurrency setting, with spawned_by set exactly once per job.
- job_claim no longer exists on the MCP surface.
- Doctrine line present in CLAUDE.md and AGENTS.md.

## Fences

- Do not touch the coordination layer (coordinate, coordination_intent, presence, rooms, mentions). It is the proven thing.
- No new statuses beyond the four. No locks of any kind anywhere in the queue.
- No time estimates, no em dashes in docs.
