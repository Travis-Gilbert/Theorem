# What is Theorem

Theorem is the Rust-native substrate spine for Theseus, an epistemic engine. Three claims define it.

## It is a substrate, not a pipeline

Most retrieval systems are pipelines: text goes in, an index is built, queries come out, and the index is a disposable artifact rebuilt on a schedule. Theorem inverts that. There is one graph store, and everything writes to it: parsed code symbols, crawled web pages, agent memory documents, harness run events, the outcomes of tool selections. Work accumulates as graph structure. The next run reads what the last run left.

This is why an epistemic engine built on Theorem gets better at whatever it is pointed at. Pointing it at a codebase grows a code graph that later code questions traverse. Pointing it at the web grows a quarantined corpus tier that later searches rank against. The improvement is not a model weight update; it is structure on the substrate.

## It is the Rust projection of a canonical system

Theseus is the canonical application: Python, Memgraph, Postgres. Theorem mirrors the parts of it that benefit from being Rust-native: the graph engine, the symbolic engines, the crawler, the harness kernel. The two are kept honest against each other. Native Rust engines must byte-match the Python reference receipts before a port is considered done, and a PyO3 bridge exports the Rust core to Python as a drop-in. A tool that proves itself on the Rust layer is a candidate for promotion into the canonical layer.

The discipline that keeps this from rotting is simple to state and hard to hold: surface drift, do not bury it. The Mirror Rule is the first thing the repository's internal map tells an agent to read.

## It is built for agents to share

Theorem assumes more than one agent will work the same code and the same graph, often at the same time. The Harness is the part of Theorem that makes that safe and cumulative: persistent memory across sessions, a coordination room, a typed and replayable event log per run, and a job board for handing work between heads. The next concept page covers it.

## The shape of the repository

The graph engine and the harness kernel live in the `rustyredcore_THG` Cargo workspace. Clients and servers that build on their own (the browser, the gRPC search server, the HTTP harness transport, the SDK bindings, the iOS and desktop clients, the Python mirror) live under `apps/`. The [crate reference](../reference/crates.md) and [app reference](../reference/apps.md) enumerate the full surface.
