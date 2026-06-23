# W3: Execution bridge (materialize-to-run)

The toolchain runs the code. The embedded engine materializes the relevant subtree
into a real OS directory, runs the real toolchain (`cargo`, `python`, `node`) as a
real process in OS-level isolation, and syncs the toolchain's edits back into the
`DocTree`. This is the layer-6 owner and the "run" half of the DocTree-primary,
materialize-to-run decision.

Dependency edges: **W0 precedes W3** (materialize-to-run needs a tree to
materialize and the workspace seam to read it). W3 is independent of W1 and W2,
though it converges with W2 at "run, then commit, then push."

## Implementation status

Partial green in `apps/rustyred-workspace`:

- `materialize_workspace(engine, prefix, dir)` projects DocTree files to a real OS
  directory and strips the engine prefix.
- `run_tool(plan)` runs a real local process with timeout and deny-by-default env
  inheritance; sensitive env keys are stripped even when supplied explicitly.
- `sync_back_sources(engine, dir, prefix)` batches source files back into the
  DocTree and skips `.git`, hidden paths, `target/`, `node_modules`, `dist`,
  `build`, `coverage`, and binary files.
- `sync_back_sources_indexed(engine, dir, prefix, code_index)` is the W3 -> W1
  bridge: it syncs only changed source files and feeds indexable changed bytes
  into CodeCrawler's source-file write path.
- `LocalProcessSandbox` now implements the receiver's existing `SandboxRuntime`
  contract as the dev/no-sidecar backend: `provision`, `put_files`, `run`,
  `get_files`, and `destroy` over a temporary worktree with deny-by-default env
  inheritance.
- `run_workspace_in_sandbox(engine, prefix, runtime, request, plan)` is the W3
  bridge from DocTree to `SandboxRuntime`: upload source files with `put_files`,
  run a `ProofPlan` in the sandbox worktree, fetch source files with `get_files`,
  and batch-write them back through the W0 seam.
- `SandboxRuntime::run_streaming`, `SandboxCancelToken`, and
  `SandboxStreamEvent` add callback-visible stdout/stderr events plus cooperative
  cancellation on the same backend trait. The default implementation preserves
  existing backends by wrapping `run`; `LocalProcessSandbox` overrides it with a
  live process-pipe reader, timeout handling, and cancellation kill path.
- `run_workspace_in_sandbox_streaming(engine, prefix, runtime, request, plan,
  cancel, on_event)` bridges that streaming/cancellation path through W3: source
  files upload to the sandbox, output events reach the caller while the command is
  running, cancellation returns a `ProofReceipt { status: "cancelled" }`, and
  source files still sync back through the W0 batch path after the run ends.
- `theorem-receiver` now carries ignored live OpenSandbox smoke tests gated on
  `OPEN_SANDBOX_BASE_URL`; `OPEN_SANDBOX_API_KEY` is optional so unauthenticated
  local sidecars can run them too. `live_open_sandbox_round_trips_files_and_receipt_shape`
  provisions, uploads a source file, runs a command, downloads the changed source,
  verifies the sandbox trust tier, and destroys the sandbox.
  `live_open_sandbox_streaming_can_cancel_running_command` provisions the same
  live sidecar path, streams stdout from a long-running command, cancels from the
  callback, checks the cancelled receipt and exit event, and destroys the sandbox.
- `OpenSandboxRuntime::run_streaming` now consumes execd `data:` event streams
  directly, emits stdout/stderr callbacks as events arrive, and can return a
  cancelled sandbox-tier receipt when the callback trips `SandboxCancelToken`.
  This is mock-proven against an HTTP execd stream; the real sidecar live run is
  now an exact ignored oracle gated on `OPEN_SANDBOX_BASE_URL`.

Validation: a test imports a tiny Cargo project via W0, materializes it, runs
`cargo build`, asserts `target/` exists only on disk and never in DocTree, then
syncs source-like output back. A second test runs a source rewrite, proves a
sensitive env var is absent in the child process, and syncs the changed source
back into the engine. A third test runs the same DocTree source-rewrite shape
through `SandboxRuntime`, proving `put_files`/`run`/`get_files` staging and
artifact exclusion through the bridge. Receiver tests separately prove the
`LocalProcessSandbox` backend strips `ANTHROPIC_API_KEY`, preserves the shared
`ProofReceipt` shape, streams stdout before cancellation, reports timeout exit
events, and cleans up its worktree.
`indexed_sync_back_updates_code_graph_for_changed_source` proves a materialized
workspace edit syncs back through W3 and updates CodeCrawler's visible callee
graph via W1. `sandbox_streaming_bridge_cancels_and_syncs_changed_source` proves
the workspace-level streaming bridge can cancel from a stdout callback and still
sync the source edit made before cancellation back into the DocTree.
`open_sandbox_runtime_streams_execd_response_and_cancels` proves the OpenSandbox
backend no longer falls back to post-hoc output emission: it streams an execd
response, cancels from stdout, and returns the sandbox trust tier.

Still open: running the ignored `OpenSandboxRuntime` smokes against a real
sidecar, including the sidecar's actual behavior for long-running
streaming/cancellation. The local-process backend and the OpenSandbox HTTP stream
parser now prove the trait and client-side bridge behavior; the live sidecar
oracles compile and are waiting on endpoint configuration.

## Thesis

An application-level filesystem cannot execute code: compilers, interpreters, and
runtimes read the real OS filesystem and spawn real processes through syscalls. The
fix is the standard cloud-dev pattern (durable workspace store plus ephemeral
execution sandbox plus sync/mount), not becoming an OS. Execution as real processes
already exists in the system (the receiver spawns `claude -p` / `codex exec`); W3 is
not a new capability to invent, only a new thing to point at the working tree, plus
the materialize and sync-back steps that do not exist yet.

## What already exists (reuse, do not rebuild)

- **Real-process execution with capture and timeout.** `run_proof(plan: &ProofPlan)
  -> ProofReceipt` at
  `rustyredcore_THG/crates/theorem-receiver/src/local_exec.rs:87`: spawns a command
  with file-based stdout/stderr capture (avoids pipe deadlock), polls with a deadline,
  kills on timeout, returns a `ProofReceipt` (`:61`) tagged with a trust tier.
  `ProofPlan` (`:35`) is `{ command, args, cwd, timeout }`. This is the execution
  primitive; W3 generalizes it from "proof/test command" to "any toolchain command."
- **An isolation backend already exists.** `SandboxRuntime` trait at
  `rustyredcore_THG/crates/theorem-receiver/src/sandbox_exec.rs:59`:
  `provision(request) -> SandboxHandle`, `run(handle, plan) -> ProofReceipt`,
  `put_files` / `get_files`, `destroy`. `OpenSandboxRuntime` (`:72`) is the first
  backend (an HTTP sidecar). The receipt shape is backend-independent
  (`proof_receipt_from_execd_body`), so local and sandbox runs return the same
  `ProofReceipt`.
- **Spawn plans strip secrets.** `SpawnPlan` (`spawn.rs:29`) + `command_from_plan`
  (`:49`) already `env_remove` stripped vars (`ANTHROPIC_API_KEY` for head spawn).
  The pattern W3 extends for toolchain env policy.
- **Worktree mapping as a fence.** `worktree_for` (`config.rs:362`) maps a repo to a
  local path and is a security fence (a job for an unmapped repo is never claimed).

## The gaps W3 closes

- **No materialization layer.** `SandboxRuntime::put_files` exists but is not wired to
  materialize a codebase from the `DocTree` before exec. W3 adds the
  materialize-from-DocTree and sync-back-into-DocTree steps.
- **`ProofPlan` is verification-specific.** It runs test/proof commands, not arbitrary
  toolchain commands, and captures one command's output, not build artifacts. W3 adds
  a generic `RunPlan` (or generalizes `ProofPlan`) plus artifact handling.
- **No source/artifact boundary at run time.** A build writes thousands of transient
  files (`target/`, `node_modules/`); re-indexing them is fatal to the loop. W3
  enforces the boundary: only source syncs back into the `DocTree`; build output stays
  on throwaway disk.

## What to build

In a new module of `apps/rustyred-workspace` (the W0 crate), or a sibling crate
path-depping into `theorem-receiver` and `rustyred-embedded`:

### W3.1: materialize-from-DocTree

- `materialize(engine: &Engine, scope: &PathSpec, dir: &Path) -> MaterializeReceipt`:
  walks the `DocTree` subtree (via the W0 `list_paths` seam + `fs_read`) and writes
  real files into `dir`. For the URL-import case this is a no-op over the already
  cloned tree; for the DocTree-primary case it is the projection that gives the
  toolchain real files.

### W3.2: run the toolchain

- Generalize the execution primitive to a `RunPlan { command, args, cwd, timeout,
  env_policy }` and reuse `run_proof`'s capture/timeout/kill mechanics (or call it
  directly for the local backend). The driving command is caller-supplied (`cargo
  build`, `pytest`, `npm test`), mirroring how the receiver already takes the head
  command as data.
- **Isolation choice (resolved):** reuse the `SandboxRuntime` trait, do not invent a
  parallel one. The first backend is the existing `OpenSandboxRuntime` HTTP sidecar
  (keeps the engine Servo-free and toolchain-free); a `LocalProcessSandbox` backend
  (a thin `run_proof` over a temp dir with env stripping) is the dev/no-sidecar path.
  In-process and RunPod variants swap behind the same trait without touching W3's
  bridge. This is the north star's open "where does isolation come from" answered:
  the trait already abstracts it; W3 ships two backends behind it.

### W3.3: sync-back into the DocTree

- After a run that edits source (the agent's tool ran `cargo fix`, or a formatter, or
  the agent itself wrote files), diff the materialized dir against the `DocTree` and
  write changed **source** files back via the W0 batch path, which fires the W1
  on-write maintenance. Excluded paths (`target/`, `node_modules/`, `.git/`) never
  sync back and are never indexed.

### W3.4: env and credential policy for toolchains

- Extend the env policy beyond `ANTHROPIC_API_KEY`: a toolchain may need
  `CARGO_NET_GIT_FETCH_WITH_CLI`, an `NPM_TOKEN`, etc. Make the allow/deny list
  per-run config, defaulting to a minimal allowlist (the receiver's secret-stripping
  is the safe default; recipe-based execution currently does not strip env, a
  pre-existing risk W3 must not inherit).

## Acceptance criteria

1. A Rust test imports a tiny buildable fixture (a hello-world `cargo` crate) via W0,
   materializes it to a temp dir, runs `cargo build` through the bridge, and asserts a
   `ProofReceipt` with exit code 0 and the build's stdout captured. Real toolchain,
   real process, real exit code.
2. The source/artifact boundary holds: after the build, `target/` exists on disk but
   no `target/` path is in the `DocTree` and no `File` node was created for a build
   artifact (re-indexing artifacts would make builds unusable; this proves it does
   not happen).
3. Sync-back works: a run that rewrites a source file (e.g. `cargo fmt`) is detected,
   the changed source file syncs back into the `DocTree`, and the W1 on-write hook
   fires so the code graph reflects the edit. The agent never called "reindex."
4. The same `RunPlan` runs identically against both backends: a test runs it through
   `LocalProcessSandbox` and (gated, `#[ignore]` for a live sidecar) through
   `OpenSandboxRuntime`, returning the same `ProofReceipt` shape. Isolation is a
   backend choice, not a rewrite.
5. Env policy strips secrets by default: a test asserts a sensitive env var is not
   present in the child process unless explicitly allowlisted for the run.
6. `cargo test -p` (the bridge crate) green; changed files clippy-clean; the bridge
   pulls in no server framework into the embedded minimal-binary path.

## Divergences and risks to surface (not bury)

- **Recipe-based execution does not strip env today.** `runtime_plan_from_recipe`
  (`head.rs:108`) sets `strip_env: Vec::new()` (grounding flagged this). W3 must not
  build on that path without fixing the env policy, or it leaks secrets into the
  toolchain child.
- **`put_files`/`get_files` are unwired.** The sandbox backend has the methods but the
  receiver never calls them to materialize a codebase. W3 is the first caller; expect
  to discover gaps in the sidecar's file-staging contract and surface them.
- **Long builds need live sidecar streaming proof.** The local-process backend now
  streams stdout/stderr and supports cooperative cancellation through
  `SandboxRuntime::run_streaming`, and `OpenSandboxRuntime` now parses execd event
  streams client-side. The remaining proof is to run the ignored live sidecar
  streaming/cancellation smoke against the actual sidecar protocol.
- **Materialize is a copy with a drift surface.** Under DocTree-primary, the
  materialized dir and the `DocTree` can drift if the toolchain edits while the engine
  also writes. W3's sync-back is the reconciliation; W6 (FUSE) removes the copy and
  the drift entirely. Until then, keep the materialize window scoped (one run, one
  sync-back) rather than a long-lived mirror.
- **Isolation strength is a product decision.** The `OpenSandboxRuntime` sidecar's
  actual isolation (container, VM, namespace) is the sidecar's concern, not the
  bridge's. W3 ships the trait and two backends; the deployment picks the sidecar's
  isolation tier. Containerized child processes vs a lighter sandbox is answered by
  "whatever backend the deployment configures," not hardcoded in the bridge.
