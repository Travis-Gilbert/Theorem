# The mental model

Read this first if you are evaluating Theorem's Harness and want the idea in plain terms, with no vocabulary to learn up front.

## What it is

Theorem's Harness is a coordination layer for AI agents that gives them a shared, durable memory and a way to work together without stepping on each other.

A normal LLM call is stateless. You send a prompt, you get an answer, and the model forgets everything the moment it returns. If you run a second agent, it knows nothing about what the first one did. If you run the same agent tomorrow, it starts from zero again. For a one-off question that is fine. For real work — a coding agent on a large repository over days, or several agents on the same task — that forgetfulness is the whole problem.

The Harness fixes it with one durable graph that every agent reads from and writes to:

- **Memory persists.** When an agent learns a decision, a constraint, or a dead end, it records it. The next session and the next agent inherit it instead of rediscovering it.
- **Agents coordinate.** Several agents working the same repository share a room: each announces what it is doing and on which files, sees what the others are doing, and builds on a peer's finished work instead of redoing it.
- **Everything is replayable.** Every run is a typed log of steps with content-addressed checkpoints, so you can replay it, fork it, or audit exactly how an answer was reached.

## The one-agent-many-heads idea

The mental model the Harness is built around is *one agent with several heads*, not several independent agents dividing a task.

The heads — Claude Code, Codex, a browser session — run in isolation so they never overwrite each other's files. What unites them is not a shared working directory; it is shared *awareness*. A head announces its intent before it acts, names the files its hands are on, and reads the room to see what its peers have already done. The phrase for this is **frequency over fences**: frequent announcement beats rigid lane assignments, because lanes produce duplicate work and stale handoffs.

This matters in practice because it catches the one failure that both file isolation and text merges miss: two changes that merge cleanly and still disagree at runtime. When two heads' announced work touches structurally coupled code, the system raises a flag.

## Why a graph and not a vector index

Most retrieval systems are pipelines: text goes in, an index is built, queries come out, and the index is a disposable artifact you rebuild on a schedule.

Theorem inverts that. There is one graph, and everything writes to it — parsed code, crawled pages, agent memory, run events, the outcomes of which tools worked. Work *accumulates as structure*. The next run reads what the last run left behind. Point the system at a codebase and it grows a code graph that later questions traverse. Point it at the web and it grows a corpus that later searches rank against. The system gets better at whatever you point it at, and the improvement is durable structure you can inspect, not an opaque model update.

## How you consume it

The Harness exposes the same capabilities three ways, so you can use whichever fits your stack:

- **As an MCP server** — the primary path. An MCP-capable client (Claude Code, Codex, claude.ai) connects and gets tools for memory, coordination, jobs, graph queries, and browsing. See the [MCP tool catalog](../reference/mcp-tools.md).
- **As an HTTP API** — a JSON/HTTP server for reading runs and rooms, submitting jobs, and registering connectors. See the [HTTP API reference](../reference/api-http.md).
- **As a native SDK** — a Rust core with generated Node and Swift bindings, for embedding the harness directly in an application. See the [SDK reference](../reference/sdks.md).

## Who it is for

Developers building AI applications where four things matter: reasoning that is **auditable**, coordination between models that is **explicit**, knowledge that **persists** across sessions, and cost that stays **predictable** by sending hard sub-problems to classical computation (graph algorithms, logic, probability) instead of another model call.

## Where the names come from

Theorem is the Rust projection of a canonical Python system called **Theseus**; the relationship is explained in [What is Theorem](what-is-theorem.md). If you hit an unfamiliar word anywhere in these docs — "head," "room," "affordance," and especially "substrate" — the [Glossary](../reference/glossary.md) defines every term in one place and tells you the plain-language equivalent.
