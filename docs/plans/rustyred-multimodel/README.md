# RustyRed Multimodel Loop

This folder is the in-repo entrypoint for the RustyRed multi-model database
north-star loop. It exists so Codex, Claude Code, dispatch jobs, and harness
rooms can all reference the same artifact from inside the checkout.

## Source map

- `NORTH-STAR-RUSTYRED-MULTIMODEL.md`: the unit graph and acceptance map.
- `SPEC-GRAPHQL-MCP.md`: GraphQL MCP surface, including A2 coordination.
- `SPEC-EPISTEMIC-FACET-AND-MULTIVECTOR.md`: epistemics as a content-node facet,
  SQL/shadow as read-only projections, cold bodies, and ColPali-style
  multi-vector tiering.
- `SPEC-RUSTYRED-STREAM-COORDINATION.md`: append-only stream coordination.
- `E0-embedded-mode.md`: embedded-mode execution handoff.

## Loop contract

Use the Theorems Harness loop already available in this repo:

1. Observe the current tree, room state, and open mentions.
2. Pick a ready unit whose dependencies are landed.
3. Read the named spec before editing.
4. Announce the footprint in the harness room before touching shared seams.
5. Implement the unit to its acceptance criteria.
6. Validate against the narrow oracle for that unit.
7. Record the result through the harness, then take the next ready unit.

Do not start an autonomous loop over a dirty, unvalidated tree. First land or
explicitly defer the existing dirty lanes with path-scoped validation evidence.

## Claude coordination

Claude Code could not read the original `~/Downloads` map because its session
was confined to this checkout. Use this in-repo copy as the `spec_ref` for
Claude, dispatch, and future loop iterations.

Before asking Claude to implement a slice:

- Read `NORTH-STAR-RUSTYRED-MULTIMODEL.md`.
- Read the referenced execution spec if it exists in this folder or elsewhere in
  `docs/plans/`.
- Announce the files and concepts in the harness room.
- Keep concrete edits path-scoped, and do not start a new unit on top of an
  unverified dirty Rust slice.

## Spec register

Present in this folder:

- `SPEC-GRAPHQL-MCP.md`
- `SPEC-EPISTEMIC-FACET-AND-MULTIVECTOR.md`
- `SPEC-RUSTYRED-STREAM-COORDINATION.md`
- `E0-embedded-mode.md`

The North Star also references specs that still need to be copied or written
in-tree:

- `SPEC-RUSTYRED-RELATIONAL-CORE.md`
- `SPEC-RUSTYRED-DOCUMENT-TIER.md`
- `SPEC-RUSTYRED-PG-WIRE.md`
- `SPEC-MEMORY-FOUR-LAYER.md`
- `compound-engineering-corpora-backlog.md`
- `SPEC-SKILLOPT-BORROWS.md`

Only work a unit once its named spec is present and read.
