# W2: Git-as-truth (gix) + GitHub remote

A real git repository is the working VCS for code, because graph-version cannot
represent a git history GitHub will accept. This unit adds a first-class pure-Rust
git capability (local commit/branch/diff/merge) and a GitHub remote (clone in,
pull, push, open a PR out), reusing the GitHub App auth that already exists.

Dependency edges: **W2 is parallel to W1 and W3** (git-as-truth underlies
versioning and the remote; the in-engine edit-index loop does not block it, and it
does not block the edit loop). W2 and the edit loop converge at "commit, push." W2
shares the import front door with W0 (clone-in is the same `stage_repo_for_ingest`
credential path).

## Thesis

Two version-control systems for two kinds of state. graph-version (already built,
`versioned_graph.rs`) versions the knowledge graph (layer 4), the thing git models
badly: its `GraphCommit.graph_version` counter feeds the commit hash, so its objects
are not git objects. Git versions the code (layers 2 and 3). Git objects and the
`DiskObjectStore` are both content-addressed, so git's objects can eventually live in
RustyRed's object store rather than a parallel store; that convergence is an
optimization, not a prerequisite.

## Critical divergence from the north star (corrected here)

The north star states: "The GitHub connector (`rustyred-thg-connectors`) is the
published-truth path (clone in, pull, open a pull request out)." **This is wrong, and
the plan corrects it.** Verified by reading source:

- `rustyred-thg-connectors` is a **generic MCP client transport** (stdio/HTTP
  JSON-RPC to an external MCP server, `tools/list`, register tools as learnable
  `Affordance` nodes, `tools/call` invoke gated by `InvokePolicy`). It has **zero**
  git-specific logic. `McpTransport` (`transport.rs:47`), `ConnectionTarget`
  (`transport.rs:21`), `invoke_affordance` (`invoke.rs:181`).
- **No pure-Rust git library exists anywhere in the tree** (no `gix`, `git2`, or
  `libgit2` usage; the `gix` matches the north star saw are unrelated JavaScript
  lockfile entries).

What **does** exist, and W2 reuses:

- `GithubApp` at `apps/theorem-harness-server/src/github_app.rs:54`: GitHub App auth
  (JWT signing, installation-token minting, a `GitCredentialResolver`). This is the
  credential source for clone/pull/push over private repos.
- `GithubWebhookState` at `apps/theorem-harness-server/src/github.rs:33`: a webhook
  receiver that writes collaboration objects into the graph and re-indexes code (an
  ingress path, not an authoring path).
- `stage_repo_for_ingest_with_credential` (`rustyred-thg-code/src/lib.rs:874`) already
  accepts a `GitCredential`, so the clone-in half of the remote already has a
  credentialed path.

So the corrected picture: the **clone-in** direction is partly built (credentialed
shallow clone for ingest). The **local git VCS** and the **push/PR-out** direction do
not exist and are W2's real work.

## What to build

A new crate `crates/rustyred-git` (or a module in the W0 `apps/rustyred-workspace`
crate; prefer a dedicated crate so the git dependency is isolated), wrapping `gix`
(the named pure-Rust choice) with a thin domain API the workspace uses.

### W2.1: local git repository

- `gix` as the git engine. `WorkspaceRepo::init(dir)` / `open(dir)`,
  `commit(message, author)`, `branch`/`checkout`, `diff` (working tree vs HEAD and
  branch vs branch), and a worktree-isolation primitive (`gix` worktree or a
  materialized branch) for parallel agent edits.
- The repo lives alongside the materialized working tree (W0/W3): the materialized
  dir is a real git checkout, so `gix` operates on real files, and the `DocTree`
  stays the source of truth that syncs into it (the DocTree-primary decision).

### W2.2: GitHub remote

- Clone-in: reuse `stage_repo_for_ingest_with_credential` for the credentialed
  shallow clone, then `gix` for subsequent `fetch`/`pull`.
- Push and PR-out: `gix` push to the remote using a token from the existing
  `GithubApp` / `GitCredentialResolver`; open a PR via the GitHub REST API (a thin
  `reqwest` call, or an external GitHub MCP server reached through
  `rustyred-thg-connectors` if a no-new-dep path is preferred; W2 picks the direct
  REST call for determinism and names the connector route as the alternative).

### W2.3: the objects-in-DiskObjectStore decision (resolved)

**Recommendation: git keeps its own `.git/objects` initially; converge later.**
Both git and `DiskObjectStore` are content-addressed, but git's object format
(loose + packfiles, zlib) is its own; making git write through `DiskObjectStore` on
day one means implementing a git ODB backend, which is heavy and not on the critical
path to a working loop. Day-one: `gix` uses a normal `.git`. Convergence (a
`gix` ODB backed by `DiskObjectStore` so there is one content store) is a named
follow-up once the loop is proven, not a W2 blocker.

## Acceptance criteria

1. A Rust test initializes a `WorkspaceRepo` over a materialized tree, writes a file,
   commits, branches, makes a divergent commit on each branch, and merges; the
   resulting tree and history are what `git` (the real CLI, or a `gix` read-back)
   reports. Local git is real, not a model.
2. A test clones a public fixture repo via the credentialed clone path, makes a
   change, commits, and produces a push-ready packfile / ref update (the push itself
   is gated on a live remote and a token, so the test asserts the prepared push, with
   a live push as an `#[ignore]` smoke needing a real token).
3. The PR-open path is exercised against a mock GitHub REST endpoint (the `GithubApp`
   token is injected), producing the correct create-PR request; a live PR is an
   `#[ignore]` smoke.
4. graph-version and git stay distinct and complementary: a test shows a code commit
   (W2) and a graph-version commit (existing `compile_graph_pack` /
   `update_graph_ref_cas`) over the same workspace produce independent histories, and
   neither is derivable from the other (the "two VCS" thesis made concrete).
5. `cargo check` clean; the new `gix` dependency is isolated to the git crate (verify
   with `cargo tree` it does not leak into the embedded engine's minimal-binary path);
   changed files clippy-clean.

## Divergences and risks to surface (not bury)

- **`gix` is a new external dependency.** It is the named choice and pure-Rust (no
  system libgit2), but it is heavy and evolving. Pin a known-good version and keep the
  domain API thin so a future swap (or a `git2`/libgit2 fallback) is a one-crate
  change. Dependencies named in a spec are information, not gates: check the tree and
  decide, but `gix` is the right call here because the alternative (shelling out to
  the `git` CLI) reintroduces a process and a `git`-must-be-installed assumption the
  embedded story wants to avoid.
- **The north-star connector claim must not be coded to.** Do not try to make
  `rustyred-thg-connectors` the clone/pull/PR path; it is the wrong layer. If a
  GitHub MCP server is ever the chosen remote transport, it is reached *through* the
  connector as one external server, not by adding git logic to the connector crate.
- **`GraphCommit.graph_version` feeds the commit hash** (`versioned_graph.rs`,
  confirmed). This is exactly why git is still needed: graph-version commits are not
  git-replayable. Keep the two stores separate; do not try to unify the histories.
- **Convergence is optional, not blocking.** The `DiskObjectStore`-backed git ODB is
  attractive (one content store, dedup across code and documents) but is a real ODB
  implementation. Do not let it gate a working push/pull loop; ship day-one with a
  normal `.git`.
