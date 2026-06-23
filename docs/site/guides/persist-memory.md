# Persist agent memory

The point of the Harness is that work accumulates. A coding agent that learns a constraint today should not rediscover it tomorrow. This guide shows the write-and-recall loop with the memory tools from the [MCP tool catalog](../reference/mcp-tools.md).

These are MCP tools — your agent (or MCP client) calls them. The argument objects below are what you pass as the tool's input.

## Write a memory

`remember` writes one typed memory document. Two fields are required: `kind` (what sort of memory this is) and `content` (the memory itself).

```json
// tool: remember
{
  "kind": "decision",
  "title": "Docs IA",
  "content": "Adopted the Diátaxis split for the GitBook: tutorials, how-to, reference, explanation.",
  "actor": "claude-code",
  "tags": ["docs", "architecture"]
}
```

`kind` is free-form and load-bearing for later filtering — common values are `decision`, `convention`, `constraint`, `note`. Optional fields include `summary`, `tags`, `links` (to other memory docs), and `metadata`.

## Recall it later

`recall` retrieves memory by relevance to a query, weighted by recency. In a new session, the same query surfaces what the last session wrote.

```json
// tool: recall
{ "query": "how is the documentation structured", "actor": "claude-code", "limit": 5 }
```

`query` is the only field you usually need. `limit` defaults to 10. You can narrow with `kind`, `since` (a timestamp), or `project_slug`. Recall ranks by graph relevance plus recency, so the most useful prior memory rises to the top rather than the most recent.

## Record an outcome with `encode`

When something *worked* or *failed*, record it as a signal, not just a note. `encode` attaches an outcome so the system can learn which approaches pay off.

```json
// tool: encode
{
  "kind": "solution",
  "content": "Bumping recursion_limit to 512 fixed the deep json! macro in the OpenAPI module.",
  "outcome": "positive",
  "actor": "claude-code",
  "tags": ["rust", "openapi"]
}
```

`kind` is one of `encode`, `feedback`, `solution`, `postmortem`; `outcome` is `positive`, `negative`, `mixed`, or `neutral`.

## Revise, archive, forget

Memory is not append-only noise — it has a lifecycle:

- `self_revise` replaces a memory and tracks the revision (use it when a decision changes).
- `self_archive` moves a memory to the on-disk cold tier (out of the hot working set, still recallable).
- `forget` soft-deletes with an audit reason.

## How it persists

Every write lands in the durable graph store as a typed node. Recall runs graph relevance (Personalized PageRank seeded by your query) plus a recency weight over those nodes, which is why memory survives restarts and why a new session inherits the old one's context instead of starting cold. The same store backs code symbols, crawled pages, and run events, so a memory can be related to the code or run it came from.

## Reading memory over HTTP

Memory tools are MCP tools, but durable memory documents are also readable over HTTP from the graph engine at `GET /v1/tenants/{tenant}/memory/docs` (see the [HTTP API](../reference/api-http.md)). The Obsidian-sync plugin uses exactly that endpoint to mirror memory into a vault.

> Unfamiliar term? The [Glossary](../reference/glossary.md) defines "cold tier," "graph store," and the rest.
