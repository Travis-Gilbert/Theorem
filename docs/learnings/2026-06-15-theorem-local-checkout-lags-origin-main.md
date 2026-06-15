# The Theorem local checkout lags origin/main; diff the working tree against origin/main before committing "uncommitted work"

**Kind:** gotcha
**Captured:** 2026-06-15
**Session signature:** `claude:travisgilbert (harness-plugin-restructure / theorem land)`
**Domain tags:** git, multi-agent, theorem, codex, stale-checkout

## Trigger

Asked to commit and push the Theorem working tree to main, I committed it on the
local HEAD (`3f617b4d`) and only on `git fetch origin main` discovered the local
branch was 6 commits BEHIND origin/main, not ahead. Codex commits via the GitHub
MCP straight to the remote, so the CRDT substrate (PR #19), the improvement-plan +
`graph_csr.rs` (`96a94b5f`), and obsidian-sync were already shipped to origin/main
while the local checkout never pulled them. `git diff --name-only origin/main`
showed only ~11 of the 33 "modified" tracked files genuinely differed; the rest
were byte-identical stale duplicates of already-shipped work. I also misread
`git rev-list --left-right --count origin/main...HEAD` ("6  0") as "6 ahead" when
left = origin/main = 6 BEHIND. Pushing the stale-base commit would have re-landed
duplicates and conflicted with main. I undid it with `git reset --mixed HEAD~1`
(reflog-safe) before any push.

## Rule

In the Theorem repo, `git fetch origin main` FIRST and treat origin/main as
canonical (the CRDT handoff says "git pull first" because Codex's commits land on
the remote, not local). Before committing "uncommitted work," run
`git diff --name-only origin/main` to separate the genuine delta from stale
duplicates; for each genuinely-new file, check `git cat-file -e origin/main:<path>`
to see whether main already has a canonical version. Resolve merge conflicts toward
origin/main for already-shipped files (e.g. a refactor that removed
`prepare_codebase_ingest`), and only combine files that carry genuinely-new local
work (here `rustyred-thg-mcp/src/lib.rs`, +481 of WS2/WS4, which auto-merged with
main's A2 fix). Read `--left-right` correctly: the left count is commits in the
first ref (origin/main) not in HEAD = how far BEHIND you are. Gate any push on a
real per-crate `cargo test`, never on the workspace `cargo build` (the root PyO3
crate is maturin-only and fails to link standalone).

## Evidence

- `git rev-list --left-right --count origin/main...HEAD` returned `6  0` (6 behind, 0 ahead) at the start; after the stale commit it was `6  1`.
- `git log --oneline HEAD..origin/main` listed `96a94b5f`, `20f03a37 Merge PR #19 (feat/crdt-substrate)`, `14e889eb obsidian-sync` — all already on the remote.
- `graph_csr.rs`, `code_kg.rs`, `improvement_plan_acceptance.rs` were `ON-MAIN`; `frontier/`, `theorem-agentd/*.toml` were `NEW-LOCAL`. Post-merge affected crates were all green (mcp 53, web 37+21, code 25, core 140, adapters 109).

## Encoded in

- `docs/learnings/2026-06-15-theorem-local-checkout-lags-origin-main.md` (this file)
