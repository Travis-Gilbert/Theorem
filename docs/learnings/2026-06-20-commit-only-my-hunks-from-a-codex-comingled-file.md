# To land your slice from a source file another agent is mid-edit on, stage only YOUR hunks via a region-filtered `git apply --cached`, then prove the staged/unstaged token split before committing

**Kind:** method
**Captured:** 2026-06-20
**Session signature:** `claude-code:travisgilbert (SPEC-GRAPHQL-MCP-FINISH A6 typed cluster domains)`
**Domain tags:** git, coordination, codex, shared-tree, partial-staging, rustyred-thg-mcp

## Trigger

`rustyred-thg-mcp/src/lib.rs` was dirty with **Codex's** in-flight `ProviderHeadInvoker` migration (8 hunks: import rewrites at lines 46-76, `composed_agent_run_to_store` at 9119-9152, a new networking test module at 12628-13104, and provider-head tests at 16944-16976). My A6 work needed to update tests in the SAME file (5 introspect `tool_result_budget_bytes = 0` edits + 2 cluster-test rewrites, all in 15824-16454). Converting `clusters.rs` to typed objects BROKE the old cluster tests, so committing `clusters.rs` without the lib.rs test edits would have left a red tree -- but `git commit -- lib.rs` (pathspec) commits the file's FULL working state, which would have bundled Codex's uncommitted migration into my commit (the exact thing CLAUDE.md forbids). `git add -p` is interactive and blocked in this harness.

## Rule

When you must commit changes to a file a peer agent is co-editing, never pathspec-commit the whole file. Stage only your hunks:

1. `git diff -- <file>` and read every `@@` hunk header; classify each hunk as yours vs theirs by **line region** (mine were old-start in `(13104, 16944)`; theirs were `<=13104` or `>=16944`).
2. Build a patch of only your hunks with an awk filter on the `@@ -N` old-start, keeping the `diff/index/---/+++` header lines: `git diff -- <file> | awk '/^@@ /{split($2,b,",");s=b[1];sub(/^-/,"",s);n=s+0;keep=(n>LO&&n<HI);if(keep)print;next} /^(diff|index|--- a\/|\+\+\+ b\/)/{print;keep=0;next} {if(keep)print}' > /tmp/mine.patch`
3. `git apply --cached --recount --check /tmp/mine.patch` (dry run), then without `--check`.
4. PROVE the split with token greps before committing: `git diff --cached -- <file> | grep -c <their-signature-tokens>` MUST be 0, and `git diff -- <file> | grep -c <their-tokens>` MUST be >0 (their work still in the working tree, untouched).
5. Verify the commit-only state is self-consistent + green: `git stash push --keep-index` (shelves their unstaged work, leaves your staged changes in the tree), run the suite, then `git stash pop`.
6. Commit with a **bare** `git commit` (commits the verified index only). Do NOT use `git commit -- <file>` here -- for a partially-staged file that re-adds the unstaged hunks.

## Evidence

- Commit `a4e7110` landed `clusters.rs` + `mod.rs` whole + only my 10 lib.rs hunks; verified `staged Codex-tokens=0 / my-tokens=19 / 10 hunks` and `unstaged Codex-tokens=8 / my-tokens=0`.
- `git stash --keep-index` test of the commit-only state: `100 passed; 0 failed` (Codex's separate `native_coordination` regression vanished, because his change was stashed -- which also proved that failure was his, not mine).
- After the commit, Codex's 8 runtime files + lib.rs hunks + untracked `provider_invoker.rs` were all still present and unstaged. His lane was never touched.
