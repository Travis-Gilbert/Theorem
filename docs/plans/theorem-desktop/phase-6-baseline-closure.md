# Theorem Desktop, phase six: baseline closure (job-006)

**Repo:** Travis-Gilbert/theorem
**Plan home:** docs/plans/theorem-desktop/
**Requires:** phases one and two complete.
**Job linkage:** job-006, kind Feature, priority P2, target_head Either.

## Decision basis

OpenHuman's feature set is the v1 baseline, ideas never code (GPLv3). This phase closes the functional gaps that matter for the substrate story and records deliberate decisions on the ones that do not. Voice and meeting presence are open product decisions, explicitly not assumed scope here; if wanted they become their own plan.

## Deliverables

### D1: per-turn cost accounting
Every rail turn records tokens in, tokens out, provider, model, and estimated USD on the turn's memory, computed from provider response metadata where available and from a maintained price table otherwise. A small per-turn cost line renders in the rail, and a running session total lives in settings. From 2026-06-15 subscription-billed claude -p draws the Agent SDK credit bucket; receiver job logs already carry the per-job line, and this deliverable gives chat the same visibility.

### D2: local model lane
Ollama as a first-class provider: endpoint in settings, model list fetched from the local daemon, works fully offline against the local node from phase two. Keyless local is the floor under the BYO-keys ceiling. Verify whether the workspace model-client already abstracts providers cleanly enough to add this as a variant; extend it rather than wrapping it.

### D3: background fetch
A scheduled background pass that refreshes known-context for the user's active domains and topics: re-run recall warmups and the fresh-signals acquisition hook on a configurable interval, writing receipts. Reuse the harness's existing scheduled-task lane if present in the workspace (verify before building); otherwise a thin timer in the node. Off by default; one toggle.

### D4: integrations decision record plus one proof
The integrations question (Composio-style OAuth catalog vs native MCP connectors) gets a one-page decision record in this directory. The standing architecture already treats MCP connector tools as learnable affordances, so the expected recommendation is MCP-first, but the record must argue it, including what a catalog buys and what it costs. Proof deliverable: one real connector wired end to end through affordance registration and usable from the rail. Full catalog parity is explicitly not this job.

## Acceptance criteria

1. A rail turn against a paid provider shows a cost line, and the session total in settings matches the sum of turns.
2. With networking disabled and Ollama running, the rail answers and memories land in the local node.
3. The background pass runs on its interval, writes receipts, and is silent when toggled off.
4. The integrations decision record exists, and the one proof connector works from the rail with its use recorded as an affordance outcome.
5. No OpenHuman source was read or ported; behavior parity only.

## Fences

- No voice, no mascot, no meeting agent in this phase; open decisions, not omissions.
- No OAuth catalog build.
- No new scheduled-task infrastructure if the harness lane exists.
- The standing no-graph-view fence holds.
