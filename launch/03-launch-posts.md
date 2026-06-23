# Launch posts (WS3)

DRAFT for Travis to post, on the schedule in the pre-launch checklist: Hacker News
first, Tuesday through Thursday, morning Eastern, then Reddit and X. Cowork does not
post these. You do.

One narrative, three cuts. Each names the two differentiators (the live coworking
feel, the trust-aware memory) and none is a bulleted feature list. Placeholders:
`[INSTALL_COMMAND]`, `[REPO_URL]`, `[DEMO_LINK]`.

---

## The core narrative (the source the three cuts come from)

I kept re-explaining myself to my own agents.

Every new session started cold. I would paste the same context, re-state the same
decisions, and watch the agent confidently hand me back something I had already ruled
out two weeks earlier, because the thing it remembered had since been contradicted and
nothing told it so. The memory tools I tried did not help. They did similarity search
over a pile of text and returned whatever matched, with no sense of whether it was
still true. A stale fact and a current one looked identical to them.

And the agents never felt like they were working with me. They produced a file, I
read the file. I produced a file, they read the file. We were passing notes under a
door, not sitting at the same table.

So I built the table. Commonplace is a space my agents write into while I am in it
with them. When an agent saves a note or ingests a document, the space files it, links
it to what it relates to, and keeps it for next time, so I stop re-explaining. When
two things disagree, it keeps both and stops surfacing the one that got contradicted,
so the memory has a sense of standing and not just similarity. And it is live: I watch
structure form as the agent works instead of waiting for an artifact.

The install is one command. It hooks into the agents I already use, Claude Code,
Cursor, Cowork, Codex, and they start writing into the same space. Underneath is an
open multi-model database, RustyRed, so the store that remembers my work also ranks it
and tracks how it connects. The database is the part your agents should run on. The
space is the part you live in.

---

## Cut 1: Hacker News (Show HN)

Title (recommended):

    Show HN: Commonplace, agent memory that demotes stale and contradicted answers

Alternate titles:

    Show HN: A shared workspace your agents write into, that knows what to trust
    Show HN: One-command memory for Claude Code/Cursor/Codex that tracks standing

Body:

I built Commonplace because I was tired of re-explaining context to my own agents
every session, and tired of memory tools that hand back whatever matches a query with
no sense of whether it is still true.

Two things are different about it.

First, it is live coworking, not file storage. A person and an agent act on the same
items at the same time. You watch the structure form while the agent works instead of
asking it for a file and reading the file later.

Second, the memory tracks standing, not just similarity. Most memory layers rank by
vector match and stop there, so a contradicted or stale fact comes back looking as
trustworthy as a current one. Commonplace demotes information that has been
contradicted or has gone stale instead of returning it with confidence. A result you
saw last week can rank lower today because something newer disagreed with it.

It installs as an MCP server in one command and works with the agents you already use
(Claude Code, Cursor, Cowork, Codex). Underneath is an open multi-model database
(RustyRed) that holds graph, vector, document, and relational data in one engine, so
the same store that remembers items also ranks them and tracks how they relate.

It is early and I would rather hear where it breaks than where it shines. The capture
is explicit: it only stores what you send it through a tool call, and it does not read
your conversations. Install and repo: [REPO_URL]. Short demo: [DEMO_LINK]. Happy to go
deep on the ranking and the contradiction handling in the comments.

<!-- HN note: stay in the comments for the first few hours and answer technical
questions plainly. HN rewards specifics and candor about limits. Do not defend, just
answer. -->

---

## Cut 2: Reddit (journey post)

Suggested subreddits: r/LocalLLaMA, r/rust (RustyRed angle), r/ClaudeAI, r/programming.
Read each subreddit's self-promotion rule first. Title is a story, not a link.

Title:

    I got tired of re-explaining context to my agents every session, so I built a
    space they write into that organizes itself and knows what to trust

Body:

This started as a personal annoyance, not a product.

I use agents all day (Claude Code, Cursor, Codex). Two things kept grinding on me.
One, every session was amnesiac. I would re-paste the same context and re-state the
same decisions, every time. Two, when an agent did remember something, it had no idea
whether that something was still true. I would get back an answer I had already thrown
out, stated with full confidence, because the memory tool was doing similarity search
over text and a stale fact and a fresh one look the same to a vector.

I tried the usual memory add-ons. They are similarity search with a nicer wrapper.
They do not have a notion of an answer going out of date, and they do not have a
notion of two sources disagreeing. That was the gap I cared about.

The other thing that bugged me was how un-collaborative it all felt. The agent makes a
file, I read it. I make a file, it reads it. Nobody is ever working on the same thing
at the same time.

So I built Commonplace. It is a space my agents write into while I am in it with them.
When an agent saves a note or ingests a doc, the space files it, links it to related
items, and keeps it, so the next session is not cold. When two items disagree, it
keeps both and stops surfacing the contradicted one, so the memory has a sense of
standing instead of just match. And it is live, so I can watch the structure form
while the agent works.

It installs in one command as an MCP server and plugs into the agents I already use.
The thing underneath is an open multi-model database I have been building called
RustyRed, which is why the same store that holds the items can also rank them and
track how they connect.

It is early. I am posting this because the part I am proudest of (memory that demotes
stale and contradicted info) is exactly the part I want people to try to break. If you
want to poke at it: [REPO_URL]. Short clip of the auto-organize: [DEMO_LINK]. Honest
feedback welcome, including "this already exists, here," because I looked and did not
find the standing-aware part.

---

## Cut 3: X thread

1/
I kept re-explaining context to my own agents every session. And when they did
remember something, they had no idea it had gone stale. So I built the thing I wanted:
a space my agents write into that organizes itself and knows what to trust.

2/
The problem with agent memory today: it is similarity search in a trench coat. You
ask, it returns whatever matches, ranked by vector distance. A fact you disproved last
week looks exactly as trustworthy as one you confirmed this morning.

3/
Commonplace tracks standing, not just similarity. When two items disagree it keeps
both and stops surfacing the one that got contradicted. A result can rank lower today
than it did last week because something newer disagreed with it.

4/
The second thing: it feels like coworking, not storage. A person and an agent act on
the same live items at the same time. You watch structure form while the agent works.
No more passing files under a door.

5/
It installs in one command as an MCP server and works with the agents you already use:
Claude Code, Cursor, Cowork, Codex. They start writing into one shared space
automatically.

6/
Underneath is an open multi-model database, RustyRed: graph, vector, document, and
relational in one engine. The same store that remembers your work ranks it and tracks
how it connects. The database is the part your agents should run on.

7/
It is early and I want it stress-tested, especially the part that demotes stale and
contradicted answers. Install, repo, and a short demo here: [REPO_URL]

<!-- X note: post tweet 1 standalone first; it has to earn the read on its own. Reply
the rest as a thread. Attach the demo clip to tweet 1 or 4. -->

---

## Notes for Travis (not part of the posts)

- All three come from the one narrative above, as the brief asked. If you change the
  core story, the cuts should change with it.
- Each cut names both differentiators and avoids a feature-list format. The HN and
  Reddit cuts lead with the frustration; the X thread compresses it to one line.
- The "it is early, break it" framing is deliberate. It fits HN and Reddit norms and
  lowers the bar for the first wave of comments. Drop it if you want a more finished
  posture.
- Keep the capture-design line ("it does not read your conversations") in at least the
  HN post. It pre-empts the first privacy question and matches the policy.
