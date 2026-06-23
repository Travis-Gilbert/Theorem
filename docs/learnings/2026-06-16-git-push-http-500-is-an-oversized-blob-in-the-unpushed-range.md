# A reproducible `git push` HTTP 500 (sideband disconnect) in this repo means an oversized blob in the UNPUSHED commit range, not a network blip: diagnose with `rev-list --objects | cat-file --batch-check | sort -rn` before retrying

**Kind:** postmortem
**Captured:** 2026-06-16
**Session signature:** `claude:travisgilbert (agent-space-viewport / transport)`
**Domain tags:** git, github, push-failure, large-files, gguf, history-hygiene

## Trigger

After committing clean agent-space work (`575a764f`, 9 files, +1161 lines) on
`feat/crdt-substrate`, `git push origin feat/crdt-substrate` failed:

```
error: RPC failed; HTTP 500 curl 22 The requested URL returned error: 500
send-pack: unexpected disconnect while reading sideband packet
fatal: the remote end hung up unexpectedly
Everything up-to-date          <- MISLEADING trailing line, exit code 0
```

It reproduced identically on retry. The "Everything up-to-date" + exit 0 made it
look harmless; `git ls-remote origin feat/crdt-substrate` proved otherwise -- the
remote ref had NOT moved (still `00012870`, two commits behind). The push range
was only 2 commits, but one of them (`f95bec1a` "Browser use servo", an UNPUSHED
prior commit by the repo owner, a `git add -A` accident) carried ~7.4 GB of
binaries:

```
git rev-list --objects 00012870..HEAD | git cat-file --batch-check='%(objecttype) %(objectsize) %(rest)' \
  | awk '$1=="blob"{print $2,$3}' | sort -rn | head
6716355328 apps/theorem-agentd/gemma-4-12B-it-qat-UD-Q4_K_XL.gguf   # 6.7 GB
 465126464 apps/theorem-agentd/mtp-gemma-4-12B-it.gguf
 209522240 apps/theorem-agentd/mmproj-F32.gguf
  61274283 rustyredcore_THG/BlockNote-main.zip
```

GitHub hard-rejects files >100 MB and chokes on the multi-GB pack -> HTTP 500. The
blob lives in `f95bec1a`'s tree, so a LATER "remove the files" commit does not
help -- the blob is still in the pushed pack. It also blocks merge-to-main: the
poison commit is an ancestor in `origin/main..HEAD`, so merging the branch and
pushing main hits the identical 500.

## Rule

When `git push` returns a reproducible HTTP 500 / "unexpected disconnect while
reading sideband packet" (NOT a one-off), do not just retry and do not trust a
trailing "Everything up-to-date" -- confirm the real remote head with
`git ls-remote origin <branch>`. Then inspect the unpushed range for oversized
blobs: `git rev-list --objects <remote>..HEAD | git cat-file
--batch-check='%(objecttype) %(objectsize) %(rest)' | sort -k2 -rn | head`. If a
>100 MB blob is in an ANCESTOR commit, the fix is to rewrite that commit out of the
range (the offending file must never be in any pushed commit's tree); a follow-up
deletion commit is insufficient. If the poison commit belongs to someone else
(here: the repo owner's own accidental commit), surface it and get a decision
before rewriting history -- do not silently force or absorb it.

## Evidence

- `git ls-remote origin feat/crdt-substrate` -> `00012870...` (unchanged) after two
  "exit 0" pushes that printed HTTP 500.
- `git show -s --format='%an %s' f95bec1a` -> `Travis Gilbert  Browser use servo`;
  `git check-ignore apps/theorem-agentd/*.gguf` -> empty (not even ignored).

## Encoded in

- `docs/learnings/2026-06-16-git-push-http-500-is-an-oversized-blob-in-the-unpushed-range.md` (this file)
