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
  memory documents and writes or updates one note per doc, into a **navigable**
  vault:
  - **Kind filter.** Graph-internal kinds (`community_summary` stubs, `orchestrate`
    coordination exhaust) are dropped client-side so they do not bury real memory.
  - **Folder by kind.** Each note lands in `<syncFolder>/<Kind>/` (Solutions,
    Decisions, Postmortems, Feedback, Revisions, Notes).
  - **Human filenames.** The filename is `slug(title).md`; identity lives in the
    frontmatter `doc_id`, so a title or kind change *renames or moves* the note
    instead of duplicating it. Collisions disambiguate with a short doc_id.
  - **Generated indexes.** A root `📍 Memory Map` plus one annotated `_<Kind>`
    index per folder are regenerated each sync (Map-of-Content pattern).
  - Frontmatter carries the scalar fields and tags; the body is the doc content,
    followed by a generated **Links** block (authored `[[wikilinks]]`) and, when the
    server surfaces them, a **Related** block (computed `MEMORY_SIMILAR` neighbors).
- **Write-back (Phase 2).** When you edit a synced note or create a new note in the
  capture scope, the plugin pushes it back. A note that carries a `doc_id` updates
  that document in place; a new note becomes a new document. Wikilinks become link
  targets the harness reconciles into `MEMORY_RELATES` edges. Note-linking is graph
  construction. Deleting a synced note tombstones its doc (status `deleted`) via the
  `forget` tool, so a following resync does not recreate the note.

## Harness endpoints it uses

- `GET /v1/tenants/:tenant/memory/docs?since=<watermark>&include_inactive=<bool>` -
  the read endpoint. Returns each doc's scalar fields, `tags`, outgoing `links`
  (target doc_ids), and a server-computed `content_hash` used as the echo gate.
- `POST /mcp` calling the `upsert_note` MCP tool - create-or-update by stable
  `doc_id` plus link reconciliation (resolved links become edges, removed links are
  tombstoned, forward references are recorded and resolved on target creation).
- `POST /mcp` calling the `forget` MCP tool - tombstone a doc when its note is
  deleted in the vault. The required arguments are `id` (the doc_id) and `reason`;
  tenant is sent the same way `upsert_note` sends it, so a delete and the write that
  created the doc always resolve to the same partition.

Both are authenticated with the bearer token. The tenant comes from plugin
settings and is sent in the request path, so a token only ever touches its own
partition.

## Install (manual)

This plugin is not in the community store, and it does not need to be: the
built `main.js` is committed beside `manifest.json`, so a checkout is already a
loadable plugin. (The whole reason a GitHub copy used to "silently not run" was
that `main.js` was gitignored, so there was nothing to load.)

Copy three files into `<your-vault>/.obsidian/plugins/theorem-harness-sync/`:

- `manifest.json`
- `main.js`
- (no `styles.css`; this plugin has none)

Then enable it in Settings -> Community plugins (reload the list first). On
mobile, sync that plugin folder into the vault.

If you changed the source, rebuild before copying:

```bash
cd apps/obsidian-sync
npm install          # first time only
npm run build        # produces main.js (production, no sourcemap)
```

For iterative development, `npm run dev` rebuilds on change; symlink the plugin
folder into a test vault. Do not commit a dev build: `npm run dev` writes an
inline sourcemap into `main.js`; only the `npm run build` output should be
committed.

## Install via BRAT (auto-updating)

[BRAT](https://github.com/TfTHacker/obsidian42-brat) installs a plugin straight
from a GitHub repo and keeps it updated, with no community-store review. BRAT
reads `manifest.json` at the repo's default-branch root and pulls the release
assets whose tag matches that version, so it needs the plugin to live at a
repo root, not in a monorepo subdirectory.

To use BRAT, publish this folder as its own public repo (it holds no secrets;
the token is entered by the user at runtime):

1. Create a public repo, e.g. `Travis-Gilbert/theorem-harness-sync`.
2. Copy this folder's `manifest.json`, `main.js`, `versions.json`, and source
   to its root.
3. Cut a GitHub release whose tag equals the manifest version (`0.2.0`) with
   `manifest.json` and `main.js` attached as assets.
4. In Obsidian: BRAT -> "Add beta plugin" -> the repo URL.

Until then, the committed `main.js` above is the in-repo install path, and the
optional Obsidian-Git mirror below keeps a checkout in sync on each device.

## Configure

Settings -> Theorem Harness Sync:

- **Harness base URL**, **Bearer token**, **Tenant** - the connection. The token is
  scoped to your tenant; each user runs the plugin against their own tenant, which
  is how "users write to the graph" works for more than one person. Tenant casing is
  significant against the deployed harness: write with the exact slug every time.
- **Test connection** - probes `/health` and the memory list endpoint and reports the
  doc count (plus a sample title) or the HTTP error. Use it to catch a silent
  misconfiguration before wondering why nothing syncs.
- **Sync folder** - where mirrored notes are written.
- **Include superseded / archived** - off keeps the vault to current notes.
- **Auto-sync interval** - minutes; 0 makes sync manual only.
- **Exclude kinds** - comma-separated kinds skipped on pull. Defaults to
  `orchestrate` because a tenant's feed is often dominated by agent-coordination
  exhaust (for one real tenant, ~185 of ~236 docs), which would otherwise bury
  the real memory (solution / feedback / postmortem / encode / decision / note).
  The read endpoint ignores a server-side kind filter, so this filtering happens
  client-side, before any note is written. Clear it to mirror every kind.
- **Only these kinds (allowlist)** - comma-separated; when set, only these kinds
  are pulled. Empty means "all kinds except the excluded ones". Exclude wins over
  the allowlist. Filtering changes the next pull only; it does not delete notes
  already synced for a now-excluded kind (delete the sync folder and Full resync
  for a clean slate).
- **Folder by kind** - on by default. Places each note in a per-kind subfolder.
  Off keeps one flat folder. Switching it on takes full effect on the next Full
  resync (notes move as their docs are re-seen).
- **Generate index notes** - on by default. Writes the root `📍 Memory Map` and
  the per-kind `_<Kind>` indexes each sync. They carry `theorem_generated: index`
  and never write back. The per-kind indexes render with no extra plugin; the root
  map's live tables need Dataview.
- **Memory map name** - basename of the root Map-of-Content note.
- **Enable write-back** - off by default. Turn it on to push edits.
- **Allow commons write-back** - off by default. While off, write-back refuses to push
  into the commons (`default`, or an empty tenant) so hand-written notes never land in
  the shared graph by accident; the first blocked push surfaces one notice and the
  settings tab shows a warning banner. Turn it on only if you really mean to write the
  commons.
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

## The graph view (and why there is no built-in galaxy)

The memory graph used to be *edgeless*: a few hand-authored wikilinks across
hundreds of notes, so the native graph view was a dust cloud and there were no
communities to color. The fix is two computed layers, not a bolted-on renderer:

1. **The substrate computes edges over memory.** A kNN-over-embeddings builder in
   the harness (`rustyred-thg-memory`) writes `MEMORY_SIMILAR` edges between
   semantically close docs. When the read endpoint surfaces them, the plugin
   renders them in each note's **Related** block, so Obsidian's *own* graph view
   clusters by meaning.
2. **Color by kind in the graph view.** Graph view -> Settings (gear) -> Groups,
   one query per kind on a viridis ramp:

   ```
   ["kind":"solution"]    #22a884
   ["kind":"decision"]    #7ad151
   ["kind":"postmortem"]  #fde725
   ["kind":"feedback"]    #414487
   ["kind":"encode"]      #2a788e
   ```

This is deliberately **not** a second cosmos.gl/WebGL galaxy inside the plugin.
The dense-semantic GPU galaxy lives in Theseus / Scene OS, fed by the same
`MEMORY_SIMILAR` edges; duplicating it here would be a weaker copy to maintain.
The plugin's job is sync + a navigable vault + feeding the native graph.

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

## Develop / test

```bash
npm run typecheck     # tsc over src/
npm test              # bundles test/*.test.ts (obsidian aliased to a stub) and runs node --test
```

The test bundle aliases the `obsidian` import to `test/obsidian-stub.ts` because the
real package ships only type declarations. `test/bundled/` is generated and ignored.

## Vault git mirror (optional)

To keep a versioned, diffable copy of the synced notes (the harness stays canonical;
the repo is a mirror, not the source of truth), install the Obsidian Git community
plugin and scope it to the sync folder. Put this `.gitignore` at the vault root so
only the synced folder and nothing personal is tracked:

```gitignore
# Ignore everything by default...
/*
# ...except the synced folder.
!/Theorem/
# Never track Obsidian's own state or this plugin's local data.
.obsidian/
```

(Replace `Theorem` with your sync folder if you changed it.) Point Obsidian Git at a
dedicated repo and let it auto-commit on an interval.

## Limitations / not yet covered

- A vault delete only tombstones a doc the plugin has a journal entry for (a note it
  pulled or pushed). Deleting a note the plugin never tracked is a no-op.
- Renames are followed by doc_id, not by a server-side title change event.
- The unresolved-link match is by title or doc_id; renaming a dangling target before
  it is created can leave a stale forward reference until the next edit.
