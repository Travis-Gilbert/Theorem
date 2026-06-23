# Theorem Desktop: HANDOFF (phase one: the Dia rebuild)

**Repo:** Travis-Gilbert/theorem
**Audience:** Claude Code + Codex, building as one agent
**Status:** ready to build
**Plan home:** docs/plans/theorem-desktop/
**Job linkage:** this document is job-001 per docs/plans/dispatch-queue/HANDOFF.md (kind App, priority P0, target_head Either).

## Decision record

- The desktop app is a LOCAL NODE of the harness. The RustyRed/THG substrate embeds as a crate; a localhost MCP serves the same tool surface as the Railway deployment; local-to-hosted sync runs over Prolly graph version packs. Tiers: A local free, B local plus hosted sync paid, C hosted only. The headless node is a severable spine, and the theorem-receiver (shipped, b6be2e4, head adapters 702f250) becomes a capability of this node in phase two.
- Design: rebuild Dia's UX anatomy and interaction model under Theorem's own visual identity. Copy the anatomy, the interaction model, and the calm chrome; do not copy trade dress. Hard fence: no graph view, no node or edge SERP, no Scene OS surface anywhere until the Dia rebuild is effectively complete.
- OpenHuman's feature set is the v1 baseline, ideas never code (GPLv3): study behavior, never read or port source. v1 means the full phase sequence below; this job builds phase one only, and the later phases are enumerated so nothing is silently dropped.
- Stack: Tauri v2 shell. General tabs run on wry/system webview in phase one. The direct Servo crate is reserved for agent ingestion surfaces in phase five. No Electron, no CEF or Chromium embed.

## What exists, reuse

- Hosted harness MCP on Railway; bearer plus tenant_slug client config per the 2026-06-05 auth fix.
- rustyredcore_THG crates in this workspace; phase two embeds the node from them.
- theorem-receiver with lanes as head adapters (702f250).
- iOS app interaction patterns (drawer nav, ask bar) for cross-surface consistency where natural, not binding.

## Phase one deliverables

### D1: scaffold
apps/desktop in the theorem workspace (verify placement against the workspace layout before scaffolding). Tauri v2, React plus TypeScript front, Rust commands. macOS on Apple Silicon is the primary target; keep Windows and Linux compiling but unpolished.

### D2: shell anatomy (the Dia core)
- One omnibox that routes by input shape: URL-like input navigates the active tab; anything else becomes a chat turn in the rail. Cmd+L focuses it.
- Left sidebar: vertical tabs, a pinned section, Spaces as named tab groups, drag to reorder.
- Right chat rail per window, toggleable, showing the conversation bound to the active tab.
- New-tab page is ask-first: focused input, recent tabs and Spaces below it.
- Calm chrome: native macOS traffic lights, standard menus only, Theorem tokens (PCB green #2a8b6c and brass #c9a23a as accents on neutral ground, IBM Plex Sans Condensed for UI type).

### D3: tabs
One wry webview per tab. Back, forward, reload, favicon and title, pinning. Session state (tabs, Spaces, pinned) persists in SQLite via tauri-plugin-sql and restores on launch. Window state is app state, not graph state.

### D4: chat rail wiring
- Context: the active tab's URL, title, selection, and extracted visible text (webview JS extraction) attach to the turn. Typing @ in the rail offers the other open tabs; mentioning one adds that tab's extracted content to the same turn. This is the signature interaction.
- Model: BYO provider keys in settings (Anthropic, OpenAI, DeepSeek), with DeepSeek as the keyless default per the standing roster decision. Verify whether a model-client crate already exists in the workspace before adding one.
- Memory: every rail turn and its page provenance is written to the harness via remember/encode over the hosted MCP. The rail surfaces recall hits for the active domain or topic as a compact known-context strip, text only, no graph rendering.

### D5: settings
Harness endpoint (hosted default, with a localhost field present for phase two), bearer token and tenant, provider keys stored in the OS keychain via the Tauri keychain plugin, Space management.

## Acceptance criteria

1. A dev build launches to the ask-first new-tab page on macOS.
2. Omnibox routing: "example.com" navigates; "what is example.com" opens a rail turn.
3. Tabs, pins, and Spaces persist across relaunch.
4. A rail question about the open page is answered with page content in context, verifiable by asking for a phrase that only appears on that page.
5. An @-mention of a second tab brings that tab's content into the same turn.
6. A rail turn produces a harness memory retrievable from claude.ai by its tags.
7. Revisiting a domain that has prior memories shows the known-context strip.
8. No graph, node, or edge visualization exists anywhere in the build.
9. Provider keys live in the keychain, never in config files or SQLite.

## Phases two through six (v1 is all of them; this job is phase one)

- P2 Local node: embed rustyred-thg, localhost MCP with the same tool surface, hosted/local switch in settings, and the receiver as a node capability (Option A) on the shipped head adapters.
- P3 Sync: Prolly version-pack local-to-hosted sync; the tier B billing seam.
- P4 Agent space surfaces: a Space binds to a coordination room; participants (heads, including the bound theorems agent) visible in the rail; job_submit from the omnibox.
- P5 Servo agent surfaces: ingestion tabs via WebViewDelegate interception, web_consume integration, konippi/servo-fetch evaluation.
- P6 Baseline closure against OpenHuman, ideas not code: integrations lane, background fetch, per-turn cost accounting, local model lane (Ollama). Voice and meeting presence are open product decisions, not assumed scope.

## Fences

- No graph view, node or edge SERP, or Scene OS in any phase-one code path.
- No OpenHuman source, ever (GPLv3).
- No Electron, no CEF.
- No GitHub Actions for anything in this app.
- Phase one makes no schema changes to the harness.

## Security

Bearer plus tenant on every harness call. Secrets in the OS keychain. Webview JS injection limited to text extraction. The app never logs page content.
