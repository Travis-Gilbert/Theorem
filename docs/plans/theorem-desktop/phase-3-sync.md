# Theorem Desktop, phase three: local-to-hosted sync (job-003)

**Repo:** Travis-Gilbert/theorem
**Plan home:** docs/plans/theorem-desktop/
**Requires:** phase two (the local node) complete.
**Job linkage:** job-003, kind Feature, priority P1, target_head Either.

## Decision basis

Local and hosted stores converge through Prolly graph version packs. The versioned-graph machinery already exists and is already exposed on the MCP surface (graph_version_compile, diff, merge, checkout, log, ref), so this phase is sync policy over shipped mechanism, not new storage. Sync enabled is the tier B seam: tier A is local free, tier B is local plus hosted sync paid, tier C is hosted only. This phase wires the capability behind a config flag; billing is out of scope.

## Deliverables

### D1: sync engine in the node
A sync module in the desktop node that runs one round: compile the local graph to a version pack, pull the hosted head pack, three-way merge with the auto_confidence strategy, push the merged result to the hosted ref and apply it locally. Ride the existing version endpoints over the authenticated MCP client; build no new transport.

### D2: triggers
Sync runs on app launch, on a configurable interval, and from a manual control in settings. Each round records a receipt: packs exchanged, nodes and edges merged, conflicts and how they resolved. Receipts append to the node's status row from phase two; the latest receipt is visible in settings as text.

### D3: conflict posture
auto_confidence resolves; anything it cannot resolve is logged with both versions retained per the merge machinery, surfaced as a count in the receipt, never as a blocking dialog. No interactive merge UI in this phase.

### D4: tier seam
sync_enabled lives in config, default off. When off, the engine is dormant and the settings control explains the tier in one sentence. No payment integration.

## Acceptance criteria

1. A memory written locally appears in hosted recall after one sync round, and a memory written hosted appears in local recall after the next round.
2. Divergent edits to the same node on both sides produce a merge receipt showing the resolution, with no data loss (the losing version remains reachable through the version log).
3. Sync survives an interrupted round: re-running converges, no duplicate nodes, idempotent by content hash.
4. With sync_enabled off, zero version-pack traffic occurs.
5. A full round on a store of realistic size completes without blocking the UI thread.

## Fences

- No new storage engines, no new transport, no schema changes.
- No billing or account UI.
- No interactive conflict resolution surface.
- The standing no-graph-view fence holds.
