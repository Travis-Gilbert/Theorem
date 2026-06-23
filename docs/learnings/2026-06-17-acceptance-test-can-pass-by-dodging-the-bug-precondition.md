# A test built to catch an ordering-dependent bug can pass for the wrong reason when its fixture ids dodge the bug's precondition; build the fixture to SATISFY the precondition with realistic values

**Kind:** gotcha
**Captured:** 2026-06-17
**Session signature:** `claude-code:travisgilbert (cuts 4+5 reconcile-with-codex / verifier)`
**Domain tags:** testing, false-negative, sort-order, btreeset, ppr, memory-adjacency, verifier

## Trigger

I wrote `cut5_acceptance::c3` to discriminate a real bug found mid-sprint in
`rustyred-thg-memory::memory_adjacency`: it accumulated the reverse anchor->member
`MEMORY_IN_PROJECT` edge with `adjacency.entry(anchor).push(...)` but then ended each
id's loop with `adjacency.insert(id, neighbors)` — so when the anchor's own (empty)
iteration ran, `insert(anchor, [])` clobbered the accumulated reverse edges and seeding
the anchor reached nobody. The bug's trigger is an ORDERING precondition: it only bites
when a member is visited BEFORE the anchor in the sorted `BTreeSet<String>` id loop.

My first fixture used member ids `mem:zin` / `mem:aout`. The test passed — but `'z' > 'p'`,
so `mem:zin` sorts AFTER the anchor `mem:project:theorem:alpha`, meaning the member's
reverse edge was pushed into `adjacency[anchor]` AFTER the anchor's `insert([])` and so
survived. The fixture could not reach the clobber. The green was meaningless. Real
production ids are `mem:doc:*` / `mem:node:*` (`'d'`,`'n' < 'p'`), which sort BEFORE the
anchor and DO hit it.

## Rule

When a test targets a bug whose trigger is a structural precondition (sort position, id
prefix, hash bucket, BTree iteration order), construct the fixture to SATISFY that
precondition with values shaped like production — not whatever is convenient. Then
sanity-check the discriminator by reasoning about WHY the broken path would fail this
exact fixture (e.g. "member sorts before anchor -> reverse edge clobbered -> tie ->
outsider wins on id tie-break"), not merely that the assertion is currently green. A
green from a fixture that cannot reach the bug is a false negative that ships as
confidence.

## Evidence

- `rustyredcore_THG/crates/rustyred-thg-memory/tests/cut5_acceptance.rs::c3_anchor_seed_lifts_member_at_equal_lexical`
  rewritten from `mem:zin`/`mem:aout` to `mem:doc:theorem:b-in`/`mem:doc:theorem:a-out`
  (both sort before `mem:project:*`, outsider sorts before the member so only a working
  anchor seed can flip the tie).
- `memory_adjacency` iterates `ids: &BTreeSet<String>`; the bug was `insert`-replace,
  the fix is `.entry(id).or_default().extend(neighbors)` (Codex's, landed in 21501c67).
- The hardened c3 still passes (fix is correct) and now fails loudly if anyone reverts
  `extend`->`insert`.
