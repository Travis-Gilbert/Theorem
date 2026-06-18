# The graph store

The graph store is one graph — what the code and internal documents call "the substrate" (see the [Glossary](../reference/glossary.md)). The interface to it is the `GraphStore` trait, defined in `rustyred-thg-core`. Everything in Theorem reads and writes through that trait, which is why a page the browser ingests and a memory a harness session records land in the same place and can be traversed together.

## Three stores, one trait

The trait has three implementations, chosen by durability need.

`InMemoryGraphStore` is ephemeral and in-process. It is for tests and scratch work.

`RedCoreGraphStore` is durable, file-backed, and in-process, using an append-only file plus snapshots. This is the in-process graph store with no API boundary that the browser persists to. Writes delegate to AOF-backed durable upserts; reads serve from an in-memory mirror that is rebuilt from the AOF on open. `RedCoreOptions::default()` is `AofEverysec`; use `AofAlways` when you need fsync-per-commit determinism.

`RedisGraphStore` connects to a Redis-protocol RustyRed server. This is the out-of-process option, an API boundary.

One sharp edge worth knowing: `RedCoreGraphStore` and `RedisGraphStore` expose inherent methods (`get_node`, `upsert_node`) that shadow the trait methods. When you need the trait method on those types, call it through UFCS, `GraphStore::get_node(&store, id)`.

## Reactive compute on the graph

The core carries a graph-level hook primitive. A post-commit event can trigger localized compute, so structure stays warm as the graph changes rather than being recomputed in batch. Code-symbol centrality and embeddings warm incrementally on commit; crawl completion triggers source classification and entity extraction. The hooks are fail-open: a handler panic is caught so it cannot poison the store, and a loop guard bounds recursion depth.

## The symbolic engines

The core also holds the symbolic engines: Datalog-style forward chaining to derive facts from rules, and probabilistic reasoning for source reliability and the expected value of running a check. These are the neuro-symbolic layer behind belief revision and source-independence reasoning, and they are exposed as MCP tools.
