# Claude Code plugin resync is 4 layers; a hardcoded MCP path in ~/.claude.json is the silent killer

**Kind:** gotcha
**Captured:** 2026-06-08
**Session signature:** `claude:1travisgilbert@Theorem:cc-plugin-resync`
**Domain tags:** claude-code, plugins, marketplace, installed_plugins, claude-json, mcp, caching, theorems-harness

## Trigger (the scar)

After pushing the `theorems-harness` fixes and bumping versions, every resync
"didn't take" in a running Claude Code. I synced what I thought were the three
plugin layers (marketplace clone, version cache, `installed_plugins.json`) to
0.4.6, the user restarted, and I checked whether the plugin actually loaded. `ps`
was decisive: the substrate MCP server was still launching from
`/Users/.../.claude/plugins/cache/codex-marketplace/theorems-harness/0.4.5/mcp/rustyred-theorem-proxy.mjs`
(21 process refs to 0.4.5, zero to 0.4.6) and the newest one had started AFTER my
registry edit. Root cause: a **manually-added global MCP server** in
`~/.claude.json` at `/mcpServers/rustyred-thg/args[0]` hardcodes an absolute path
into a specific cache version dir. Plugin-cache and `installed_plugins.json` edits
never touch it, so it pins the version forever. That is the real reason resyncs
silently fail. Two more discoveries: (a) the running 0.4.5 already contained the
earlier hook fix, so the fix was live regardless of the version label; (b)
`~/.codex/plugins/.../0.4.6` processes existed — Codex keeps a SEPARATE plugin
cache under `~/.codex`, already on 0.4.6, independent of Claude's `~/.claude`.

## Rule

Claude Code loads a marketplace plugin through FOUR independent layers. A resync
takes effect only when all advance together:
1. marketplace clone `~/.claude/plugins/marketplaces/<mkt>/` — `git pull --ff-only`
2. version cache `~/.claude/plugins/cache/<mkt>/<plugin>/<ver>/` — rebuild from the clone (preserve `mcp/node_modules`)
3. registry `~/.claude/plugins/installed_plugins.json` — repoint the `<plugin>@<mkt>` entry
4. **hardcoded MCP pins in `~/.claude.json` `mcpServers[*].args`** — absolute paths into a specific cache version

For layer 4, prefer the SAFE move: overwrite whatever cache dir those args point
at with the latest content (works even while CC runs; takes effect on restart).
Do NOT hand-edit `~/.claude.json` while CC is running — CC rewrites that file live
(its mtime moves mid-session), so your edit gets clobbered. The clean one-time fix
(repoint those args at a version-stable path, e.g. a `current` symlink) must be
done with CC FULLY QUIT. Then full-quit + relaunch (not `/clear`). MCP servers
reconnect on their own, so new MCP tools appearing is NOT proof the plugin
version reloaded — verify with `ps`/launch paths. `sync-plugins.sh` is a different
lane (symlinks into `local-desktop-app-uploads`) and touches none of this.
`codex-plugins/resync-codex-plugin.sh` does all four layers in one command.

## Evidence

- `ps` + `lsof -d cwd`: substrate MCP (`rustyred-theorem-proxy.mjs`) launched from
  `.../theorems-harness/0.4.5/...`; plugin `server.mjs` ran from
  `~/.codex/plugins/.../0.4.6` (Codex's separate cache).
- `~/.claude.json` `/mcpServers/rustyred-thg/args[0]` = the hardcoded 0.4.5 path;
  it was the only theorems-harness version ref in that file.
- `installed_plugins.json` (my edit) said 0.4.6 yet the MCP ran 0.4.5 — proof the
  registry is not the operative lever for a hardcoded MCP pin.
- Running 0.4.5 cache had `blocker-scan` under `PostToolUse` (the fix was already in).
- Fix: overwrote the pinned dir with the latest content; hardened
  `resync-codex-plugin.sh` to reconcile layer 4. On re-run it auto-advanced to a
  freshly-pushed 0.4.7 (`b3ae1f9`) and staged all four layers.

## Encoded in

- `docs/learnings/2026-06-08-cc-plugin-resync-four-layers.md` (this file; supersedes the earlier "three-layers" draft)
- `codex-plugins/resync-codex-plugin.sh` (all four layers, layer-4 reconciliation included)
