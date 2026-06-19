# A background auto-commit-push hook swept the entire dirty tree (incl. another head's buggy WIP) to main while I was preparing a scoped pathspec commit

**Kind:** postmortem
**Captured:** 2026-06-19
**Session signature:** `claude-code:travisgilbert (PG-WIRE + DOCUMENT-TIER engine tiers)`
**Domain tags:** git, auto-commit-push-hook, main, pathspec, multi-agent, codex, scope-control

## Trigger

The working tree had ~40 dirty files across many lanes (my engine-tier work; Codex's relational-core/doc-tier; and Codex's **known-buggy mid-flight tenant-partition WIP** in `theorem-harness-runtime/{canonical_write,coordination}.rs` + new `tenant.rs`). The user asked me to "commit and push the diff to main." Per the repo's pathspec rule I carefully built an explicit pathspec to land ONLY the buildable engine-tier unit and EXCLUDE the buggy tenant WIP + unrelated lanes (obsidian-sync, epistemic, agentd).

While I ran the pre-push gates (`cargo check --workspace`, a peer-review workflow), the repo's **auto-commit-push hook fired**: it did a `git add -A`-style sweep, committed ALL 81 dirty files as `0d574f4 "feat(rustyred): add native document relational and pg wire tiers"`, and pushed to origin/main. My subsequent `git add -- <pathspec>` + `git commit -- <pathspec>` then reported "no changes added to commit" (everything was already committed), and `git rev-list --count origin/main...HEAD` was `0 0` (already pushed). The buggy tenant WIP landed on main despite my plan to exclude it.

## Rule

This repo runs a background auto-commit-push hook that periodically `git add -A` + commits + pushes the WHOLE working tree to main (it has done this in prior sessions too -- see memory `sessions/...auto-commit`). Therefore: a carefully-scoped manual `git commit -- <pathspec>` is NOT a reliable way to keep buggy/co-agent WIP off main -- the hook will sweep whatever is dirty. If you need scope control, either (a) commit your slice EARLY and keep the tree otherwise clean, or (b) accept that everything dirty WILL land on main and ensure nothing buggy is uncommitted when the hook can fire. Do NOT run long pre-push gates (full workspace build, multi-agent review) while a known-buggy lane sits uncommitted in the shared tree -- the hook can ship it from under you mid-gate. When you discover the sweep already happened, switch to fix-forward (a new commit), never history rewrite on pushed main.

## Evidence

- `0d574f4` = 81 files changed, +13270 -591, including `theorem-harness-runtime/src/{canonical_write,coordination}.rs` (the tenant-partition bug, flagged unfixed in room tension `record_9fa3a5cf9dcada25`), `rustyred-thg-memory/src/similarity.rs` (obsidian-sync), `epistemic.rs`, `theorem-agentd/*` -- all lanes I intended to exclude.
- `git rev-list --left-right --count origin/main...HEAD` returned `0 0` (HEAD already == pushed origin/main) before my manual commit ran.
- My `git add -- <16 explicit paths>` then `git commit` printed "On branch main / no changes added to commit"; the tree had collapsed to just `.harness/code-kg-manifest.json` (M) + one untracked zip.
- A peer-review finding (a SQL-injection seam in my pg-server param path) was already on main inside `0d574f4`; the only safe remedy was a fix-forward commit `069c6c7`, not a rewrite.
