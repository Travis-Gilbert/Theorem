# Theorem

Theorem is the Rust-native substrate spine for Theseus, an epistemic engine. It carries the graph engine (RustyRed / THG), the harness that coordinates agents and persists their memory, a substrate-native browser, the RustyWeb crawler and search kernel, and the language bindings generated from one Rust core.

Two ideas hold the project together.

The first is the substrate. Everything Theorem does writes to and reads from one graph store: code symbols, crawled pages, memory documents, harness runs, tool-selection outcomes. The graph is not a cache bolted onto a model. It is the place work accumulates, so the system gets better at what it is pointed at because each run leaves structure the next run can use.

The second is the Harness. The Harness is how agents share that substrate. It gives a coding agent persistent memory across sessions, a coordination room for several agents working the same repository, and a typed event log for every run. The mental model is one agent with several heads, not several agents dividing a task. The heads share a scratchpad and announce what they are doing; they do not carve the work into rigid lanes.

This site documents Theorem for people building on it or evaluating it. For the internal navigation map that agents working in the repository read first, see `CLAUDE.md` at the repository root.

## Where to start

If you want the concept, read [What is Theorem](concepts/what-is-theorem.md), then [The Harness](concepts/the-harness.md).

If you want to build, read [Getting started](getting-started.md).

If you want the full surface, the [Crate reference](reference/crates.md) and [App reference](reference/apps.md) list every component.

## The Mirror Rule

Theorem mirrors a canonical workspace. Theseus, the Python plus Memgraph plus Postgres application, is canonical. Theorem is the Rust projection of it, free to move at its own cadence but kept auditable against the source through parity receipts and a PyO3 bridge. A good tool built on the Theorem (Rust) layer is a promotion candidate to the Theseus layer, not canon by default.
