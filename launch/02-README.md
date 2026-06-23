# README draft (the product page)

DRAFT for Travis to place in the public repo. Two placeholders to fill: the install
command pointing at the bundled distribution, and the demo GIF. Voice follows the
theseus-copy rules (verb-led, own the trust attribute, no em dashes, defer the deep
proper nouns).

The block between the rulers below is the README itself. Everything outside the
rulers is a note to you.

---

# Commonplace

**The database and memory your agents should run on. Not another agent workspace.**

Install it once, and the agents you already use (Claude Code, Cursor, Cowork, Codex)
start writing their work into a shared space that organizes itself and knows which of
its own results to trust.

## What you do with it

You keep working with your agents the way you already do. Commonplace sits
underneath. When an agent saves a note, ingests a document, or looks something up, it
goes into one space that files it, links it to what it relates to, and remembers it
for next time. When two things disagree, the space keeps both and stops surfacing the
one that got contradicted. You stop re-explaining context to every new session, and
you stop trusting an answer that went stale a week ago.

![Commonplace auto-organizing an agent's work in real time](./[DEMO_GIF])

<!-- DEMO_GIF: the auto-organize clip (an agent ingests a few documents and they file
and link themselves) or the live coworking canvas (a person and an agent moving the
same items). Pick the one that reads in five seconds with no audio. -->

## Install

```
[INSTALL_COMMAND]
```

<!-- INSTALL_COMMAND points at the bundled distribution (the package or single binary
that includes the runtime), not a clone of this repo. Likely shape, to confirm with
the server work:
    claude mcp add commonplace [SERVER_URL]
or a one-line installer:
    curl -fsSL https://theoremsweb.com/install.sh | sh
The point of being first-class is that a person runs one command and is done. -->

That is the whole setup. Your agents can now read and write Commonplace.

## What makes it different

Two things, and neither is a feature you toggle.

**It feels like coworking, not storage.** A person and an agent act on the same live
items at the same time. You watch structure form while the agent works, instead of
asking it to produce a file and reading the file later.

**Its memory knows what to trust.** Most memory layers hand back whatever matches your
query, ranked by similarity, with no sense of whether it is still true. Commonplace
demotes information that has been contradicted or has gone stale instead of returning
it with confidence. A result you saw last week can come back lower today because
something newer disagreed with it. That is the property to lean on: the search and
memory carry a notion of standing, not just a notion of match.

## What your agents can do through it

Five tools, each doing one thing:

| Tool | What it does | Reads or writes |
|------|--------------|-----------------|
| `put_note` | Save a note with a title, text, and tags | writes |
| `ingest` | Drop in a document and let the space embed, file, and link it | writes |
| `get_item` | Fetch one item by id | reads |
| `list_items` | List items, optionally by kind | reads |
| `search` | Similarity search across everything saved | reads |

## What runs underneath

Commonplace is the space you work in. The substrate underneath it is RustyRed, an
open multi-model database that holds graph, vector, document, and relational data in
one engine, so the same store that remembers your items also ranks them and tracks how
they relate. The database is open. The workspace is the product.

## Privacy

The connector collects only what you send it through an explicit tool call. It does
not read your conversations, your chat history, or files you upload to your agent.
Full policy: https://theoremsweb.com/privacy.

## Links

- Documentation: [DOCS_URL]
- Privacy policy: https://theoremsweb.com/privacy
- Issues and support: [REPO_URL]/issues

---

## Notes for Travis (not part of the README)

- The headline is the positioning line from the brief, as requested. The line under
  it is the verb-led "what it is" so the page opens with an action, not a noun stack.
- One attribute is foregrounded on purpose: trust-aware memory. The coworking feel is
  the second beat, not a co-equal bullet list. Resist adding a five-feature grid; it
  flattens the one thing competitors cannot copy.
- "RustyRed," "epistemic," "THG," and "graph" are deferred to the bottom section by
  design. A cold reader should get the promise before the architecture.
- If you want a shorter top-of-repo README and a longer product page, this works as
  the product page; the repo README can be the first three sections plus install.
