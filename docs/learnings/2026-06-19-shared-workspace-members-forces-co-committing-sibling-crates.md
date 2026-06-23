# Committing one new workspace-member crate forces co-committing every sibling crate its shared `members` list references but that isn't committed yet, or the fresh-checkout build breaks

**Kind:** gotcha
**Captured:** 2026-06-19
**Session signature:** `claude-code:travisgilbert (verify + land theorem-copresence handoff)`
**Domain tags:** git, cargo, workspace, members, scope-control, multi-agent, codex

## Trigger

Asked to "commit and push" ONLY the verified `theorem-copresence` crate. It was already a member of the shared `rustyredcore_THG/Cargo.toml`, but `git diff rustyredcore_THG/Cargo.toml` showed the uncommitted manifest had added TWO member lines: `crates/commonplace` (another lane's complete-but-uncommitted crate) AND `crates/theorem-copresence`. To wire copresence as a buildable member I had to commit Cargo.toml; but committing it with the `crates/commonplace` member line while `crates/commonplace/` stayed untracked makes a fresh checkout fail at `cargo build`/`cargo metadata` with "failed to load manifest for workspace member `.../crates/commonplace`" (member listed, dir absent). Partial-staging the manifest (members minus commonplace) then restoring it is fragile surgery on a shared file in a live-commit env. So the buildable scoped commit `cf877fc` had to BUNDLE commonplace.

## Rule

Before scoping a commit that wires one new workspace-member crate: run `git diff <workspace>/Cargo.toml` and read every `+    "crates/X",` member addition. For each added member whose crate dir is NOT already committed (`git ls-tree origin/main:<...>/crates | grep X` is empty), you MUST either (a) also commit that crate dir, or (b) not commit the manifest at all and commit your new crate dir ORPHANED -- cargo silently ignores a dir absent from `members`, so the workspace still builds, but your crate is dark until the manifest lands. You cannot commit a manifest whose member line points at an absent dir. If you bundle a sibling, `cargo check -p <sibling>` first so you are not vouching for unverified code.

## Evidence

- `git diff rustyredcore_THG/Cargo.toml` = `+    "crates/commonplace",` and `+    "crates/theorem-copresence",` (a 2-line add to `members`).
- `cf877fc` = 24 files: both crate dirs + Cargo.toml + Cargo.lock + CLAUDE.md + README.md. `cargo check -p theorem-copresence -p commonplace` was green before the commit.
- Both crates confirmed present on origin/main after the PR-#25 merge: `git ls-tree -d origin/main:rustyredcore_THG/crates | grep -E "copresence|commonplace"` -> both.
- `git diff --cached --name-only | grep -v <my-paths>` returned NONE before commit -> zero Codex core/agentd/harness WIP swept in.
