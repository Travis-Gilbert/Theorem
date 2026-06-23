# Connectors Directory submission package (WS4)

DRAFT and assembly for Travis to submit through the public MCP directory submission
form (individual plan, so not the in-product portal). Cowork assembles this. Travis
submits. The review runs on a queue and returns in weeks, so submit early in the
sequence, not on launch day.

Items marked BLOCKED need an open value filled (see the index). Items marked
CLAUDE CODE are server-side work in the other lane. Everything else is drafted here.

---

## 1. Server and auth

- Server URL: `[SERVER_URL]` BLOCKED. Must be a remote HTTPS server on Streamable
  HTTP. The current `commonplace-mcp` is a local stdio binary, so the remote HTTPS
  endpoint is the release gate. CLAUDE CODE.
- Auth: `[OAUTH_DETAILS]` BLOCKED. Either an OAuth user-consent flow, or a custom
  connection where the user supplies their own URL or credentials. The directory needs
  one or the other named. The store already uses per-instance API keys
  (`x-api-key`), so the custom-connection scheme (user supplies their instance URL and
  key) is the lower-lift path if OAuth is not ready. CLAUDE CODE to confirm which.
- Endpoints are owned and controlled by the developer. Confirm at submission.

## 2. Tool list with titles and safety annotations

The MCP server exposes five tools. The current definitions in
`apps/commonplace-api/src/mcp.rs` carry a name and description but no `title` and no
annotations. Add the `annotations` block below to each tool before submitting. Missing
or wrong annotations are the second most common rejection.

| Tool | Title | readOnlyHint | destructiveHint | What it does |
|------|-------|--------------|-----------------|--------------|
| `get_item` | Get item | true | false | Fetch one item by id. Reads only. |
| `list_items` | List items | true | false | List items, optionally filtered by kind. Reads only. |
| `search` | Search workspace | true | false | Similarity search across saved items. Reads only. |
| `put_note` | Save note | false | true | Creates a new note item. Writes. |
| `ingest` | Ingest document | false | true | Creates a new item and auto-structures it. Writes. |

Drop-in annotation blocks:

```json
"get_item":    { "annotations": { "title": "Get item",        "readOnlyHint": true,  "openWorldHint": false } }
"list_items":  { "annotations": { "title": "List items",      "readOnlyHint": true,  "openWorldHint": false } }
"search":      { "annotations": { "title": "Search workspace", "readOnlyHint": true,  "openWorldHint": false } }
"put_note":    { "annotations": { "title": "Save note",        "readOnlyHint": false, "destructiveHint": true, "openWorldHint": false } }
"ingest":      { "annotations": { "title": "Ingest document",  "readOnlyHint": false, "destructiveHint": true, "openWorldHint": false } }
```

Annotation note: `put_note` and `ingest` are additive creates. They do not delete or
overwrite existing items. They are marked `destructiveHint: true` to match the
directory's stated rule (creates and writes get the destructive flag), which is the
safe side of a safety annotation. If the reviewer guidance distinguishes additive
creates from destructive overwrites, these can be softened to `destructiveHint: false`.
`openWorldHint: false` because no tool reaches an external system; every tool acts on
the user's own store.

If the GraphQL surface later exposes write tools over MCP (for example `editItem`,
which modifies an existing item), annotate that one `readOnlyHint: false,
destructiveHint: true` as well, since it overwrites.

## 3. Example prompts (work end to end through the tools)

1. "Save a note titled 'Stripe rate limits' with the text 'Stripe caps at 100 requests
   per second per key' and tag it stripe and limits." (put_note)
2. "Ingest this onboarding doc and tell me which existing items it relates to." (ingest)
3. "Search my workspace for everything about authentication and show the top five."
   (search)
4. "List every document I have saved." (list_items)
5. "What is our current stance on auth tokens? Show me, and flag anything that has been
   contradicted by something newer." (search, demonstrating the trust-aware demotion,
   the core differentiator)

## 4. Reviewer test account and access instructions

Cowork drafts the access instructions and the sample-data description. Travis (or the
server work) stands up the actual account and fills the credentials.

Access instructions for the reviewer:

1. Go to `[REVIEWER_TEST_URL]`.
2. Connect the Commonplace connector using `[OAUTH_DETAILS]`: either click Connect and
   approve the consent screen, or paste the instance URL `[SERVER_URL]` and the test
   API key `[REVIEWER_CREDENTIALS]` into the custom-connection fields.
3. The test workspace is pre-populated (see the sample data below). No setup needed.
4. Run the five example prompts above in order. Each maps to one tool.
5. To see the trust-aware behavior directly, run example prompt 5. The workspace
   contains a contradiction pair on purpose, so the contradicted item comes back
   demoted, not surfaced with confidence.

Sample data the test workspace should contain (small, real, and built to show the
differentiator):

- About 15 to 20 items, a mix of notes and ingested docs, on one coherent theme (for
  example, the engineering decisions of a small web service).
- At least one explicit contradiction pair, so the demotion is visible. Example: an
  earlier note "We authenticate with JWT bearer tokens" and a later, ingested doc "We
  moved off JWT to server-side session cookies in March." A search for the auth stance
  should rank the current one above the contradicted one.
- At least one cluster of three or four related items, so `ingest` of a new related doc
  visibly links into an existing neighborhood.
- A couple of stale items dated well in the past, so recency demotion has something to
  act on.

This data set lets a reviewer reach every tool and witness the one behavior that
makes the connector worth listing, in under five minutes.

## 5. Support channel and contact

- Product questions and bug reports: GitHub Issues at `[REPO_URL]/issues`.
- Privacy, security, and direct contact: 1travisgilbert@gmail.com.
- Both are listed in the privacy policy and the README, so the contact is consistent
  across the listing, the policy, and the repo. Verify the email is monitored before
  submitting.

## 6. How the connector works, purpose, and troubleshooting

Purpose: Commonplace gives a person's existing agents a shared workspace they write
into through explicit tool calls. It organizes what is saved, links related items, and
ranks search and memory by standing, demoting contradicted or stale information rather
than returning it with confidence.

How it works, briefly: the agent calls a tool (`put_note`, `ingest`, `get_item`,
`list_items`, `search`). Writes create items in the user's instance and auto-structure
them (embed, file, link). Reads return matches ranked with a sense of standing. The
connector stores only what is sent through a tool call and does not read conversation
data.

Troubleshooting:

- "Connector will not connect": confirm the instance URL is reachable over HTTPS and
  the API key (or OAuth consent) is valid. Keys are per-instance.
- "Search returns nothing": the workspace may be empty; save or ingest an item first.
  Search is similarity-based, so a brand-new workspace has nothing to match.
- "A result I expected is missing or ranked low": it may have been contradicted or
  gone stale. That is the trust-aware demotion working, not a bug. Fetch it directly
  with `get_item` to confirm it is still stored.
- "Errors instead of results": the connector returns a tool error with a message
  rather than a generic 500. Read the message; it names the missing argument or the
  store error.

## 7. Branding assets the listing needs

BLOCKED on assets. The listing typically needs:

- A square app icon (provide at 512x512 PNG, transparent background).
- A logo (horizontal lockup, SVG or high-res PNG).
- A listing image or screenshot (the auto-organize or coworking view; reuse the README
  demo frame).
- A one-line description (use the README subhead: "The database and memory your agents
  should run on. Not another agent workspace.").

Confirm the exact sizes against the submission form at submission time, since the spec
can change.

## 8. External link targets

No tool opens external links. Every tool acts on the user's own Commonplace instance.
Allowed link target URLs: none required. State this on the form.

---

## The hard gates, as a readiness checklist

Submit only when these are all true. This is the release gate, not paperwork.

- [ ] Remote HTTPS server on Streamable HTTP, live. CLAUDE CODE.
- [ ] OAuth user-consent flow with a reviewer test account, or a documented
      custom-connection scheme. CLAUDE CODE.
- [ ] Privacy policy live at https://theoremsweb.com/privacy, covering the five
      disclosures and the no-conversation-data constraint. TRAVIS hosts (draft in WS1).
- [ ] Capture verified explicit-tool-call, not conversation-vacuuming. VERIFY in code.
- [ ] Every tool annotated with a title and readOnlyHint or destructiveHint. Drafted
      in section 2; CLAUDE CODE adds to the tool defs.
- [ ] Reviewer test account populated with the sample data set, end-to-end access
      instructions filled in. Drafted in section 4.
- [ ] At least three working example prompts. Done, section 3.
- [ ] Graceful errors rather than generic 500s. The MCP layer returns tool errors with
      messages; confirm the HTTP/transport layer does too. CLAUDE CODE to confirm.
- [ ] Scoped, paginated results rather than huge payloads. `list_items` and `search`
      should cap and paginate; confirm. CLAUDE CODE.
- [ ] Owned or controlled endpoints. Confirm at submission.
- [ ] Verified support channel. Done, section 5; confirm the inbox is monitored.
- [ ] Every tool behaves when called, since reviewers run all of them and a compliance
      scan. Test with the reviewer account before submitting.
