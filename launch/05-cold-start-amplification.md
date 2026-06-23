# Cold-start and amplification list (WS5)

DRAFT for launch morning. Cowork drafts the list and the messages. Travis sends them.
A friend submits the awesome-list PRs (not Travis, so they read as third-party). Cowork
does not send, post, or submit anything here.

Directory URLs below were verified live on 2026-06-21. Re-check them at submission time;
directories move.

---

## 1. The velocity seed (20 to 50 people who star on launch morning)

I cannot fill this with real names; it is your network. Here is the structure to fill,
the categories that tend to convert, and a target of 30. Aim for 30 yes-leaning
contacts so 20-plus actually star in the first hours, which is what moves a Show HN or a
subreddit ranking.

Fill this table:

| # | Name | How you know them | Channel (DM/email/text) | Likely to star? | Sent? |
|---|------|-------------------|-------------------------|-----------------|-------|
| 1 |      |                   |                         |                 |       |
| 2 |      |                   |                         |                 |       |
| ... up to 30 | | | | | |

Categories to pull from, roughly in order of conversion:

- Close friends and former colleagues who will star because you asked. The reliable
  core. Aim for 10.
- People who already know Theseus, Theorem, or RustyRed and have engaged before. They
  get the wedge fastest. Aim for 8.
- Devs active in the communities the posts target: MCP, Rust, agent tooling, local LLM.
  People you have actually talked to, not cold strangers. Aim for 8.
- Anyone who has said "tell me when this is live." Aim for 4.

Timing: message them the night before or the morning of, not days early. Ask them to
star when you send the link, not "sometime." Velocity is the point.

## 2. Draft messages for the seed

Keep them personal and short. Three templates by closeness. Swap the specifics so they
do not read as a blast.

Close contact:

> Launching Commonplace this morning (the agent-memory thing I have been building). If
> you have 30 seconds, a star on launch would genuinely help it get seen: [REPO_URL].
> No pressure. Here is a 20-second demo if you are curious: [DEMO_LINK].

Peer or acquaintance:

> Hey [name], I am launching a project today and your read would mean a lot. It is
> Commonplace: a shared workspace your agents write into that organizes itself and
> demotes stale or contradicted info instead of surfacing it confidently. Live on HN
> now: [HN_LINK]. A star or a comment helps it surface: [REPO_URL].

Community contact (someone you know from MCP/Rust/agent circles):

> [name], you have poked at agent memory before, so I wanted you to see this. Commonplace
> tracks standing, not just similarity: contradicted or stale results get demoted. One-
> command MCP install, works with Claude Code/Cursor/Codex. Would love your honest take,
> including where it breaks: [REPO_URL].

## 3. Awesome-list PRs (prepare for a friend to submit)

A friend opens these so they are not self-submitted. For each, the entry text is ready;
confirm the exact category name and line format against the list's current README at PR
time, since these lists reorganize.

awesome-mcp-servers (canonical, ~60k stars):
- Repo: https://github.com/punkpeye/awesome-mcp-servers
- Place under the "Knowledge & Memory" section (or "Databases" if that fits better at
  PR time). Match the emoji legend the list uses for language and hosting.
- Entry:
  `- [commonplace](REPO_URL) 🦀 ☁️ - A shared workspace your agents write into that organizes itself and demotes contradicted or stale results instead of surfacing them confidently.`
- PR title: `Add Commonplace (agent memory with trust-aware ranking)`

awesome-rust:
- Repo: https://github.com/rust-unofficial/awesome-rust
- Place under "Database" (RustyRed is the multi-model database underneath). Read the
  list's contribution rules first; it is stricter than most.
- Entry:
  `- [RustyRed](REPO_URL) - A multi-model database (graph, vector, document, relational) in one engine, with trust-aware ranking.`
- PR title: `Add RustyRed multi-model database`

awesome-ai-agents:
- Repo: https://github.com/e2b-dev/awesome-ai-agents
- Place under the tools/memory section that fits at PR time.
- Entry:
  `- [Commonplace](REPO_URL) - Memory and workspace for agents that demotes stale and contradicted information; one-command MCP install for Claude Code, Cursor, Codex.`
- PR title: `Add Commonplace agent memory`

Note for the friend: one entry per PR, follow each repo's CONTRIBUTING file, keep the
description to one line, no marketing adjectives. These lists reject hype.

## 4. Other MCP directories to submit to

Verify each is live and grab the current submission method at submission time.

- Official MCP Registry: https://github.com/modelcontextprotocol/registry. Accepts a
  `server.json` record (PR or CLI publish). This is the feed many clients read; do it
  first. CLAUDE CODE can produce the `server.json` from the tool list.
- Smithery: https://smithery.ai. Submission for hosted/remote servers. Good fit once the
  remote HTTPS server is live.
- Glama: https://glama.ai/mcp/servers. Indexes daily; has a submit flow and pulls from
  GitHub. Listed as the most comprehensive registry.
- mcp.so: https://mcp.so. Community directory with a submit form.
- Optional, if time: PulseMCP and cursor.directory. Both index MCP servers and reach the
  Cursor crowd.

Sequence: official registry and Glama first (widest reach), then Smithery (needs the
remote server), then mcp.so. Most depend on the remote HTTPS server being live, so they
follow the WS4 release gate.

## 5. Repo Topics

Set these on the GitHub repo (GitHub caps at 20). They drive discovery on GitHub search
and the topic pages.

```
mcp, model-context-protocol, ai-agents, agent-memory, memory, llm, claude,
claude-code, cursor, codex, rust, database, multi-model-database, vector-database,
graph-database, knowledge-graph, rag, embeddings, coworking, developer-tools
```

---

## Notes for Travis (not part of the deliverables)

- The seed list is a fill-in, not invented contacts. Filling it is the one thing only
  you can do here; everything else is ready.
- The awesome-list entries lead with the trust-aware behavior and avoid adjectives,
  because those lists reject hype and a clean one-liner is what gets merged.
- Most directory submissions wait on the remote HTTPS MCP server (WS4 release gate), so
  this list runs in parallel with the seed and the posts but lands after the server.
