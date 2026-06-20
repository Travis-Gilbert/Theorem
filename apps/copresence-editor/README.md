# copresence-editor — edit with Gemma over Velt

The **browser adapter** for the `theorem-copresence` substrate (handoff §12). A
Velt + Tiptap CRDT (Yjs) collaborative editor: humans co-edit with live presence,
and Gemma joins as a co-writer. Standalone web app (Vite + React + TS), NOT a
`rustyredcore_THG` cargo member.

Why this is cheap: Velt's collaborative editing is **Yjs**, and copresence text
regions are **yrs** (Rust Y-CRDT) whose update bytes are wire-compatible with Yjs.
The browser holds the Velt-synced `Y.Doc`; anything written into it syncs to every
peer. Plan + phases: [`docs/plans/copresence-velt-gemma/PLAN.md`](../../docs/plans/copresence-velt-gemma/PLAN.md).

## Run

```bash
cd apps/copresence-editor
cp .env.example .env        # then set VITE_VELT_API_KEY (already set in the local .env)
npm install
npm run dev                 # http://localhost:5173
```

Then, ONE-TIME in the Velt console (console.velt.dev): add `localhost:5173` (and any
deploy domain) under **Managed Domains** — the API key is client-side and domain-gated,
so collaboration silently fails until the domain is safelisted.

**Test collaboration:** open the URL in two browser profiles (one incognito). Each
gets a distinct dev identity; typing in one shows the text + a colored cursor in the
other.

## Ask Gemma (Phase 2)

The "Ask Gemma to continue / refine" buttons read the live document, call an
**OpenAI-compatible** chat endpoint, and insert Gemma's reply (which then syncs to
every peer via Velt). Configure the endpoint in `.env`:

```
VITE_GEMMA_BASE_URL=http://localhost:11434/v1   # agentd, Ollama, or hosted
VITE_GEMMA_MODEL=gemma2:9b
VITE_GEMMA_API_KEY=                              # optional bearer
```

It is base-URL-swappable on purpose: agentd's local Gemma loop, an Ollama
`ollama run gemma2`, or any hosted Gemma 12B all work. No endpoint configured ->
the button surfaces a clear error, the editor still collaborates.

## What's built vs planned

- **P1 (built):** Velt + Tiptap CRDT editor, presence, comments, dev identities.
- **P2 (built):** browser-driven Gemma over an OpenAI-compatible endpoint.
- **P3 (planned):** Gemma as an independent **headless substrate peer** — a thin Rust
  wire server over `theorem-copresence::SubstratePeer` (yrs region) bridged to the
  browser `Y.Doc`, with Gemma driving it through agentd and getting its own Velt
  presence + graph/memory. See the PLAN for the headless-bridge design.

## Not verified here

The scaffold has not been `npm install`-ed or browser-run in this environment
(no browser; Velt needs the domain safelisted in the user's console; Phase 2 needs
a running Gemma endpoint). The code follows Velt's documented v2 `useCollaboration`
API. Verify by running the steps above.
