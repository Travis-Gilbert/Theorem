# The shell here is zsh: an unquoted `$var` holding space-separated paths does NOT word-split, so `git add -- $PATHS` collapses into one bogus pathspec — inline the paths or use a zsh array

**Kind:** gotcha
**Captured:** 2026-06-18
**Session signature:** `claude-code:travisgilbert (harness product docs + generated OpenAPI)`
**Domain tags:** zsh, shell, word-splitting, git-pathspec, bash-tool, darwin

## Trigger

Making three pathspec-scoped commits, I tried to DRY the path lists into variables:

```sh
C1="apps/.../openapi.rs apps/.../lib.rs docs/.../api-http.md"
git add -- $C1            # expected: 3 args; got: 1 arg
```

It failed exit 128: `fatal: pathspec 'apps/.../openapi.rs apps/.../lib.rs docs/.../api-http.md' did not match any files` and `warning: could not open directory '<the whole joined string>'`. zsh — unlike bash — does NOT perform field/word splitting on unquoted parameter expansions, so `$C1` was passed as a single argument with embedded spaces. The whole `&&` chain aborted before any `git commit` ran; nothing was staged or committed (so it was a clean, recoverable failure — just a wasted round-trip).

## Rule

The Bash tool here runs **zsh** (Platform darwin). Do not rely on bash-style word splitting of unquoted variables. For any command that needs a variable to expand into multiple arguments (git pathspecs, file lists, flags):
- **Inline the words literally** (simplest and what fixed it): `git add -- path/a path/b path/c`.
- Or use a **zsh array**: `paths=(path/a path/b path/c); git add -- $paths` (array expansion DOES split per element).
- Or force splitting with the `=` flag: `git add -- ${=C1}`.

Instant-recognition symptom: a "pathspec '<one long space-joined string>' did not match any files" error, or "could not open directory '<everything you passed>'" — that's the whole variable arriving as a single argument.

## Evidence

- First commit attempt: `git add -- $C1` exit 128, pathspec = the entire space-joined string. No files staged; `&&` chain stopped, so zero commits made.
- Re-run with all paths inlined as separate literal words: the three commits `3e139495`, `6ebd971a`, `46b9c706` all created and verified clean (`git show --stat` showed only the intended files per commit).
