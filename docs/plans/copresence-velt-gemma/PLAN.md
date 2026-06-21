# Edit-with-Gemma over Velt (the copresence browser + agent adapters)

This is the §12 follow-up to the verified `theorem-copresence` substrate (`crates/theorem-copresence`, landed `cf877fc`): the **browser adapter** + **agent adapter** that let a human and Gemma co-edit a live document. It is grounded in two confirmed facts:

- **Velt's collaborative-editing CRDT is Yjs** (`@veltdev/tiptap-crdt`, exposes `manager.getDoc(): Y.Doc`, `getAwareness()`, `getProvider()`). Presence/cursors come free.
- **copresence text regions are `yrs`** (Rust Y-CRDT), wire-compatible with Yjs update bytes (the handoff's whole reason for choosing yrs).

So the browser holds a Velt-synced `Y.Doc`; anything written into that doc syncs to every peer. Gemma becomes a peer by writing into the doc.

## Architecture

```
  Human browser                 Velt backend (Yjs sync)              Gemma
  ┌────────────────┐            ┌──────────────────┐
  │ Tiptap + Velt  │◄──────────►│  shared Y.Doc +   │
  │ useCollaboration│  presence  │  awareness        │
  │  manager.getDoc()│◄─────────►│  (multiplayer)    │
  └───────┬────────┘            └──────────────────┘
          │
   Phase 2│ browser reads Y.Doc text -> OpenAI-compatible call -> writes edit back
          ▼
     agentd / any OpenAI-compatible Gemma 12B endpoint  (VITE_GEMMA_BASE_URL)

   Phase 3 (substrate): Gemma drives a Rust copresence SubstratePeer (yrs region) bridged
   to the Y.Doc over a websocket; its own Velt presence; structure + memory through RustyRed.
```

## Phases

| Phase | What | Status |
|------|------|--------|
| **P1** | Velt + Tiptap CRDT editor (`apps/copresence-editor`): human↔human collaborative editing + presence, API-key wired. | scaffolded |
| **P2** | Browser-driven Gemma: an "Ask Gemma" action reads the `Y.Doc` text, calls an OpenAI-compatible Gemma 12B endpoint (agentd or any), inserts the result; the edit syncs to all peers via Velt. | scaffolded |
| **P3** | Gemma as an independent co-present peer: a Rust wire server over `theorem-copresence::SubstratePeer` (yrs region) bridged to the browser `Y.Doc`; Gemma drives the peer through agentd; structure (graph CRDT) + presence + memory flow through RustyRed. | planned |

## P3 open question (resolve before building it)

Can a **headless** (non-browser) peer join a Velt CRDT doc? Velt's client is browser-centric. Two routes:
- **(a) Browser-bridge-to-Rust:** the browser opens a websocket to a small Rust wire server wrapping a `SubstratePeer`; it relays `Y.Doc` update bytes <-> the peer's yrs region (both Y-CRDT, bytes are wire-compatible). Velt stays the human↔human transport; the Rust bridge is the human↔Gemma transport. Keeps Gemma fully headless (Rust + agentd). **Recommended.**
- **(b) Velt server/webhook/REST:** if Velt exposes server-side `Y.Doc` read/update, a Rust process syncs directly. Needs confirmation from Velt's REST/webhook docs.

The copresence crate is intentionally headless + networking-free (handoff §11), so the wire server is a NEW thin crate/app (`theorem-copresence-wire` or an `apps/` bin), not an edit to `theorem-copresence`.

## Runtime prerequisites (P1/P2)

1. Velt console: **safelist the dev domain** (`localhost:5173`) under Managed Domains (the API key is domain-gated, client-side).
2. P2 only: an **OpenAI-compatible Gemma 12B endpoint** at `VITE_GEMMA_BASE_URL` (agentd's model loop, Ollama `gemma`, or hosted). Decoupled so agentd's local-Gemma state is not a blocker.

## Coordination

P2/P3 touch agentd / Gemma serving, which is Codex's lane. The OpenAI-compatible call is the seam (base-URL-swappable). Announce the agentd endpoint contract in the room before wiring P3.
