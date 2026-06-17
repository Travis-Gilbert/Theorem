---
title: "Theorem: a substrate that gets better at what you point it at"
author: Travis Gilbert
site: travisgilbert.me
status: draft
---

# Theorem: a substrate that gets better at what you point it at

Most systems that read for you throw away what they learned. You point a retrieval pipeline at a corpus, it builds an index, it answers a query, and the next time the corpus changes it rebuilds the index from scratch. The index is scaffolding. Nothing accumulates. The system is exactly as smart on its thousandth question as on its first.

Theorem is built on the opposite bet: that the most valuable thing a reading system produces is not the answer but the structure it leaves behind, and that structure should be the permanent part.

## One graph, everything in it

Theorem is the Rust-native spine of Theseus, an epistemic engine. The whole thing rests on one move. There is a single graph store, and everything writes to it. Parsed code symbols go in the graph. Crawled web pages go in the graph. An agent's memories go in the graph. The record of every task an agent runs goes in the graph. Even the outcomes of the system's own choices, which tool it reached for and whether that worked, go in the graph.

When everything lands in the same place, things that were separate become traversable together. The page a browser ingested sits next to the memory a session recorded, which sits next to the code symbol a parser found, and a query can walk from one to another. The graph is not a cache bolted onto a model. It is where work lives.

This is what people mean when they say Theseus gets better at whatever you point it at. Point it at a codebase and it grows a code graph that later questions about that code traverse. Point it at the web and it grows a corpus, kept in a lower-trust quarantined tier until it earns its place, that later searches rank against. The improvement is not a fine-tune. No weights move. The system is better next week because the substrate has more structure on it, and the structure was the point of doing the work at all.

There is a discipline this demands, and it is the hard part. A substrate that accumulates everything also accumulates mistakes, stale beliefs, and contradictions. So Theorem carries the machinery to revise: forward-chaining rules that derive new facts from what is known, source-reliability scoring, and a notion of productive forgetting. Belief that does not decay is just sediment. The substrate has to be able to change its mind.

## Why Rust, and the Mirror Rule

Theseus proper is a Python application with a graph database and a relational store behind it. That is the canonical system, the one that is allowed to be the truth. Theorem is its Rust projection: the parts that benefit from being native and fast, rewritten in Rust and kept honest against the original.

Kept honest is not a figure of speech. A Rust port of one of the symbolic engines is not considered done because it compiles and looks right. It is done when it byte-matches the Python reference output over a fixed set of cases. There is a bridge that exports the Rust core back into Python as a drop-in module, so the two halves can be swapped and compared rather than trusted. And a tool that proves itself on the Rust side is treated as a candidate for promotion into the canonical system, not as canon by default.

The rule that keeps this from rotting is short: surface drift, do not bury it. When the projection and the original disagree, that disagreement is information, and the worst thing you can do is paper over it. I wrote that rule into the first thing any agent reads when it opens the repository, because it is the convention most easily skipped under deadline, and the conventions most easily skipped are the ones that need to be written down hardest.

## The Harness: one agent, several heads

Here is the part that changed how I work.

I do not build Theorem with one assistant. I build it with several at once: Claude Code in a terminal, Codex in another, the chat surface for planning. The naive way to run that is to treat them as separate workers and divide the files between them. That fails. They duplicate each other, they hand off stale context, and two of them editing the same file lose each other's work, because a source file has no clean merge the way a database does.

The Harness is the answer I landed on, and the framing is the whole thing: this is not several agents dividing a task. It is one agent with several heads. One identity, one shared scratchpad, one budget. The heads are hands.

The hands still work in isolation, each in its own copy of the code, because that fence is real and removing it loses work. What crosses the fence is not a shared file. It is shared awareness. A head announces what it is about to do and which surface its hands are on, before it touches anything. It reads the room at the start of every turn. When it finishes, it closes its announcement as a handoff and writes a reflection so the next head can pick up cold. The slogan I kept coming back to is frequency over fences. You do not coordinate by carving rigid territory. You coordinate by announcing often enough that a peer can build on your last move instead of redoing it.

The Harness gives that model teeth. It carries persistent memory, so a decision made on Tuesday is inherited on Friday instead of re-litigated. It carries a coordination room with live intents and open tensions. It carries a semantic-overlap guard that watches for the one failure that both isolation and text-merge miss: two edits that merge cleanly and still disagree at runtime. And it carries a typed, replayable log of every run, so a session can be replayed or forked instead of reconstructed from memory.

Underneath, it is layered so the pure logic stays pure. A kernel holds run state and transitions and content-addressed hashes, with no storage in it, parity-tested against a reference. A runtime persists that kernel's output into the graph. An SDK sits on top, and it is the single source of truth: the Node, Swift, and browser bindings are generated from it, so no language binding can drift away from the core. That last property is the same instinct as the Mirror Rule, applied to SDKs instead of to Python. Generate from one source so the copies cannot lie.

## What this is for

The honest version of why this exists: I wanted a system that compounds. Not a chatbot that is brilliant and amnesiac, but a substrate where every hour of reading, crawling, coding, and deciding leaves something the next hour can stand on. And I wanted to be able to put several capable agents on the same hard problem without them tripping over each other, the way a small team of people can share a hard problem when they talk enough.

Theorem is the federated platform underneath that: the graph engine, the harness, the browser, the crawler, the bindings. Theseus is the epistemic engine it serves. The thing I keep testing it against is simple. Point it at something hard. Come back later. Is it better at that thing than it was, because of structure it kept rather than a model it called?

That is the whole bet, and the substrate is how you make it pay.

---

*Theorem is in active development. The internal architecture, crate by crate, is documented in the project's developer docs.*
