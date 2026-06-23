# SPEC-9 Implementation Notes

## Confirm-Point Resolution

- **GraphQL endpoint:** use `apps/commonplace-api` as the CommonPlace desktop data contract on `http://127.0.0.1:17890/graphql`, with `x-api-key: dev-key` in Tauri. This intentionally supersedes the spec's named default of mounting the `rustyred-thg-mcp` schema on the local harness node, because the CommonPlace UI needs the full CommonPlace contract (`ask`, `briefing`, `discover`, collections, import/export) rather than the generic THG MCP GraphQL surface.
- **Frontend location:** consume the external `travisgilbert.me` CommonPlace frontend export instead of vendoring it into `apps/`. The Tauri shell is configured to use that repo's dev server at `http://localhost:3000` and static export directory at `../../../../travisgilbert.me/out`.
- **Primary surface:** the external Next.js CommonPlace app is the product surface. `apps/desktop/src` remains only a Vite command-contract/reference harness for the Tauri backend.
- **Desktop-local agents:** local model use is desktop-only. CommonPlace calls the Tauri `model_chat` command with a protocol, endpoint, and model; the default OpenAI-compatible endpoint is `http://127.0.0.1:8080/v1/chat/completions`, while Ollama-compatible agents use `http://127.0.0.1:11434`. Gemma is one local model choice, not a product-level special case.

## Current Split

- Codex owns Theorem-side code on branch `Travis-Gilbert/spec-9-commonplace-desktop-tauri`.
- Claude owns the frontend repo branch `spec-9-commonplace-desktop-frontend`.
- Full `tauri build` depends on the frontend repo producing a static `out/` export through `npm --prefix ../../../travisgilbert.me run build:desktop`.
