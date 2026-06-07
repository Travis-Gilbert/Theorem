# Theorem Harness Sync (Obsidian plugin)

Mirror your Theorem harness memory graph into an Obsidian vault as markdown notes,
and write note edits and `[[wikilinks]]` back into the graph. RustyRed stays
canonical; the vault is a working surface over it. Obsidian's own graph view and
links do the visualization, so there is no graph UI here.

Plan: `docs/plans/obsidian-sync/README.md`. This is the Phase 1 (pull) + Phase 2
(write-back) implementation.

## Why a device-side plugin

The harness runs on Railway; the vault is files on your Mac and phone. A Railway
server has no write access to those devices, so it cannot push files across the
network. The writer has to run where the vault is, which is exactly where this
plugin already runs. No delivery layer, no git repo, no cron, no Tailscale.

## What it does

- **Pull (Phase 1).** On a `Sync now` command or on a timer, it GETs the tenant's
  memory documents and writes or updates one note per doc. Filename is
  `slug(title)-<shortDocId>.md` (stable, collision-free, human-readable).
  Frontmatter carries the scalar fields and tags; the body is the doc content,
  followed by a generated links block rendering each outgoing link as a
  `[[wikilink]]`.
- **Write-back (Phase 2).** When you edit a synced note or create a new note in the
  capture scope, the plugin pushes it back. A note that carries a `doc_id` updates
  that document in place; a new note becomes a new document. Wikilinks become link
  targets the harness reconciles into `MEMORY_RELATES` edges. Note-linking is graph
  construction.

## Harness endpoints it uses

- `GET /v1/tenants/:tenant/memory/docs?since=<watermark>&include_inactive=<bool>` -
  the read endpoint. Returns each doc's scalar fields, `tags`, outgoing `links`
  (target doc_ids), and a server-computed `content_hash` used as the echo gate.
- `POST /mcp` calling the `upsert_note` MCP tool - create-or-update by stable
  `doc_id` plus link reconciliation (resolved links become edges, removed links are
  tombstoned, forward references are recorded and resolved on target creation).

Both are authenticated with the bearer token. The tenant comes from plugin
settings and is sent in the request path, so a token only ever touches its own
partition.

## Install (manual)

This plugin is not in the community store. Build it and drop it into your vault:

```bash
cd apps/obsidian-sync
npm install
npm run build        # produces main.js
```

Then copy `manifest.json` and `main.js` into
`<your-vault>/.obsidian/plugins/theorem-harness-sync/` and enable it in
Settings -> Community plugins. (On mobile, sync that plugin folder into the vault.)

For iterative development, `npm run dev` rebuilds on change; symlink the plugin
folder into a test vault.

## Configure

Settings -> Theorem Harness Sync:

- **Harness base URL**, **Bearer token**, **Tenant** - the connection. The token is
  scoped to your tenant; each user runs the plugin against their own tenant, which
  is how "users write to the graph" works for more than one person.
- **Sync folder** - where mirrored notes are written.
- **Include superseded / archived** - off keeps the vault to current notes.
- **Auto-sync interval** - minutes; 0 makes sync manual only.
- **Enable write-back** - off by default. Turn it on to push edits.
- **Capture folder** + **Capture flag** - which new notes write back: those inside
  the capture folder (defaults to the sync folder) OR those carrying the capture
  flag in frontmatter (`graph: true` by default). Arbitrary notes stay out of the
  graph, so the vault stays usable as a vault.
- **Default kind** - kind for a hand-written note that sets none. Set
  `kind: feedback` / `solution` / `postmortem` (with `outcome`/`signal` in
  frontmatter) when you mean a first-class encode.
- **Conflict resolution** - what to do when a note and its graph doc both changed.

## The echo problem and the three guards

Bidirectional sync loops if a graph-written note looks like a user edit and gets
pushed back. Three guards prevent it:

1. **Hash gate (primary).** The plugin records, per doc, the `content_hash` it last
   pulled and a local hash of the body it last wrote. A note whose body still
   matches what the graph wrote never pushes; a doc whose content matches the local
   note is not re-written.
2. **Remote-write suppression.** While the pull writes notes, the write-back path is
   suppressed for those paths, so graph-originated writes never echo back.
3. **Conflict surfacing.** If both sides changed since the last sync, the incoming
   graph version is written as a `... (graph conflict).md` sibling instead of
   clobbering your edit (configurable). The graph's versioned store is the safety
   net underneath.

## Dangling wikilinks

A `[[link]]` can point at a note that is not a graph node yet. The write-back sends
the link target as the note title; the harness records it as an unresolved forward
reference and creates the real edge when the target note is later created or synced
(it matches the unresolved entry against the new note's title or doc_id). This
mirrors Obsidian's own unresolved-link behavior; no node is created for the target
until it exists.

## Notes on the implementation vs. the plan

The plan was written ahead of the harness code; a few things differ from its letter
to match what the substrate actually supports (verified by reading the source):

- **`upsert_note`, not `self_revise`, drives edits.** The native `self_revise` mints
  a new doc_id and supersedes the old one, which would orphan a note's frontmatter
  id and spawn duplicates on every edit. `upsert_note` updates in place by stable
  doc_id (history is still kept by the versioned store underneath) and is also the
  only verb that can carry a changed link set and remove edges.
- **Edge type is `MEMORY_RELATES`** (the actual link edge), rendered as wikilinks.
- **`content_hash` is computed by the read endpoint**, not stored on the node.
- **Tenant is path-scoped**, carried from settings, since the harness auth maps a
  token to scopes, not to a tenant.

## Limitations / not yet covered

- Deleting a note in the vault does not delete the graph doc (no `forget` on delete).
- Renames are followed by doc_id, not by a server-side title change event.
- The unresolved-link match is by title or doc_id; renaming a dangling target before
  it is created can leave a stale forward reference until the next edit.
