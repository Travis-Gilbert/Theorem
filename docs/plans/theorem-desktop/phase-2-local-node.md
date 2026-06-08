# Theorem Desktop, phase two: the local node (job-002)

**Repo:** Travis-Gilbert/theorem
**Plan home:** docs/plans/theorem-desktop/
**Requires:** phase one (the Dia shell, HANDOFF.md) complete.
**Job linkage:** job-002, kind Feature, priority P1, target_head Either.

## Decision basis

The desktop app is a local node of the harness: the RustyRed/THG substrate embeds in the app binary, a localhost MCP serves the same tool surface as the Railway deployment, and the dispatch receiver becomes a capability of this node (Option A from the receiver note). Once this phase lands, the app is the receiver host: no terminal daemon, no separate process to babysit.

## Deliverables

### D1: embedded node
Embed the rustyred-thg-server router in the Tauri Rust backend (the crates already live in this workspace). Mount it on localhost, port configurable with a default chosen at build time (verify no collision with common dev ports), loopback only. Storage under the platform app-data dir (macOS: ~/Library/Application Support/Theorem/store). The local store starts empty; sync is phase three.

### D2: hosted/local switch
A settings control selects which harness the chat rail's memory calls target: hosted (Railway, bearer plus tenant) or local (loopback, token optional). The active target is visible in settings. Switching does not migrate data; it changes the target.

### D3: receiver as a node capability
Embed the theorem_receiver library (it ships as lib plus bin) and drive run_loop on its own thread, exactly per the embed snippet in crates/theorem-receiver/README.md. Settings expose: receiver on/off, the repo-to-worktree map (editable), claim interval. The receiver capability always claims against the HOSTED queue regardless of the memory switch, because jobs are a cloud coordination concern. Lane detection on toggle-on; lanes shown in settings.

### D4: lifecycle and health
Node and receiver start/stop with the app cleanly. A small status row in settings shows: node up, store path, receiver state, last claim time, last job result. Text only.

## Acceptance criteria

1. With networking disabled, the app launches, the rail writes and recalls memories against the local node, and tools/list on the localhost MCP returns the same tool names as the hosted surface.
2. Flipping the memory switch changes where new memories land, verifiable by querying each store.
3. With the receiver toggled on and a Queued job in the hosted store, the app claims it and spawns the head in the mapped worktree, identical behavior to the standalone binary.
4. Quitting the app terminates node, receiver, and any health threads; no orphan processes.
5. The idle app with receiver on holds no engine state beyond the local node it already runs.

## Fences

- No graph view anywhere (the standing fence holds through every phase until the Dia rebuild is complete).
- No schema changes to the harness store.
- The standalone receiver binary stays buildable and untouched; the embed reuses the library, never forks it.
- Requires the Railway deployment to expose the job verbs (b6be2e4); verify before wiring D3, and if absent, note it in the room rather than building around it.
