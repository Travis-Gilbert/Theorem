# Ensemble coordination affordances: default to coordinated execution heads, not lanes

Date: 2026-06-05
Status: PLAN. Companion to `docs/plans/ensemble/README.md`. Reshapes the plugin coordination surface mapped in `docs/plans/theorems-harness-plugin-switchover/tool-contract-matrix.md` so Claude Code and Codex default to co-working as two execution heads of one agent over the shared substrate, rather than carving file lanes.
Coordination: room `repo:theorem:branch:main`, tenant `default`. API commit from claude.ai, so `git fetch origin` to see it.

## The problem, precisely

Claude Code and Codex reported that the previous plugin layer pushed them to default to choosing lanes (strict file ownership) when they would be more effective as coordinated execution heads over a shared substrate. The cause is in the affordances. The most prominent coordination primitive is `coordination_intent`, framed as "claim a file before overlapping edits" (the switchover working agreement says exactly that). A primitive that frames coordination as staking out territory to avoid collision produces territory. Lanes are the rational response when the tooling treats collisions as the thing to prevent.

The substrate already supports the better mode. The coordination surface is rich: `coordinate`, `presence`, `coordination_room`, `coordination_context`, `coordination_record`, `mentions`, `read_intents_for_room`, `read_records_for_room`, `coordination_contribution`, `handoff`. What is missing is which moves are the cheap default and how the plugin frames the work. This plan changes the defaults and the framing; it does not add heavy new machinery.

## The reframe the affordances must embody

The plugin should present Claude Code and Codex as two execution heads of one agent working one goal over one shared state, not two actors who each need a job. Two actors with jobs carve territory; one agent with two hands does not. This is the same pattern as Ensemble (peers over a shared substrate, not conducted lanes) and the composed agent, applied to the two coding heads. Every change below serves that frame.

## What makes dropping lanes safe

Lanes exist to avoid merge pain. Remove the merge pain and the reason to lane evaporates. For source, the shared git worktree and git already absorb concurrent edits. For knowledge and decisions, the Prolly merge in `rustyred-thg-core/src/versioned_graph.rs` does. So intents become advisory, not locks: collisions are cheap to resolve by merge rather than expensive to prevent by territory. This is the precondition that lets the rest be safe.

## The affordance changes

### `coordination_intent`: from claim to focus signal

Today `coordination_intent` is the lane primitive, claim a file before editing. Reframe its semantics and its description. It becomes "I am focused here right now", an advisory broadcast for the other head's awareness, not an exclusive lock. The other head reads it through `read_intents_for_room` and complements rather than waits. Nothing about it blocks an edit.

A real claim is the exception, reached only when two heads are about to collide on a genuinely conflict-prone file (the standing example is the large `rustyred-thg-mcp/src/lib.rs`). Even then it is a soft, short-lived "I will take this slice now", not a territory grant, resolved by one head deferring rather than by a lock the runtime enforces. If both heads assert the same hot file at once, a deterministic tiebreak settles it without a negotiation round: the head already deepest in that file keeps it, otherwise the earliest intent timestamp wins.

### `coordination_context`: the turn-start default

The matrix already lists `coordination_context` as "one-call room packet for turn-start injection" and suggests wiring it into hooks. Make it automatic at the start of every head's turn. The packet gives each head, in one call, the shared task and frontier, the other head's current focus and intents, the recent records and decisions, and the latest contributions. This is the single highest-leverage change, because lanes are what heads do when watching each other is expensive. Make watching free and continuous and they self-organize moment to moment instead of dividing up front.

### A shared frontier object: one goal, pull-based micro-tasks

The missing structure is a single shared task and frontier both heads read and write, representing the one goal the agent is working and the set of open micro-tasks under it. Build it as a typed `coordination_record` (record_type `frontier` or `task`) held in the room, or as a thin shared-task node in the tenant graph, so it is one object both heads see and update, not per-head assignments. Heads pull the next useful micro-task from it by fit and availability, and post completion back. Pull-based selection over a shared frontier is what replaces pre-divided lanes: the work is the agent's, and either hand takes the next piece it is best placed to do.

### `coordination_contribution`: the end-of-slice default

The matrix lists `coordination_contribution` as a compact contribution receipt for end-of-slice reporting. Make posting one the natural close of every micro-task: what was done, what changed, what is now open. The other head sees it through the next context packet, so the shared state stays coherent without anyone owning a lane. Contributions plus the frontier object are how the agent keeps one coherent picture of its own progress across two hands.

### Live in-flight awareness

Turn-start context is necessary but not sufficient; heads also need to feel each other's in-flight work. Three existing channels carry it without new machinery: `presence` heartbeats that include the head's current focus and the files it is touching, the room push transport that streams intents and contributions live, and a mid-task re-pull of `coordination_context` when a head is about to enter a busy area. This is ambient awareness assembled from primitives that already exist. One dependency to call out: the live room push is the SSE path with the cross-tenant leak still open (`stream_room_handler` scoped on `room_id` only). The leak fix, scoping the subscription on `tenant_slug` and `room_id`, is a precondition for trusting the live stream, so it should land first.

### `subscribe`: fold

`subscribe` is already marked rename-or-fold; the gossip subscription is not the awareness path. The context packet, the room push, and presence are. Fold it.

## The framing the plugin hands the heads

Affordances set the cheap path; framing sets the intent. The plugin's agent-facing framing (the manifest description and the tool docs) should say plainly: you are two execution heads of one agent, working one goal over a shared substrate; the default is to co-work, pull the next piece from the shared frontier, broadcast your focus, read the context packet at turn start, post your contribution; you do not own files; claim a slice only when you are about to collide on a hot file, and resolve it by one of you deferring; the substrate keeps you coherent and merges concurrent work, so you do not need territory to stay out of each other's way. State the failure mode too, so they avoid the opposite error: do not both drive the same line; the goal is the same problem over shared state with dynamic micro-division, not lock-step.

## The art: loose but coherent

The target is not maximal coordination. Two heads on the same line is thrash. The target is enough shared live awareness that they naturally avoid colliding, plus one lightweight fast arbitration for genuine clashes, minus the heavy lanes. Loose enough to self-organize, coherent enough not to churn.

## What changes where

The semantic and description changes to `coordination_intent`, the turn-start wiring of `coordination_context`, the shared-frontier record, the end-of-slice `coordination_contribution` default, and the agent-facing framing all live in the plugin source (`server.mjs` and the manifest) and the route policy, against the native coordination MCP verbs the tool-contract-matrix already routes to. The substrate verbs exist; this is defaults, semantics, and framing, plus the SSE-leak fix as a precondition for the live stream. Keep coordination on the shared native MCP, never a per-plugin local binding, so both heads see the same room (the matrix is explicit on this).

## Confirm before building

Whether the shared frontier is best a typed `coordination_record` or a thin graph node. The exact tiebreak rule for simultaneous hot-file claims. The current state of the room push transport and the SSE-leak fix, since the live-awareness channel depends on it.
