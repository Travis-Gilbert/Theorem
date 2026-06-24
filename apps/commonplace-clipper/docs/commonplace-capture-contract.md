# CommonPlace Capture Contract

The CommonPlace save target posts one JSON object to the configured capture endpoint.
It is the same pushed capture shape used by the CommonPlace frontend.

```json
{
  "id": "local-<uuid>",
  "title": "Rendered clip title",
  "body": "Rendered markdown body, including frontmatter when the template emits it",
  "objectType": "source",
  "capturedAt": "2026-06-24T00:00:00.000Z",
  "captureMethod": "clipped",
  "status": "local",
  "sourceUrl": "https://example.com/page",
  "properties": {
    "author": "Example",
    "published": "2026-06-24"
  }
}
```

Field rules:

| Field | Rule |
|---|---|
| `id` | A local uuid with the `local-` prefix. The server may replace it with a slug. |
| `title` | The clipper note name, falling back to the source domain. |
| `body` | The rendered markdown from the upstream clipper pipeline. Highlight-only clips use the highlighted excerpt body. |
| `objectType` | `source` when a page URL is present, otherwise `note`. |
| `capturedAt` | ISO timestamp created at post time. |
| `captureMethod` | Always `clipped` for this extension path. |
| `status` | Always `local` at post time. |
| `sourceUrl` | The original page URL when available. |
| `properties` | The template property map. The same values are also present in the rendered markdown when frontmatter is enabled. |

The request uses `Content-Type: application/json`. When an API token is configured,
the extension sends `Authorization: Bearer <token>`.
