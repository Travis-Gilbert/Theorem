# Harness Console

The developer control surface for the Theorems Harness: a standalone Next.js (App Router) web console, target `harness.theoremsweb.com`. It is a **front door and a control surface**, not a place to sit and work: humans work inside their coding agent (Claude Code, Codex, Gemini); the harness is consumed over MCP. This console issues keys, programs the harnessed agent (Memory + Skills), and observes runs and rooms.

Built greenfield. It does **not** inherit Context-Theorem-UI. The only shared substrate is the harness backend (MCP tools, memory graph, connector gateway), which the console consumes and does not rebuild.

## Stack

- Next.js 15 (App Router) + React 19 + TypeScript
- Tailwind v3, tokens-first (the 4x4 design math lives in `src/app/globals.css`)
- Radix primitives, hand-authored shadcn-style components (`src/components/ui`)
- `motion` (Dynamic Island), `@cosmos.gl/graph` (Lane A memory graph), `d3` (Lane B)
- CodeMirror 6 (`@uiw/react-codemirror`) for editors; Yjs + `y-codemirror.next` for the collaborative agent IDE
- `cmdk` (command palette), `sonner` (toasts), `react-dropzone` (ingestion)

## Run

```bash
npm install
npm run dev      # http://localhost:3000  (defaults to NEXT_PUBLIC_HARNESS_SOURCE=mock)
npm run build
npm run typecheck
```

Copy `.env.example` to `.env.local`. `NEXT_PUBLIC_HARNESS_SOURCE=mock` (default) renders every surface from deterministic local fixtures; `=live` routes the typed client (`src/lib/harness`) to the real harness over MCP/HTTP at `NEXT_PUBLIC_HARNESS_URL`.

## Architecture

- `src/lib/harness/` is the single typed client contract (`client.ts`), with two implementations: `mock.ts` (fixtures, the default) and `mcp.ts` (the JSON-RPC/GraphQL wiring surface). `index.ts` selects by env. Surfaces import `harness` and never touch a transport.
- `src/app/(console)/` are the authenticated surfaces inside the global shell (`src/components/shell`): Agent, Memory, Skills, Rooms, Runs, API Keys, Providers, Usage, Connections, Settings.
- `src/app/(onboarding)/` is the claim / first-run flow (modeled on Browser Use), outside the shell.
- The **Dynamic Island** (`src/components/island`) is the unified omnibar + TOC + RustyWeb search + Cmd-K command palette, bottom-center and permanent.
- The **depth system**: `DotGrid` ambient field (`src/components/depth`), the `.material` surface treatment, and a four-level elevation scale (`elev-0..3`), all in `globals.css`.

## The two-key model

- **Harness keys (inbound)** — pasted into Claude Code / Codex / Gemini so they can call the harness. Surface: API Keys. The tenant is baked into the key and resolved server side.
- **Provider keys (outbound)** — the Anthropic / DeepSeek / Mistral / OpenAI keys the harness uses to run the composed agent's heads. Surface: Providers. Stored as `credential_ref` references, resolved at run time.

## Backend dependencies (server-side, not in this repo)

Register endpoint (anonymous tenant + scoped key + claim URL), key issuance/resolution, per-tenant metering, the ingestion pipeline, the provider credential store, and the kind-filter on the memory list endpoint. The console names these as dependencies and calls them through the typed client.
