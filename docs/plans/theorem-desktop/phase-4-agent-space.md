# Theorem Desktop, phase four: agent space surfaces (job-004)

**Repo:** Travis-Gilbert/theorem
**Plan home:** docs/plans/theorem-desktop/
**Requires:** phase one complete; phase two recommended (works against hosted alone if the node phase is in flight).
**Job linkage:** job-004, kind Feature, priority P1, target_head Either.

## Decision basis

The agent space is the product object: the room plus the work graph plus shared memory where multiple heads coexist. On the desktop, a Space (the tab group from phase one) binds to a coordination room. This is the lean-into-agent-space decision made explicit in UI, and it is the seam where the composed theorems agent (agent binding: many models, one agent) becomes visible to the user later without new chrome.

## Deliverables

### D1: Space-to-room binding
Creating a Space offers an optional "make this an agent space" step that starts or joins a coordination room and stores the room id on the Space. Existing Spaces can bind later from the Space's context menu. Unbound Spaces behave exactly as phase one shipped them.

### D2: room feed in the rail
When the active tab belongs to a bound Space, the rail gains a second view: the room feed (messages, intents, records, mentions), newest last, text only. Posting from the rail in that view calls coordinate as actor desktop with the Space's room id. The feed polls on the standing interval; SSE upgrade follows the push.rs tenant-scoping fix and is not this job.

### D3: participants strip
A compact strip above the feed shows room participants from presence: head names and their last-seen state. When agent binding ships its surface, the bound theorems agent appears here as one participant; this phase renders whatever presence returns and invents nothing.

### D4: jobs from the omnibox
A /job command in the omnibox: /job <title> | <spec path> submits to the hosted queue (kind defaulted by spec path shape, priority P1 unless given, target_head Either). A queue panel reachable from the sidebar lists jobs by status, plain rows: title, status, head, age. Submitting requires the job verbs live on the hosted deployment; if absent at build time, wire the panel read path through the graph query the receiver uses and note the gap in the room.

## Acceptance criteria

1. A Space bound on the desktop shows the same room feed that claude.ai sees, and a message posted from either surface appears on the other.
2. The participants strip reflects presence within one poll interval of a head joining or leaving.
3. /job from the omnibox produces a Queued job visible in the queue panel and in the store.
4. Unbound Spaces show no agent-space chrome at all.
5. No graph, node, or edge visualization anywhere; feeds and panels are text rows.

## Fences

- No new coordination primitives; this phase renders and calls what the harness already ships.
- No notification system, no badges beyond an unread count on the rail toggle.
- The standing no-graph-view fence holds.
