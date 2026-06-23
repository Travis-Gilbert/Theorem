# Theorem

Theorem is the Rust-native graph and harness layer for Theseus, an epistemic engine. It carries the graph engine (RustyRed / THG), the harness that coordinates agents and persists their memory, a graph-native browser, the RustyWeb crawler and search kernel, and the language bindings generated from one Rust core.

Two ideas hold the project together.

The first is the shared graph. Everything Theorem does writes to and reads from one graph store: code symbols, crawled pages, memory documents, harness runs, tool-selection outcomes. The graph is not a cache bolted onto a model. It is the place work accumulates, so the system gets better at what it is pointed at because each run leaves structure the next run can use.

The second is the Harness. The Harness is how agents share that graph. It gives a coding agent persistent memory across sessions, a coordination room for several agents working the same repository, and a typed event log for every run. The mental model is one agent with several heads, not several agents dividing a task. The heads share a scratchpad and announce what they are doing; they do not carve the work into rigid lanes.

This site documents Theorem for people building on it or evaluating it. For the internal navigation map that agents working in the repository read first, see `CLAUDE.md` at the repository root.

## Where to start

If you want the idea in plain terms with no vocabulary to learn first, read [The mental model](concepts/mental-model.md).

For the concept in depth, read [What is Theorem](concepts/what-is-theorem.md), then [The Harness](concepts/the-harness.md).

If you want to build, read [Getting started](getting-started.md), then the [HTTP API](reference/api-http.md), the [MCP tool catalog](reference/mcp-tools.md), or the [SDKs](reference/sdks.md).

If a term is unfamiliar — "head," "room," "affordance," "substrate" — the [Glossary](reference/glossary.md) defines every in-house word and gives the plain-language equivalent.

If you want the full surface, the [Crate reference](reference/crates.md) and [App reference](reference/apps.md) list every component.

## The Mirror Rule

Theorem mirrors a canonical workspace. Theseus, the Python plus Memgraph plus Postgres application, is canonical. Theorem is the Rust projection of it, free to move at its own cadence but kept auditable against the source through parity receipts and a PyO3 bridge. A good tool built on the Theorem (Rust) layer is a promotion candidate to the Theseus layer, not canon by default.
