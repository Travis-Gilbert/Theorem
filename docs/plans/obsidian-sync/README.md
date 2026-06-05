# Obsidian sync: device-side pull-plugin + write-back to the graph

Date: 2026-06-05
Status: PLAN. Phase 1 (read endpoint + pull-plugin) greenlit; Phase 2 (write-back from Obsidian into the graph, multi-user) planned here. Build target: Claude Code (TypeScript plugin + Rust harness endpoints). Coordination room `repo:theorem:branch:main`, tenant `default`. API commit from claude.ai, so `git fetch origin` to see it.

## Shape

Obsidian becomes a read and write surface on each user's harness graph. A small Obsidian plugin runs inside Obsidian on each device, talks to the harness with the user's token, materializes every memory doc as a markdown note (Phase 1), and writes the user's note edits and new linked notes back into the graph as memory docs and `LINKS_TO` edges (Phase 2). RustyRed stays canonical; the vault is a working surface over it. Obsidian's own graph view and links do the visualization, so there is no graph UI to build.

## The crux this design is built around

The harness runs on Railway; the vault is files on the user's Mac and phone. A Railway server has no write access to those devices, so it cannot write into the vault across the network. The writer has to run where the vault is. That is why this is a device-side plugin and not a server job pushing files: the plugin is already on the device, so there is no delivery layer, no git repo, no cron, no Tailscale.

## Phase 1, harness side: a read endpoint

The plugin needs one tidy way to pull memory docs instead of scraping the graph node by node through `node_match`. `node_match` is exact-scalar only, so it cannot do a since-timestamp filter, and enumerate-then-fetch-neighbors per node is ugly even though it works at the current ~600 nodes. Add the list-since endpoint already flagged at `state.rs:1799` / `graph_store.rs`.

Shape: an authenticated GET that returns the tenant's memory docs, optionally since a timestamp, each doc carrying its scalar fields (`doc_id`, `kind`, `title`, `summary`, `content`, `content_hash`, `created_at`, `updated_at`, `status`), its `tags` list (tags is a list field on the node, present in the payload; it is not in the exact-scalar property index, which is why it cannot be `node_match`ed, but it is on the doc), and its outgoing `LINKS_TO` targets as doc_ids. Tenant comes from the bearer token, not a body field, so a user only ever reads their own partition. `since` lets the plugin pull incrementally once it has done a first full sync; without `since` it returns all, which is cheap at this size.

This is the only new server surface Phase 1 needs. Everything else is the plugin.

## Phase 1, Obsidian side: the pull-plugin

A standard community-style plugin: `manifest.json` with `isDesktopOnly: false` so it runs on mobile, bundled with esbuild into `main.js`, a settings tab holding the harness base URL, the bearer token, the tenant, and the target vault folder. HTTP goes through Obsidian's `requestUrl` so there is no CORS problem on desktop or mobile. Vault writes go through the Vault API (`create` and `modify`, `getAbstractFileByPath` to check existence).

The sync, on a sync-now command and on a periodic timer: GET the read endpoint, and for each doc write or update a note. Filename is a slug of the title plus the short `doc_id`, which is stable, collision-free, and human-readable. Frontmatter carries the scalar fields and the tags list. Body is the `content` field, followed by a links section rendering each `LINKS_TO` target as a `[[wikilink]]` to that target's note. Skip writing if the doc's `content_hash` matches what the plugin last wrote, which it keeps in a small per-doc_id last-synced-hash map in plugin data, so unchanged docs cost nothing. The file watcher (Phase 2) is suppressed while these writes happen so graph-originated writes never echo back.

## Phase 2: writing to the graph from Obsidian

This is the half that makes Obsidian a write surface, and for multiple users, not just a mirror. The promise: a user writing linked notes in Obsidian is building their graph. A note becomes a `MemoryDocument`; the `[[wikilinks]]` in it become `LINKS_TO` edges. Note-linking is graph construction.

### Which notes write back

Not every note in the vault. Two cases write back, nothing else does. A note that carries a `doc_id` in frontmatter (it came from the graph, or was pushed before) and whose body has changed is a round-trip update. A new note the user creates inside the designated capture scope (the sync folder, or a folder the settings name, or a frontmatter flag) is a new doc. Arbitrary notes the user never intended for the graph stay out of it, so the vault stays usable as a vault without everything leaking into the graph.

### The write path, on existing harness verbs

The harness already has the verbs; Phase 2 is mostly the plugin calling them, plus echo control. A new note with no `doc_id`: `encode` creates the memory doc and returns its `doc_id`; the plugin writes that `doc_id` back into the note's frontmatter so future edits round-trip as updates rather than duplicates. Use native `encode` so outcome, signal, fitness, and training metadata are first-class, not a generic `remember`. An edited note with a `doc_id`: `self_revise` updates the doc by `doc_id` (the native verb keeps `doc_id` and `docId` aliases). Wikilinks: `relate` upserts a `LINKS_TO` edge per `[[link]]`, `seed_id` being this note's doc and target being the linked note's doc, reconciling the set on each write so removing a link removes the edge.

A small convenience endpoint (`upsert_note`) that does encode-or-revise plus link reconciliation in one call would make the plugin simpler, but it is composable from `encode`, `self_revise`, and `relate`, so it is optional, not a blocker. `kind` comes from frontmatter; a plain hand-written note defaults to a generic kind, and the user can set feedback, solution, or postmortem in frontmatter when they mean it.

### Dangling wikilinks

A `[[link]]` can point at a note that is not a graph node yet, when the user links ahead of creating the target, exactly as Obsidian allows unresolved links. The write-back creates the `LINKS_TO` edge and either creates a stub `MemoryDocument` for the target or records the link as unresolved, then reconciles when the target note is created or synced. Mirror Obsidian's own unresolved-link behavior so the graph never rejects a forward reference.

### The echo problem, which is the real hazard

Bidirectional sync loops if a graph-written note looks like a user edit and gets pushed back, bumping the version, which re-syncs the note, and so on. Three guards together. The hash gate: the plugin only pushes a note if its current body diverges from the last-synced `content_hash` for that `doc_id`, so a note the graph just wrote matches that hash and never pushes. Watcher suppression: the plugin disables its vault watcher while it writes notes during a pull, then re-enables it. Conflict surfacing rather than clobber: if both sides changed since the last sync, the user edited the note and the graph doc also moved, write the incoming graph version as a conflict copy beside the user's note instead of overwriting, and let the user resolve. The graph's Prolly version history is the safety net underneath.

### Multi-user and tenant routing

"Users can write to the graph" means each user runs the plugin against their own tenant. The bearer token in the plugin settings is scoped to that user's tenant, so both the read endpoint and every write-back land in that user's partition and never in the shared `default` tenant. This is the tenant-bound-to-authenticated-identity decision realized at the Obsidian edge: identity, not a typed tenant name, decides the partition. The normal case is one user, one tenant, one vault. If one vault ever holds more than one tenant, Phase 2's per-tenant folders route each folder to its tenant; that is the only reason folders enter the picture.

## New vs reused

Phase 1 adds exactly one server endpoint, the read/list-since, plus the plugin. Phase 2 reuses `encode`, `self_revise`, and `relate` for the write path, adds the plugin's write-back logic and the three echo guards, and optionally adds the `upsert_note` convenience endpoint. No new heavy machinery, no graph UI, no model work.

## Build ownership and what to confirm

The Rust endpoints land in the harness: `rustyred-thg-server` plus the graph-store stats path at `state.rs:1799` for read; `encode` / `self_revise` / `relate` already exist for write. The TypeScript plugin is a new artifact, its own small repo or a plugin subdir. Both go to Claude Code since the plugin is TS to iterate on.

Confirm before building: the capture-scope rule for new notes (designated folder vs frontmatter flag vs both); the dangling-link policy (stub node vs recorded-unresolved); the conflict-surfacing behavior (conflict copy is the proposed default); and whether to add the `upsert_note` convenience endpoint or have the plugin call `encode` / `self_revise` / `relate` directly.
