# Handoff: Multi-Head Run Execution

The run becomes sovereign over work, not only state. Two execution heads (Claude Code, Codex) attach to one living run: shared task graph, shared claims and evidence ledgers, shared receipts, shared run memory. Judgment stays independent. This is the substrate layer for plural agency, and it is the first thing the two heads build as one agent, so the pattern proves itself by building itself.

Locked principle: unify state and the work queue, do not unify judgment. The value of two heads is the disagreement that catches defects (this arc: the contract-parity divergence, the read_only write bug, the dead robots code). The waste to kill is reconstruction, each head re-discovering what the other did, and collision, two heads starting the same unit.

## What already exists and is reused (read before building)

theorem-harness-core is further along than a blank slate. Reuse these rather than reinventing:

- The composed-agent deliberation kernel (`agent_binding.rs`): a shared `BindingMemoryScope` with a versioned `ScratchpadDocument` and `ScratchpadRevision`s, `MemoryZone`/`MemoryZoneKind` separating shared from private work (`PRIVATE_WORK.OPENED`), `GroundedClaim` with grounding enforcement, `HeadContributionRecord`, budget, `ActionTierPolicy`, and a single guarded entry point `apply_binding_transition`. This is the shared-memory substrate. The run-level shared state extends this, it does not replace it.
- The critique role and consensus gate (`head_invocation.rs` `HeadInvocationKind::Critique`; `alignment.rs` `evaluate_publication` + `MIN_CONSENSUS_HEADS`). Critique already exists as a first-class adversarial step, and publication already passes a multi-head consensus gate. The verify node below is the evolution of this from a fixed step into a continuously claimable node type.
- The single guarded-transition pattern (`state_machine.rs` `apply_transition`, `state_hash.rs` `hash_run_state`/`stable_value_hash`). Node state changes and the claim compare-and-swap go through one guarded function in the same style, with the same hashing discipline and idempotency keys. The CAS is the existing state-hash discipline applied to a claim field.
- Replay and fork (`replay.rs`). A run is already replayable and forkable; the work graph rides the same event ledger so a multi-head run stays replayable.

What is missing, and is the whole build: a task graph, claimable nodes, a compare-and-swap claim, write arbitration across heads, node-type to skill-pack bindings, and the ambient verify loop. The run is sovereign over state today; it is not yet sovereign over work distribution. The duplicate browser-agent crate happened because there is no claimable "create the crate" node, heads coordinate through a mailbox (intents, records, mentions), not by inhabiting a shared queue.

## The work graph (data model)

A transactional work graph lives in the GraphStore beside RunState as a new node type plus edges, not as a field on RunState (RunState stays the lifecycle; the graph is the work).

```
TaskNode {
  id
  run_id
  parent_id            // null for roots; set when a node is refined into children
  node_type            // binds a skill-pack motor program (see below)
  goal                 // what "accepted" means for this node
  prerequisites[]      // node ids that must be accepted before this is ready
  file_scope[]         // declared at refine time; a ROUTING HINT, not a lock
  status               // open | claimed | patch_proposed | verifying | accepted | rejected
  claim_owner          // head_id or null
  claim_epoch          // monotonic; the CAS target
  patch_ref            // the proposed patch, when status >= patch_proposed
  receipts[]           // proof attached at completion (see receipt contract)
  review_required_by   // head_id of the head that must verify (the other head)
  created_by           // head_id
}
```

Edges: `PREREQUISITE_OF`, `REFINED_INTO` (parent to child), `VERIFIES` (verify node to its target), `CLAIMED_BY` (node to head). A node is ready when every prerequisite is accepted.

## Claim-and-refine (the atomic primitive)

The atomic operation is claim-and-refine, not claim. You cannot declare `file_scope` for work you have not scoped yet (this arc: the file scope of the browse loop was unknowable until the code was read and a new helper in router.rs was discovered). So the graph is co-constructed by the heads as structure is revealed, a dynamic blackboard, not a static job queue.

- `claim_task_node(run_id, node_id, head_id, expected_epoch)`: compare-and-swap on `claim_epoch`. Success sets `claim_owner` and bumps the epoch. Only ready nodes are claimable. Losing the CAS returns the current state (`claimed` by the other head); the losing head reads the queue for the next ready node, this is work-stealing and self-balances when units are uneven.
- `refine_task_node(node_id, children[], discovered_file_scope)`: a claimed node may split into children, declare the `file_scope` just discovered, and spawn its own verify child. Refinement is itself recorded as a node event so it replays. A coarse node ("ship browse_for_me") becomes concrete children as the head learns the shape.
- The coarse collision (duplicate crate) is killed by the CAS alone: "create the crate" is one claimable node, only one head can own it, the other immediately sees `claimed` and routes elsewhere. No file lock is needed for this.

## Write arbitration (resolves the Codex/Claude Code disagreement)

The disagreement: Codex proposed file tokens (claim a node, lock its files); Claude Code argued for optimistic concurrency with a patch sequencer. Resolution, on the merits: file tokens are the wrong primitive for this codebase, and the build is optimistic, with `file_scope` kept as a routing hint rather than a lock.

Rationale: the real work is cross-cutting. This arc, the robots fix touched browser_engine.rs and fetch_cascade.rs; the browse loop and the playbooks both touched router.rs in different functions. Big files (router.rs, lib.rs) are touched by many tasks. A file lock serializes any two nodes that share a file even when they never touch the same lines, which is the common case here. The CAS already prevents the coarse collision the file lock was meant to stop, so a lock adds false serialization while solving a problem CAS already handles.

The model:
- Heads commit to a shared base. The substrate serializes the apply step only (an apply queue, one node's patch applied at a time), and git's own three-way merge detects true line-level conflicts. Two nodes touching different functions in router.rs both apply cleanly; you pay only on genuine line overlap.
- On a true conflict, the later node is marked `rejected` with the conflicting hunk in its receipt and re-opened against the new base; its owning head redoes that one node. No speculative rebase engine, the cost is bounded to rare real conflicts, not the common cross-cutting case.
- `file_scope` survives as Codex intended that scope matters, but as a soft signal: the scheduler avoids handing two heads ready nodes with overlapping `file_scope` when other ready work exists, which makes true conflicts rarer still. Declared-and-discovered at refine time, used for routing, never as a hard lock.

This is the genuinely new and hard layer (an apply sequencer with git conflict detection); everything else extends existing code.

## The ambient verify loop (the peer-review skill collapses into the run)

Verification is not a scheduled skill; it is what an idle head does. When a head finishes a node, or loses a claim CAS, or has no ready implementation node, its default behavior is to claim a `verify` node over the other head's `patch_proposed` work. Because completion of any node auto-creates a sibling `verify` node assigned to the other head (`review_required_by`), review is continuous and ambient. The peer-review skill becomes unneeded, the heads are essentially always reviewing each other as their idle state. This is the existing critique role made continuous and claimable.

Teeth (or verify decays to LGTM and you lose the only reason two heads beat two of the same): a `verify` node's receipt must be a falsification attempt, what was checked, what was tried that could have failed, what evidence was produced. A node is `accepted` only when its verify node is accepted. Fitness rewards defect discovery, a verify that finds a real defect is a high-value outcome, not a delay, so the incentive is to break the patch, not bless it. Verify reuses the consensus gate (`evaluate_publication`, `MIN_CONSENSUS_HEADS`) at node granularity.

## Heads are not interchangeable

Do not schedule heads as identical workers pulling the next ready node. This arc was a clean natural experiment: one head moved fast on implementation, the other caught the parity divergence and the write bug. Route node-types to head strengths, learned from receipts (which head's output for this node type survives verification), so the scheduler's head-to-node-type preference is itself a learned prior updated by node outcomes. Cold-start with a declared default; let receipts move it.

## Skills as motor programs (node-type to skill-pack binding)

Node types bind skill packs, the substrate hands a head the right cognitive apparatus for the unit of work, rather than the head remembering how to work. A `rust_contract_parity` node loads the rust-engineering pack plus the parity skill plus the fixture oracle; a `peer_refute_patch` node loads review heuristics and known substrate bugs; a `browser_use_surface` node loads the browser playbook and the action-risk vocabulary. This is Ensemble selection keyed on `node_type`, the existing pack selection, now triggered by the work graph. This is where the skill encoder meets the work graph.

## The substrate protocol (answers the thought-vector question)

Heads do not chat; they transact with the substrate. The load-bearing property is groundedness, not density: every shared statement is falsifiable, a claim is real because the CAS says so, a test passed because the receipt hashes prove it, a patch changed exactly these bytes. A dense latent vector is uncheckable and cannot hold a claim or prove a test, so denser language is not the goal, more grounded language is.

Channels, in authority order:
- Authority: typed graph facts, CAS claims, receipts, decisions. The source of truth.
- Patch: diffs against a base, affected files, apply status.
- Proof: tests, logs, hashes, runtime receipts.
- Semantic: embeddings over packets and code for retrieval and routing only, the vector says "this resembles the prior parity bug," it does not decide truth.
- Human render: a compact operational notation (low-entropy, human and LLM readable) generated from the typed facts, never the source of truth.

No shared raw-latent channel exists today (hosted Claude and Codex APIs do not expose activations); if local-model heads later expose activations, treat them as a routing sidecar, not an authority. The typed fact is the truth; the compact render and the embeddings are projections on top.

## Receipts as fitness (the encoder improves the harness skill)

Node receipts are the outcome-labeled training signal. Which `node_type` to skill-pack bindings led to accepted versus rejected nodes, and which head's output survived verification, feed the held-out gate and fitness. So the harness skill's motor-program bindings and the head-to-node-type routing are promoted and retired by real outcomes through the same encoder loop that improves the rust and design packs. Typing the harness skill means expressing it as node types plus their pack bindings plus the receipt contract, the typed form is this work-graph schema, so typing the harness skill and building the work graph are the same task. One promoter (node outcomes); the encoder updates the bindings.

## Reuse map (each new piece extends existing code)

- Run-level shared state extends `BindingMemoryScope` (shared scratchpad, memory zones, private work).
- Node transitions and the claim CAS extend the `apply_transition` guarded pattern plus `state_hash` hashing and idempotency keys.
- The verify node extends `HeadInvocationKind::Critique` and the `evaluate_publication`/`MIN_CONSENSUS_HEADS` consensus gate.
- The receipt contract extends `OUTCOME.RECORDED`'s validator-evidence discipline and `GroundedClaim`.
- Per-node and per-head budget extends `budget.rs`.
- Replay and fork of a multi-head run extend `replay.rs` (the work graph rides the event ledger).
- Node-type pack binding extends Ensemble selection, keyed on `node_type`.

## Observable acceptance

- A run holds a TaskNode graph in the GraphStore; nodes have status, claim_owner, claim_epoch, and edges (prerequisite, refined-into, verifies).
- `claim_task_node` is an atomic compare-and-swap on claim_epoch: two heads racing the same node produce exactly one winner; the loser observes `claimed` and is handed a different ready node.
- A claimed node can refine into children that declare a discovered file_scope and spawn a verify child, and the refinement replays from the event ledger.
- Two nodes touching different functions in the same file both apply through the sequencer with no false block; two nodes touching the same lines produce one accepted and one rejected-and-reopened against the new base.
- Completing a node auto-creates a verify node assigned to the other head; a node reaches `accepted` only after its verify node is accepted; a verify receipt records a falsification attempt.
- A `node_type` resolves to a skill-pack selection through Ensemble.
- Node receipts are queryable as outcome-labeled signal for the encoder (which bindings and which head produced accepted versus rejected nodes).
- A multi-head run replays and forks through the existing replay path.

## Open

- Read `agent_binding.rs` in full before implementing the shared-state extension; this handoff is built from its public surface plus the deliberation loop, and the internal consensus and budget mechanics should be confirmed so the node layer composes with them rather than around them.
- The apply sequencer's conflict path (git three-way merge plus reopen) is the one component with no existing analog; build it minimal (serialize apply, detect, reopen one node) and resist growing it into a speculative rebase engine.
- Whether `verify` over a rejected-and-reopened node reuses the same verify node or spawns a fresh one (epoch the verify node alongside its target).
