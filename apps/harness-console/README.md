# Harness Console

The developer control surface for the Theorems Harness: a standalone Next.js (App Router) web console, target `harness.theoremsweb.com`. It is a **front door and a control surface**, not a place to sit and work: humans work inside their coding agent (Claude Code, Codex, Gemini); the harness is consumed over MCP. This console issues keys, programs the harnessed agent (Memory + Skills), and observes runs and rooms.

Built greenfield. It does **not** inherit Context-Theorem-UI. The only shared substrate is the harness backend (MCP tools, memory graph, connector gateway), which the console consumes and does not rebuild.

## Stack

- Next.js 16 (App Router) + React 19 + TypeScript (ESLint 9 flat config; Next 16 removed `next lint`)
- Tailwind v4 (CSS-first), tokens-first: the 4x4 design math + a shadcn semantic-token bridge (`@theme inline`) live in `src/app/globals.css`
- Radix primitives, hand-authored shadcn-style components (`src/components/ui`)
- `motion` (Dynamic Island), `@cosmos.gl/graph` (Lane A memory graph), `@xyflow/react` (Canvas), `d3` (declared for the planned Lane-B charts; not yet imported)
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

### Live-cutover gates (must be resolved before flipping `NEXT_PUBLIC_HARNESS_SOURCE=live`)

The console ships mock-default; the `live` client (`src/lib/harness/mcp.ts`) is the wiring surface, and these are deliberately deferred to the backend cutover:

- **Auth transport.** `EventSource` (the realtime stream in `useHarnessStream`) cannot set an `Authorization` header, and the MCP client's bearer-key parameter is plumbed but not yet sourced. Do NOT put the key in the URL (it leaks into logs/history). Authenticate via a same-site `HttpOnly` cookie, a Next route-handler proxy that injects the bearer server-side, or a short-lived stream ticket. Confirm the backend rejects unauthenticated tool/stream calls.
- **`saveAtom` fidelity.** The live `reviseMemory` mutation currently sends only `id/title/body`; the editor also edits `kind/summary/tags/links`. Either extend the mutation to accept those fields or narrow the optimistic return so the UI doesn't report success for fields the backend never received.
- **Search bindings.** Live `search` / graph-search (`fractal_expansion`, `web_search_graph`, `hippo_retrieve`) still read through the mock projection; wire them before claiming live search is feature-complete.
