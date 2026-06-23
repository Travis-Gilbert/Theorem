# Glossary

This page is the terminology contract for Theorem's documentation. It exists because the codebase grew faster than its vocabulary, and a few words ended up carrying several meanings at once. The worst offender is **substrate**, which appears over 1,700 times across the repository and means at least five different concrete things.

The rule for anyone writing user-facing docs: **pick the precise word from this page, not the in-house word.** Internal code, plans, and `CLAUDE.md` can keep their own vocabulary; the published docs a developer reads should not make them guess.

Each entry says what the term actually means, and whether to **use it**, **define it once then use it**, or **avoid it** in user-facing writing.

---

## "Substrate" — read this first

"Substrate" is not one concept. It is five, and a reader cannot tell which one a sentence means. In user-facing docs, never write "substrate" on its own. Say the specific thing:

| When the text means... | The plain term to use | Example rewrite |
|---|---|---|
| The graph database itself (nodes, edges, durability, indexes) | **the graph store** | ~~"writes to the substrate"~~ -> "writes to the graph store" |
| The in-process Rust engine where work runs | **the Rust engine** / **the engine** | ~~"substrate-resident compute"~~ -> "computation that runs in the engine" |
| A standardized record format fed to an engine | **fact pack** / **engine input** | ~~"substrate input"~~ -> "a fact pack" |
| The adapter boundary between two subsystems | **seam** (already used consistently in code) | "the page-to-graph **seam**" is fine |
| The overall design philosophy (one shared graph many agents reason over) | **the shared-graph model** | ~~"the cognition substrate"~~ -> "Theorem's shared-graph model" |

If you genuinely need the umbrella idea, write **"the shared graph"** or **"the shared-graph model."** Reserve the bare word "substrate" for internal documents only.

> Why we are not renaming the code. The crates (`rustyred-thg-core`, etc.) keep their names. Renaming 30 crates mid-development, while a second agent works the same tree, for a cosmetic gain, is a large risk for no functional benefit. The fix is a translation layer in the docs, not a churn of the source. Branding the crates is a deliberate, post-launch decision.

---

## Core product vocabulary (use these)

These are the words a developer building on Theorem's Harness needs. Define each once in a doc, then use it freely.

### Theorem
The Rust-native projection of a canonical system called Theseus. In product terms: the engine, the harness, the graph store, the crawler, and the SDKs in this repository. When talking to a customer, "Theorem" is the platform; **"Theorem's Harness"** is the product they consume.

### Theorem's Harness (the Harness)
The session-coordination runtime. It gives an AI agent three things a raw model call does not have: **memory that persists across sessions**, a **shared room** where several agents see each other's work, and a **typed, replayable log** of everything that happened in a run. The mental model is *one agent with several heads*, not several agents splitting a task.

### Head / multihead
A model or agent instance that can do work: Claude Code, Codex, the claude.ai surface. The Harness treats them as several **heads** of one unit that share awareness, rather than as rival agents. **Multihead** is the durable work graph that lets several heads claim, patch, and verify tasks in parallel. In user-facing copy, you can say "model" or "agent" for *head* on first contact, then introduce the term.

### Room (coordination room)
The shared workspace inside the Harness where heads announce what they are doing, post messages, record decisions, and read each other's presence. State lives in the graph store, so a room is durable and replayable, not an ephemeral chat.

### Memory
Durable, typed documents the Harness stores and retrieves: decisions, conventions, things ruled out, feedback. `remember` writes it; `recall` retrieves it by relevance. This is what lets the next session inherit context instead of rediscovering it.

### Graph store
The database under everything. One graph holds code symbols, crawled pages, memory, run events, and tool-selection outcomes, all traversable together. The interface is the `GraphStore` trait with three implementations (in-memory, durable file-backed, and over-the-network). This is the precise term for what older text calls "the substrate" in its database sense.

### Job / dispatch
A typed unit of work handed from a planning surface to an executing head. `job_submit` creates one; a receiver picks it up and runs the locally authenticated CLI. The **dispatch queue** is the hot execution state behind it.

### Projection / the Mirror Rule
**Theseus** (Python + Memgraph + Postgres) is the canonical system. **Theorem** is its Rust **projection** — a copy kept honest against the source through parity tests and a Python bridge. The **Mirror Rule** is the discipline: surface drift between the two, do not bury it. Keep these terms; they are precise and a developer evaluating the project will want to understand the relationship.

### Affordance
A capability an agent can invoke: a tool, an API call, a graph operation. Theorem makes affordances *learnable* — it records which ones get used, which succeed, and predicts which to offer next. Define it once ("an affordance is a tool or capability the system can use and learn from"), then "tool" and "capability" are fine synonyms.

### Seam
An adapter boundary between two subsystems, e.g. the page-to-graph seam that turns a loaded web page into graph state. Used consistently and precisely in the code. Keep it.

---

## Engine internals (know these, avoid in user-facing docs)

These are accurate engineering terms, but they read as jargon to someone evaluating the product. Translate them in published material; keep them in `CLAUDE.md`, plans, and code.

| Term | What it actually means | Say instead (user-facing) |
|---|---|---|
| **THG** | "Theorem HotGraph" — the namespace prefix on the graph crates (`rustyred-thg-*`) | "Theorem's graph engine"; never ship a bare "THG" |
| **RustyRed** | Internal codename for the Rust graph engine | "the graph engine" (or brand it deliberately) |
| **epistemic** | Relating to knowledge, confidence, or source reliability | "confidence-weighted," "knowledge-related" |
| **designate / designation** | Register a property for an index (vector, spatial, full-text) | "register for vector search," "index" |
| **reflexive** | The system learning from its own outputs | "self-improving," "adaptive" |
| **membrane** | The gate that ranks and filters candidates before they enter reasoning | "ranking gate," "candidate filter" |
| **spine / storage spine** | The tiered storage design (hot in memory, cold on disk, rehydrate on demand) | "tiered storage," "warm/cold storage" |
| **warm tier / cold tier** | In-memory working data vs. durable on-disk data | "in-memory tier" / "on-disk tier" (define once, then fine to use) |
| **organ / learned organ** | A self-improving specialized component | "adaptive module," "learned component" |
| **fractal expansion** | A retrieval strategy that starts broad then recursively refines promising branches | "progressive-refinement search" |
| **reflexive executor / pairformer / MPNN** | The learned ranking models that steer within bounded choices | "the learned ranker" (link to internals for the curious) |

The shape of the rule: a term that names a **precise contract a developer will call** (graph store, room, job, affordance, head) earns a glossary entry and gets used. A term that names an **internal mechanism** (membrane, spine, designate) gets translated to its effect.

---

## "Say this, not that" — quick card for doc authors

```
substrate (database sense) ........ graph store
substrate (compute sense) ......... the Rust engine
substrate (philosophy) ............ the shared-graph model
THG / rustyred-thg ................ Theorem's graph engine
epistemic neighbors ............... confidence-weighted related nodes
designate a vector ................ register a vector for search
reflexive ......................... self-improving / adaptive
membrane .......................... ranking gate
storage spine ..................... tiered storage
fractal expansion ................. progressive-refinement search
head .............................. model / agent (then: head)
```

When in doubt, write the sentence as if the reader has never opened the repository. If a word forces them to, replace it.
