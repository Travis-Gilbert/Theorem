# When a file you're about to create already exists, or source files mutate under you mid-turn, codex is sprinting the same lane — pivot to verifier/acceptance-tests, never co-write the same `.rs` file

**Kind:** anti_pattern
**Captured:** 2026-06-17
**Session signature:** `claude-code:travisgilbert (hipporag+search-rerank verify / railway restore)`
**Domain tags:** coordination, multi-head, codex, git, no-text-crdt, verifier-lane, acceptance-tests

## Trigger

Asked to "build SPEC-HIPPORAG2 and SPEC-SEARCH-RERANK-GATE, coordinating with codex." I claimed the HippoRAG lane and started scaffolding `rustyred-hipporag`. Two signals, seconds apart, proved codex was already inside the same lane:

1. `Write` of `rustyred-hipporag/Cargo.toml` failed with **"File has not been read yet"** — codex had created the crate (lib.rs already declared `pub mod retrieve;` + schema/indexing/raptor) in the seconds between my `mkdir` and my `Write`.
2. Earlier in the same turn, `rustyred-membrane/src/scorer.rs` returned **different content on a second read** (the `mmr_lambda` lambda-form appeared) — codex was live-editing the membrane.

Had I proceeded to write `retrieve.rs` (which codex had declared but not yet filled), both heads would have written the same file into one git tree with no text-CRDT — last writer clobbers, both lose work.

## Rule

Source `.rs` files are a single shared git tree with no merge CRDT. "Build on overlap, don't clobber" means **add adjacent files, not edit the same file**. At turn-start, read the room AND `git status`:

- If the dirty set is the spec's implementation files, or a file you intended to create already exists, **codex owns that lane.** Do not co-write its `.rs` files.
- Take the non-overlapping gap that delivers real value: author the **acceptance/integration test suite** as a NEW file under `tests/` (integration tests can use the crate's normal deps, so zero edits to codex's sources), proving the spec's acceptance criteria 1:1. Own the **external surface** (e.g. an MCP tool) only if its file is NOT in codex's dirty set.
- Surface findings on codex's files via `coordination_record` + `@codex` — flag, don't edit.

This is the Travis-confirmed division when codex solo-sprints git-only: codex implements, claude-code verifies + integration-tests + fills the external seam.

## Evidence

- codex scaffolded all 5 `rustyred-hipporag` source files + wired `hippo_retrieve` into `rustyred-thg-server/router.rs` under me. I added ONLY `rustyred-hipporag/tests/spec_acceptance.rs` (7 tests, acceptance #1–#7, all green) — zero source collisions.
- The "is rustyred-thg-mcp dirty?" check (it was NOT) is what made the MCP-surface lane safe to consider; router.rs WAS dirty, so the call-site stayed codex's.
- Same pattern recorded for the CRDT-substrate build (memory: "Codex solo-sprints all impl git-only; claude-code is verifier").
