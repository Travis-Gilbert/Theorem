# `git add -A` stages multi-GB model weights; size-scan before commit, do not trust an extension grep

**Kind:** anti_pattern
**Captured:** 2026-06-15
**Session signature:** `claude:travisgilbert (harness-plugin-restructure / theorem land)`
**Domain tags:** git, commit-hygiene, theorem-agentd, deploy

## Trigger

Landing the Theorem working tree for a push to main, I ran `git add -A` then a
`git reset` of a known-junk list. To sanity-check before committing I grepped the
staged file names for binaries with an extension allowlist:
`grep -iE '\.(zip|bin|key|pem|env|so|dylib|a)$|secret|credential|token'` — it
reported "none". But `apps/theorem-agentd/` holds three multi-gigabyte Gemma GGUF
weights (`gemma-4-12B-it-qat-UD-Q4_K_XL.gguf`, `mmproj-F32.gguf`,
`mtp-gemma-4-12B-it.gguf`) plus `.agentd-ledger.jsonl`. `.gguf` was not in my
allowlist, so all three were staged and the grep said clean. They were caught only
because I separately eyeballed `git diff --cached --name-status | grep '^A'`.
Committing them would have bloated the repo and GitHub would have rejected the push
(100MB hard limit).

## Rule

Before committing after any broad `git add -A` / `git add <dir>` in a repo that may
contain model artifacts, run a SIZE scan, never an extension allowlist:
`git diff --cached --name-only -z | xargs -0 -I{} sh -c 'test -f "{}" && find "{}" -size +5M -exec ls -lh {} \;'`.
Explicitly exclude model weights (`*.gguf`, `*.safetensors`, `*.bin`, `*.pt`, `*.onnx`),
runtime ledgers/state (`*.jsonl` logs, `.agentd-ledger.jsonl`), and IDE/build dirs
(`.idea/`, `*/build/`, `*.zip`). Prefer surgical `git add -u` + explicit untracked
paths over `git add -A` when the tree contains an `apps/*-agentd/` or any model dir.

## Evidence

- `git diff --cached --name-only --diff-filter=A` showed `apps/theorem-agentd/gemma-4-12B-it-qat-UD-Q4_K_XL.gguf` and two more `.gguf` staged after the extension grep returned "none (clean)".
- After `git reset` of the three `.gguf` + the ledger, the size scan (`-size +5M`) returned empty and the commit went through clean (53 files, then the surgical re-stage avoided the GGUF churn entirely).

## Encoded in

- `docs/learnings/2026-06-15-size-scan-before-commit-not-extension-grep.md` (this file)
