# Dispatch v2: permissionless session launcher

Date: 2026-06-08. Supersedes simplify-remove-governing-layer.md entirely and the claim, capacity, status-lifecycle, and dependency-enforcement sections of HANDOFF.md (3effa4a2). The receiver's auth model, key stripping, worktree mapping, and head adapters (702f250) stand unchanged.

## Principle

The queue does one thing: deliver a spec and start a session with the harness up and informed. It is permissionless. No claims, no lanes, no ownership, no states to guard. Dependencies named in a spec are information for the agent, never conditions in the infrastructure. Agents self-manage through the coordination layer; the queue fades into the background.

## Deliverables

1. `theorem-harness-core/src/job.rs`: delete the JobStatus enum. State is derived from fields:
   - pending: `started_at` null and `archived_at` null
   - started: `started_at` set, with `session_ref`
   - archived: `archived_at` set, with `archived_reason`
   Core fields: `job_id`, `title`, `spec_ref` or `spec_inline` (one required), `priority` (P0|P1|P2, a hint), `target_head` (claude|codex|either, a hint), `repo`, `not_before` (optional timestamp), `submitted_by`, `submitted_at`, `receipts` (append-only list of {actor, at, text, refs}). No transition logic anywhere.
   Migration table, applied at read time for existing nodes: Queued and Open become pending; Claimed, Running, PrOpen, Verifying set `started_at` from `claimed_at` if present else migration time; Done becomes archived with reason "done"; Failed, Cancelled, Dropped become archived with the old status as reason. Old `claimed_by` is folded into a receipt for provenance.

2. `rustyred-thg-mcp` verb surface, exactly four verbs, every one callable by any actor with no permission distinction between humans and agents:
   - `job_submit`: create or upsert on `idempotency_key`; upsert may change priority, spec_ref, not_before. Accepts spec_ref or spec_inline.
   - `job_list`: the board. Derived state, sorted priority then submitted_at, receipts tail included. Optional filter by state and repo.
   - `job_note`: append a receipt.
   - `job_archive`: set archived_at and reason.
   Removed from the surface: job_claim, job_complete, job_done, job_promote, queue_status. No aliases. Call sites updated (receiver, desktop queue panel from 1c65be2a).

3. `theorem-receiver` becomes the session launcher. Loop:
   a. `job_list` pending, drop jobs whose `not_before` is in the future, order by priority then age.
   b. Up to local concurrency (a receiver-local config value, default one child per configured worktree, never represented in graph state), take the next job and write `started_at` plus `session_ref` with compare-and-set on `started_at` null. This set-once write is the only exactly-once in the system; it exists solely so two receivers never spawn two children for one job. On a lost race, skip to the next job.
   c. Resolve spec text: `spec_inline` verbatim, else read `spec_ref` from the worktree.
   d. Compose the launch prompt: full spec text, then a context packet (recent room messages and open coordination_intents via the harness coordination_context call, plus top-k recall on the job title keywords), then a footer naming the actor identity, job_id, room id, the doctrine lines below, and one instruction: refresh with coordination_context if this packet looks stale.
   e. Probe the harness once (tools/list). If unreachable, append a receipt explaining the abort, clear `started_at`, and retry the job next cycle. Sessions never start blind.
   f. Spawn through the head adapter for `target_head` with provider keys stripped per HANDOFF.md, MCP config pointing at the harness.
   g. On child exit, append one receipt (exit code, branch tip if any). No other writes, no monitoring, no lifecycle.
   The receiver performs no dependency checks, no capacity accounting in graph state, and writes no status of any kind.

4. Doctrine, added to CLAUDE.md and AGENTS.md alongside the existing line:
   "Dependencies named in a spec are information for you, not gates. Check the tree, decide, and note your reasoning on the job."

5. Tests, rewritten to this model: set-once start race (two launchers, one pending job, exactly one child); migration table covering all eight legacy statuses plus the interim four; submit upsert semantics; receipts append from multiple actors interleaved; launch prompt composition snapshot (spec plus packet plus footer); not_before skip.

## Enabling semantics, named choices

- Self-dispatch: agents call job_submit to queue follow-up work for future sessions. The background-process lane (consolidation, deferred verification, page monitors from the servo step-3 spec) rides this with `not_before`.
- Job as thread: receipts accumulate from any actor across sessions and days; the job node is the durable thread of the work.
- The context packet is composed at spawn so sessions start informed at token zero.

## Acceptance

- A job submitted from claude.ai with a spec_ref appears pending in job_list; a running receiver starts exactly one session whose first prompt contains the full spec text and a context packet; the node shows started_at and session_ref.
- Two receivers against one pending job produce one child.
- An agent-submitted job is indistinguishable in handling from a human-submitted one.
- No verb refuses any actor any action on any job. A grep for "claim" in the queue crates matches only migration code and tests.
- A job with future not_before is not launched until the time passes.
- Closing the loop end to end: submit, launch, child commits, child calls job_note with the commit ref, anyone calls job_archive with reason "done".

## Fences

- The coordination layer (coordinate, coordination_intent, presence, rooms, mentions) is untouched.
- No status enum, no transition guards, no lanes, no graph-state capacity, no dependency enforcement anywhere in queue or receiver.
- No time estimates, no em dashes in docs.
