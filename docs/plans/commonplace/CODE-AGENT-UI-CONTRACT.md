# CommonPlace Code Agent UI Contract

## Ownership

- Assistant UI owns the Code page thread, message rendering, composer behavior, keyboard submit, disabled state, auto-scroll, and streaming state.
- The existing CommonPlace omnibar remains the command surface. In the Theorem preview route, `ComposerPrimitive.Root`, `ComposerPrimitive.Input`, and `ComposerPrimitive.Send` are mounted inside the shared omnibar chrome so the runtime can be tested without adding a second terminal or command button.
- The terminal is never a persistent Code page panel. Terminal execution is queued from the existing shell control and can surface later as an assistant message receipt or drawer.
- CodeMirror is reserved for file inspection and diff artifacts. It is not the agentic coding surface.

## Agent Transport

The omnibar is the only launch surface for Code agent turns.

- API agent mode posts to the local Next proxy at `/api/theorem/agent`.
- The proxy normalizes the upstream base URL to `/v1/theorem/agent/run` and attaches the server-side bearer token when `THEOREM_AGENT_API_TOKEN`, `THEOREM_API_TOKEN`, or `HARNESS_API_KEY` is present.
- ACP modes connect to `/v1/commonplace/acp/ws`, send `start_session`, then send `prompt` after `session_started`.
- ACP `file_write_review` events become collapsed CodeMirror diff artifacts under the assistant turn.
- ACP `command_approval` events stay receipts/approval cards. They do not mount a persistent terminal.

Supported omnibar transports:

| Transport | Target | Use |
| --- | --- | --- |
| `api` | `/api/theorem/agent` -> `/v1/theorem/agent/run` | Composed API agent |
| `acp:claude` | `/v1/commonplace/acp/ws` | Claude ACP session |
| `acp:codex` | `/v1/commonplace/acp/ws` | Codex ACP session |

## Message Artifacts

Assistant turns may attach collapsed artifacts in `message.metadata.custom.diffArtifacts`.

```ts
interface CodeDiffArtifact {
  id: string;
  title: string;
  path: string;
  before: string;
  after: string;
  additions: number;
  deletions: number;
}
```

The artifact renders as a collapsed row under the assistant message. Opening it mounts the CodeMirror diff viewer.

## Dot Matrix Model State

API model state is the source of truth for the dot matrix. The UI maps model states through `API_MODEL_DOT_MATRIX_CONTRACT` in `apps/harness-console/src/lib/commonplace/code-agent-contract.ts`.

| API state | Dot state | Terminal |
| --- | --- | --- |
| `idle` | `idle` | yes |
| `connecting` | `connecting` | no |
| `queued` | `waiting` | no |
| `routing` | `searching` | no |
| `thinking` | `thinking` | no |
| `searching` | `searching` | no |
| `reading` | `downloading` | no |
| `editing` | `syncing` | no |
| `streaming` | `streaming` | no |
| `waiting_for_tool` | `waiting` | no |
| `waiting_for_approval` | `paused` | no |
| `syncing` | `syncing` | no |
| `success` | `success` | yes |
| `warning` | `warning` | yes |
| `error` | `error` | yes |
| `paused` | `paused` | no |
| `stopped` | `stopped` | yes |
| `offline` | `offline` | yes |

`terminal: true` means the run is terminal for that model slot, not that a terminal panel should open.

## Background

The Code page background uses a deterministic Mersenne Twister 19937 binary glyph field. The field is visual only, anchored to the lower right, faint, and drifts diagonally upward without changing the layout.
