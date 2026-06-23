# Privacy Policy

DRAFT for Travis to review and host at https://theoremsweb.com/privacy (mirror on
GitHub Pages). Not legal advice. Have someone qualified read it before you publish,
especially the storage and subprocessor sections, which depend on your final hosting.

Two values to fill before hosting: the effective date, and any server-side log
retention window (see Storage). One thing to verify in the code before hosting: the
capture is explicit-tool-call only (see the flag at the end).

---

## Privacy Policy for the Commonplace connector

Effective date: [DATE]

This policy explains what the Commonplace connector collects, how it uses that data,
where it stores it, whether it shares it, and how to reach the developer. The
connector is the Model Context Protocol (MCP) server that lets your agents (Claude
Code, Cursor, Cowork, Codex, and other MCP clients) read from and write to your
Commonplace workspace.

The developer is Travis Gilbert. Contact details are at the end.

### The short version

The connector collects only the data you or your agent send it through explicit tool
calls. It does not read your Claude conversations, your chat history, conversation
summaries, or files you upload to Claude. It does not collect any of that, even for
logging. What you save to Commonplace is what the connector stores, and you can
delete it.

### What data the connector collects

The connector collects only what an agent passes to it through an explicit tool call.
The tools are:

- `put_note`: the note title, text, and any tags you provide.
- `ingest`: the document title, text, and kind you provide.
- `get_item`, `list_items`, `search`: the item id, kind filter, or search query you
  provide. These read your existing data and return matches.

When you connect, the connector also handles the authentication details needed to
identify your workspace (your account or instance identifier and the access token
your client holds). That is the full list. The connector does not collect anything an
agent does not explicitly send through one of these tool calls.

The connector does not collect:

- Your Claude conversation content, chat history, or conversation summaries.
- Files you upload to Claude or to your agent.
- Background telemetry about what you type, browse, or run.

If the connector ever logs a request for reliability, the log records the tool name
and the outcome, not your conversation. It does not capture conversation data.

### How the connector uses the data

The connector uses the data you send it to do the one job you asked for: to store
what you save into your Commonplace workspace, to organize it (embed it, file it into
a collection, and link it to similar items), and to return it when you read or search.
It does not use your data to train models, to build advertising profiles, or for any
purpose other than running the workspace for you.

### Where and how long it stores data

Your items are stored in your Commonplace instance, the data store the connector
writes to on your behalf. They stay there until you delete them. Deleting an item
removes it from your workspace.

[If the hosted server keeps short-lived operational logs, state the window here, for
example: operational logs that record tool name and outcome are retained for [N] days
and then deleted. If there are no such logs, state that no request logs are kept.]

### Whether it shares data with third parties

The connector does not sell your data and does not share it with third parties for
their own use. The connector runs on infrastructure providers that process data only
to host the service (for example, the cloud host that runs the server and stores the
database). These providers act as processors under the developer's instruction; they
do not receive your data for any independent purpose. [List the hosting and database
providers here once final, for example the cloud platform that runs the server.]

The connector does not send your stored data to AI model providers. Your agent (such
as Claude) is your own client; the connector only responds to the tool calls your
client makes.

### How to contact the developer

For product questions and bug reports, open an issue on the public repository:
[REPO_URL]/issues. For privacy, security, or any direct concern, email
1travisgilbert@gmail.com. The developer aims to respond within a reasonable time.

### Changes to this policy

If this policy changes, the updated version will be posted at this URL with a new
effective date. Material changes will be noted at the top.

---

## Internal note (do not publish): the capture-design flag

The directory rejects connectors that collect conversation data. This policy states
explicit-tool-call capture. Before hosting, confirm in the capture code that:

- No hook reads the full conversation, chat history, or summaries.
- No uploaded files are read or stored unless the user passes them through an
  explicit tool call.
- Any logging records tool name and outcome only, not conversation content.

The MCP surface in `apps/commonplace-api/src/mcp.rs` exposes exactly five explicit
tools (`put_note`, `ingest`, `get_item`, `list_items`, `search`), which is consistent
with explicit-tool-call capture. The thing to verify is that nothing upstream of the
connector (a Cowork or client-side capture hook) vacuums the conversation. If
anything does, change it before submission and this policy stays accurate.
