# Loop status: RustyRed as Code Workspace

Durable state for the `/theorems-harness:execute` self-paced build loop over this
plan tree, so the next session (or Codex) resumes cold. Coordination room:
`rustyred-code-workspace` (tenant `Travis-Gilbert`). Two heads: `claude-code` and
`codex`.

Ship decision on 2026-06-22: this branch ships without macFUSE as a merge
requirement. W6's platform-neutral mount core, default pure-Rust
`rustyred-fuse` proof crate, and feature-gated host/actor coverage are the PR
scope; live macFUSE/fuse3 mounting and mounted execution parity are optional
follow-up smokes.

## Lane split (agreed, with one collision corrected)

| Head | Lane | Files |
|---|---|---|
| `codex` | W0: import path + embedded workspace seam | `apps/rustyred-workspace` (canonical), `apps/rustyred-embedded/src/lib.rs` |
| `claude-code` | W2: git-as-truth | `apps/rustyred-git` |

Collision note: both heads initially grabbed W0 and built the importer in parallel.
Codex claimed first; `claude-code` yielded W0 and pivoted to W2. The `claude-code`
W0 importer at `apps/rustyred-workspace` on branch `claude/nice-newton-56f8dd` is a
**review reference only** (green, 2 tests) and must not be committed (a second
`apps/rustyred-workspace` would collide with Codex's canonical W0 at merge).

## Built this loop (claude-code lane)

| Unit | Slice | State | Tests |
|---|---|---|---|
| W0 | per-file `fs_write` importer (artifact filter, source-only) | green, review-reference only (NOT to commit) | 2 |
| W2 | local git VCS: init/commit/read-back/branch/divergent heads | green | 2 |
| W2 | three-way merge (clean) + conflict surfaced + abort | green | 2 |
| W2 | push to local bare remote + clone round-trip | green | 1 |
| W2 | `gix` backend slice: init/open/head/current-branch/read-at-HEAD/clone | green | 2 |
| W2 | `gix` materialized-tree commit-all: write blobs/trees/commit + advance HEAD | green | 1 |
| W2 | `gix` branch create + force checkout/rematerialize selected tree | green | 1 |
| W2 | `gix` local bare push: copy reachable object closure + fast-forward branch ref + clone round-trip | green | 1 |
| W2 | `gix` merge/conflict surfacing: clean divergent merge commit + same-file conflict without advancing HEAD | green | 2 |
| W2 | `gix` branch/worktree tree diff: branch-to-branch + dirty worktree-to-HEAD without committing | green | 2 |
| W2 | `gix` materialized-worktree status receipt: current branch/HEAD + unborn additions + dirty commit candidates without moving refs | green | 1 |
| W2 | `gix` staged/index porcelain: stage-all, HEAD-to-index staged diff, index-to-worktree unstaged diff, untracked files | green | 1 |
| W2 | `gix` branch isolation snapshot: materialize committed branch into separate plain source directory | green | 1 |
| W2 | `gix` push-ready ref update: validated refspec + expected old head + minimal object closure + parser-verified V2 packfile + decoded receive-pack request body + non-fast-forward rejection | green | 2 |
| W2 | `gix` local receive-pack transport: handshake, advertised-head/capability negotiation, raw receive-pack body send, report-status parse, two fast-forward pushes + clone round-trip | green | 1 |
| W2 | `gix` smart-HTTP receive-pack: GitHub-compatible Basic auth, advertise-refs GET, request-body POST, report-status parse, mock-proven network push shape; new-branch pushes reuse advertised remote refs as known objects | green | 2 |
| W2 | ignored live GitHub push/PR smoke: Basic-auth clone setup, smart-HTTP branch push, draft PR creation against `Travis-Gilbert/Theorem` | green live (#33) | 1 |
| W2 | GitHub REST PR-open seam with injected bearer token + mock endpoint | green | 1 |
| W2 | graph-version vs git distinctness proof | green | 1 |
| W3 | local materialize/run/sync: cargo build, artifact exclusion, source rewrite sync, env strip | green | 2 |
| W3 | `SandboxRuntime` bridge: put_files/run/get_files source rewrite through `LocalProcessSandbox` | green | 1 |
| W3 | local `SandboxRuntime` streaming/cancellation: stdout callback cancel + timeout events + workspace sync-back after cancelled run | green | 3 |
| W3 | `OpenSandboxRuntime` execd streaming parser: SSE stdout/stderr events + callback cancellation against mock sidecar stream | green | 2 |
| W3 | ignored live OpenSandbox put/run/get smoke scaffold compiles; API key optional for unauthenticated local sidecar | green scaffold, live run still open | 1 |
| W3 | ignored live OpenSandbox streaming/cancellation smoke scaffold compiles; API key optional for unauthenticated local sidecar | green scaffold, live run still open | 1 |
| W1 | opt-in persistent `CodeSymbolName` bucket index primitive | green | 1 |
| W1 | source-file write indexing: parse caller-provided bytes, carry unchanged graph nodes, touched-bucket outgoing edge rewrite with tombstones | green | 1 |
| W1 | reverse incoming edge rewrite for newly defined names using carried source reconstruction | green | 1 |
| W1 | embedding-on-write proof: `incremental_embed_hook` refreshes source-file write embeddings and stays idempotent for token-equivalent edits | green | 1 |
| W1 | generic embedded File-write hook: direct `fs_write_batch` source writes feed CodeCrawler without W3 sync-back | green | 1 |
| W1 | large-repo touched-bucket bound + W1.5 collapse measurement (6 -> 3 round trips; 20 -> 14 proxy tokens) | green | 1 |
| W3/W1 | indexed sync-back: changed source files sync through W3 and refresh CodeCrawler edges through W1 | green | 1 |
| W4 | shared code embedder seam: hash default + HTTP mock + configurable File/CodeSymbol dimensions | green for W4.1 | 1 |
| W4 | explicit File re-embed batch after encoder/dimension change + vector index redesignation rebuild | green | 1 |
| W4 | local Candle/BGE feature path: BAAI/bge-small-en-v1.5 loader, dimension validation, live ranking smoke, consumer feature passthroughs | green | 1 |
| W4 | ignored live hosted HTTP embedder smoke scaffold: endpoint vectors are finite, dimensioned, and rank related code closer | green scaffold, live run still open | 1 |
| W5 | code surface adapter: file-position presence, edit footprints, no CRDT file-byte merge, cursor-order guarantee | green | 3 |
| W6 | DocTree mount core: getattr/readdir/read/source-write over W0 Engine seam, W1 hook firing, single-copy visibility, artifact throwaway routing | green | 2 |
| W6 | durable DocTree unlink/rename: RedCore `NodeDelete`, embedded `fs_unlink`/`fs_rename` restart proof, mount-core unlink/rename + artifact throwaway receipts | green | 5 |
| W6 | feature-gated `fuser` host adapter: `DocTreeFuseBackend` seam, inode/path translation, read/write/create/unlink/rename forwarding, conservative single-threaded mount config | green compile/test | 1 |
| W6 | actor-backed real Engine FUSE backend: opens `Engine` on a single-threaded worker, forwards backend ops through `DocTreeMount`, and runs setup-installed File-write hooks | green | 2 |
| W6 | directory POSIX subset: mount-core prefix directory rename, actor-backed directory rename, `rmdir` missing/not-directory/not-empty/throwaway outcomes, and recursive inode-path rename | green | 3 |
| W6 | persistent empty directories: embedded `fs_mkdir`/`fs_rmdir` markers rehydrate without File nodes, materialize to disk, and round-trip through mount/actor/FUSE mkdir-rmdir | green | 4 |
| W6 | configurable FUSE attrs + inode cleanup: `FuseAttrPolicy` controls permissions/uid/gid/blksize and cached directory forget evicts descendants | green | 2 |
| W6 | host-side throwaway disk tree: artifact paths are temp-backed through FUSE read/getattr/readdir/write/mkdir/rename/unlink/rmdir without entering DocTree | green | 1 |
| W6 | source/artifact rename crossings: artifact-to-source enters the backend; source-to-artifact unlinks DocTree and moves bytes to temp-backed throwaway storage | green | 1 |
| W6 | inode forget/write-buffer cleanup: `Filesystem::forget` and path removals evict descendant inode mappings plus buffered writes | green | 1 |
| W6 | recursive directory-tree source/artifact crossings: whole generated directories move between throwaway disk and backend storage without stale DocTree/source entries | green | 1 |

`apps/rustyred-git` total: 26 tests in the Codex worktree: 25 non-ignored tests
green plus one ignored live GitHub push/PR smoke that is live-green when run
with a GitHub token. The original
CLI backend still provides the full local VCS slice; the new `GixWorkspaceRepo`
proves the pure-Rust `gix` path for initialization, opening, branch/head
inspection, materialized-tree commit-all via gix blob/tree/commit objects,
force branch checkout/rematerialization, committed-tree file reads, and local
bare-repo clone. It also prepares a push-ready ref update/object closure/packfile
through `GixPreparedPush` (validated branch refspec, expected remote old head,
new local head, minimal objects not already reachable from the remote head, a
parser-verified V2 packfile containing exactly that closure, and
non-fast-forward rejection before mutation), encodes the checked update plus
packfile into a receive-pack request body (command pkt-line, flush, raw `PACK`
bytes), and the local bare push consumes
the object-closure artifact without the git CLI before proving a gix clone sees
the updated tree. It also sends that receive-pack body through a real local
`git-receive-pack` transport, drains the advertised ref stream before the
request, parses `report-status`, and proves two fast-forward pushes plus a clone
round-trip. It now also has a mock-proven smart-HTTP receive-pack sender: it
GETs `info/refs?service=git-receive-pack` with GitHub-compatible Basic auth,
parses the
advertised old head/capabilities, POSTs the raw receive-pack request body to
`git-receive-pack`, and requires `report-status`. For new PR branches, it reuses
all advertised remote refs as known objects so it does not resend the whole
remote tree when the target branch itself is absent. Its ignored live GitHub
oracle is green against `Travis-Gilbert/Theorem`: with
`RUSTYRED_GIT_LIVE_REMOTE_URL` plus `GITHUB_TOKEN`, it cloned `main`, pushed
`rustyred-live-smoke-23784-1782118919` via smart HTTP, and opened draft PR #33.
It now performs pure-gix clean divergent merges into two-parent merge
commits and surfaces unresolved same-file conflicts without advancing HEAD or
rewriting the materialized worktree. It also exposes tree-level diff for
branch-to-branch review and dirty materialized worktree-to-HEAD commit decisions
without moving refs, wraps that diff in a `GixWorktreeStatus` receipt
(current branch, HEAD, clean/dirty, unborn additions), exposes true
HEAD/index/worktree status through `GixIndexStatus` (`staged`, `unstaged`,
`untracked`), and materializes committed branch snapshots into separate plain
source directories for parallel-agent isolation without touching the main
worktree. The GitHub PR-open seam is direct REST with bearer-token injection and
mock coverage for
method/path/headers/body/receipt. The
distinctness proof shows the existing graph-version history and W2 git history
are separate but complementary.

`apps/rustyred-workspace` total: 16 default tests green in the Codex worktree;
with `--features fuse-host`, 26 tests are green (16 shared tests plus 10
feature-gated host/actor tests). W3 now
proves the local materialize/run/sync path, indexed sync-back into the W1 code
graph, and the receiver `SandboxRuntime` put/run/get bridge through
`LocalProcessSandbox`. The workspace bridge also proves streaming stdout can
cancel a run and still sync the source edit made before cancellation back into
DocTree. W6 has started with `DocTreeMount`, a platform-neutral mount core that
maps filesystem-style read/list/getattr/source-write/unlink/rename operations
onto `Engine::list_paths`, `fs_read`, `fs_write`, `fs_unlink`, and `fs_rename`;
tests prove mount writes are immediately visible through `fs_read`, direct
Engine writes are immediately visible through the mount core, source writes fire
W1 hooks, source unlink/rename update DocTree plus `File` graph metadata through
RedCore's durable `NodeDelete` path, and artifact writes/unlinks/renames return
`Throwaway` without entering DocTree or firing hooks. W6 now also covers a
directory POSIX subset: synthetic directory rename moves every source file under
the prefix through `Engine::fs_rename`, source rmdir reports missing vs file vs
non-empty, artifact rmdir stays in the throwaway lane, and cached inode paths are
renamed recursively. W6 now also has persistent empty directories: explicit
directory markers rehydrate through `rustyred-embedded`, stay out of `File` graph
nodes and file listings, materialize as real OS directories, and can be created
and removed through the actor-backed FUSE seam. W6 now also has a
feature-gated `fuser` host adapter over a `DocTreeFuseBackend: Send + Sync`
contract, with inode/path translation, read/write/create/mkdir/unlink/rmdir/rename forwarding,
configurable `FuseAttrPolicy` permissions/uid/gid/blksize, recursive descendant
forget for cached directory inodes, a private temp-backed throwaway disk tree for
artifact read/getattr/readdir/write/mkdir/rename/unlink/rmdir, source/artifact
rename crossings that shuttle bytes between throwaway disk and the backend, and
inode forget/removal cleanup for descendant write buffers. It still uses
conservative single-threaded mount config. Source/artifact rename crossings now
also recurse for directory trees, so generated directories can move into source and
source directories can move into artifact storage without stale backend entries.
The real embedded engine is wired
to that backend seam through `EngineFuseBackend`, which opens `Engine` on a
single-threaded actor, forwards filesystem operations through `DocTreeMount`, and
proves setup-installed File-write hooks fire on FUSE writes without making
`SharedStore<Rc<RefCell<_>>>` thread-safe. It compiles with `macos-no-mount` on
this machine; an optional real platform mount remains unrun and is no longer a
ship blocker. Changed-package clippy is clean.

`apps/rustyred-embedded` total: 17 lib tests plus stdio acceptance green in the
Codex worktree. The W6 additions expose durable `fs_unlink` and `fs_rename`:
unlink removes the DocTree entry and the matching `file:{path}` node, rename
writes the destination through the normal File-write seam and then unlinks the
source; both survive engine restart. W6 also adds durable explicit directory
markers (`fs_mkdir`, `fs_rmdir`, `fs_is_dir`, `list_directories`) that rehydrate
without creating zero-byte `File` nodes. `rustyred-thg-core` also has a direct
`redcore_delete_node_survives_aof_replay` proof for the underlying `NodeDelete`
primitive.

`theorem-receiver` streaming validation green in the Codex worktree:
`local_process_sandbox_streams_output_and_cancels_running_command` and
`local_process_sandbox_streaming_reports_timeout_exit_event` prove
`SandboxRuntime::run_streaming` over `LocalProcessSandbox`, including stdout
events, cancellation, timeout exit events, and shared `ProofReceipt` shape.
`execd_stream_emits_events_and_receipt_shape` and
`open_sandbox_runtime_streams_execd_response_and_cancels` prove
`OpenSandboxRuntime::run_streaming` consumes execd `data:` events, emits stdout
callbacks before completion, returns sandbox-tier receipts, and can cancel from
the callback against a mock sidecar stream. The ignored live
`live_open_sandbox_round_trips_files_and_receipt_shape` and
`live_open_sandbox_streaming_can_cancel_running_command` tests both compile;
running them remains gated on `OPEN_SANDBOX_BASE_URL` and an actual sidecar.

`rustyred-thg-code` W1 targeted tests green in the Codex worktree:
`source_file_write_indexes_one_file_without_repo_walk`,
`source_file_write_rewrites_touched_outgoing_edges`,
`source_file_write_recomputes_incoming_edges_for_new_definition`,
`ingest_materializes_symbol_name_bucket_index`,
`reindex_edges_match_a_full_ingest_after_a_moved_target`,
`reindex_links_carried_symbol_to_a_newly_defined_name`, and
`finished_job_survives_a_runtime_restart`. Changed-package clippy is clean.

W4.1 validation green in the Codex worktree:
`rustyred-code-embedding` default tests (5), `rustyred-code-embedding --features
http` tests (7 compiled: mock endpoint plus one ignored hosted-endpoint smoke),
`apps/rustyred-embedded` full suite
(12 lib + stdio), CodeCrawler `source_file_write` suite (5),
`configured_code_embedder_controls_symbol_dimension_and_similarity`,
`code_kg_hooks`, HTTP feature checks for both consumers, and clippy for
`rustyred-code-embedding`, `rustyred-thg-code`, and `rustyred-embedded`. The
ignored `live_http_embedder_endpoint_prefers_related_code` smoke is the hosted
endpoint oracle; running it remains gated on `RUSTYRED_CODE_EMBED_URL`.

W4 re-embed validation green in the Codex worktree:
`reembed_files_refreshes_existing_file_vectors_after_dimension_change` proves a
workspace reopened with a new File embedding dimension is not searchable until
`Engine::reembed_files("src")` rewrites File vectors, then `vectorSearch`
returns the file. `vector_redesignate_rebuilds_index_for_new_dimension` and the
core `vector` test filter prove the vector index is rebuilt when a designation
dimension changes.

W4 local encoder path green in the Codex worktree:
`rustyred-code-embedding --features local` now builds a `LocalCodeEmbedder`
backed by Candle + `BAAI/bge-small-en-v1.5` (default D=384, overridable by
`RUSTYRED_CODE_EMBED_LOCAL_MODEL`) and validates the loaded model hidden size
against the configured vector designation dimension. The crate has an ignored
live smoke, `local_bge_embedder_loads_and_prefers_related_code`, that loads the
model and checks semantic ranking when explicitly run; it is green in this
worktree after bumping `hf-hub` to 0.5.0 for the current Hugging Face redirect
flow. `apps/rustyred-embedded` and `rustyred-thg-code` both expose
`code-embedding-local` feature passthroughs.

W5 validation green in the Codex worktree:
`theorem-copresence` now has `CursorPos::FilePosition` and `CodeSurfaceAdapter`
as an in-substrate, per-open-file awareness adapter. It announces file-position
presence through the working log, writes pending edit footprints as graph
metadata, rejects text-region file-byte writes, and returns `GitMergeOnly` as
the content strategy. Tests cover two-peer file presence/footprints, explicit
non-CRDT code content handling, file-position serialization, and deterministic
working-log cursor-order snapshots.

## Backend decision (W2): CLI full surface plus gix slice

The plan names `gix` (pure-Rust) for the embedded no-git case. The first W2
slice shipped a `git`-CLI backend behind `WorkspaceRepo` because the host was at
100% disk and the CLI path kept the local VCS acceptance moving. After disk was
freed, Codex added `GixWorkspaceRepo` and compiled `gix` 0.84.0 successfully.

Current split:

- `WorkspaceRepo`: full local VCS behavior through the installed git CLI
  (commit-all, branch, checkout, merge, conflict surfacing, push to bare, clone).
- `GixWorkspaceRepo`: pure-Rust backend slice for init/open/head/current-branch/
  read-at-HEAD/clone plus materialized-tree commit-all via direct gix
  blob/tree/commit writes and force branch checkout/rematerialization from the
  selected committed tree, pure-gix clean merge/conflict surfacing, tree-level
  branch/worktree diff, materialized-worktree status receipt, staged/index
  porcelain status, branch snapshot isolation, push-ready ref update/object
  closure/packfile/receive-pack body preparation, local bare push by consuming
  that prepared object closure and fast-forwarding the destination ref, plus a
  local receive-pack transport send through `git-receive-pack` with
  `report-status` confirmation, plus mock-proven smart-HTTP receive-pack push
  with GitHub-compatible Basic auth and advertised-remote-ref object reuse.

The ignored live GitHub remote push/PR smoke is now green against
`Travis-Gilbert/Theorem` and opened draft PR #33.

## Disk state

Disk was freed by removing rebuildable Rust/Cargo artifacts. Current check in
this worktree shows ~20 GiB free / 96% Data volume, enough to build the current
W1/W2/W3/W6 dependency trees.

## External/live follow-ups

| Unit | Follow-up |
|---|---|
| W3: execution bridge (materialize-to-run) | local-process slice, indexed sync-back, `SandboxRuntime` put/run/get bridge, local streaming/cancellation, OpenSandbox execd streaming parser, and ignored live round-trip + streaming/cancellation smokes are green; live OpenSandbox sidecar run remains |
| W4: real code embedding | shared seam, HTTP mock, dimension proofs, explicit File re-embed batch, local Candle/BGE feature path, live local-model smoke, and ignored live hosted HTTP smoke are green; live hosted endpoint run remains |
| W6 FUSE endgame | no-macFUSE ship target is green: mount core, durable file unlink/rename primitives, feature-gated `fuser` host adapter, actor-backed real Engine backend, directory ops, persistent empty dirs, configurable attrs, recursive inode cleanup, host-side throwaway disk backing, source/artifact rename crossings including directory trees, and write-buffer cleanup on inode forget are green; live macFUSE/fuse3 mount, live-host POSIX/performance tuning, and execution parity against a mounted path are optional follow-ups |

## Scorecard (claude-code solo lane)

| Unit | Status | Gate |
|---|---|---|
| W2 local git (CLI backend) | MET (green) | none |
| W2 gix backend | MET (green) | init/open/head/read/clone, materialized-tree commit, stage/index status, force branch checkout, merge/conflict surfacing, branch/worktree diff/status, branch snapshot isolation, push-ready ref update/object closure/packfile/receive-pack body, local bare push, local receive-pack transport send, smart-HTTP push mock, advertised-ref object reuse, and live GitHub smoke green |
| W2 GitHub remote + PR-open | MET (green) | PR-open request seam, smart-HTTP push mock, ignored live smoke, live push, and draft PR #33 green |
| W3 execution bridge | PARTIAL | local materialize/run/sync/env-strip, indexed sync-back, `SandboxRuntime` put/run/get bridge, local streaming/cancellation, OpenSandbox execd streaming parser, and ignored live round-trip + streaming/cancellation smokes green; live sidecar run remains |
| W4 real code embedding | PARTIAL | shared seam wired into File + CodeSymbol paths; hash default, HTTP mock, local Candle/BGE feature path, live local-model smoke, dimension proofs, File re-embed batch, and ignored live hosted-endpoint smoke green; live endpoint run remains |
| W5 collaboration adapter | MET (green) | code adapter slice green; W2 now provides the local gix merge/conflict primitive |
| W6 mount/FUSE | GREEN for no-macFUSE PR | `DocTreeMount` core proves single-copy read/write/list/getattr/unlink/rename, durable File-node deletion via RedCore `NodeDelete`, artifact throwaway routing, a feature-gated `fuser` host adapter over a `Send + Sync` backend seam, actor-backed real Engine ops, directory semantics, empty dirs, configurable attrs, recursive inode cleanup, temp-backed artifact storage, source/artifact rename crossings including directory trees, and write-buffer cleanup on forget; live platform mount + live-host tuning + mounted execution parity remain optional follow-ups |
| W0 | MET | Codex canonical W0 green; Claude W0 remains review-reference only |
| W1 | MET (green) | opt-in `CodeSymbolName` buckets, source-file indexing, touched outgoing/incoming edge rewrite, W1.4 embedding proof, generic embedded File-write hook, W3 indexed sync-back proof, large-repo bucket bound, and collapse measurement green |

## Resume

1. Optional live follow-ups: run the W3 live OpenSandbox smokes, run the W4 live
   hosted encoder smoke, or run a W6 platform FUSE host smoke on a machine that
   has opted into macFUSE/fuse3. W0 + W1 + W2 + W3
   local now prove the near-term edit-run-commit spine at the local-process
   layer, and W3 also proves the sandbox trait bridge, streaming, cancellation,
   and sync-back-after-cancel with the local backend.
2. At turn-start: drain mentions, read room `rustyred-code-workspace`, retry the
   `coordination_intent` post (the endpoint has been timing out).
3. Do not commit Claude's alternate `apps/rustyred-workspace` from
   `claude/nice-newton-56f8dd`; Codex's W0 is canonical.
