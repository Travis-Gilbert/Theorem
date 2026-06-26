# Run the Harness and submit a job

A worked example: start the Harness HTTP server, submit a dispatch job, and read it back. Every call here maps to a route in the [HTTP API reference](../reference/api-http.md).

## 1. Start the server

```bash
cd apps/theorem-harness-server
PORT=50080 THEOREM_HARNESS_DATA_DIR=harness-data cargo run
```

The server opens a durable file-backed store at `harness-data/` and listens on `0.0.0.0:50080`. The store is empty on first run, so list endpoints honestly return empty arrays until something is written.

Confirm it is up:

```bash
curl -s localhost:50080/healthz
# ok
```

## 2. Submit a job

A job is a typed unit of work for an executing head. `title` and `repo` are the only required fields; everything else has a default.

```bash
curl -s localhost:50080/harness/jobs \
  -H 'content-type: application/json' \
  -d '{
        "title": "Document the harness API",
        "tenant": "Travis-Gilbert",
        "repo": "Theorem",
        "priority": "P1",
        "target_head": "claude"
      }'
```

The response includes the minted `job_id`, whether it was newly `created`, whether it was mirrored to the Postgres dispatch queue, the full `job`, and the `wake_event` that was emitted:

```json
{
  "tenant": "Travis-Gilbert",
  "room_id": "repo:theorem:branch:main",
  "job_id": "job-01J...",
  "created": true,
  "dispatch_mirrored": false,
  "job": { "job_id": "job-01J...", "title": "Document the harness API", "repo": "Theorem", "priority": "P1", "target_head": "claude" },
  "wake_event": { "room_id": "repo:theorem:branch:main", "author": "theorem-harness-server", "urgency": "ask", "message": "dispatch job submitted: job-01J... (Document the harness API)", "delivery": "Wake" }
}
```

`dispatch_mirrored` is `false` unless you set `THEOREM_DISPATCH_DATABASE_URL` to a Postgres connection string before starting the server. With it set, the job is mirrored into the hot dispatch queue and `GET /harness/jobs/counts` reports state counts.

> Priority maps to coordination urgency: `P0` -> block, `P1` -> ask, `P2` -> info. `target_head` of `claude`, `codex`, or `either` decides which head(s) the wake event @mentions.

## 3. Read the room

Submitting a job writes a wake message into the room `repo:theorem:branch:main`. Read that room's records and intents:

```bash
curl -s 'localhost:50080/harness/rooms/repo:theorem:branch:main/records?tenant=Travis-Gilbert&limit=10'
curl -s 'localhost:50080/harness/rooms/repo:theorem:branch:main/intents?tenant=Travis-Gilbert'
```

## 4. Watch the room live

Open a Server-Sent Events stream and you will see each new message as it is written. The `tenant` query parameter is required here.

```bash
curl -N 'localhost:50080/harness/rooms/repo:theorem:branch:main/stream?tenant=Travis-Gilbert'
```

Leave that running, submit another job from a second terminal, and the wake event arrives on the stream.

## 5. Register an MCP connector (optional)

To make an external MCP server's tools available as learnable affordances:

```bash
curl -s localhost:50080/connectors/register \
  -H 'content-type: application/json' \
  -d '{
        "tenant": "Travis-Gilbert",
        "server_id": "my-tools",
        "target": { "type": "stdio", "command": "my-mcp-server", "args": [] }
      }'
```

The server spawns the target, performs the MCP handshake, lists its tools, and registers each as an affordance. List them back with `GET /connectors?tenant=Travis-Gilbert`.

For the built-in content extraction lane, register content-core without hand-writing the target:

```bash
curl -s localhost:50080/connectors/register/content-core \
  -H 'content-type: application/json' \
  -d '{ "tenant": "Travis-Gilbert" }'
```

That persists the `content-core` connector and exposes its extraction tools through the `content_extraction` affordance family.

## A reminder on auth

This server does not authenticate requests. The examples above work because the tenant is just a string. Before exposing the server beyond a trusted network, front it with an authenticating proxy — the write endpoints (`/harness/jobs`, `/connectors/register`, room messages) are otherwise open.
