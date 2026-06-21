# A Write blocked by "file has not been read yet" on a file you never created is the co-edit signal in this multi-head repo

**Kind:** gotcha
**Captured:** 2026-06-19
**Session signature:** `claude-code:travisgilbert (CommonPlace consumer loop)`
**Domain tags:** coordination, multi-head, co-edit, codex, git-working-tree

## Trigger

Building the CommonPlace loop, CC created a NEW crate `rustyredcore_THG/crates/commonplace` and started unit F2 (its own `ingest.rs`, `vector.rs`, and `tests/f2_ingest_acceptance.rs`). Every one of those `Write` calls failed with "File has not been read yet" / "File has not been read" -- even though CC had never created them this session. The cause: **Codex was concurrently building the same unit in the same new crate**, so the files already existed on disk. CC had no signal other than the failed Write: the crate dir was `??` untracked, the harness room had no Codex message (Codex works git-only/passive), and `git log` showed no new commit. The failed Write WAS the coordination signal.

CC then read the on-disk files, found a complete, coherent F2 (Codex's `EmbeddingGraphStore`/`IngestPipeline`/`DeterministicEmbedder`, wired into `lib.rs`, with `add_similarity` added to the shared `store.rs`), DROPPED its redundant parallel design (deleted `vector.rs`), adopted Codex's, and verified it (11 tests green). `store.rs` had merged cleanly because the two heads edited different regions (additive). Outcome: zero clobber, converged slice.

## Rule

In this repo (Codex is frequently co-active, source `.rs` files are one shared git tree with no text-CRDT), when a `Write` to a file you did not create this session fails with "file has not been read yet": do NOT assume a stale tool state and force it. READ the file first -- another head likely created it. If it is a coherent implementation of the unit you were about to build, build ON it (verify + extend), do not clobber it with a parallel design. Detect co-edit from three channels in order: a Write blocked on an unexpectedly-existing file, the harness room (drain mentions / read intents), and `git status`/`log`. The working tree is the real coordination channel; treat an untracked `??` dir as potentially shared.

## Evidence

- CC's F2 `ingest.rs` Write returned "File has not been read yet"; reading it revealed Codex's `EmbeddingGraphStore` design already wired into `lib.rs`. CC deleted its orphan `vector.rs`, kept Codex's, ran `cargo test -p commonplace` -> 11 green.
- `store.rs` simultaneously carried CC's F1 consts/helpers AND Codex's `add_similarity` (lines ~236-258) -- a clean region-disjoint merge, no conflict.
- Coordination then made explicit via `coordinate` (`msg_8284478895106bda`, @codex) claiming a SEPARATE lane (`apps/commonplace-api`) so the two heads stopped overlapping; Codex subsequently left to `crates/theorem-copresence`.
- Related: [[coedit-crdt-covers-graph-not-source]] (graph/memory converge lock-free; source `.rs` race), [[no-scope-confirmation-questions]] (real coordination = read git first + claim a lane).
